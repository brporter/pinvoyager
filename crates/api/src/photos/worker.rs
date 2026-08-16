//! Variant generation, off the request path.
//!
//! CONTRACT — signatures are fixed; implementation owned by the worker agent.
//!
//! Two properties this module exists to provide, both from D8:
//!
//! **Uploads return before variants exist.** A fifty-file batch must not hold a
//! request open for the minutes it takes to encode. Photos land in `pending`
//! and become `ready` when their variants are written.
//!
//! **The queue is bounded.** An unbounded channel turns a burst of uploads into
//! unbounded memory. Backpressure that surfaces as a slower upload is a much
//! better failure than an out-of-memory kill, so a full queue makes the
//! producer wait rather than dropping work or growing.
//!
//! And the rule that is easy to violate without noticing: **no pool connection
//! is held across image processing.** Read what is needed, release, do the CPU
//! work, reacquire to write the result. Holding one for the duration of a batch
//! drains the pool and stalls every unrelated request in the service.

use std::path::Path;
use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::{Semaphore, mpsc};
use uuid::Uuid;

use crate::blocking::run_blocking;
use crate::config::Config;
use crate::db::repo::photos;
use crate::error::{AppError, AppResult};
use crate::photos::storage::{self, ContentHash};
use crate::photos::{ImageFormat, PhotoError, VariantKind, decode, exif, variants};

/// The three `photos.variant_status` values, matching the migration's CHECK
/// constraint. Written out rather than derived so a typo is a compile error at
/// exactly one place instead of a row that silently never becomes viewable.
pub const STATUS_PENDING: &str = "pending";
pub const STATUS_READY: &str = "ready";
pub const STATUS_FAILED: &str = "failed";

/// How many stranded photos one startup sweep will pick up.
///
/// Bounded because the sweep runs before anything is known about how many rows
/// are pending, and an unbounded fetch on a large backlog would be the first
/// query a restarting process issues. If the cap is hit the rest are picked up
/// by the next restart, and the log says so.
const RECOVERY_LIMIT: i64 = 1_000;

/// One photo awaiting variants.
#[derive(Debug, Clone)]
pub struct VariantJob {
    pub photo_id: Uuid,
}

/// Handle held in [`AppState`] so handlers can enqueue.
#[derive(Clone)]
pub struct VariantQueue {
    pub sender: mpsc::Sender<VariantJob>,
}

impl VariantQueue {
    /// Enqueues, applying backpressure when the queue is full.
    ///
    /// Returns an error only when the worker is gone, which means shutdown —
    /// the caller should surface that as a 503 rather than pretending the
    /// upload succeeded.
    pub async fn enqueue(&self, job: VariantJob) -> Result<(), mpsc::error::SendError<VariantJob>> {
        self.sender.send(job).await
    }

    /// A queue with no worker behind it, for tests that exercise the router
    /// rather than the pipeline.
    ///
    /// Enqueueing on it fails, which is the honest behaviour — a test that
    /// accidentally depends on variants being generated should notice, rather
    /// than silently passing against a queue that swallows work.
    pub fn detached() -> Self {
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);
        Self { sender }
    }
}

/// Spawns the worker and returns the queue handle.
///
/// Takes the pool and config rather than [`AppState`] because `AppState` holds
/// the returned handle — the worker has to exist first.
///
/// Concurrency is bounded by `config.photos.variant_concurrency` — separate
/// from queue depth, and the number that actually protects the machine, since
/// image work is CPU-bound and unbounded width starves everything else.
///
/// On startup the worker should also pick up any photos left `pending` by a
/// previous run: a process that died mid-batch would otherwise leave them
/// stranded forever, visible in the library and never becoming viewable.
pub fn spawn(pool: PgPool, config: Arc<Config>) -> VariantQueue {
    // `max(1)` on both: a configured zero would otherwise mean a channel that
    // cannot be created (tokio panics on capacity 0) or a semaphore that never
    // issues a permit, and "the service silently stops generating variants" is
    // a worse answer to a bad config value than "it uses one".
    let depth = config.photos.variant_queue_depth.max(1);
    let concurrency = config.photos.variant_concurrency.max(1);

    let (sender, receiver) = mpsc::channel(depth);

    tokio::spawn(dispatch(
        pool.clone(),
        config,
        receiver,
        Arc::new(Semaphore::new(concurrency)),
    ));

    // Separate task so that a slow or unavailable database at startup delays
    // only the sweep, never the return of this function — `AppState` and the
    // listener are built from it, and a service that will not start because a
    // recovery query is slow is a worse outage than a late sweep.
    tokio::spawn(recover(pool, sender.clone()));

    tracing::info!(queue_depth = depth, concurrency, "variant worker started");

    VariantQueue { sender }
}

/// Pulls jobs off the queue and runs at most `permits` of them at once.
///
/// A permit is taken *before* the receive, so the channel is only drained when
/// there is capacity to act on what comes out of it. Receiving first and then
/// waiting for a permit would move the backlog out of the bounded channel and
/// into an unbounded pile of spawned tasks, which is precisely the unbounded
/// growth the bounded channel exists to prevent.
async fn dispatch(
    pool: PgPool,
    config: Arc<Config>,
    mut receiver: mpsc::Receiver<VariantJob>,
    permits: Arc<Semaphore>,
) {
    loop {
        let Ok(permit) = permits.clone().acquire_owned().await else {
            // The semaphore is only closed at shutdown.
            break;
        };

        let Some(job) = receiver.recv().await else {
            // Every sender is gone: the state holding the queue has been
            // dropped, which means the process is on its way out.
            break;
        };

        let pool = pool.clone();
        let config = config.clone();
        tokio::spawn(async move {
            if let Err(err) = process(&pool, &config, &job).await {
                // `process` has already marked the row `failed`; this line is
                // the *why*, which the row cannot carry.
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

    tracing::info!("variant worker stopped");
}

/// Requeues photos left `pending` by a previous run.
///
/// Without this a process that died mid-batch strands them forever: the row
/// exists, the library shows it, and nothing will ever generate its variants
/// because the only thing that ever enqueued it was the upload request that has
/// long since returned.
async fn recover(pool: PgPool, sender: mpsc::Sender<VariantJob>) {
    let pending = match photos::pending(&pool, RECOVERY_LIMIT).await {
        Ok(pending) => pending,
        Err(err) => {
            // Not fatal. The sweep is a repair, and the next restart runs it
            // again; refusing to serve because it failed would turn a stranded
            // photo into a stranded service.
            tracing::error!(error = %err, "could not sweep photos left pending");
            return;
        }
    };

    if pending.is_empty() {
        return;
    }

    tracing::info!(
        count = pending.len(),
        "requeuing photos left pending by a previous run"
    );
    if pending.len() as i64 >= RECOVERY_LIMIT {
        tracing::warn!(
            limit = RECOVERY_LIMIT,
            "pending sweep hit its limit; the remainder is picked up by the next restart"
        );
    }

    for photo_id in pending {
        // Awaits when the queue is full, which is the same backpressure an
        // upload gets. A failure here means the worker is gone.
        if sender.send(VariantJob { photo_id }).await.is_err() {
            break;
        }
    }
}

/// Processes one photo: decode the original, derive every variant, write them,
/// and mark the row `ready`.
///
/// A failure marks the row `failed` rather than retrying forever — a file that
/// cannot be decoded will not decode on the tenth attempt either, and an
/// infinite retry loop on a poisoned job is how a queue stops draining.
pub async fn process(
    pool: &PgPool,
    config: &Config,
    job: &VariantJob,
) -> Result<(), crate::error::AppError> {
    let Err(err) = generate(pool, config, job).await else {
        return Ok(());
    };

    // Deliberately a separate statement rather than part of a transaction: the
    // work that failed is on the filesystem, not in the database, so there is
    // nothing to roll back — and if *this* write also fails (the database is
    // down, which is a plausible cause of the failure above) the row stays
    // `pending` and the next startup sweep retries it. That is the right
    // direction to fail in: a transient outage should not permanently poison a
    // photo, and a genuinely undecodable file fails again immediately.
    if let Err(mark) = photos::set_variant_status(pool, job.photo_id, STATUS_FAILED).await {
        tracing::error!(
            photo_id = %job.photo_id,
            error = %mark,
            "could not mark photo failed; it stays pending for the next sweep"
        );
    }

    Err(err)
}

/// The fallible core of [`process`], split out so that every failure path goes
/// through the same `failed` marking.
///
/// **This is where D8's connection rule is kept.** Each `photos::` call takes
/// `&PgPool`, which acquires a connection for that one statement and returns it
/// to the pool when the `await` completes. Between the read and the write there
/// is no connection held, no transaction open, and no guard alive — the CPU
/// phase in the middle can take seconds without costing the rest of the service
/// anything.
async fn generate(pool: &PgPool, config: &Config, job: &VariantJob) -> AppResult<()> {
    // ---- read: one statement, connection released before it returns --------
    let Some(photo) = photos::find_by_id(pool, job.photo_id).await? else {
        // Deleted between enqueue and processing. Not an error: the files are
        // already gone, and marking a row that no longer exists is a no-op.
        tracing::debug!(photo_id = %job.photo_id, "photo deleted before variants were generated");
        return Ok(());
    };

    if photo.variant_status == STATUS_READY {
        // The startup sweep and a live enqueue can both name the same photo.
        // Re-encoding it would be correct but wasteful.
        return Ok(());
    }

    let format = parse_format(&photo.original_format)?;
    let hash = ContentHash(photo.content_hash);
    let root = config.photos.storage_root.clone();
    let quality = config.photos.webp_quality;

    // ---- CPU: decode, resize, encode, write. No connection, no runtime
    //      worker thread (D8). One `run_blocking` around the whole sequence
    //      rather than one per file operation, per the note in `blocking`.
    run_blocking(move || derive(&root, &hash, format, quality)).await?;

    // ---- write: reacquire, one statement, released again -------------------
    photos::set_variant_status(pool, job.photo_id, STATUS_READY).await?;

    tracing::debug!(photo_id = %job.photo_id, "variants ready");

    Ok(())
}

/// Decode once, encode every size, write each to its content-addressed path.
///
/// Blocking and CPU-bound from top to bottom; the only caller runs it under
/// [`run_blocking`].
///
/// The original is decoded exactly once and the pixel buffer reused for all
/// three variants — decode is the expensive half, and doing it per size would
/// triple the cost of the phase's single most CPU-heavy operation.
fn derive(root: &Path, hash: &ContentHash, format: ImageFormat, quality: f32) -> AppResult<()> {
    let original = storage::original_path(root, hash);

    // Read from the *original*: variants are stripped by design (D6), so
    // reading orientation off one would always yield nothing. Absent EXIF is
    // ordinary, and orientation 1 is "as stored".
    let orientation = exif::extract(&original).orientation.unwrap_or(1);

    let image = decode::decode(&original, format).map_err(internal)?;
    // Baked into the pixels here, because the variant carries no tag for a
    // viewer to honour later. Skipping this is how every portrait photo in the
    // product ends up sideways.
    let image = decode::apply_orientation(image, orientation);

    for kind in VariantKind::ALL {
        let bytes = variants::encode_variant(&image, kind, quality).map_err(internal)?;
        write_atomic(&storage::variant_path(root, hash, kind), &bytes)?;
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

/// Resolves the stored format string back to a decoder choice.
///
/// Driven off [`ImageFormat::as_str`] rather than a second `match`, so this
/// cannot drift from the values the column's CHECK constraint permits.
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
mod tests {
    use std::time::Duration;

    use super::*;

    /// The mapping the worker uses to pick a decoder. A value the column can
    /// hold but this cannot resolve would make every photo of that format fail.
    #[test]
    fn every_stored_format_resolves() {
        for format in [
            ImageFormat::Jpeg,
            ImageFormat::Png,
            ImageFormat::WebP,
            ImageFormat::Heic,
        ] {
            let parsed = parse_format(format.as_str()).expect("round trips");
            assert_eq!(parsed, format);
        }
    }

    #[test]
    fn an_unknown_format_is_refused_rather_than_guessed() {
        assert!(parse_format("gif").is_err());
        assert!(parse_format("").is_err());
    }

    /// The statuses have to match the migration's CHECK constraint exactly; a
    /// mismatch is a constraint violation on a row nobody is watching.
    #[test]
    fn statuses_match_the_schema() {
        assert_eq!(STATUS_PENDING, "pending");
        assert_eq!(STATUS_READY, "ready");
        assert_eq!(STATUS_FAILED, "failed");
    }

    /// T7: a full queue makes the producer wait. It does not grow, and it does
    /// not drop the job on the floor.
    #[tokio::test]
    async fn a_full_queue_applies_backpressure() {
        let (sender, mut receiver) = mpsc::channel(1);
        let queue = VariantQueue { sender };

        queue
            .enqueue(VariantJob {
                photo_id: Uuid::now_v7(),
            })
            .await
            .expect("first job fits");

        // The second send cannot complete until something is taken off.
        let blocked = tokio::time::timeout(
            Duration::from_millis(100),
            queue.enqueue(VariantJob {
                photo_id: Uuid::now_v7(),
            }),
        )
        .await;
        assert!(blocked.is_err(), "a full queue accepted a second job");

        // ...and it completes once there is room, rather than having failed.
        receiver.recv().await.expect("first job is still there");
        tokio::time::timeout(
            Duration::from_secs(5),
            queue.enqueue(VariantJob {
                photo_id: Uuid::now_v7(),
            }),
        )
        .await
        .expect("space became available")
        .expect("the worker is still there");
    }

    /// A detached queue refuses work rather than swallowing it, so a test that
    /// accidentally depends on variant generation fails loudly.
    #[tokio::test]
    async fn a_detached_queue_refuses_work() {
        let queue = VariantQueue::detached();
        assert!(
            queue
                .enqueue(VariantJob {
                    photo_id: Uuid::now_v7(),
                })
                .await
                .is_err()
        );
    }
}
