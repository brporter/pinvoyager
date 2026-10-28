//! Frozen publication consent, tracked expected uploads, and durable worker finalization.
use crate::{
    AppError, AppResult,
    db::repo::{users, voyages},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sqlx::{PgConnection, PgPool};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const ACTIVE: [&str; 2] = ["waiting_for_uploads", "processing"];
const MAX_PHOTOS: usize = 10_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExistingPhoto {
    pub photo_id: Uuid,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FuturePhoto {
    pub upload_key: Uuid,
    pub content_hash: String,
}
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum PhotoReference {
    Existing(ExistingPhoto),
    Future(FuturePhoto),
}

#[derive(Serialize, Deserialize)]
pub struct Review {
    pub metadata_digest: String,
    pub expires_at: DateTime<Utc>,
    pub photos: Vec<PhotoReference>,
}

#[derive(Serialize)]
pub struct Status {
    pub id: Uuid,
    pub status: String,
    pub expires_at: DateTime<Utc>,
    pub revision: String,
    pub published: bool,
    pub waiting_count: usize,
    pub processing_count: usize,
    pub reason: Option<String>,
}

#[derive(sqlx::FromRow)]
struct Intent {
    id: Uuid,
    owner_id: Uuid,
    voyage_id: Uuid,
    metadata_digest: String,
    photos: Value,
    tracked_revision: i64,
    expires_at: DateTime<Utc>,
    status: String,
    reason: Option<String>,
}

pub fn metadata_digest(
    title: &str,
    description: Option<&str>,
    granularity: &str,
) -> AppResult<String> {
    let bytes =
        serde_json::to_vec(&(title, description, granularity)).map_err(AppError::internal)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

async fn lock_voyage(
    connection: &mut PgConnection,
    owner: Uuid,
    voyage: Uuid,
) -> AppResult<voyages::Voyage> {
    let id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM voyages WHERE id=$1 AND owner_id=$2 FOR UPDATE")
            .bind(voyage)
            .bind(owner)
            .fetch_optional(&mut *connection)
            .await?;
    id.ok_or(AppError::NotFound)?;
    voyages::find_by_id(connection, voyage)
        .await?
        .ok_or(AppError::NotFound)
}

async fn revision(connection: &mut PgConnection, voyage: Uuid) -> AppResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT revision FROM voyages WHERE id=$1")
            .bind(voyage)
            .fetch_one(connection)
            .await?,
    )
}

#[derive(sqlx::FromRow)]
struct Photo {
    id: Uuid,
    variant_status: String,
    content_hash: String,
}
#[derive(sqlx::FromRow)]
struct Receipt {
    upload_key: String,
    state: String,
    photo_id: Option<Uuid>,
    content_hash: Option<String>,
}
struct Resolved {
    order: Vec<Uuid>,
    waiting: usize,
    processing: usize,
    issue: Option<&'static str>,
}

async fn resolve(
    connection: &mut PgConnection,
    owner: Uuid,
    voyage: Uuid,
    references: &[PhotoReference],
) -> AppResult<Resolved> {
    let photos: Vec<Photo> =
        sqlx::query_as("SELECT id,variant_status,content_hash FROM photos WHERE voyage_id=$1")
            .bind(voyage)
            .fetch_all(&mut *connection)
            .await?;
    let photos: HashMap<_, _> = photos.into_iter().map(|photo| (photo.id, photo)).collect();
    let keys: Vec<_> = references
        .iter()
        .filter_map(|reference| {
            if let PhotoReference::Future(photo) = reference {
                Some(format!("{}:0", photo.upload_key))
            } else {
                None
            }
        })
        .collect();
    let receipts: Vec<Receipt>=sqlx::query_as("SELECT upload_key,state,photo_id,content_hash FROM upload_receipts WHERE account_id=$1 AND voyage_id=$2 AND upload_key=ANY($3)")
        .bind(owner).bind(voyage).bind(keys).fetch_all(connection).await?;
    let receipts: HashMap<_, _> = receipts
        .into_iter()
        .map(|receipt| (receipt.upload_key.clone(), receipt))
        .collect();
    let mut result = Resolved {
        order: Vec::new(),
        waiting: 0,
        processing: 0,
        issue: None,
    };
    let mut seen_keys = HashSet::new();
    for reference in references {
        let photo_id = match reference {
            PhotoReference::Existing(photo) => Some(photo.photo_id),
            PhotoReference::Future(photo) => {
                if photo.content_hash.len() != 64
                    || !photo
                        .content_hash
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit())
                    || !seen_keys.insert(photo.upload_key)
                {
                    result.issue = Some("The reviewed upload list is invalid.");
                    return Ok(result);
                }
                match receipts.get(&format!("{}:0", photo.upload_key)) {
                    Some(receipt)
                        if receipt.state == "accepted"
                            && receipt.photo_id.is_some()
                            && receipt.content_hash.as_deref().is_some_and(|hash| {
                                hash.eq_ignore_ascii_case(&photo.content_hash)
                            }) =>
                    {
                        receipt.photo_id
                    }
                    Some(_) => {
                        result.issue = Some("A reviewed upload was changed, canceled or deleted.");
                        return Ok(result);
                    }
                    None => {
                        result.waiting += 1;
                        None
                    }
                }
            }
        };
        if let Some(id) = photo_id {
            let Some(photo) = photos.get(&id) else {
                result.issue = Some("A reviewed photograph was deleted.");
                return Ok(result);
            };
            if let PhotoReference::Future(expected) = reference
                && !photo
                    .content_hash
                    .eq_ignore_ascii_case(&expected.content_hash)
            {
                result.issue = Some("A reviewed photograph has different bytes.");
                return Ok(result);
            }
            if photo.variant_status == "failed" {
                result.issue = Some("A reviewed photograph could not be processed.");
                return Ok(result);
            }
            if photo.variant_status != "ready" {
                result.processing += 1;
            }
            result.order.push(id);
        }
    }
    let selected: HashSet<_> = result.order.iter().copied().collect();
    if selected.len() != result.order.len() || selected != photos.keys().copied().collect() {
        result.issue = Some("The voyage's photo list changed after review.");
    }
    Ok(result)
}

pub async fn register(
    connection: &mut PgConnection,
    owner: Uuid,
    voyage_id: Uuid,
    expected: Option<i64>,
    review: Review,
) -> AppResult<Status> {
    let voyage = lock_voyage(connection, owner, voyage_id).await?;
    ensure_eligible(connection, voyage_id).await?;
    let current = revision(connection, voyage_id).await?;
    if expected.is_some_and(|expected| expected != current) {
        return Err(AppError::PreconditionFailed);
    }
    let named: bool = sqlx::query_scalar("SELECT display_name IS NOT NULL FROM users WHERE id=$1")
        .bind(owner)
        .fetch_one(&mut *connection)
        .await?;
    if !named
        || voyage.published
        || voyage.title.trim().is_empty()
        || voyage
            .description
            .as_deref()
            .is_none_or(|description| description.trim().is_empty())
        || review.photos.is_empty()
        || review.photos.len() > MAX_PHOTOS
    {
        return Err(AppError::BadRequest(
            "Review a private voyage with a name, description and at least one photograph."
                .to_owned(),
        ));
    }
    let active: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM publication_intents WHERE voyage_id=$1 AND status IN ('waiting_for_uploads','processing'))").bind(voyage_id).fetch_one(&mut *connection).await?;
    if active {
        return Err(AppError::Conflict(
            "Cancel the scheduled publication before replacing it.".to_owned(),
        ));
    }
    if metadata_digest(
        &voyage.title,
        voyage.description.as_deref(),
        &voyage.granularity,
    )? != review.metadata_digest
    {
        return Err(AppError::PreconditionFailed);
    }
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT now()")
        .fetch_one(&mut *connection)
        .await?;
    if review.expires_at <= now {
        return Err(AppError::BadRequest(
            "Publication consent expired. Review publication again.".to_owned(),
        ));
    }
    let resolved = resolve(connection, owner, voyage_id, &review.photos).await?;
    if let Some(issue) = resolved.issue {
        return Err(AppError::BadRequest(issue.to_owned()));
    }
    let id = Uuid::now_v7();
    let photos = serde_json::to_value(review.photos).map_err(AppError::internal)?;
    let expires = review.expires_at.min(now + Duration::days(7));
    sqlx::query("INSERT INTO publication_intents(id,owner_id,voyage_id,metadata_digest,photos,tracked_revision,expires_at,status) VALUES($1,$2,$3,$4,$5,$6,$7,'waiting_for_uploads')")
        .bind(id).bind(owner).bind(voyage_id).bind(review.metadata_digest).bind(photos).bind(current).bind(expires).execute(&mut *connection).await?;
    finalize_locked(connection, &voyage).await?;
    status(connection, owner, voyage_id, id).await
}

pub async fn status(
    connection: &mut PgConnection,
    owner: Uuid,
    voyage: Uuid,
    id: Uuid,
) -> AppResult<Status> {
    let intent: Intent=sqlx::query_as("SELECT id,owner_id,voyage_id,metadata_digest,photos,tracked_revision,expires_at,status,reason FROM publication_intents WHERE id=$1 AND owner_id=$2 AND voyage_id=$3")
        .bind(id).bind(owner).bind(voyage).fetch_optional(&mut *connection).await?.ok_or(AppError::NotFound)?;
    let resolved = match serde_json::from_value::<Vec<PhotoReference>>(intent.photos) {
        Ok(references) => resolve(connection, owner, voyage, &references).await?,
        Err(_) => Resolved {
            order: Vec::new(),
            waiting: 0,
            processing: 0,
            issue: Some("The saved publication review could not be read."),
        },
    };
    let (revision, published): (i64, bool) =
        sqlx::query_as("SELECT revision,published FROM voyages WHERE id=$1 AND owner_id=$2")
            .bind(voyage)
            .bind(owner)
            .fetch_optional(connection)
            .await?
            .ok_or(AppError::NotFound)?;
    Ok(Status {
        id: intent.id,
        status: intent.status,
        expires_at: intent.expires_at,
        revision: revision.to_string(),
        published,
        waiting_count: resolved.waiting,
        processing_count: resolved.processing,
        reason: intent.reason,
    })
}

pub async fn cancel(
    connection: &mut PgConnection,
    owner: Uuid,
    voyage: Uuid,
    id: Uuid,
) -> AppResult<Status> {
    lock_voyage(connection, owner, voyage).await?;
    let found: Option<Uuid>=sqlx::query_scalar("UPDATE publication_intents SET status=CASE WHEN status='published' THEN status ELSE 'canceled' END,reason=NULL WHERE id=$1 AND owner_id=$2 AND voyage_id=$3 RETURNING id")
        .bind(id).bind(owner).bind(voyage).fetch_optional(&mut *connection).await?;
    found.ok_or(AppError::NotFound)?;
    status(connection, owner, voyage, id).await
}

async fn set_status(
    connection: &mut PgConnection,
    id: Uuid,
    status: &str,
    reason: Option<&str>,
) -> AppResult<()> {
    sqlx::query("UPDATE publication_intents SET status=$2,reason=$3 WHERE id=$1")
        .bind(id)
        .bind(status)
        .bind(reason)
        .execute(connection)
        .await?;
    Ok(())
}

async fn finalize_locked(connection: &mut PgConnection, voyage: &voyages::Voyage) -> AppResult<()> {
    let intent: Option<Intent>=sqlx::query_as("SELECT id,owner_id,voyage_id,metadata_digest,photos,tracked_revision,expires_at,status,reason FROM publication_intents WHERE voyage_id=$1 AND status IN ('waiting_for_uploads','processing') FOR UPDATE")
        .bind(voyage.id).fetch_optional(&mut *connection).await?;
    let Some(intent) = intent else {
        return Ok(());
    };
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT now()")
        .fetch_one(&mut *connection)
        .await?;
    if intent.expires_at <= now {
        return set_status(
            connection,
            intent.id,
            "expired",
            Some("Publication consent expired."),
        )
        .await;
    }
    if voyage.published
        || revision(connection, voyage.id).await? != intent.tracked_revision
        || metadata_digest(
            &voyage.title,
            voyage.description.as_deref(),
            &voyage.granularity,
        )? != intent.metadata_digest
    {
        return set_status(
            connection,
            intent.id,
            "needs_review",
            Some("Voyage details or membership changed."),
        )
        .await;
    }
    let references: Vec<PhotoReference> = match serde_json::from_value(intent.photos) {
        Ok(references) => references,
        Err(_) => {
            return set_status(
                connection,
                intent.id,
                "needs_review",
                Some("The saved publication review could not be read. Review publication again."),
            )
            .await;
        }
    };
    let resolved = resolve(connection, intent.owner_id, intent.voyage_id, &references).await?;
    if let Some(issue) = resolved.issue {
        return set_status(connection, intent.id, "needs_review", Some(issue)).await;
    }
    if resolved.waiting > 0 {
        return set_status(connection, intent.id, ACTIVE[0], None).await;
    }
    if resolved.processing > 0 {
        return set_status(connection, intent.id, ACTIVE[1], None).await;
    }
    sqlx::query("SELECT set_config('pinvoyager.expected_intent',$1,true)")
        .bind(intent.id.to_string())
        .execute(&mut *connection)
        .await?;
    voyages::reorder(
        &mut *connection,
        voyage.id,
        intent.owner_id,
        &resolved.order,
    )
    .await?;
    sqlx::query(
        "UPDATE voyages SET published=true,published_at=COALESCE(published_at,now()) WHERE id=$1",
    )
    .bind(voyage.id)
    .execute(&mut *connection)
    .await?;
    set_status(connection, intent.id, "published", None).await
}

/// Call before inserting an upload, while holding its account and voyage locks.
pub async fn authorize_expected_upload(
    connection: &mut PgConnection,
    voyage: Uuid,
    key: Option<&str>,
    hash: &str,
) -> AppResult<()> {
    let active: Option<(Uuid,Value)>=sqlx::query_as("SELECT id,photos FROM publication_intents WHERE voyage_id=$1 AND status IN ('waiting_for_uploads','processing')").bind(voyage).fetch_optional(&mut *connection).await?;
    let (Some((id, photos)), Some(key)) = (active, key) else {
        return Ok(());
    };
    let references: Vec<PhotoReference> =
        serde_json::from_value(photos).map_err(AppError::internal)?;
    for reference in references {
        if let PhotoReference::Future(photo) = reference
            && format!("{}:0", photo.upload_key) == key
        {
            if !photo.content_hash.eq_ignore_ascii_case(hash) {
                return Err(AppError::Conflict(
                    "The reviewed upload key is bound to different image bytes.".to_owned(),
                ));
            }
            sqlx::query("SELECT set_config('pinvoyager.expected_intent',$1,true)")
                .bind(id.to_string())
                .execute(connection)
                .await?;
            break;
        }
    }
    Ok(())
}

pub async fn finalize(pool: &PgPool, voyage_id: Uuid) -> AppResult<()> {
    let owner: Option<Uuid> = sqlx::query_scalar("SELECT owner_id FROM voyages WHERE id=$1")
        .bind(voyage_id)
        .fetch_optional(pool)
        .await?;
    let Some(owner) = owner else {
        return Ok(());
    };
    let mut tx = pool.begin().await?;
    match users::require_active(&mut tx, owner).await {
        Ok(()) => {}
        Err(AppError::Forbidden) => {
            sqlx::query("UPDATE publication_intents SET status='needs_review',reason='The account cannot publish.' WHERE owner_id=$1 AND status IN ('waiting_for_uploads','processing')").bind(owner).execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(());
        }
        Err(error) => return Err(error),
    }
    let voyage = match lock_voyage(&mut tx, owner, voyage_id).await {
        Ok(voyage) => voyage,
        Err(AppError::NotFound) => return Ok(()),
        Err(error) => return Err(error),
    };
    finalize_locked(&mut tx, &voyage).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn photo_changed(pool: &PgPool, photo: Uuid) -> AppResult<()> {
    let voyage: Option<Uuid> = sqlx::query_scalar("SELECT voyage_id FROM photos WHERE id=$1")
        .bind(photo)
        .fetch_optional(pool)
        .await?;
    if let Some(voyage) = voyage {
        finalize(pool, voyage).await?;
    }
    Ok(())
}

pub async fn sweep(pool: &PgPool) -> AppResult<()> {
    let mut before: Option<Uuid> = None;
    loop {
        let page: Vec<(Uuid,Uuid)>=sqlx::query_as("SELECT id,voyage_id FROM publication_intents WHERE status IN ('waiting_for_uploads','processing') AND ($1::uuid IS NULL OR id>$1) ORDER BY id LIMIT 100")
            .bind(before).fetch_all(pool).await?;
        if page.is_empty() {
            break;
        }
        for (id, voyage) in page {
            before = Some(id);
            finalize(pool, voyage).await?;
        }
    }
    Ok(())
}

pub async fn ensure_eligible(connection: &mut PgConnection, voyage: Uuid) -> AppResult<()> {
    let held: bool = sqlx::query_scalar("SELECT moderation_held FROM voyages WHERE id=$1")
        .bind(voyage)
        .fetch_one(connection)
        .await?;
    if held {
        Err(AppError::Conflict(
            "This voyage is held for moderation. Contact support before publishing.".into(),
        ))
    } else {
        Ok(())
    }
}
