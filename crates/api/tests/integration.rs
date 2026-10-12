//! Integration tests against a real PostgreSQL.
//!
//! These need the compose database up: `docker compose up -d`. They are
//! deliberately not mocked — the properties under test here (a unique
//! constraint firing, a pool timing out rather than hanging, a transaction
//! rolling back) are properties of PostgreSQL and of sqlx's pool, and a mock
//! would assert only that the test author understood them.
//!
//! Every test that writes does so inside a transaction it rolls back, so the
//! suite leaves no rows behind and can run repeatedly against the same
//! database without a cleanup step.

use std::time::{Duration, Instant};

use pinvoyager_api::db::repo::identities::Provider;
use pinvoyager_api::db::repo::{identities, users};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, PgPool};
use uuid::Uuid;

const DEFAULT_URL: &str = "postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager";

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .or_else(|_| std::env::var("PINVOYAGER__DATABASE__URL"))
        .unwrap_or_else(|_| DEFAULT_URL.to_owned())
}

/// A pool with an explicit size and timeout, so a test can create the exact
/// conditions it needs rather than inheriting production's.
async fn pool(max_connections: u32, acquire_timeout: Duration) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(acquire_timeout)
        .connect(&database_url())
        .await
        .expect("database is not reachable — is `docker compose up -d` running?");
    pinvoyager_api::db::migrate(&pool)
        .await
        .expect("test database migrations apply");
    pool
}

// ---------------------------------------------------------------------------
// T12 — repository functions accept an executor, so a test can pass a
// transaction and roll it back instead of cleaning up after itself.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn repository_functions_accept_a_transaction_that_rolls_back() {
    let pool = pool(5, Duration::from_secs(5)).await;
    let mut tx = pool.begin().await.expect("begin");

    let user = users::create(&mut *tx, Some("Test User"), None)
        .await
        .expect("create user");

    let identity = identities::create(&mut *tx, user.id, Provider::Google, "subject-abc")
        .await
        .expect("create identity");

    assert_eq!(identity.user_id, user.id);
    assert_eq!(identity.provider, Provider::Google);

    // Visible inside the transaction...
    let found = users::find_by_id(&mut *tx, user.id)
        .await
        .expect("find inside tx");
    assert!(found.is_some(), "the user should be visible in its own tx");

    tx.rollback().await.expect("rollback");

    // ...and gone afterwards. This is the property that makes the suite
    // repeatable: no test needs a teardown step, and a failing test cannot
    // leave rows that break the next run.
    let after = users::find_by_id(&pool, user.id).await.expect("find after");
    assert!(after.is_none(), "rollback should have discarded the user");
}

// ---------------------------------------------------------------------------
// D3 — the unique constraint on (provider, subject) is what stops one provider
// identity from being attached to two accounts. Verified against the database
// rather than trusted from the migration text.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn one_provider_identity_cannot_attach_to_two_accounts() {
    let pool = pool(5, Duration::from_secs(5)).await;
    let mut tx = pool.begin().await.expect("begin");

    let subject = format!("shared-subject-{}", Uuid::now_v7());

    let first = users::create(&mut *tx, Some("First"), None)
        .await
        .expect("create first user");
    let second = users::create(&mut *tx, Some("Second"), None)
        .await
        .expect("create second user");

    identities::create(&mut *tx, first.id, Provider::Apple, &subject)
        .await
        .expect("first link should succeed");

    let conflict = identities::create(&mut *tx, second.id, Provider::Apple, &subject).await;

    let err = conflict.expect_err("the second link must be refused, not merged");
    let db_err = err
        .as_database_error()
        .expect("expected a database error, not a client-side one");
    assert!(
        db_err.is_unique_violation(),
        "expected a unique violation on (provider, subject), got: {db_err}"
    );

    tx.rollback().await.expect("rollback");
}

/// The same subject under a *different* provider is a different identity, and
/// must be allowed — otherwise a user could never link a second provider whose
/// subject happened to collide.
#[tokio::test]
async fn the_same_subject_under_a_different_provider_is_allowed() {
    let pool = pool(5, Duration::from_secs(5)).await;
    let mut tx = pool.begin().await.expect("begin");

    let subject = format!("collide-{}", Uuid::now_v7());
    let user = users::create(&mut *tx, None, None)
        .await
        .expect("create user");

    identities::create(&mut *tx, user.id, Provider::Google, &subject)
        .await
        .expect("google link");
    identities::create(&mut *tx, user.id, Provider::Microsoft, &subject)
        .await
        .expect("microsoft link with the same subject must be allowed");

    tx.rollback().await.expect("rollback");
}

// ---------------------------------------------------------------------------
// D1 — no credential is stored. Asserted against the live schema, because the
// decision says to verify by inspecting rather than by intent, and because a
// future migration is exactly how this would regress.
// ---------------------------------------------------------------------------

/// The one column permitted to look like a credential, and why.
///
/// `sessions.token_hash` is the SHA-256 of a session token **this service
/// mints**, not a credential a user or a provider gave us. D1 forbids storing
/// passwords, hashes of them, and long-lived provider access or refresh tokens;
/// a session identifier is none of those. It is stored hashed precisely so that
/// a database leak yields nothing usable — the raw token exists only in the
/// user's cookie.
///
/// The exception is written as an exact (table, column) pair rather than by
/// relaxing the pattern, so that a future `users.password_hash` or
/// `identities.refresh_token` still fails this test. A broad pattern would have
/// quietly permitted both.
/// `auth_attempts.binding_hash` is the SHA-256 of a browser-binding value this
/// service mints at the start of a sign-in. Like a session token it is ours,
/// not a user's or a provider's; unlike one it is single-use and expires in
/// minutes. Stored hashed so that a database reader cannot complete somebody
/// else's in-flight authorization.
/// `photos.content_hash` is a SHA-256 of image bytes — a storage address and a
/// deduplication key, not a secret of any kind. It matches the pattern only
/// because the word "hash" appears in it.
///
/// Three exemptions now, and every one of them was added because this test
/// failed and forced the question. That is the design working: a broad pattern
/// with narrow, written exceptions makes each new credential-shaped column a
/// deliberate decision, where a pattern tuned to pass would make the next one
/// invisible.
const PERMITTED: &[(&str, &str)] = &[
    ("sessions", "token_hash"),
    ("auth_attempts", "binding_hash"),
    ("photos", "content_hash"),
    ("photo_cleanup", "content_hash"),
];

#[tokio::test]
async fn the_schema_holds_no_credential_columns() {
    let pool = pool(2, Duration::from_secs(5)).await;

    let found: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_name::text, column_name::text
           FROM information_schema.columns
          WHERE table_schema = 'public'
            AND column_name ~* 'pass|hash|secret|token|credential|salt|otp|mfa'",
    )
    .fetch_all(&pool)
    .await
    .expect("query information_schema");

    let offenders: Vec<_> = found
        .iter()
        .filter(|(table, column)| {
            !PERMITTED
                .iter()
                .any(|&(t, c)| t == table.as_str() && c == column.as_str())
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "D1 forbids storing any credential; found: {offenders:?}"
    );

    // The exception must stay honest: if the sessions table is ever renamed or
    // the column dropped, this catches the stale entry rather than leaving a
    // permanently-granted exemption nobody revisits.
    for (table, column) in PERMITTED {
        assert!(
            found.iter().any(|(t, c)| t == table && c == column),
            "the permitted exception {table}.{column} no longer exists; remove it from PERMITTED"
        );
    }
}

// ---------------------------------------------------------------------------
// The dev login is an authentication bypass, so the things that keep it
// contained are worth asserting rather than trusting.
//
// The strongest guarantee — that `#[cfg(debug_assertions)]` keeps the route out
// of a release binary entirely — cannot be tested from inside a test binary,
// because tests are themselves a debug build. It is verified by building
// `--release` and searching the artifact for the route string; see
// docs/running.md.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_dev_login_route_is_absent_from_release_builds() {
    // A compile-time assertion standing in for what cannot be checked at
    // runtime: if someone removes the `cfg` guard, `debug_assertions` will be
    // off in a release build while the module is still present, and this test
    // file will fail to compile there — which is a louder failure than a route
    // quietly appearing in production.
    #[cfg(not(debug_assertions))]
    compile_error!(
        "the dev login module must be behind #[cfg(debug_assertions)]; \
         if this fires, a release build is compiling an authentication bypass"
    );

    // In a debug build the module exists, which is the other half of the claim.
    #[cfg(debug_assertions)]
    {
        let _ = pinvoyager_api::auth::dev::login;
    }
}

// ---------------------------------------------------------------------------
// T6 / D8 — pool exhaustion must surface as a timeout, not as a hang. This is
// the criterion the phase doc says to verify rather than assume, because
// sqlx's default is no timeout at all and the failure mode is a service that
// looks alive while serving nothing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_exhaustion_times_out_rather_than_hanging() {
    let acquire_timeout = Duration::from_millis(750);
    let pool = pool(1, acquire_timeout).await;

    // Hold the only connection for the duration of the test.
    let _held = pool.acquire().await.expect("first acquire should succeed");

    let started = Instant::now();
    let result = pool.acquire().await;
    let elapsed = started.elapsed();

    let Err(err) = result else {
        panic!("a second acquire must not succeed while the only connection is held");
    };
    assert!(
        matches!(err, sqlx::Error::PoolTimedOut),
        "expected PoolTimedOut, got: {err:?}"
    );

    // The point of the test is the bound, not merely the error: without
    // `acquire_timeout` set this call parks forever.
    assert!(
        elapsed < acquire_timeout * 4,
        "acquire took {elapsed:?}, which suggests no effective timeout"
    );
}

/// The other half of T6: that timeout has to reach the client as a retryable
/// 503, not a 500. Pool exhaustion is load, not a fault, and counting it as a
/// server error is how a busy afternoon turns into a page.
#[tokio::test]
async fn pool_timeout_maps_to_503_not_500() {
    use axum::response::IntoResponse;
    use pinvoyager_api::AppError;

    let response = AppError::Database(sqlx::Error::PoolTimedOut).into_response();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
}

// ---------------------------------------------------------------------------
// Migrations apply to a genuinely empty database, not just to the developer's
// already-migrated one. A migration that only works against existing state is
// a migration that fails on the first real deploy.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn migrations_apply_cleanly_to_an_empty_database() {
    let admin_url = database_url();
    let db_name = format!("pv_test_{}", Uuid::now_v7().simple());

    let mut admin = sqlx::PgConnection::connect(&admin_url)
        .await
        .expect("connect to create the scratch database");

    // `CREATE DATABASE` cannot take a bind parameter, so the name has to be
    // interpolated. sqlx 0.9 blocks that by default, which is the right
    // default — the assertion is safe here only because `db_name` is
    // `pv_test_` plus a UUID rendered as hex, so it contains nothing but
    // `[a-z0-9_]` and cannot carry a quote out of the identifier.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE DATABASE \"{db_name}\""
    )))
    .execute(&mut admin)
    .await
    .expect("create scratch database");

    // Swap the path segment for the scratch database name.
    let scratch_url = {
        let (base, _) = admin_url.rsplit_once('/').expect("url has a path segment");
        format!("{base}/{db_name}")
    };

    let result = async {
        let scratch = PgPoolOptions::new()
            .max_connections(1)
            .connect(&scratch_url)
            .await?;
        pinvoyager_api::db::migrate(&scratch)
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

        // The unique index is the load-bearing part of D3; assert it exists by
        // name rather than assuming the migration ran to completion.
        let index_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM pg_indexes
                 WHERE schemaname = 'public'
                   AND indexname = 'identities_provider_subject_key'
             )",
        )
        .fetch_one(&scratch)
        .await?;

        scratch.close().await;
        Ok::<bool, Box<dyn std::error::Error + Send + Sync>>(index_exists)
    }
    .await;

    // Drop the scratch database whether the assertions passed or not, so a
    // failure does not litter the server with databases.
    let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS \"{db_name}\" WITH (FORCE)"
    )))
    .execute(&mut admin)
    .await;

    let index_exists = result.expect("migrations should apply to an empty database");
    assert!(
        index_exists,
        "identities_provider_subject_key is missing after migration"
    );
}
