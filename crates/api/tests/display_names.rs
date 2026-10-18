use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use pinvoyager_api::{
    AppState, Config,
    auth::{ProviderRegistry, session},
    db::repo::{users, voyages},
    http,
    photos::worker::VariantQueue,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt as _;
use uuid::Uuid;

fn app(pool: PgPool) -> Router {
    let mut config = Config::load().expect("config");
    config.auth.dev_login = true;
    http::router(AppState::new(
        pool,
        Arc::new(config),
        ProviderRegistry::empty(),
        VariantQueue::detached(),
    ))
}
async fn caller(pool: &PgPool) -> (Uuid, String) {
    let user = users::create(pool, None, None).await.expect("user");
    let config = Config::load().expect("config");
    let token = session::create(pool, user.id, &config.auth.session)
        .await
        .expect("session");
    (
        user.id,
        format!("{}={}", config.auth.session.cookie_name, token.expose()),
    )
}
async fn send(
    app: &Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        },
    )
}

#[sqlx::test]
async fn first_sign_in_does_not_claim_a_provider_name_and_profile_save_is_authenticated(
    pool: PgPool,
) {
    let app = app(pool.clone());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/dev/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"subject":"new-user", "display_name":"Provider Name"}).to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("login");
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie value")
        .to_owned();
    let (status, me) = send(&app, "GET", "/api/me", Some(&cookie), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(me["user"]["display_name"].is_null());
    assert_eq!(
        send(
            &app,
            "PATCH",
            "/api/me",
            None,
            json!({"display_name":"betaporter"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            "PATCH",
            "/api/me",
            Some(&cookie),
            json!({"display_name":"BetaPorter"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, "GET", "/api/me", Some(&cookie), Value::Null)
            .await
            .1["user"]["display_name"],
        "betaporter"
    );
}

#[sqlx::test]
async fn names_are_unique_under_concurrency_and_old_names_remain_owned(pool: PgPool) {
    let (_, first) = caller(&pool).await;
    let (_, second) = caller(&pool).await;
    let app = app(pool);
    let (a, b) = tokio::join!(
        send(
            &app,
            "PATCH",
            "/api/me",
            Some(&first),
            json!({"display_name":"shared-name"})
        ),
        send(
            &app,
            "PATCH",
            "/api/me",
            Some(&second),
            json!({"display_name":"SHARED-NAME"})
        ),
    );
    let (winner, loser) = if a.0 == StatusCode::NO_CONTENT {
        (&first, &second)
    } else {
        (&second, &first)
    };
    let mut statuses = [a.0.as_u16(), b.0.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [204, 409]);
    assert_eq!(
        send(
            &app,
            "PATCH",
            "/api/me",
            Some(winner),
            json!({"display_name":"new-name"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            "PATCH",
            "/api/me",
            Some(loser),
            json!({"display_name":"shared-name"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        send(&app, "GET", "/api/profiles/shared-name", None, Value::Null)
            .await
            .1["user"]["display_name"],
        "new-name"
    );
    assert_eq!(
        send(
            &app,
            "PATCH",
            "/api/me",
            Some(winner),
            json!({"display_name":"shared-name"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test]
async fn reserved_names_and_invalid_url_names_cannot_be_claimed(pool: PgPool) {
    let (_, cookie) = caller(&pool).await;
    let app = app(pool);
    for name in [
        "api",
        "auth",
        "upload",
        "voyages",
        "settings",
        "signin",
        "admin",
        "assets",
        "onboarding",
        "health",
        "ready",
    ] {
        assert_eq!(
            send(
                &app,
                "PATCH",
                "/api/me",
                Some(&cookie),
                json!({"display_name":name})
            )
            .await
            .0,
            StatusCode::CONFLICT,
            "{name}"
        );
    }
    for name in [
        "",
        "ab",
        "a b",
        "_system",
        "2beta",
        "beta-",
        "a/b",
        "résumé",
        "abcdefghijklmnopqrstuvwxyz12345",
    ] {
        assert_eq!(
            send(
                &app,
                "PATCH",
                "/api/me",
                Some(&cookie),
                json!({"display_name":name})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST,
            "{name}"
        );
    }
}

#[sqlx::test]
async fn anonymous_profiles_filter_by_owner_and_publication_and_paginate(pool: PgPool) {
    let (owner, cookie) = caller(&pool).await;
    let (other, _) = caller(&pool).await;
    let app = app(pool.clone());
    assert_eq!(
        send(
            &app,
            "PATCH",
            "/api/me",
            Some(&cookie),
            json!({"display_name":"betaporter"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    for (owner_id, title, published) in [
        (owner, "First", true),
        (owner, "Second", true),
        (owner, "Private draft", false),
        (other, "Other user", true),
    ] {
        let voyage = voyages::create(
            &pool,
            &voyages::NewVoyage {
                owner_id,
                title: title.into(),
                description: None,
                granularity: "km1".into(),
            },
        )
        .await
        .expect("voyage");
        if published {
            voyages::update(
                &pool,
                voyage.id,
                owner_id,
                &voyages::VoyagePatch {
                    published: Some(true),
                    ..Default::default()
                },
            )
            .await
            .expect("publish");
        }
    }
    let (status, page) = send(
        &app,
        "GET",
        "/api/profiles/BetaPorter?limit=1",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["user"]["display_name"], "betaporter");
    assert_eq!(page["voyages"].as_array().expect("voyages").len(), 1);
    assert_eq!(page["voyages"][0]["title"], "Second");
    assert!(page.get("identities").is_none());
    let cursor = page["next_before"].as_str().expect("cursor");
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("before", cursor)
        .finish();
    let next = send(
        &app,
        "GET",
        &format!("/api/profiles/betaporter?{encoded}"),
        None,
        Value::Null,
    )
    .await
    .1;
    assert_eq!(next["voyages"].as_array().expect("voyages").len(), 1);
    assert_eq!(next["voyages"][0]["title"], "First");
    assert!(next["next_before"].is_null());
    assert_eq!(
        send(&app, "GET", "/api/profiles/unknown", None, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&app, "GET", "/api/profiles/settings", None, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/profiles/betaporter?before=bad",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test(migrations = false)]
async fn migration_retains_unique_valid_handles_and_requires_ambiguous_legacy_names_to_choose(
    pool: PgPool,
) {
    sqlx::raw_sql(include_str!(
        "../migrations/20260815000001_create_users_and_identities.sql"
    ))
    .execute(&pool)
    .await
    .expect("old schema");
    for name in [
        "BetaPorter",
        "Provider Full Name",
        "settings",
        "Duplicate",
        "DUPLICATE",
    ] {
        sqlx::query("INSERT INTO users(id,display_name,avatar_url) VALUES ($1,$2,'/avatar')")
            .bind(Uuid::now_v7())
            .bind(name)
            .execute(&pool)
            .await
            .expect("legacy user");
    }
    sqlx::raw_sql(include_str!(
        "../migrations/20261010000001_unique_display_names.sql"
    ))
    .execute(&pool)
    .await
    .expect("migration");
    let names: Vec<Option<String>> =
        sqlx::query_scalar("SELECT display_name FROM users ORDER BY display_name NULLS LAST")
            .fetch_all(&pool)
            .await
            .expect("names");
    assert_eq!(
        names,
        vec![Some("betaporter".into()), None, None, None, None]
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users WHERE avatar_url='/avatar'")
            .fetch_one(&pool)
            .await
            .expect("avatars"),
        5
    );
    // Inspect this historical migration's schema directly; current repository
    // projections may include columns added by later migrations.
    let resolves: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users u JOIN profile_names n ON n.user_id=u.id
         WHERE n.name='betaporter' AND u.display_name='betaporter')",
    )
    .fetch_one(&pool)
    .await
    .expect("migrated name resolves");
    assert!(resolves);
}

#[sqlx::test]
async fn deleted_accounts_leave_all_their_profile_names_reserved(pool: PgPool) {
    let (owner, cookie) = caller(&pool).await;
    let (_, other) = caller(&pool).await;
    let app = app(pool.clone());
    for name in ["former-name", "current-name"] {
        assert_eq!(
            send(
                &app,
                "PATCH",
                "/api/me",
                Some(&cookie),
                json!({"display_name":name})
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .expect("delete");
    assert_eq!(
        send(&app, "GET", "/api/profiles/former-name", None, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    for name in ["former-name", "current-name"] {
        assert_eq!(
            send(
                &app,
                "PATCH",
                "/api/me",
                Some(&other),
                json!({"display_name":name})
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
    }
}

#[sqlx::test]
async fn a_conflicting_name_write_rolls_back_the_registry_claim(pool: PgPool) {
    // Even an internal writer that bypassed the registry cannot create duplicates.
    users::create(&pool, Some("already-owned"), None)
        .await
        .expect("existing name");
    let (_, cookie) = caller(&pool).await;
    let app = app(pool.clone());
    assert_eq!(
        send(
            &app,
            "PATCH",
            "/api/me",
            Some(&cookie),
            json!({"display_name":"already-owned"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM profile_names WHERE name='already-owned'",
    )
    .fetch_one(&pool)
    .await
    .expect("claim rollback");
    assert_eq!(count, 0);
    let me = send(&app, "GET", "/api/me", Some(&cookie), Value::Null)
        .await
        .1;
    assert!(me["user"]["display_name"].is_null());
}

#[sqlx::test]
async fn failed_name_writes_do_not_leak_database_details_or_keep_claims(pool: PgPool) {
    let (_, cookie) = caller(&pool).await;
    sqlx::raw_sql(
        "CREATE FUNCTION reject_name_update() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'private database detail'; END $$;
         CREATE TRIGGER reject_name_update BEFORE UPDATE OF display_name ON users
         FOR EACH ROW EXECUTE FUNCTION reject_name_update();",
    )
    .execute(&pool)
    .await
    .expect("simulate a failed write");
    let app = app(pool.clone());
    let (status, response) = send(
        &app,
        "PATCH",
        "/api/me",
        Some(&cookie),
        json!({"display_name":"failed-write"}),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!response.to_string().contains("private database detail"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM profile_names WHERE name='failed-write'",
        )
        .fetch_one(&pool)
        .await
        .expect("rollback"),
        0
    );
}

#[sqlx::test]
async fn malformed_name_requests_cannot_claim_a_name(pool: PgPool) {
    let (_, cookie) = caller(&pool).await;
    let app = app(pool);
    for body in [
        json!({}),
        json!({"display_name":null}),
        json!({"display_name":12}),
    ] {
        assert_eq!(
            send(&app, "PATCH", "/api/me", Some(&cookie), body).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        send(&app, "GET", "/api/me", Some(&cookie), Value::Null)
            .await
            .1["user"]["display_name"],
        Value::Null
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/profiles/unknown?limit=invalid",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test]
async fn profiles_resolve_ids_and_names_without_exposing_drafts(pool: PgPool) {
    let (owner, owner_cookie) = caller(&pool).await;
    let (_, stranger_cookie) = caller(&pool).await;
    let app = app(pool.clone());
    for name in ["old-profile", "current-profile"] {
        assert_eq!(
            send(
                &app,
                "PATCH",
                "/api/me",
                Some(&owner_cookie),
                json!({"display_name": name})
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    for published in [false, true] {
        let voyage = voyages::create(
            &pool,
            &voyages::NewVoyage {
                owner_id: owner,
                title: if published { "Public" } else { "Private" }.into(),
                description: None,
                granularity: "km1".into(),
            },
        )
        .await
        .expect("voyage");
        if published {
            voyages::update(
                &pool,
                voyage.id,
                owner,
                &voyages::VoyagePatch {
                    published: Some(true),
                    ..Default::default()
                },
            )
            .await
            .expect("publish");
        }
    }
    for identifier in [
        owner.to_string(),
        "current-profile".into(),
        "CURRENT-PROFILE".into(),
        "old-profile".into(),
    ] {
        for cookie in [
            None,
            Some(owner_cookie.as_str()),
            Some(stranger_cookie.as_str()),
        ] {
            let (status, page) = send(
                &app,
                "GET",
                &format!("/api/profiles/{identifier}"),
                cookie,
                Value::Null,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{identifier}");
            assert_eq!(page["user"]["id"], owner.to_string());
            assert_eq!(page["voyages"].as_array().expect("voyages").len(), 1);
            assert_eq!(page["voyages"][0]["title"], "Public");
        }
    }
    let unnamed = users::create(&pool, None, None)
        .await
        .expect("unnamed user");
    let (status, page) = send(
        &app,
        "GET",
        &format!("/api/profiles/{}", unnamed.id),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page["user"]["display_name"].is_null());
    assert_eq!(page["voyages"], json!([]));
    for identifier in [Uuid::now_v7().to_string(), "invalid-id".into()] {
        assert_eq!(
            send(
                &app,
                "GET",
                &format!("/api/profiles/{identifier}"),
                None,
                Value::Null
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(unnamed.id)
        .execute(&pool)
        .await
        .expect("delete");
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("/api/profiles/{}", unnamed.id),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}
