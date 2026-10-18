//! Generates variants after upload acceptance using a bounded queue.
//!
//! Photos are saved as pending and become ready after all variants are written.
//! Uploads wait for queue capacity. Decoding and encoding use the blocking pool
//! without a database connection; final writes hold the content lock.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::{Semaphore, mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::blocking::run_blocking;
use crate::config::Config;
use crate::db::repo::photos;
use crate::error::{AppError, AppResult};
use crate::photos::storage::{self, ContentHash};
use crate::photos::{ImageFormat, PhotoError, VariantKind, decode, exif, variants};

/// Stored processing status names: pending, ready, and failed.
pub const STATUS_PENDING: &str = "pending";
pub const STATUS_READY: &str = "ready";
pub const STATUS_FAILED: &str = "failed";

/// Page size for each periodic pending-photo scan. Every page is visited.
const RECOVERY_LIMIT: i64 = 1_000;

/// One photo awaiting variants.
#[derive(Debug, Clone)]
pub struct VariantJob {
    pub photo_id: Uuid,
    context: opentelemetry::Context,
    voyage_id: Option<Uuid>,
    queue_wait: tracing::Span,
}

impl VariantJob {
    /// Keep only OpenTelemetry context: retaining the request's tracing span
    /// would incorrectly extend its duration until background work finishes.
    pub fn new(photo_id: Uuid) -> Self {
        let context = tracing::Span::current().context();
        let queue_wait = tracing::info_span!(parent: None, "photo.queue_wait",
            photo.id = %photo_id, voyage.id = tracing::field::Empty);
        let _ = queue_wait.set_parent(context.clone());
        Self {
            photo_id,
            context,
            voyage_id: None,
            queue_wait,
        }
    }

    pub fn with_voyage(mut self, voyage_id: Uuid) -> Self {
        self.queue_wait.record("voyage.id", voyage_id.to_string());
        self.voyage_id = Some(voyage_id);
        self
    }
}

/// Handle held in [`crate::state::AppState`] so handlers can enqueue.
#[derive(Clone)]
pub struct VariantQueue {
    sender: mpsc::Sender<VariantJob>,
    lifecycle: Option<Arc<Lifecycle>>,
}

struct Lifecycle {
    stop: watch::Sender<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl VariantQueue {
    /// Wrap an explicitly owned channel, used by embedded router tests.
    pub fn from_sender(sender: mpsc::Sender<VariantJob>) -> Self {
        Self {
            sender,
            lifecycle: None,
        }
    }

    /// Stop maintenance and accept no further jobs, then drain accepted work.
    pub async fn shutdown(&self, deadline: Duration) {
        if let Some(lifecycle) = &self.lifecycle {
            let _ = lifecycle.stop.send(true);
            let task = lifecycle
                .task
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(mut task) = task
                && tokio::time::timeout(deadline, &mut task).await.is_err()
            {
                task.abort();
                let _ = task.await;
                tracing::warn!("worker drain deadline exceeded; pending rows will recover");
            }
        }
    }

    /// Waits for queue capacity and enqueues a job. Returns an error if the
    /// receiver has closed.
    #[tracing::instrument(name = "photo.enqueue", skip_all, fields(photo.id = %job.photo_id), err)]
    pub async fn enqueue(&self, job: VariantJob) -> Result<(), mpsc::error::SendError<VariantJob>> {
        self.sender.send(job).await
    }

    /// Reserve acceptance before committing the photo row. Dropping an unused
    /// permit returns capacity; sending with it is synchronous.
    #[tracing::instrument(name = "photo.enqueue", skip_all, err)]
    pub async fn reserve(
        &self,
    ) -> Result<mpsc::OwnedPermit<VariantJob>, mpsc::error::SendError<()>> {
        self.sender.clone().reserve_owned().await
    }

    /// Returns a queue with no receiver for tests. Enqueue and reservation fail.
    pub fn detached() -> Self {
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);
        Self::from_sender(sender)
    }
}

/// Starts conversion dispatch and periodic pending-photo recovery.
/// Conversion concurrency is limited by `variant_concurrency`. Returns the
/// queue handle used to enqueue jobs and shut down the worker.
pub fn spawn(pool: PgPool, config: Arc<Config>) -> VariantQueue {
    let (sender, receiver) = mpsc::channel(config.photos.variant_queue_depth.max(1));
    let permits = Arc::new(Semaphore::new(config.photos.variant_concurrency.max(1)));
    let (stop, stopped) = watch::channel(false);
    let lifecycle = Arc::new(Lifecycle {
        stop,
        task: Mutex::new(None),
    });
    let maintenance_pool = pool.clone();
    let maintenance_config = config.clone();
    let maintenance_sender = sender.clone();
    let mut maintenance_stop = stopped.clone();
    let task = tokio::spawn(async move {
        // Owning both tasks in a JoinSet ensures cancellation stops maintenance
        // as well as dispatch; conversion futures are owned by dispatch below.
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = maintenance_stop.changed() => break,
                    _ = interval.tick() => {
                        tokio::select! {
                            _ = maintenance_stop.changed() => break,
                            _ = maintain(&maintenance_pool, &maintenance_config, &maintenance_sender) => {}
                        }
                    }
                }
            }
        });
        tasks.spawn(dispatch_owned(pool, config, receiver, permits, stopped));
        while tasks.join_next().await.is_some() {}
    });
    *lifecycle
        .task
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(task);
    VariantQueue {
        sender,
        lifecycle: Some(lifecycle),
    }
}

async fn maintain(pool: &PgPool, config: &Config, sender: &mpsc::Sender<VariantJob>) {
    recover(pool.clone(), sender.clone()).await;
    if let Err(error) = crate::auth::session::sweep_expired(pool).await {
        tracing::warn!(%error, "session cleanup will be retried");
    }
    if let Err(error) = crate::auth::attempt::sweep_expired(pool).await {
        tracing::warn!(%error, "authorization cleanup will be retried");
    }
    if let Err(error) = super::cleanup::sweep(pool, &config.photos.storage_root).await {
        tracing::warn!(%error, "file cleanup will be retried");
    }
}

#[cfg(test)]
async fn dispatch(
    pool: PgPool,
    config: Arc<Config>,
    receiver: mpsc::Receiver<VariantJob>,
    permits: Arc<Semaphore>,
) {
    let (_stop, stopped) = watch::channel(false);
    dispatch_owned(pool, config, receiver, permits, stopped).await;
}

/// Receives queued jobs only after acquiring a concurrency permit.
/// Holds each permit until that job finishes, bounding active conversion tasks.
async fn dispatch_owned(
    pool: PgPool,
    config: Arc<Config>,
    mut receiver: mpsc::Receiver<VariantJob>,
    permits: Arc<Semaphore>,
    mut stopped: watch::Receiver<bool>,
) {
    let mut jobs = JoinSet::new();
    let mut draining = false;
    loop {
        while jobs.try_join_next().is_some() {}
        let Ok(permit) = permits.clone().acquire_owned().await else {
            // The semaphore is only closed at shutdown.
            break;
        };

        let next = tokio::select! {
            _ = stopped.changed(), if !draining => {
                receiver.close();
                draining = true;
                receiver.recv().await
            },
            job = receiver.recv() => job,
        };
        let Some(mut job) = next else {
            // Every sender is gone: the state holding the queue has been
            // dropped, which means the process is on its way out.
            break;
        };

        // Queue wait ends when received, before scheduling actual conversion.
        job.queue_wait = tracing::Span::none();
        let pool = pool.clone();
        let config = config.clone();
        jobs.spawn(async move {
            if let Err(err) = process(&pool, &config, &job).await {
                // `process` attempts to mark pending rows failed; log the cause
                // even when a duplicate job has already made the photo ready.
                tracing::error!(
                    photo_id = %job.photo_id,
                    error = %err,
                    "variant generation failed"
                );
            }
            // Held until the work is done, not until it is scheduled.
            drop(permit);
        });
    }

    while jobs.join_next().await.is_some() {}
    tracing::info!("variant worker stopped");
}

/// Scans pending photos and requeues them in pages, including uploads
/// interrupted by an earlier process exit.
#[tracing::instrument(name = "photo.recover", skip_all)]
async fn recover(pool: PgPool, sender: mpsc::Sender<VariantJob>) {
    let mut after = None;
    loop {
        let pending: Vec<Uuid> = match sqlx::query_scalar("SELECT id FROM photos WHERE variant_status='pending' AND ($1::uuid IS NULL OR id > $1) ORDER BY id LIMIT $2")
            .bind(after).bind(RECOVERY_LIMIT).fetch_all(&pool).await {
            Ok(pending) => pending,
            Err(error) => { tracing::warn!(%error, "pending scan will retry on the next maintenance pass"); return; }
        };
        let count = pending.len();
        for id in pending {
            after = Some(id);
            if sender.send(VariantJob::new(id)).await.is_err() {
                return;
            }
        }
        if count < RECOVERY_LIMIT as usize {
            return;
        }
    }
}

/// Processes one photo and marks it ready after all variants are written.
/// On failure, attempts to mark a still-pending row failed and returns the original
/// error. Ready rows remain ready. If marking fails, pending rows remain recoverable.
pub async fn process(
    pool: &PgPool,
    config: &Config,
    job: &VariantJob,
) -> Result<(), crate::error::AppError> {
    let span = tracing::info_span!(parent: None, "photo.process",
        otel.name = %format!("photo.process {}", job.photo_id),
        photo.id = %job.photo_id,
        voyage.id = tracing::field::Empty,
        photo.format = tracing::field::Empty,
        photo.width = tracing::field::Empty,
        photo.height = tracing::field::Empty,
        photo.status = tracing::field::Empty,
        otel.status_code = tracing::field::Empty);
    let _ = span.set_parent(job.context.clone());
    async {
        let Err(err) = generate(pool, config, job, &span).await else {
            return Ok(());
        };
        // Failure marking remains a separate statement, not a transaction.
        // If it fails, the row stays pending for the next startup recovery.
        span.record("photo.status", STATUS_FAILED);
        span.record("otel.status_code", "ERROR");
        if let Err(mark) = photos::set_variant_status(pool, job.photo_id, STATUS_FAILED)
            .instrument(tracing::info_span!("photo.mark_failed"))
            .await
        {
            tracing::error!(photo_id = %job.photo_id, error = %mark,
                "could not mark photo failed; pending rows remain eligible for recovery");
        }
        Err(err)
    }
    .instrument(span.clone())
    .await
}

/// Loads the photo, decodes and encodes without a database connection, then
/// locks its content and rechecks the row before writing variants. Missing or
/// already-processed photos are skipped. Read, conversion, and write failures
/// propagate to `process` for failure marking.
#[tracing::instrument(name = "photo.generate", skip_all, fields(photo.id = %job.photo_id))]
async fn generate(
    pool: &PgPool,
    config: &Config,
    job: &VariantJob,
    processing: &tracing::Span,
) -> AppResult<()> {
    // ---- read: one statement, connection released before it returns --------
    let Some(photo) = photos::find_by_id(pool, job.photo_id)
        .instrument(tracing::info_span!("photo.load"))
        .await?
    else {
        // Deleted between enqueue and processing. Not an error: the files are
        // already gone, and marking a row that no longer exists is a no-op.
        tracing::debug!(photo_id = %job.photo_id, "photo deleted before variants were generated");
        return Ok(());
    };

    if let Some(voyage_id) = job.voyage_id {
        processing.record("voyage.id", voyage_id.to_string());
    }
    processing.record("photo.format", &photo.original_format);
    processing.record("photo.width", photo.width);
    processing.record("photo.height", photo.height);
    if photo.variant_status == STATUS_READY {
        // The startup sweep and a live enqueue can both name the same photo.
        // Re-encoding it would be correct but wasteful.
        return Ok(());
    }

    let format = parse_format(&photo.original_format)?;
    let hash = ContentHash::try_from(photo.content_hash)?;
    let root = config.photos.storage_root.clone();
    let encode_hash = hash.clone();
    let quality = config.photos.webp_quality;

    // Decode and encode on the blocking pool without a database connection.
    let encoded = run_blocking(move || encode(&root, &encode_hash, format, quality)).await?;

    // Encoding holds no database connection. Only final filesystem writes are
    // serialized with upload/deletion. A deletion during conversion wins: the
    // worker must not recreate variants for a row that no longer exists.
    let mut tx = pool.begin().await?;
    photos::lock_content(&mut tx, hash.as_str()).await?;
    if photos::find_by_id(&mut *tx, job.photo_id).await?.is_none() {
        return Ok(());
    }
    let root = config.photos.storage_root.clone();
    // The blocking task owns the transaction while writing. Canceling this
    // future cannot release the content lock before those writes have stopped.
    let mut tx = run_blocking(move || {
        write_variants(&root, &hash, encoded)?;
        Ok(tx)
    })
    .await?;

    // ---- write: reacquire, one statement, released again -------------------
    photos::set_variant_status(&mut *tx, job.photo_id, STATUS_READY)
        .instrument(tracing::info_span!("photo.mark_ready"))
        .await?;
    tx.commit().await?;
    processing.record("photo.status", STATUS_READY);

    tracing::debug!(photo_id = %job.photo_id, "variants ready");

    Ok(())
}

/// Test helper that decodes once, encodes every variant size, and writes all
/// variants to content-addressed paths. Returns conversion or filesystem errors.
#[cfg(test)]
fn derive(root: &Path, hash: &ContentHash, format: ImageFormat, quality: f32) -> AppResult<()> {
    write_variants(root, hash, encode(root, hash, format, quality)?)
}

fn encode(
    root: &Path,
    hash: &ContentHash,
    format: ImageFormat,
    quality: f32,
) -> AppResult<Vec<(VariantKind, Vec<u8>)>> {
    let original = storage::original_path(root, hash);

    // Read from the *original*: variants are stripped by design (D6), so
    // reading orientation off one would always yield nothing. Absent EXIF is
    // ordinary, and orientation 1 is "as stored".
    let orientation = tracing::info_span!("photo.exif")
        .in_scope(|| exif::extract(&original).orientation.unwrap_or(1));

    let image = tracing::info_span!("photo.decode", photo.format = format.as_str())
        .in_scope(|| decode::decode(&original, format))
        .map_err(internal)?;
    // Baked into the pixels here, because the variant carries no tag for a
    // viewer to honor later. Skipping this is how every portrait photo in the
    // product ends up sideways.
    let image = tracing::info_span!("photo.orient", photo.orientation = orientation)
        .in_scope(|| decode::apply_orientation(image, orientation));

    VariantKind::ALL
        .into_iter()
        .map(|kind| {
            let span = tracing::info_span!(
                "photo.variant",
                photo.variant = kind.as_str(),
                photo.max_edge = kind.max_edge()
            );
            span.in_scope(|| {
                variants::encode_variant(&image, kind, quality)
                    .map(|bytes| (kind, bytes))
                    .map_err(internal)
            })
        })
        .collect()
}

fn write_variants(
    root: &Path,
    hash: &ContentHash,
    encoded: Vec<(VariantKind, Vec<u8>)>,
) -> AppResult<()> {
    for (kind, bytes) in encoded {
        tracing::info_span!(
            "photo.write",
            photo.variant = kind.as_str(),
            photo.bytes = bytes.len()
        )
        .in_scope(|| write_atomic(&storage::variant_path(root, hash, kind), &bytes))?;
    }
    Ok(())
}

/// Writes via a temporary file in the same directory, then renames.
///
/// The rename is atomic within a filesystem, so a reader never sees a partially
/// written variant. That matters because the row flips to `ready` only after
/// every variant is written, but a crash between two writes leaves the row
/// `pending` and the startup sweep re-derives it — and the sweep must overwrite
/// a truncated file rather than trip over it.
fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let parent = path.parent().ok_or_else(|| {
        AppError::Internal(format!("variant path {} has no parent", path.display()).into())
    })?;
    std::fs::create_dir_all(parent).map_err(storage_failed)?;

    // A unique temporary name, so two workers deriving the same content
    // concurrently (two users uploading identical bytes) cannot interleave
    // writes into one another's file.
    let temp = path.with_extension(format!("{}.tmp", Uuid::now_v7()));

    let write = std::fs::write(&temp, bytes).and_then(|()| std::fs::rename(&temp, path));
    if write.is_err() {
        let _ = std::fs::remove_file(&temp);
    }

    write.map_err(storage_failed)
}

/// Parses the stored format using the accepted format names.
/// Returns an internal error if the stored string is unrecognized.
fn parse_format(value: &str) -> AppResult<ImageFormat> {
    [
        ImageFormat::Jpeg,
        ImageFormat::Png,
        ImageFormat::WebP,
        ImageFormat::Heic,
    ]
    .into_iter()
    .find(|format| format.as_str() == value)
    .ok_or_else(|| AppError::Internal(format!("unknown stored image format {value:?}").into()))
}

/// Every failure in here is ours, not a caller's: there is no request in flight
/// and nobody to hand a 400 to. The row becomes `failed` and the log carries the
/// cause.
fn internal(err: PhotoError) -> AppError {
    AppError::Internal(Box::new(err))
}

fn storage_failed(err: std::io::Error) -> AppError {
    AppError::Internal(Box::new(PhotoError::Storage(err)))
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
