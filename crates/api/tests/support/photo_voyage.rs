use sqlx::PgPool;
use uuid::Uuid;

/// Each fixture starts in its own private voyage; tests may move it explicitly.
pub async fn fixture_voyage(pool: &PgPool, owner: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO voyages (id, owner_id, title) VALUES ($1, $2, 'Photo fixture') RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(owner)
    .fetch_one(pool)
    .await
    .expect("photo fixture voyage")
}
