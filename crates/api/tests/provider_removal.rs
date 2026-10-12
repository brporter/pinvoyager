//! Upgrading a database removes the retired provider without deleting account data.

use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn removing_a_provider_preserves_users_sessions_and_other_identities(pool: PgPool) {
    sqlx::raw_sql(concat!(
        include_str!("../migrations/20260815000001_create_users_and_identities.sql"),
        include_str!("../migrations/20260816000001_create_sessions_and_auth_attempts.sql")
    ))
    .execute(&pool)
    .await
    .expect("create the previously deployed auth schema");
    sqlx::raw_sql(
        "INSERT INTO users (id) VALUES ('00000000-0000-0000-0000-000000000001');
         INSERT INTO identities (id, user_id, provider, subject) VALUES
         ('00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'facebook', 'retired-subject'),
         ('00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000001', 'google', 'retained-subject');
         INSERT INTO sessions (token_hash, user_id, absolute_expires_at) VALUES
         ('test-session', '00000000-0000-0000-0000-000000000001', now() + interval '1 day');
         INSERT INTO auth_attempts (state, provider, nonce, pkce_verifier, expires_at) VALUES
         ('retired-attempt', 'facebook', 'nonce', 'verifier', now() + interval '1 hour'),
         ('retained-attempt', 'google', 'nonce', 'verifier', now() + interval '1 hour');",
    )
    .execute(&pool)
    .await
    .expect("seed old identities, sessions, and attempts");

    sqlx::raw_sql(include_str!(
        "../migrations/20261011000001_remove_facebook_auth.sql"
    ))
    .execute(&pool)
    .await
    .expect("apply provider removal");
    for (table, query) in [
        ("users", "SELECT count(*) FROM users"),
        ("sessions", "SELECT count(*) FROM sessions"),
        ("identities", "SELECT count(*) FROM identities"),
        ("auth_attempts", "SELECT count(*) FROM auth_attempts"),
    ] {
        let count: i64 = sqlx::query_scalar(query)
            .fetch_one(&pool)
            .await
            .expect("count retained rows");
        assert_eq!(
            count, 1,
            "the migration preserves the expected rows in {table}"
        );
    }
    let provider: String = sqlx::query_scalar("SELECT provider FROM identities")
        .fetch_one(&pool)
        .await
        .expect("read retained identity");
    assert_eq!(provider, "google");
    let provider: String = sqlx::query_scalar("SELECT provider FROM auth_attempts")
        .fetch_one(&pool)
        .await
        .expect("read retained attempt");
    assert_eq!(provider, "google");

    for provider in ["google", "microsoft", "apple", "facebook"] {
        let result = sqlx::query("INSERT INTO identities (id, user_id, provider, subject) VALUES ($1, '00000000-0000-0000-0000-000000000001', $2, 'new-subject')")
            .bind(uuid::Uuid::now_v7()).bind(provider).execute(&pool).await;
        assert_eq!(result.is_ok(), provider != "facebook");
        let result = sqlx::query("INSERT INTO auth_attempts (state, provider, nonce, pkce_verifier, expires_at) VALUES ($1, $2, 'nonce', 'verifier', now() + interval '1 hour')")
            .bind(provider).bind(provider).execute(&pool).await;
        assert_eq!(result.is_ok(), provider != "facebook");
    }
}
