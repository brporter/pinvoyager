//! Durable upload identities survive photo deletion and serialize cancellation with ingestion.
use crate::{
    AppError, AppResult, AppState,
    auth::CurrentUser,
    db::repo::photos::{self, Photo},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/voyages/{id}/uploads/{key}",
        get(lookup).delete(cancel),
    )
}

#[derive(sqlx::FromRow)]
struct Receipt {
    state: String,
    photo_id: Option<Uuid>,
    content_hash: Option<String>,
    revision: Option<i64>,
    previous_revision: Option<i64>,
}

pub(super) async fn replay(
    connection: &mut PgConnection,
    owner: Uuid,
    voyage: Uuid,
    key: &str,
    hash: &str,
) -> AppResult<Option<Photo>> {
    let receipt: Option<Receipt>=sqlx::query_as("SELECT state,photo_id,content_hash,revision,previous_revision FROM upload_receipts WHERE account_id=$1 AND voyage_id=$2 AND upload_key=$3")
        .bind(owner).bind(voyage).bind(key).fetch_optional(&mut *connection).await?;
    let Some(receipt) = receipt else {
        return Ok(None);
    };
    if receipt.state != "accepted" {
        return Err(AppError::Conflict(
            "This upload was canceled or deleted. It cannot be replayed.".to_owned(),
        ));
    }
    if receipt.content_hash.as_deref() != Some(hash) {
        return Err(AppError::Conflict(
            "This upload key was used for different bytes.".to_owned(),
        ));
    }
    let photo = photos::find_by_id(connection, receipt.photo_id.ok_or(AppError::NotFound)?)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Some(photo))
}

pub(super) async fn accepted(
    connection: &mut PgConnection,
    owner: Uuid,
    voyage: Uuid,
    key: &str,
    photo: &Photo,
    previous_revision: Option<i64>,
) -> AppResult<()> {
    let revision = super::mutations::revision(connection, voyage, None).await?;
    sqlx::query("INSERT INTO upload_receipts(account_id,voyage_id,upload_key,state,photo_id,content_hash,revision,previous_revision) VALUES($1,$2,$3,'accepted',$4,$5,$6,$7)")
        .bind(owner).bind(voyage).bind(key).bind(photo.id).bind(&photo.content_hash).bind(revision).bind(previous_revision).execute(connection).await?;
    Ok(())
}

async fn lookup(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((voyage, key)): Path<(Uuid, Uuid)>,
) -> AppResult<Response> {
    let key = format!("{key}:0");
    let receipt: Option<Receipt>=sqlx::query_as("SELECT state,photo_id,content_hash,revision,previous_revision FROM upload_receipts WHERE account_id=$1 AND voyage_id=$2 AND upload_key=$3")
        .bind(user.user_id).bind(voyage).bind(key).fetch_optional(&state.pool).await?;
    let value = if let Some(receipt) = receipt {
        let mut value = json!({"status":receipt.state,"photo_id":receipt.photo_id,"revision":receipt.revision.map(|revision|revision.to_string())});
        if let Some(previous) = receipt.previous_revision {
            value["previous_revision"] = json!(previous.to_string());
        }
        value
    } else {
        let owned: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM voyages WHERE id=$1 AND owner_id=$2)")
                .bind(voyage)
                .bind(user.user_id)
                .fetch_one(&state.pool)
                .await?;
        if !owned {
            return Err(AppError::NotFound);
        }
        json!({"status":"not_seen"})
    };
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response())
}

async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((voyage, key)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    super::mutations::run(
        &state,
        user.user_id,
        &headers,
        super::mutations::Request {
            method: "DELETE",
            path: format!("/api/voyages/{voyage}/uploads/{key}"),
            payload: json!(null),
            requires_revision: false,
        },
        async |connection, _| cancel_in(connection, user.user_id, voyage, key).await,
    )
    .await
}

async fn cancel_in(
    connection: &mut PgConnection,
    owner: Uuid,
    voyage: Uuid,
    key: Uuid,
) -> AppResult<super::mutations::Outcome> {
    let owned: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM voyages WHERE id=$1 AND owner_id=$2 FOR UPDATE")
            .bind(voyage)
            .bind(owner)
            .fetch_optional(&mut *connection)
            .await?;
    owned.ok_or(AppError::NotFound)?;
    let previous_revision = super::mutations::revision(connection, voyage, None).await?;
    let revision = previous_revision;
    let internal_key = format!("{key}:0");
    let receipt: Option<Receipt> = sqlx::query_as("SELECT state,photo_id,content_hash,revision,previous_revision FROM upload_receipts WHERE account_id=$1 AND voyage_id=$2 AND upload_key=$3")
        .bind(owner).bind(voyage).bind(&internal_key).fetch_optional(&mut *connection).await?;
    let mut target = key;
    let mut removed = false;
    if let Some(receipt) = &receipt
        && receipt.state == "accepted"
        && let Some(photo) = receipt.photo_id
    {
        super::photos::destroy_in(connection, owner, photo, Some(voyage), Some(revision)).await?;
        target = photo;
        removed = true;
    }
    if !removed
        && receipt
            .as_ref()
            .is_none_or(|receipt| receipt.state != "canceled")
    {
        sqlx::query("SELECT record_photo_revision($1)")
            .bind(voyage)
            .execute(&mut *connection)
            .await?;
    }
    let revision = super::mutations::revision(connection, voyage, None).await?;
    sqlx::query("INSERT INTO upload_receipts(account_id,voyage_id,upload_key,state,revision,previous_revision) VALUES($1,$2,$3,'canceled',$4,$5) ON CONFLICT(account_id,voyage_id,upload_key) DO UPDATE SET state='canceled',revision=EXCLUDED.revision,previous_revision=EXCLUDED.previous_revision")
        .bind(owner).bind(voyage).bind(internal_key).bind(revision).bind(previous_revision).execute(&mut *connection).await?;
    let mut outcome = super::mutations::Outcome::deleted(target, voyage, "photo", revision);
    outcome.previous_revision = Some(previous_revision);
    Ok(outcome)
}
