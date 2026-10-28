#[path = "support/native_client.rs"]
mod client;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use client::{Client, body, id};
use pinvoyager_api::{
    accounts::deletion,
    auth::session::SessionToken,
    db::repo::identities::{self, Provider},
    photos::{
        cleanup,
        storage::{self, ContentHash},
    },
};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;
const CAP: &str = "status-capability-saved-before-confirmation-test-only-123456";
async fn proof(client: &Client, provider: Provider, subject: &str) -> String {
    identities::create(&client.pool, client.user_id, provider, subject)
        .await
        .expect("identity");
    let token = SessionToken::generate();
    sqlx::query("INSERT INTO deletion_proofs(token_hash,account_id,provider,subject_hash,expires_at) VALUES($1,$2,$3,$4,now()+interval '10 minutes')").bind(token.hash()).bind(client.user_id).bind(match provider {Provider::Google=>"google",Provider::Apple=>"apple",Provider::Microsoft=>"microsoft"}).bind(Sha256::digest(subject.as_bytes()).to_vec()).execute(&client.pool).await.expect("proof");
    token.expose().to_owned()
}
async fn status(client: &Client, receipt: Uuid) -> serde_json::Value {
    let request = Request::builder()
        .uri(format!("/api/account-deletions/{receipt}"))
        .header("authorization", format!("Deletion {CAP}"))
        .body(Body::empty())
        .expect("request");
    body(
        client
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router"),
    )
    .await
}
#[sqlx::test]
async fn deletion_waits_for_durable_file_cleanup_and_keeps_names_reserved(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let uploaded = body(
        client
            .upload(id(&voyage), Uuid::now_v7(), Some("draft"), &client::jpeg())
            .await,
    )
    .await;
    let photo: Uuid = uploaded["photos"][0]["id"]
        .as_str()
        .expect("photo")
        .parse()
        .expect("UUID");
    let hash: String = sqlx::query_scalar("SELECT content_hash FROM photos WHERE id=$1")
        .bind(photo)
        .fetch_one(&client.pool)
        .await
        .expect("hash");
    let original =
        storage::original_path(&client.root, &ContentHash::try_from(hash).expect("hash"));
    tokio::fs::remove_file(&original).await.expect("remove");
    tokio::fs::create_dir(&original)
        .await
        .expect("block cleanup");
    sqlx::query("INSERT INTO profile_names(name,user_id) VALUES('delete-reserved',$1)")
        .bind(client.user_id)
        .execute(&client.pool)
        .await
        .expect("name");
    let proof = proof(&client, Provider::Google, "delete-subject").await;
    let response = client
        .request(
            "POST",
            "/api/me/deletion-requests",
            Some(json!({"proof_token":proof,"status_capability":CAP})),
            Some(Uuid::now_v7()),
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let receipt = id(&body(response).await);
    assert_eq!(
        client
            .request(
                "POST",
                &format!("/api/me/deletion-requests/{receipt}/confirm"),
                None,
                None,
                None
            )
            .await
            .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(status(&client, receipt).await["status"], "deleting");
    cleanup::sweep(&client.pool, &client.root)
        .await
        .expect("cleanup sweep records retry");
    deletion::sweep(&client.pool)
        .await
        .expect("reconciliation must tolerate a failed file cleanup");
    assert_eq!(status(&client, receipt).await["status"], "deleting");
    tokio::fs::remove_dir(&original)
        .await
        .expect("unblock cleanup");
    sqlx::query("UPDATE photo_cleanup SET next_attempt_at=now()")
        .execute(&client.pool)
        .await
        .expect("retry");
    cleanup::sweep(&client.pool, &client.root)
        .await
        .expect("retry cleanup");
    deletion::sweep(&client.pool)
        .await
        .expect("reconcile completion");
    assert_eq!(status(&client, receipt).await["status"], "complete");
    let reserved: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM profile_names WHERE name='delete-reserved'")
            .fetch_one(&client.pool)
            .await
            .expect("reserved");
    assert!(reserved.is_none());
}
#[sqlx::test]
async fn expired_or_missing_apple_proof_never_deletes_an_account(pool: PgPool) {
    let client = Client::new(pool).await;
    let google = proof(&client, Provider::Google, "google-subject").await;
    let apple = proof(&client, Provider::Apple, "apple-subject").await;
    let key = Uuid::now_v7();
    let request = json!({"proof_token":google,"status_capability":CAP});
    assert_eq!(
        client
            .request(
                "POST",
                "/api/me/deletion-requests",
                Some(request),
                Some(key),
                None
            )
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let request = json!({"proof_token":google,"apple_proof_token":apple,"status_capability":CAP});
    let response = client
        .request(
            "POST",
            "/api/me/deletion-requests",
            Some(request.clone()),
            Some(key),
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let receipt = id(&body(response).await);
    assert_eq!(client.request("POST","/api/me/deletion-requests",Some(json!({"proof_token":google,"apple_proof_token":apple,"status_capability":"changed-status-capability-with-enough-characters"})),Some(key),None).await.status(),StatusCode::CONFLICT);
    sqlx::query("UPDATE account_deletions SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(receipt)
        .execute(&client.pool)
        .await
        .expect("expire");
    assert_eq!(status(&client, receipt).await["status"], "expired");
    assert_eq!(
        client
            .request(
                "POST",
                &format!("/api/me/deletion-requests/{receipt}/confirm"),
                None,
                None,
                None
            )
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .request("GET", "/api/me", None, None, None)
            .await
            .status(),
        StatusCode::OK
    );
}

#[sqlx::test]
async fn concurrent_self_deletions_cannot_remove_the_last_active_administrator(pool: PgPool) {
    let first = Client::new(pool.clone()).await;
    let second = Client::new(pool).await;
    sqlx::query("UPDATE users SET is_site_admin=true")
        .execute(&first.pool)
        .await
        .expect("administrators");
    let a = proof(&first, Provider::Google, "admin-a").await;
    let b = proof(&second, Provider::Google, "admin-b").await;
    let response = first
        .request(
            "POST",
            "/api/me/deletion-requests",
            Some(json!({"proof_token":a,"status_capability":CAP})),
            Some(Uuid::now_v7()),
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let a = id(&body(response).await);
    let response = second
        .request(
            "POST",
            "/api/me/deletion-requests",
            Some(json!({"proof_token":b,"status_capability":CAP})),
            Some(Uuid::now_v7()),
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let b = id(&body(response).await);
    let a_path = format!("/api/me/deletion-requests/{a}/confirm");
    let b_path = format!("/api/me/deletion-requests/{b}/confirm");
    // Hold both deletions after their eligibility check. With serialization, the
    // second request waits on the administrator lock instead of passing that check.
    sqlx::query("CREATE FUNCTION test_pause_user_delete() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(991331::bigint); RETURN OLD; END $$").execute(&first.pool).await.expect("barrier function");
    sqlx::query("CREATE TRIGGER test_pause_user_delete BEFORE DELETE ON users FOR EACH ROW EXECUTE FUNCTION test_pause_user_delete()").execute(&first.pool).await.expect("barrier trigger");
    let mut barrier = first.pool.begin().await.expect("barrier");
    sqlx::query("SELECT pg_advisory_xact_lock(991331::bigint)")
        .execute(&mut *barrier)
        .await
        .expect("hold deletions");
    let request = |path: &str, token: &str| {
        Request::builder()
            .method("POST")
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("request")
    };
    let a_router = first.router.clone();
    let a_request = request(&a_path, &first.token);
    let b_router = second.router.clone();
    let b_request = request(&b_path, &second.token);
    let a_task = tokio::spawn(async move { a_router.oneshot(a_request).await.expect("router") });
    let b_task = tokio::spawn(async move { b_router.oneshot(b_request).await.expect("router") });
    for _ in 0..200 {
        let waiting:i64=sqlx::query_scalar("SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database())").fetch_one(&first.pool).await.expect("waiting requests");
        if waiting >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    barrier.commit().await.expect("release barrier");
    let a = a_task.await.expect("first request");
    let b = b_task.await.expect("second request");
    let mut statuses = [a.status().as_u16(), b.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [202, 409]);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE is_site_admin AND NOT suspended")
            .fetch_one(&first.pool)
            .await
            .expect("remaining administrator");
    assert_eq!(count, 1);
}

#[sqlx::test]
async fn apple_deletion_proofs_bind_the_current_subject_and_status_capability(pool: PgPool) {
    let client = Client::new(pool.clone()).await;
    let stranger = Client::new(pool).await;
    let apple = proof(&client, Provider::Apple, "fresh-apple-subject").await;
    let key = Uuid::now_v7();
    let payload = json!({"proof_token":apple,"status_capability":CAP});
    assert_eq!(
        client
            .request(
                "POST",
                "/api/me/deletion-requests",
                Some(payload.clone()),
                None,
                None
            )
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    for capability in ["short".to_owned(), "!".repeat(43), "a".repeat(129)] {
        assert_eq!(
            client
                .request(
                    "POST",
                    "/api/me/deletion-requests",
                    Some(json!({"proof_token":apple,"status_capability":capability})),
                    Some(key),
                    None
                )
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        stranger
            .request(
                "POST",
                "/api/me/deletion-requests",
                Some(payload.clone()),
                Some(key),
                None
            )
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    sqlx::query(
        "UPDATE identities SET subject='changed-subject' WHERE user_id=$1 AND provider='apple'",
    )
    .bind(client.user_id)
    .execute(&client.pool)
    .await
    .expect("changed provider binding");
    assert_eq!(
        client
            .request(
                "POST",
                "/api/me/deletion-requests",
                Some(payload.clone()),
                Some(key),
                None
            )
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    sqlx::query(
        "UPDATE identities SET subject='fresh-apple-subject' WHERE user_id=$1 AND provider='apple'",
    )
    .bind(client.user_id)
    .execute(&client.pool)
    .await
    .expect("restore binding");
    let prepared = client
        .request(
            "POST",
            "/api/me/deletion-requests",
            Some(payload),
            Some(key),
            None,
        )
        .await;
    assert_eq!(prepared.status(), StatusCode::CREATED);
    let receipt = id(&body(prepared).await);
    let wrong = Request::builder()
        .uri(format!("/api/account-deletions/{receipt}"))
        .header("authorization", "Deletion unrelated-private-capability")
        .body(Body::empty())
        .expect("request");
    assert_eq!(
        client
            .router
            .clone()
            .oneshot(wrong)
            .await
            .expect("router")
            .status(),
        StatusCode::NOT_FOUND
    );
    let confirm = format!("/api/me/deletion-requests/{receipt}/confirm");
    sqlx::query("UPDATE identities SET subject='changed-after-preparation' WHERE user_id=$1 AND provider='apple'").bind(client.user_id).execute(&client.pool).await.expect("fresh Apple binding required");
    assert_eq!(
        client
            .request("POST", &confirm, None, None, None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .request("GET", "/api/me", None, None, None)
            .await
            .status(),
        StatusCode::OK
    );
    sqlx::query(
        "UPDATE identities SET subject='fresh-apple-subject' WHERE user_id=$1 AND provider='apple'",
    )
    .bind(client.user_id)
    .execute(&client.pool)
    .await
    .expect("restore proof binding");
    assert_eq!(
        client
            .request("POST", &confirm, None, None, None)
            .await
            .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(status(&client, receipt).await["status"], "complete");
    sqlx::query("UPDATE account_deletions SET retain_until=now()-interval '1 second' WHERE id=$1")
        .bind(receipt)
        .execute(&client.pool)
        .await
        .expect("retention expires");
    deletion::sweep(&client.pool)
        .await
        .expect("expire minimal status");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM account_deletions WHERE id=$1")
        .bind(receipt)
        .fetch_one(&client.pool)
        .await
        .expect("minimal record count");
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn deletion_storage_failures_roll_back_proof_consumption_and_account_removal(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let token = proof(&client, Provider::Google, "recoverable-deletion-owner").await;
    let key = Uuid::now_v7();
    let payload = json!({"proof_token":token,"status_capability":CAP});
    for (disable, restore) in [
        (
            "ALTER TABLE deletion_proofs RENAME TO unavailable_deletion_records",
            "ALTER TABLE unavailable_deletion_records RENAME TO deletion_proofs",
        ),
        (
            "ALTER TABLE identities RENAME TO unavailable_deletion_records",
            "ALTER TABLE unavailable_deletion_records RENAME TO identities",
        ),
        (
            "ALTER TABLE account_deletions RENAME TO unavailable_deletion_records",
            "ALTER TABLE unavailable_deletion_records RENAME TO account_deletions",
        ),
    ] {
        sqlx::query(disable)
            .execute(&client.pool)
            .await
            .expect("simulate unavailable deletion storage");
        let response = client
            .request(
                "POST",
                "/api/me/deletion-requests",
                Some(payload.clone()),
                Some(key),
                None,
            )
            .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !body(response)
                .await
                .to_string()
                .contains("unavailable_deletion")
        );
        sqlx::query(restore)
            .execute(&client.pool)
            .await
            .expect("restore deletion storage");
        let used: bool = sqlx::query_scalar("SELECT used FROM deletion_proofs WHERE token_hash=$1")
            .bind(SessionToken::from_cookie_value(&token).hash())
            .fetch_one(&client.pool)
            .await
            .expect("proof remains");
        assert!(!used);
    }
    sqlx::query("CREATE FUNCTION reject_deletion_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'private deletion diagnostics'; END $$").execute(&client.pool).await.expect("failure function");
    sqlx::query("CREATE TRIGGER reject_deletion_write BEFORE INSERT ON account_deletions FOR EACH ROW EXECUTE FUNCTION reject_deletion_write()").execute(&client.pool).await.expect("fail preparation commit");
    assert_eq!(
        client
            .request(
                "POST",
                "/api/me/deletion-requests",
                Some(payload.clone()),
                Some(key),
                None
            )
            .await
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    sqlx::query("DROP TRIGGER reject_deletion_write ON account_deletions")
        .execute(&client.pool)
        .await
        .expect("restore preparation");
    let prepared = client
        .request(
            "POST",
            "/api/me/deletion-requests",
            Some(payload),
            Some(key),
            None,
        )
        .await;
    assert_eq!(prepared.status(), StatusCode::CREATED);
    let receipt = id(&body(prepared).await);
    let confirm = format!("/api/me/deletion-requests/{receipt}/confirm");
    for (disable, restore) in [
        (
            "CREATE TRIGGER reject_deletion_write BEFORE DELETE ON users FOR EACH ROW EXECUTE FUNCTION reject_deletion_write()",
            "DROP TRIGGER reject_deletion_write ON users",
        ),
        (
            "CREATE TRIGGER reject_deletion_write BEFORE UPDATE ON account_deletions FOR EACH ROW WHEN (NEW.state IS DISTINCT FROM OLD.state) EXECUTE FUNCTION reject_deletion_write()",
            "DROP TRIGGER reject_deletion_write ON account_deletions",
        ),
    ] {
        sqlx::query(disable)
            .execute(&client.pool)
            .await
            .expect("fail confirmation commit");
        let response = client.request("POST", &confirm, None, None, None).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !body(response)
                .await
                .to_string()
                .contains("private deletion")
        );
        sqlx::query(restore)
            .execute(&client.pool)
            .await
            .expect("restore confirmation");
        assert_eq!(status(&client, receipt).await["status"], "prepared");
        assert_eq!(
            client
                .request("GET", "/api/me", None, None, None)
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            client
                .request(
                    "GET",
                    &format!("/api/voyages/{}", id(&voyage)),
                    None,
                    None,
                    None
                )
                .await
                .status(),
            StatusCode::OK
        );
    }
    sqlx::query("ALTER TABLE account_deletions RENAME TO unavailable_confirmation")
        .execute(&client.pool)
        .await
        .expect("receipt unavailable");
    assert_eq!(
        client
            .request("POST", &confirm, None, None, None)
            .await
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(status(&client, receipt).await["code"], "internal");
    sqlx::query("ALTER TABLE unavailable_confirmation RENAME TO account_deletions")
        .execute(&client.pool)
        .await
        .expect("restore receipt");
    assert_eq!(
        client
            .request("POST", &confirm, None, None, None)
            .await
            .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(status(&client, receipt).await["status"], "complete");
}

#[sqlx::test]
async fn deletion_status_rejects_duplicate_capabilities_and_cleanup_outages_remain_errors(
    pool: PgPool,
) {
    let client = Client::new(pool).await;
    let request = Request::builder()
        .uri(format!("/api/account-deletions/{}", Uuid::now_v7()))
        .header("authorization", format!("Deletion {CAP}"))
        .header("authorization", format!("Deletion {CAP}"))
        .body(Body::empty())
        .expect("ambiguous capability request");
    assert_eq!(
        client
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router")
            .status(),
        StatusCode::NOT_FOUND
    );
    for (disable, restore) in [
        (
            "ALTER TABLE account_deletions RENAME TO unavailable_cleanup",
            "ALTER TABLE unavailable_cleanup RENAME TO account_deletions",
        ),
        (
            "ALTER TABLE deletion_proofs RENAME TO unavailable_cleanup",
            "ALTER TABLE unavailable_cleanup RENAME TO deletion_proofs",
        ),
    ] {
        sqlx::query(disable)
            .execute(&client.pool)
            .await
            .expect("simulate unavailable cleanup ledger");
        assert!(deletion::sweep(&client.pool).await.is_err());
        sqlx::query(restore)
            .execute(&client.pool)
            .await
            .expect("restore cleanup ledger");
        deletion::sweep(&client.pool)
            .await
            .expect("cleanup recovers");
    }
}
