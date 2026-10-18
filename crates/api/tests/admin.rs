use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use pinvoyager_api::{
    AppState, Config,
    auth::{ProviderRegistry, session},
    db::repo::users,
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
async fn administration_requires_an_administrator(pool: PgPool) {
    let app = app(pool.clone());
    let (id, cookie) = caller(&pool).await;
    let paths = [
        ("GET", "/api/admin/users".to_string()),
        ("GET", format!("/api/admin/users/{id}")),
        ("POST", format!("/api/admin/users/{id}/actions")),
    ];
    for (method, path) in &paths {
        assert_eq!(
            send(&app, method, path, None, json!({"action":"suspend"}))
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            send(
                &app,
                method,
                path,
                Some(&cookie),
                json!({"action":"suspend"})
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
}

async fn administrator(pool: &PgPool) -> (Uuid, String) {
    let (id, cookie) = caller(pool).await;
    sqlx::query("UPDATE users SET is_site_admin=true WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await
        .expect("test setup or assertion");
    (id, cookie)
}

async fn voyage(pool: &PgPool, owner: Uuid, published: bool) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO voyages (id, owner_id, title, description, published, published_at) VALUES ($1,$2,'Ocean','Private description',$3,CASE WHEN $3 THEN now() ELSE NULL END)")
        .bind(id).bind(owner).bind(published).execute(pool).await.expect("test setup or assertion");
    id
}

async fn action(app: &Router, cookie: &str, id: Uuid, body: Value) -> StatusCode {
    send(
        app,
        "POST",
        &format!("/api/admin/users/{id}/actions"),
        Some(cookie),
        body,
    )
    .await
    .0
}

#[sqlx::test]
async fn lists_accounts_and_private_voyage_metadata_with_stable_pages(pool: PgPool) {
    let app = app(pool.clone());
    let (admin, cookie) = administrator(&pool).await;
    let (owner, owner_cookie) = caller(&pool).await;
    let first = voyage(&pool, owner, false).await;
    let second = voyage(&pool, owner, true).await;
    let (status, page) = send(
        &app,
        "GET",
        "/api/admin/users?limit=1",
        Some(&cookie),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        page["users"]
            .as_array()
            .expect("test setup or assertion")
            .len(),
        1
    );
    assert_eq!(page["users"][0]["id"], owner.to_string());
    assert_eq!(page["users"][0]["voyage_count"], 2);
    assert_eq!(page["users"][0]["photo_count"], 0);
    assert_eq!(page["users"][0]["storage_bytes"], 0);
    let mut normalized = page.clone();
    normalized["users"][0]["id"] = json!("00000000-0000-0000-0000-000000000001");
    normalized["users"][0]["created_at"] = json!("2026-10-18T00:00:00Z");
    normalized["next_before"] = normalized["users"][0]["id"].clone();
    let expected: Value = serde_json::from_str(include_str!(
        "../../../web/src/api/__fixtures__/admin_users.json"
    ))
    .expect("fixture JSON");
    assert_eq!(normalized, expected);

    let (_, last) = send(
        &app,
        "GET",
        &format!(
            "/api/admin/users?limit=1&before={}",
            page["next_before"]
                .as_str()
                .expect("test setup or assertion")
        ),
        Some(&cookie),
        Value::Null,
    )
    .await;
    assert_eq!(last["users"][0]["id"], admin.to_string());
    assert!(last["next_before"].is_null());
    let (_, detail) = send(
        &app,
        "GET",
        &format!("/api/admin/users/{owner}?limit=1"),
        Some(&cookie),
        Value::Null,
    )
    .await;
    assert_eq!(detail["voyages"][0]["id"], second.to_string());
    assert_eq!(detail["voyages"][0]["description"], "Private description");
    let mut normalized = detail.clone();
    normalized["user"]["id"] = json!("00000000-0000-0000-0000-000000000001");
    normalized["user"]["created_at"] = json!("2026-10-18T00:00:00Z");
    normalized["voyages"][0]["id"] = json!("00000000-0000-0000-0000-000000000002");
    normalized["next_before"] = normalized["voyages"][0]["id"].clone();
    let expected: Value = serde_json::from_str(include_str!(
        "../../../web/src/api/__fixtures__/admin_user_detail.json"
    ))
    .expect("fixture JSON");
    assert_eq!(normalized, expected);

    let (_, detail) = send(
        &app,
        "GET",
        &format!(
            "/api/admin/users/{owner}?limit=1&before={}",
            detail["next_before"]
                .as_str()
                .expect("test setup or assertion")
        ),
        Some(&cookie),
        Value::Null,
    )
    .await;
    assert_eq!(detail["voyages"][0]["id"], first.to_string());
    assert!(detail["next_before"].is_null());
    // The admin metadata endpoint does not grant access to draft photos/maps.
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("/api/voyages/{first}"),
            Some(&cookie),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&app, "GET", "/api/me", Some(&cookie), Value::Null)
            .await
            .1["user"]["is_site_admin"],
        true
    );
    assert_eq!(
        send(&app, "GET", "/api/me", Some(&owner_cookie), Value::Null)
            .await
            .1["user"]["is_site_admin"],
        false
    );
    for query in ["limit=0", "limit=101", "before=bad"] {
        assert_eq!(
            send(
                &app,
                "GET",
                &format!("/api/admin/users?{query}"),
                Some(&cookie),
                Value::Null
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("/api/admin/users/{}", Uuid::new_v4()),
            Some(&cookie),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn suspension_revokes_sessions_blocks_signin_and_can_be_reversed(pool: PgPool) {
    let app = app(pool.clone());
    let (_, cookie) = administrator(&pool).await;
    let (owner, old_cookie) = caller(&pool).await;
    pinvoyager_api::db::repo::identities::create(
        &pool,
        owner,
        pinvoyager_api::db::repo::identities::Provider::Google,
        "suspended",
    )
    .await
    .expect("test setup or assertion");
    assert_eq!(
        action(&app, &cookie, owner, json!({"action":"suspend"})).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, "GET", "/api/me", Some(&old_cookie), Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/auth/dev/login",
            None,
            json!({"subject":"suspended"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // Even a session inserted outside the login flow cannot authenticate a suspended account.
    let token = session::create(
        &pool,
        owner,
        &Config::load()
            .expect("test setup or assertion")
            .auth
            .session,
    )
    .await
    .expect("test setup or assertion");
    let forged = format!(
        "{}={}",
        Config::load()
            .expect("test setup or assertion")
            .auth
            .session
            .cookie_name,
        token.expose()
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/voyages",
            Some(&forged),
            json!({"title":"blocked"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        action(&app, &cookie, owner, json!({"action":"enable"})).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/auth/dev/login",
            None,
            json!({"subject":"suspended"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send(&app, "GET", "/api/me", Some(&old_cookie), Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test]
async fn promotion_unpublishing_and_self_protection(pool: PgPool) {
    let app = app(pool.clone());
    let (admin, cookie) = administrator(&pool).await;
    let (owner, owner_cookie) = caller(&pool).await;
    let first = voyage(&pool, owner, true).await;
    let second = voyage(&pool, owner, true).await;
    let foreign = voyage(&pool, admin, true).await;
    assert_eq!(
        action(
            &app,
            &cookie,
            owner,
            json!({"action":"unpublish_voyage","voyage_id":foreign})
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        action(&app, &cookie, owner, json!({"action":"unpublish_voyage"})).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        action(
            &app,
            &cookie,
            owner,
            json!({"action":"unpublish_voyage","voyage_id":first})
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("/api/voyages/{first}"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("/api/voyages/{second}"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        action(&app, &cookie, owner, json!({"action":"unpublish_all"})).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("/api/voyages/{second}"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        action(&app, &cookie, owner, json!({"action":"promote"})).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/admin/users",
            Some(&owner_cookie),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    for operation in ["suspend", "delete"] {
        assert_eq!(
            action(&app, &cookie, admin, json!({"action":operation})).await,
            StatusCode::CONFLICT
        );
    }
    assert_eq!(
        action(&app, &cookie, Uuid::new_v4(), json!({"action":"delete"})).await,
        StatusCode::NOT_FOUND
    );
    // Previously authenticated administrators lose access after suspension.
    assert_eq!(
        action(&app, &cookie, owner, json!({"action":"suspend"})).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/admin/users",
            Some(&owner_cookie),
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

async fn photo(pool: &PgPool, owner: Uuid, voyage: Uuid, hash: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO photos (id,owner_id,voyage_id,content_hash,byte_size,width,height,original_format,position) VALUES ($1,$2,$3,$4,10,1,1,'png',1)")
        .bind(id).bind(owner).bind(voyage).bind(hash).execute(pool).await.expect("test setup or assertion");
    id
}

fn app_with_storage(pool: PgPool, root: &std::path::Path) -> Router {
    let mut config = Config::load().expect("test setup or assertion");
    config.photos.storage_root = root.to_path_buf();
    http::router(AppState::new(
        pool,
        Arc::new(config),
        ProviderRegistry::empty(),
        VariantQueue::detached(),
    ))
}

#[sqlx::test]
async fn deletion_removes_account_data_and_unshared_files_but_reserves_names(pool: PgPool) {
    use pinvoyager_api::photos::{VariantKind, storage};
    let root = ScratchRoot::new();
    let app = app_with_storage(pool.clone(), root.path());
    let (_, cookie) = administrator(&pool).await;
    let (owner, owner_cookie) = caller(&pool).await;
    let (other, _) = caller(&pool).await;
    let voyage_id = voyage(&pool, owner, true).await;
    let other_voyage = voyage(&pool, other, true).await;
    let unique = "a".repeat(64);
    let shared = "b".repeat(64);
    for hash in [&unique, &shared, &unique] {
        photo(&pool, owner, voyage_id, hash).await;
    }
    photo(&pool, other, other_voyage, &shared).await;
    let mut unique_paths = vec![];
    let mut shared_paths = vec![];
    for (hash, paths) in [(&unique, &mut unique_paths), (&shared, &mut shared_paths)] {
        let hash = storage::ContentHash::try_from(hash.clone()).expect("test setup or assertion");
        paths.push(storage::original_path(root.path(), &hash));
        for kind in VariantKind::ALL {
            paths.push(storage::variant_path(root.path(), &hash, kind));
        }
        for path in paths {
            std::fs::create_dir_all(path.parent().expect("test setup or assertion"))
                .expect("test setup or assertion");
            std::fs::write(path, [0; 10]).expect("test setup or assertion");
        }
    }
    assert_eq!(
        send(
            &app,
            "PATCH",
            "/api/me",
            Some(&owner_cookie),
            json!({"display_name":"departing-user"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    pinvoyager_api::db::repo::identities::create(
        &pool,
        owner,
        pinvoyager_api::db::repo::identities::Provider::Google,
        "departing",
    )
    .await
    .expect("test setup or assertion");
    sqlx::query("INSERT INTO auth_attempts (state,provider,nonce,pkce_verifier,link_user_id,expires_at,binding_hash) VALUES ('attempt','google','nonce','pkce',$1,now()+interval '1 minute',$2)")
        .bind(owner).bind(vec![0u8;32]).execute(&pool).await.expect("test setup or assertion");
    let (_, detail) = send(
        &app,
        "GET",
        &format!("/api/admin/users/{owner}"),
        Some(&cookie),
        Value::Null,
    )
    .await;
    assert_eq!(detail["user"]["photo_count"], 3);
    assert_eq!(detail["user"]["storage_bytes"], 80);
    assert_eq!(
        action(&app, &cookie, owner, json!({"action":"delete"})).await,
        StatusCode::NO_CONTENT
    );
    assert!(unique_paths.iter().all(|path| !path.exists()));
    assert!(shared_paths.iter().all(|path| path.exists()));
    let remaining: (i64,i64,i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM users WHERE id=$1),(SELECT count(*) FROM voyages WHERE owner_id=$1),(SELECT count(*) FROM photos WHERE owner_id=$1),(SELECT count(*) FROM identities WHERE user_id=$1),(SELECT count(*) FROM sessions WHERE user_id=$1),(SELECT count(*) FROM auth_attempts WHERE link_user_id=$1)")
        .bind(owner).fetch_one(&pool).await.expect("test setup or assertion");
    assert_eq!(remaining, (0, 0, 0, 0, 0, 0));
    assert_eq!(
        send(&app, "GET", "/api/me", Some(&owner_cookie), Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let reserved: bool =
        sqlx::query_scalar("SELECT user_id IS NULL FROM profile_names WHERE name='departing-user'")
            .fetch_one(&pool)
            .await
            .expect("test setup or assertion");
    assert!(reserved);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM photo_cleanup")
            .fetch_one(&pool)
            .await
            .expect("test setup or assertion"),
        0
    );
}

#[sqlx::test]
async fn missing_files_count_as_zero_and_failed_deletions_remain_queued(pool: PgPool) {
    use pinvoyager_api::photos::storage;
    let root = ScratchRoot::new();
    let app = app_with_storage(pool.clone(), root.path());
    let (_, cookie) = administrator(&pool).await;
    let (owner, _) = caller(&pool).await;
    let voyage_id = voyage(&pool, owner, false).await;
    let hash = "c".repeat(64);
    photo(&pool, owner, voyage_id, &hash).await;
    let path = format!("/api/admin/users/{owner}");
    assert_eq!(
        send(&app, "GET", &path, Some(&cookie), Value::Null).await.1["user"]["storage_bytes"],
        0
    );
    let original = storage::original_path(
        root.path(),
        &storage::ContentHash::try_from(hash.clone()).expect("test setup or assertion"),
    );
    std::fs::create_dir_all(&original).expect("test setup or assertion"); // remove_file must reject a directory.
    assert_eq!(
        action(&app, &cookie, owner, json!({"action":"delete"})).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT content_hash FROM photo_cleanup")
            .fetch_one(&pool)
            .await
            .expect("test setup or assertion"),
        hash
    );
    std::fs::remove_dir(original).expect("test setup or assertion");
    pinvoyager_api::photos::cleanup::sweep(&pool, root.path())
        .await
        .expect("test setup or assertion");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM photo_cleanup")
            .fetch_one(&pool)
            .await
            .expect("test setup or assertion"),
        0
    );
}

#[sqlx::test]
async fn simultaneous_administrators_cannot_disable_each_other(pool: PgPool) {
    let app = app(pool.clone());
    let (first, first_cookie) = administrator(&pool).await;
    let (second, second_cookie) = administrator(&pool).await;
    let (a, b) = tokio::join!(
        action(&app, &first_cookie, second, json!({"action":"suspend"})),
        action(&app, &second_cookie, first, json!({"action":"suspend"}))
    );
    assert_eq!(
        usize::from(a == StatusCode::NO_CONTENT) + usize::from(b == StatusCode::NO_CONTENT),
        1
    );
    assert!(matches!(
        a,
        StatusCode::NO_CONTENT | StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED
    ));
    assert!(matches!(
        b,
        StatusCode::NO_CONTENT | StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM users WHERE is_site_admin AND NOT suspended"
        )
        .fetch_one(&pool)
        .await
        .expect("test setup or assertion"),
        1
    );
}

struct ScratchRoot(std::path::PathBuf);
impl ScratchRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("pv-admin-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&path).expect("scratch storage");
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for ScratchRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[sqlx::test]
async fn storage_read_errors_are_reported_and_demoted_sessions_lose_access(pool: PgPool) {
    let root = ScratchRoot::new();
    let app = app_with_storage(pool.clone(), root.path());
    let (admin, cookie) = administrator(&pool).await;
    let (owner, _) = caller(&pool).await;
    let voyage_id = voyage(&pool, owner, false).await;
    photo(&pool, owner, voyage_id, &"d".repeat(64)).await;
    std::fs::write(root.path().join("originals"), "not a directory").expect("broken storage");
    assert_eq!(
        send(&app, "GET", "/api/admin/users", Some(&cookie), Value::Null)
            .await
            .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    sqlx::query("UPDATE users SET is_site_admin=false WHERE id=$1")
        .bind(admin)
        .execute(&pool)
        .await
        .expect("remove role");
    assert_eq!(
        send(&app, "GET", "/api/admin/users", Some(&cookie), Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        action(&app, &cookie, owner, json!({"action":"promote"})).await,
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test]
async fn development_login_applies_initial_admin_emails(pool: PgPool) {
    let mut config = Config::load().expect("config");
    config.auth.dev_login = true;
    config.auth.initial_admins = pinvoyager_api::initial_admin::InitialAdmins::parse_args([
        std::ffi::OsString::from("--initial-admin-email=bootstrap@example.test"),
        std::ffi::OsString::from("--initial-admin-email=second@example.test"),
    ])
    .expect("arguments")
    .expect("run");
    let configured = |config| {
        http::router(AppState::new(
            pool.clone(),
            Arc::new(config),
            ProviderRegistry::empty(),
            VariantQueue::detached(),
        ))
    };
    let app = configured(config);

    // Both new and returning accounts receive the role on the issued session.
    for subject in ["bootstrap@EXAMPLE.test", "second@example.test"] {
        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/auth/dev/login")
                        .header("content-type", "application/json")
                        .body(Body::from(json!({"subject":subject}).to_string()))
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK);
            let cookie = response.headers()["set-cookie"]
                .to_str()
                .expect("cookie")
                .split(';')
                .next()
                .expect("cookie pair")
                .to_owned();
            let (status, me) = send(&app, "GET", "/api/me", Some(&cookie), Value::Null).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(me["user"]["is_site_admin"], true);
            assert_eq!(
                send(&app, "GET", "/api/admin/users", Some(&cookie), Value::Null)
                    .await
                    .0,
                StatusCode::OK
            );
            // Removing the role does not disable the configured grant on next sign-in.
            sqlx::query("UPDATE users SET is_site_admin=false WHERE id=$1")
                .bind(
                    me["user"]["id"]
                        .as_str()
                        .expect("id")
                        .parse::<Uuid>()
                        .expect("UUID"),
                )
                .execute(&pool)
                .await
                .expect("remove role");
        }
    }

    // Only the subject is a forged email input; extra request claims grant nothing.
    for subject in [
        "ordinary@example.test",
        "Bootstrap@example.test",
        "opaque-subject",
    ] {
        let (status, body) = send(
            &app,
            "POST",
            "/auth/dev/login",
            None,
            json!({"subject":subject,"email":"bootstrap@example.test","email_verified":true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let id: Uuid = body["user_id"].as_str().expect("id").parse().expect("UUID");
        assert!(
            !sqlx::query_scalar::<_, bool>("SELECT is_site_admin FROM users WHERE id=$1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .expect("role")
        );
    }

    let (status, body) = send(
        &app,
        "POST",
        "/auth/dev/login",
        None,
        json!({"subject":"second@example.test"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id: Uuid = body["user_id"].as_str().expect("id").parse().expect("UUID");
    // Existing-account sign-ins serialize promotion without upgrading shared locks.
    sqlx::query("UPDATE users SET is_site_admin=false WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .expect("remove role");
    let concurrent = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(
            send(
                &app,
                "POST",
                "/auth/dev/login",
                None,
                json!({"subject":"second@example.test"})
            ),
            send(
                &app,
                "POST",
                "/auth/dev/login",
                None,
                json!({"subject":"second@example.test"})
            )
        )
    })
    .await
    .expect("concurrent promotions finish");
    assert_eq!(concurrent.0.0, StatusCode::OK);
    assert_eq!(concurrent.1.0, StatusCode::OK);

    // Clearing the startup list preserves existing grants and stops new ones.
    let mut config = Config::load().expect("config");
    config.auth.dev_login = true;
    let unconfigured = configured(config);
    assert_eq!(
        send(
            &unconfigured,
            "POST",
            "/auth/dev/login",
            None,
            json!({"subject":"second@example.test"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT is_site_admin FROM users WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("retained role")
    );
    sqlx::query("UPDATE users SET is_site_admin=false WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .expect("remove role");
    assert_eq!(
        send(
            &unconfigured,
            "POST",
            "/auth/dev/login",
            None,
            json!({"subject":"second@example.test"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT is_site_admin FROM users WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("no grant")
    );

    sqlx::query("UPDATE users SET suspended=true WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .expect("suspend");
    let sessions_before: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("sessions");
    assert_eq!(
        send(
            &app,
            "POST",
            "/auth/dev/login",
            None,
            json!({"subject":"second@example.test"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT is_site_admin FROM users WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("suspended role")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sessions WHERE user_id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("sessions"),
        sessions_before
    );
}
