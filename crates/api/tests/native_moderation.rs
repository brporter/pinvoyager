#[path = "support/native_client.rs"]
mod client;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use client::{Client, body, id};
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

async fn publish(client: &Client) -> Uuid {
    let voyage = client.create().await;
    sqlx::query("UPDATE voyages SET published=true,published_at=now() WHERE id=$1")
        .bind(id(&voyage))
        .execute(&client.pool)
        .await
        .expect("publish");
    id(&voyage)
}
#[sqlx::test]
async fn blocks_filter_before_feed_pagination_and_hide_profile_detail_and_map(pool: PgPool) {
    let reader = Client::new(pool.clone()).await;
    let visible = Client::new(pool.clone()).await;
    let hidden = Client::new(pool).await;
    let visible_voyage = publish(&visible).await;
    let hidden_voyage = publish(&hidden).await;
    let key = Uuid::now_v7();
    let response = reader
        .request(
            "PUT",
            &format!("/api/me/blocks/{}", hidden.user_id),
            None,
            Some(key),
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        reader
            .request(
                "PUT",
                &format!("/api/me/blocks/{}", hidden.user_id),
                None,
                Some(key),
                None
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let list = body(
        reader
            .request("GET", "/api/voyages?limit=1", None, None, None)
            .await,
    )
    .await;
    assert_eq!(list["voyages"][0]["id"], visible_voyage.to_string());
    for path in [
        format!("/api/profiles/{}", hidden.user_id),
        format!("/api/voyages/{hidden_voyage}"),
        format!("/api/voyages/{hidden_voyage}/map"),
    ] {
        assert_eq!(
            reader
                .request("GET", &path, None, None, None)
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    let blocks = body(
        reader
            .request("GET", "/api/me/blocks", None, None, None)
            .await,
    )
    .await;
    assert_eq!(blocks["user_ids"], json!([hidden.user_id]));
    assert_eq!(
        reader
            .request(
                "DELETE",
                &format!("/api/me/blocks/{}", hidden.user_id),
                None,
                Some(Uuid::now_v7()),
                None
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        reader
            .request(
                "GET",
                &format!("/api/voyages/{hidden_voyage}"),
                None,
                None,
                None
            )
            .await
            .status(),
        StatusCode::OK
    );
}
#[sqlx::test]
async fn reports_are_private_idempotent_and_moderator_holds_prevent_publication(pool: PgPool) {
    let reporter = Client::new(pool.clone()).await;
    let owner = Client::new(pool.clone()).await;
    let moderator = Client::new(pool).await;
    sqlx::query("UPDATE users SET is_site_admin=true WHERE id=$1")
        .bind(moderator.user_id)
        .execute(&moderator.pool)
        .await
        .expect("admin");
    let voyage = publish(&owner).await;
    let key = Uuid::now_v7();
    let report = json!({"target":format!("voyage:{voyage}"),"reason":"Harassment","details":"Please review this voyage."});
    let response = reporter
        .request(
            "POST",
            "/api/reports",
            Some(report.clone()),
            Some(key),
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let report_id = id(&body(response).await);
    assert_eq!(
        id(&body(
            reporter
                .request("POST", "/api/reports", Some(report), Some(key), None)
                .await
        )
        .await),
        report_id
    );
    assert_eq!(
        reporter
            .request("GET", "/api/admin/reports", None, None, None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let queue = body(
        moderator
            .request("GET", "/api/admin/reports?limit=1", None, None, None)
            .await,
    )
    .await;
    assert_eq!(queue["reports"][0]["id"], report_id.to_string());
    let detail = body(
        moderator
            .request(
                "GET",
                &format!("/api/admin/reports/{report_id}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(detail["details"], "Please review this voyage.");
    assert_eq!(
        moderator
            .request(
                "POST",
                &format!("/api/admin/reports/{report_id}/actions"),
                Some(json!({"action":"hold","note":"Under review"})),
                None,
                None
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let anonymous = Request::builder()
        .uri(format!("/api/voyages/{voyage}"))
        .body(Body::empty())
        .expect("request");
    assert_eq!(
        reporter
            .router
            .clone()
            .oneshot(anonymous)
            .await
            .expect("router")
            .status(),
        StatusCode::NOT_FOUND
    );
    let current = body(
        owner
            .request("GET", &format!("/api/voyages/{voyage}"), None, None, None)
            .await,
    )
    .await;
    assert_eq!(current["published"], false);
    assert_eq!(
        owner
            .request(
                "PATCH",
                &format!("/api/voyages/{voyage}"),
                Some(json!({"published":true})),
                Some(Uuid::now_v7()),
                current["revision"].as_str()
            )
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        moderator
            .request(
                "POST",
                &format!("/api/admin/reports/{report_id}/actions"),
                Some(json!({"action":"release","note":"Review complete"})),
                None,
                None
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let current = body(
        owner
            .request("GET", &format!("/api/voyages/{voyage}"), None, None, None)
            .await,
    )
    .await;
    assert_eq!(
        owner
            .request(
                "PATCH",
                &format!("/api/voyages/{voyage}"),
                Some(json!({"published":true})),
                Some(Uuid::now_v7()),
                current["revision"].as_str()
            )
            .await
            .status(),
        StatusCode::OK
    );
}

#[sqlx::test]
async fn report_limits_validation_and_target_resolution_are_enforced(pool: PgPool) {
    let reader = Client::new(pool.clone()).await;
    let owner = Client::new(pool).await;
    let voyage = publish(&owner).await;
    let target = format!("voyage:{voyage}");
    for input in [
        json!({"target":target,"reason":"","details":""}),
        json!({"target":"invalid","reason":"Spam","details":""}),
        json!({"target":"unknown:01999999-9999-7999-8999-999999999999","reason":"Spam","details":""}),
        json!({"target":target,"reason":"a".repeat(101),"details":""}),
        json!({"target":target,"reason":"Spam","details":"a".repeat(2001)}),
    ] {
        assert_eq!(
            reader
                .request(
                    "POST",
                    "/api/reports",
                    Some(input),
                    Some(Uuid::now_v7()),
                    None
                )
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        reader
            .request(
                "PUT",
                &format!("/api/me/blocks/{}", reader.user_id),
                None,
                Some(Uuid::now_v7()),
                None
            )
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        reader
            .request(
                "PUT",
                &format!("/api/me/blocks/{}", Uuid::now_v7()),
                None,
                Some(Uuid::now_v7()),
                None
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    for index in 0..20 {
        let report = json!({"target":if index%2==0 {target.clone()}else{format!("user:{}",owner.user_id)},"reason":"Spam","details":"Review requested"});
        assert_eq!(
            reader
                .request(
                    "POST",
                    "/api/reports",
                    Some(report),
                    Some(Uuid::now_v7()),
                    None
                )
                .await
                .status(),
            StatusCode::CREATED
        );
    }
    let operation = Uuid::now_v7();
    let response = reader
        .request(
            "POST",
            "/api/reports",
            Some(json!({"target":target,"reason":"Spam","details":"Later"})),
            Some(operation),
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(response.headers().contains_key("retry-after"));
    let receipt = body(
        reader
            .request(
                "GET",
                &format!("/api/sync/operations/{operation}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(
        receipt["status"], "not_seen",
        "temporary limits must not become permanent rejection receipts"
    );
}

#[sqlx::test]
async fn moderator_queue_pages_and_rejects_invalid_or_missing_actions(pool: PgPool) {
    let reporter = Client::new(pool.clone()).await;
    let moderator = Client::new(pool.clone()).await;
    let target = Client::new(pool).await;
    sqlx::query("UPDATE users SET is_site_admin=true WHERE id=$1")
        .bind(moderator.user_id)
        .execute(&moderator.pool)
        .await
        .expect("admin");
    let mut ids = Vec::new();
    for index in 0..3 {
        let response = reporter.request("POST","/api/reports",Some(json!({"target":format!("user:{}",target.user_id),"reason":"Spam","details":format!("Report {index}")})),Some(Uuid::now_v7()),None).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        ids.push(id(&body(response).await));
    }
    let first = body(
        moderator
            .request("GET", "/api/admin/reports?limit=2", None, None, None)
            .await,
    )
    .await;
    assert_eq!(first["reports"].as_array().expect("reports").len(), 2);
    let before = first["next_before"].as_str().expect("cursor");
    let next = body(
        moderator
            .request(
                "GET",
                &format!("/api/admin/reports?limit=2&before={before}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(next["reports"].as_array().expect("reports").len(), 1);
    assert!(next["next_before"].is_null());
    assert_eq!(next["reports"][0]["id"], ids[0].to_string());
    for limit in [0, 101] {
        assert_eq!(
            moderator
                .request(
                    "GET",
                    &format!("/api/admin/reports?limit={limit}"),
                    None,
                    None,
                    None
                )
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let path = format!("/api/admin/reports/{}/actions", ids[0]);
    for input in [
        json!({"action":"unknown","note":"Review"}),
        json!({"action":"hold","note":"  "}),
        json!({"action":"hold","note":"a".repeat(2001)}),
    ] {
        assert_eq!(
            moderator
                .request("POST", &path, Some(input), None, None)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        moderator
            .request(
                "GET",
                &format!("/api/admin/reports/{}", Uuid::now_v7()),
                None,
                None,
                None
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        moderator
            .request(
                "POST",
                &format!("/api/admin/reports/{}/actions", Uuid::now_v7()),
                Some(json!({"action":"dismiss","note":"Reviewed"})),
                None,
                None
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        reporter
            .request(
                "GET",
                &format!("/api/admin/reports/{}", ids[0]),
                None,
                None,
                None
            )
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        reporter
            .request(
                "POST",
                &path,
                Some(json!({"action":"dismiss","note":"Reviewed"})),
                None,
                None
            )
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        moderator
            .request(
                "POST",
                &path,
                Some(json!({"action":"dismiss","note":"No violation"})),
                None,
                None
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let detail = body(
        moderator
            .request(
                "GET",
                &format!("/api/admin/reports/{}", ids[0]),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(detail["status"], "resolved");
    assert_eq!(detail["moderator_note"], "No violation");
    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(target.user_id)
        .execute(&target.pool)
        .await
        .expect("deleted target");
    assert_eq!(
        moderator
            .request(
                "POST",
                &path,
                Some(json!({"action":"hold","note":"Target disappeared"})),
                None,
                None
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn photo_reports_resolve_public_membership_and_reject_missing_or_private_targets(
    pool: PgPool,
) {
    let reporter = Client::new(pool.clone()).await;
    let owner = Client::new(pool).await;
    let voyage = owner.create().await;
    let voyage_id = id(&voyage);
    let accepted = body(
        owner
            .upload(voyage_id, Uuid::now_v7(), Some("draft"), &client::jpeg())
            .await,
    )
    .await;
    let photo = accepted["photos"][0]["id"].as_str().expect("photo");
    for target in [
        format!("photo:{photo}"),
        format!("photo:{}", Uuid::now_v7()),
        format!("user:{}", Uuid::now_v7()),
        format!("voyage:{}", Uuid::now_v7()),
    ] {
        assert_eq!(reporter.request("POST","/api/reports",Some(json!({"target":target,"reason":"privacy_concern","details":"Please review"})),Some(Uuid::now_v7()),None).await.status(),StatusCode::NOT_FOUND);
    }
    assert_eq!(
        reporter
            .request(
                "POST",
                "/api/reports",
                Some(json!({"target":"photo:invalid","reason":"spam","details":""})),
                Some(Uuid::now_v7()),
                None
            )
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    sqlx::query("UPDATE voyages SET published=true,published_at=now() WHERE id=$1")
        .bind(voyage_id)
        .execute(&owner.pool)
        .await
        .expect("publish");
    let response = reporter.request("POST","/api/reports",Some(json!({"target":format!("photo:{photo}"),"reason":"privacy_concern","details":"Public photo needs review"})),Some(Uuid::now_v7()),None).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let report = body(response).await;
    let saved: (Uuid, Option<Uuid>) =
        sqlx::query_as("SELECT target_owner_id,voyage_id FROM content_reports WHERE id=$1")
            .bind(id(&report))
            .fetch_one(&owner.pool)
            .await
            .expect("saved report");
    assert_eq!(saved, (owner.user_id, Some(voyage_id)));
}

#[sqlx::test]
async fn failed_moderation_writes_never_apply_a_hold_without_its_review_record(pool: PgPool) {
    let reporter = Client::new(pool.clone()).await;
    let owner = Client::new(pool.clone()).await;
    let moderator = Client::new(pool).await;
    sqlx::query("UPDATE users SET is_site_admin=true WHERE id=$1")
        .bind(moderator.user_id)
        .execute(&moderator.pool)
        .await
        .expect("moderator");
    let voyage = publish(&owner).await;
    let report = body(reporter.request("POST","/api/reports",Some(json!({"target":format!("voyage:{voyage}"),"reason":"privacy_concern","details":"Review this public voyage"})),Some(Uuid::now_v7()),None).await).await;
    let report_id = id(&report);
    let path = format!("/api/admin/reports/{report_id}/actions");
    sqlx::query("CREATE FUNCTION reject_moderation_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'private moderation diagnostics'; END $$").execute(&moderator.pool).await.expect("failure function");
    for (disable, restore) in [
        (
            "CREATE TRIGGER reject_moderation_write BEFORE UPDATE ON voyages FOR EACH ROW EXECUTE FUNCTION reject_moderation_write()",
            "DROP TRIGGER reject_moderation_write ON voyages",
        ),
        (
            "CREATE TRIGGER reject_moderation_write BEFORE UPDATE ON content_reports FOR EACH ROW EXECUTE FUNCTION reject_moderation_write()",
            "DROP TRIGGER reject_moderation_write ON content_reports",
        ),
    ] {
        sqlx::query(disable)
            .execute(&moderator.pool)
            .await
            .expect("simulate failed review commit");
        let response = moderator
            .request(
                "POST",
                &path,
                Some(json!({"action":"hold","note":"Reviewed by moderator"})),
                None,
                None,
            )
            .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !body(response)
                .await
                .to_string()
                .contains("private moderation")
        );
        sqlx::query(restore)
            .execute(&moderator.pool)
            .await
            .expect("restore review writes");
        let status: String = sqlx::query_scalar("SELECT status FROM content_reports WHERE id=$1")
            .bind(report_id)
            .fetch_one(&moderator.pool)
            .await
            .expect("report remains");
        assert_eq!(status, "open");
        let visible: (bool, bool) =
            sqlx::query_as("SELECT published,moderation_held FROM voyages WHERE id=$1")
                .bind(voyage)
                .fetch_one(&moderator.pool)
                .await
                .expect("voyage remains");
        assert_eq!(visible, (true, false));
    }
    sqlx::query("ALTER TABLE content_reports RENAME TO unavailable_review_records")
        .execute(&moderator.pool)
        .await
        .expect("review read outage");
    for path in [
        "/api/admin/reports".into(),
        format!("/api/admin/reports/{report_id}"),
    ] {
        assert_eq!(
            moderator
                .request("GET", &path, None, None, None)
                .await
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    let submission = json!({"target":format!("user:{}",owner.user_id),"reason":"spam","details":"Retry after review storage recovers"});
    let key = Uuid::now_v7();
    assert_eq!(
        reporter
            .request(
                "POST",
                "/api/reports",
                Some(submission.clone()),
                Some(key),
                None
            )
            .await
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    sqlx::query("ALTER TABLE unavailable_review_records RENAME TO content_reports")
        .execute(&moderator.pool)
        .await
        .expect("restore review reads");
    assert_eq!(
        reporter
            .request("POST", "/api/reports", Some(submission), Some(key), None)
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        moderator
            .request(
                "POST",
                &path,
                Some(json!({"action":"hold","note":"Reviewed by moderator"})),
                None,
                None
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test]
async fn a_failed_report_insert_remains_retryable_without_an_acknowledgment(pool: PgPool) {
    let reporter = Client::new(pool.clone()).await;
    let target = Client::new(pool).await;
    sqlx::raw_sql("CREATE FUNCTION reject_report_insert() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'private report diagnostics'; END $$; CREATE TRIGGER reject_report_insert BEFORE INSERT ON content_reports FOR EACH ROW EXECUTE FUNCTION reject_report_insert();")
        .execute(&reporter.pool).await.expect("simulate report write failure");
    let key = Uuid::now_v7();
    let payload = json!({"target":format!("user:{}",target.user_id),"reason":"spam","details":"Review requested"});
    let response = reporter
        .request(
            "POST",
            "/api/reports",
            Some(payload.clone()),
            Some(key),
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body(response).await.to_string().contains("private report"));
    assert_eq!(
        body(
            reporter
                .request(
                    "GET",
                    &format!("/api/sync/operations/{key}"),
                    None,
                    None,
                    None
                )
                .await
        )
        .await["status"],
        "not_seen"
    );
    sqlx::query("DROP TRIGGER reject_report_insert ON content_reports")
        .execute(&reporter.pool)
        .await
        .expect("restore report writes");
    assert_eq!(
        reporter
            .request("POST", "/api/reports", Some(payload), Some(key), None)
            .await
            .status(),
        StatusCode::CREATED
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM content_reports WHERE reporter_id=$1")
            .bind(reporter.user_id)
            .fetch_one(&reporter.pool)
            .await
            .expect("report count");
    assert_eq!(count, 1);
}
