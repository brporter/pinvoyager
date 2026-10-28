#[path = "support/native_client.rs"]
mod client;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use client::{Client, body, id, revision};
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

#[sqlx::test]
async fn creates_are_idempotent_and_receipts_survive_destination_deletion(pool: PgPool) {
    let client = Client::new(pool).await;
    let key = Uuid::now_v7();
    let payload = json!({"title":"Saved once","description":"Private","granularity":"km1"});
    let first = client
        .request(
            "POST",
            "/api/voyages",
            Some(payload.clone()),
            Some(key),
            None,
        )
        .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let voyage = body(first).await;
    let second = client
        .request(
            "POST",
            "/api/voyages",
            Some(payload.clone()),
            Some(key),
            None,
        )
        .await;
    assert_eq!(second.status(), StatusCode::CREATED);
    assert_eq!(body(second).await, voyage);
    let mismatch = client
        .request(
            "POST",
            "/api/voyages",
            Some(json!({"title":"Different"})),
            Some(key),
            None,
        )
        .await;
    assert_eq!(mismatch.status(), StatusCode::CONFLICT);
    let receipt = body(
        client
            .request(
                "GET",
                &format!("/api/sync/operations/{key}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(receipt["status"], "applied");
    assert_eq!(receipt["target_id"], voyage["id"]);
    assert_eq!(
        client
            .request(
                "DELETE",
                &format!("/api/voyages/{}", id(&voyage)),
                None,
                Some(Uuid::now_v7()),
                Some(revision(&voyage))
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let replay = client
        .request("POST", "/api/voyages", Some(payload), Some(key), None)
        .await;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(body(replay).await["status"], "deleted");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM voyages WHERE owner_id=$1")
        .bind(client.user_id)
        .fetch_one(&client.pool)
        .await
        .expect("count");
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn native_mutations_require_keys_and_revisions_and_web_writers_advance_them(pool: PgPool) {
    let client = Client::new(pool).await;
    assert_eq!(
        client
            .request(
                "POST",
                "/api/voyages",
                Some(json!({"title":"unsafe"})),
                None,
                None
            )
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let voyage = client.create().await;
    let path = format!("/api/voyages/{}", id(&voyage));
    assert_eq!(
        client
            .request(
                "PATCH",
                &path,
                Some(json!({"title":"no revision"})),
                Some(Uuid::now_v7()),
                None
            )
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let key = Uuid::now_v7();
    let changed = client
        .request(
            "PATCH",
            &path,
            Some(json!({"title":"Changed"})),
            Some(key),
            Some(revision(&voyage)),
        )
        .await;
    assert_eq!(changed.status(), StatusCode::OK);
    let changed = body(changed).await;
    assert_ne!(revision(&changed), revision(&voyage));
    assert_eq!(
        client
            .request(
                "PATCH",
                &path,
                Some(json!({"title":"Overwrite"})),
                Some(Uuid::now_v7()),
                Some(revision(&voyage))
            )
            .await
            .status(),
        StatusCode::PRECONDITION_FAILED
    );
    let replay = client
        .request(
            "PATCH",
            &path,
            Some(json!({"title":"Changed"})),
            Some(key),
            Some(revision(&voyage)),
        )
        .await;
    assert_eq!(
        replay.status(),
        StatusCode::OK,
        "a receipt is checked before the old precondition"
    );
    let browser = pinvoyager_api::auth::session::create(
        &client.pool,
        client.user_id,
        &pinvoyager_api::config::SessionConfig::default(),
    )
    .await
    .expect("browser token");
    let request = Request::builder()
        .method("PATCH")
        .uri(&path)
        .header(header::COOKIE, format!("pv_session={}", browser.expose()))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{\"description\":\"Website edit\"}"))
        .expect("request");
    assert_eq!(
        client
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router")
            .status(),
        StatusCode::OK
    );
    let current = body(client.request("GET", &path, None, None, None).await).await;
    assert_ne!(revision(&current), revision(&changed));
    assert_eq!(
        client
            .request(
                "PATCH",
                &path,
                Some(json!({"description":"Old phone edit"})),
                Some(Uuid::now_v7()),
                Some(revision(&changed))
            )
            .await
            .status(),
        StatusCode::PRECONDITION_FAILED
    );
}

#[sqlx::test]
async fn upload_cancellation_is_durable_and_a_deleted_upload_never_returns(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let voyage_id = id(&voyage);
    let bytes = client::jpeg();
    let canceled = Uuid::now_v7();
    let path = format!("/api/voyages/{voyage_id}/uploads/{canceled}");
    assert_eq!(
        client
            .request(
                "DELETE",
                &path,
                None,
                Some(Uuid::now_v7()),
                Some(revision(&voyage))
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        body(client.request("GET", &path, None, None, None).await).await["status"],
        "canceled"
    );
    assert!(
        !client
            .upload(voyage_id, canceled, Some("draft"), &bytes)
            .await
            .status()
            .is_success()
    );
    let before_upload = body(
        client
            .request(
                "GET",
                &format!("/api/voyages/{voyage_id}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    let key = Uuid::now_v7();
    let accepted = client.upload(voyage_id, key, Some("draft"), &bytes).await;
    assert_eq!(accepted.status(), StatusCode::CREATED);
    let accepted = body(accepted).await;
    let photo_id: Uuid = accepted["photos"][0]["id"]
        .as_str()
        .expect("photo")
        .parse()
        .expect("UUID");
    let receipt = body(
        client
            .request(
                "GET",
                &format!("/api/voyages/{voyage_id}/uploads/{key}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(receipt["previous_revision"], before_upload["revision"]);
    assert_eq!(receipt["status"], "accepted");
    assert_eq!(receipt["photo_id"], photo_id.to_string());
    let current = body(
        client
            .request(
                "GET",
                &format!("/api/voyages/{voyage_id}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(
        client
            .request(
                "DELETE",
                &format!("/api/voyages/{voyage_id}/photos/{photo_id}"),
                None,
                Some(Uuid::now_v7()),
                Some(revision(&current))
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        body(
            client
                .request(
                    "GET",
                    &format!("/api/voyages/{voyage_id}/uploads/{key}"),
                    None,
                    None,
                    None
                )
                .await
        )
        .await["status"],
        "deleted"
    );
    assert!(
        !client
            .upload(voyage_id, key, Some("draft"), &bytes)
            .await
            .status()
            .is_success()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM photos WHERE voyage_id=$1")
        .bind(voyage_id)
        .fetch_one(&client.pool)
        .await
        .expect("photos");
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn upload_visibility_is_explicit_and_complete_order_rejects_omissions(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let voyage_id = id(&voyage);
    let bytes = client::jpeg();
    assert_eq!(
        client
            .upload(voyage_id, Uuid::now_v7(), None, &bytes)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .upload(voyage_id, Uuid::now_v7(), Some("public"), &bytes)
            .await
            .status(),
        StatusCode::PRECONDITION_FAILED
    );
    let mut photos = Vec::new();
    for _ in 0..3 {
        let accepted = body(
            client
                .upload(voyage_id, Uuid::now_v7(), Some("draft"), &bytes)
                .await,
        )
        .await;
        photos.push(accepted["photos"][0]["id"].clone());
    }
    let path = format!("/api/voyages/{voyage_id}");
    let current = body(client.request("GET", &path, None, None, None).await).await;
    let order_path = format!("{path}/photo-order");
    assert_eq!(
        client
            .request(
                "PUT",
                &order_path,
                Some(json!({"photo_ids":photos[..2]})),
                Some(Uuid::now_v7()),
                Some(revision(&current))
            )
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let desired = photos.into_iter().rev().collect::<Vec<_>>();
    let ordered = client
        .request(
            "PUT",
            &order_path,
            Some(json!({"photo_ids":desired})),
            Some(Uuid::now_v7()),
            Some(revision(&current)),
        )
        .await;
    assert_eq!(ordered.status(), StatusCode::OK);
    let final_voyage = body(client.request("GET", &path, None, None, None).await).await;
    assert_eq!(
        final_voyage["photos"]
            .as_array()
            .expect("photos")
            .iter()
            .map(|photo| photo["id"].clone())
            .collect::<Vec<_>>(),
        desired
    );
}

#[sqlx::test]
async fn mutation_headers_are_unambiguous_and_rejection_receipts_are_durable(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let path = format!("/api/voyages/{}", id(&voyage));
    for (name, value) in [
        ("If-Match", "\"v:1\""),
        ("Idempotency-Key", "01999999-9999-7999-8999-999999999999"),
    ] {
        let mut request = Request::builder()
            .method("PATCH")
            .uri(&path)
            .header("authorization", format!("Bearer {}", client.token))
            .header("content-type", "application/json");
        request = request
            .header("Idempotency-Key", Uuid::now_v7().to_string())
            .header("If-Match", format!("\"v:{}\"", revision(&voyage)))
            .header(name, value);
        assert_eq!(
            client
                .router
                .clone()
                .oneshot(
                    request
                        .body(Body::from("{\"title\":\"ambiguous\"}"))
                        .expect("request")
                )
                .await
                .expect("router")
                .status(),
            StatusCode::BAD_REQUEST,
            "{name}"
        );
    }
    let key = Uuid::now_v7();
    let payload = json!({"title":"","published":true});
    let response = client
        .request(
            "PATCH",
            &path,
            Some(payload.clone()),
            Some(key),
            Some(revision(&voyage)),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let recorded = body(response).await;
    let receipt = body(
        client
            .request(
                "GET",
                &format!("/api/sync/operations/{key}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(receipt["status"], "rejected");
    assert!(receipt["message"].is_string());
    assert_eq!(
        body(
            client
                .request(
                    "PATCH",
                    &path,
                    Some(payload),
                    Some(key),
                    Some(revision(&voyage))
                )
                .await
        )
        .await,
        recorded
    );
    for invalid in [
        "17",
        "W/\"v:1\"",
        "\"v:-1\"",
        "\"v:\"",
        "\"v:999999999999999999999999999999\"",
    ] {
        let request = Request::builder()
            .method("PATCH")
            .uri(&path)
            .header("authorization", format!("Bearer {}", client.token))
            .header("content-type", "application/json")
            .header("Idempotency-Key", Uuid::now_v7().to_string())
            .header("If-Match", invalid)
            .body(Body::from("{\"title\":\"Invalid\"}"))
            .expect("request");
        assert_eq!(
            client
                .router
                .clone()
                .oneshot(request)
                .await
                .expect("router")
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[sqlx::test]
async fn upload_key_cancellation_remains_valid_after_its_own_upload_advanced_the_revision(
    pool: PgPool,
) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let key = Uuid::now_v7();
    assert_eq!(
        client
            .upload(id(&voyage), key, Some("draft"), &client::jpeg())
            .await
            .status(),
        StatusCode::CREATED
    );
    let path = format!("/api/voyages/{}/uploads/{key}", id(&voyage));
    let before = body(
        client
            .request(
                "GET",
                &format!("/api/voyages/{}", id(&voyage)),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    let cancellation = Uuid::now_v7();
    assert_eq!(
        client
            .request(
                "DELETE",
                &path,
                None,
                Some(cancellation),
                Some(revision(&voyage))
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let upload_receipt = body(client.request("GET", &path, None, None, None).await).await;
    assert_eq!(upload_receipt["status"], "canceled");
    assert_eq!(upload_receipt["previous_revision"], before["revision"]);
    let operation_receipt = body(
        client
            .request(
                "GET",
                &format!("/api/sync/operations/{cancellation}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(operation_receipt["previous_revision"], before["revision"]);
    assert_eq!(operation_receipt["revision"], upload_receipt["revision"]);
    assert_eq!(
        client
            .upload(id(&voyage), key, Some("draft"), &client::jpeg())
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM photos WHERE voyage_id=$1")
        .bind(id(&voyage))
        .fetch_one(&client.pool)
        .await
        .expect("photos");
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn native_upload_rejects_additional_or_incomplete_parts_before_accepting_any_photo(
    pool: PgPool,
) {
    let client = Client::new(pool).await;
    let voyage = id(&client.create().await);
    for (files, complete, keyed) in [(1, true, false), (2, true, true), (1, false, true)] {
        let key = Uuid::now_v7();
        let mut bytes = Vec::new();
        for _ in 0..files {
            bytes.extend_from_slice(b"--native\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n");
            bytes.extend_from_slice(&client::jpeg());
            bytes.extend_from_slice(b"\r\n");
        }
        bytes.extend_from_slice(if complete {
            b"--native--\r\n"
        } else {
            b"--native\r\n"
        });
        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/api/voyages/{voyage}/photos"))
            .header(header::AUTHORIZATION, format!("Bearer {}", client.token))
            .header(header::CONTENT_TYPE, "multipart/form-data; boundary=native")
            .header("X-PinVoyager-Upload-Visibility", "draft");
        if keyed {
            request = request.header("Idempotency-Key", key.to_string());
        }
        let response = client
            .router
            .clone()
            .oneshot(request.body(Body::from(bytes)).expect("multipart"))
            .await
            .expect("router");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM photos WHERE voyage_id=$1")
            .bind(voyage)
            .fetch_one(&client.pool)
            .await
            .expect("photos");
        assert_eq!(
            count, 0,
            "no native photo is accepted until the complete one-file request is validated"
        );
        let receipt = body(
            client
                .request(
                    "GET",
                    &format!("/api/voyages/{voyage}/uploads/{key}"),
                    None,
                    None,
                    None,
                )
                .await,
        )
        .await;
        assert_eq!(receipt["status"], "not_seen");
    }
}

#[sqlx::test]
async fn conditional_legacy_order_and_direct_photo_deletion_share_revisions_and_receipts(
    pool: PgPool,
) {
    let client = Client::new(pool.clone()).await;
    let stranger = Client::new(pool).await;
    let voyage = id(&client.create().await);
    let uploaded = body(
        client
            .upload(voyage, Uuid::now_v7(), Some("draft"), &client::jpeg())
            .await,
    )
    .await;
    let photo: Uuid = uploaded["photos"][0]["id"]
        .as_str()
        .expect("photo")
        .parse()
        .expect("UUID");
    let detail = body(
        client
            .request("GET", &format!("/api/voyages/{voyage}"), None, None, None)
            .await,
    )
    .await;
    let ordered = client
        .request(
            "PATCH",
            &format!("/api/voyages/{voyage}/photos/order"),
            Some(json!({"photo_ids":[photo]})),
            Some(Uuid::now_v7()),
            Some(revision(&detail)),
        )
        .await;
    assert_eq!(ordered.status(), StatusCode::OK);
    let ordered = body(ordered).await;
    assert_eq!(
        stranger
            .request(
                "DELETE",
                &format!("/api/photos/{photo}"),
                None,
                Some(Uuid::now_v7()),
                Some(revision(&ordered))
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .request(
                "DELETE",
                &format!("/api/voyages/{}/photos/{photo}", Uuid::now_v7()),
                None,
                Some(Uuid::now_v7()),
                Some(revision(&ordered))
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let key = Uuid::now_v7();
    assert_eq!(
        client
            .request(
                "DELETE",
                &format!("/api/photos/{photo}"),
                None,
                Some(key),
                Some(revision(&ordered))
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let receipt = body(
        client
            .request(
                "GET",
                &format!("/api/sync/operations/{key}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(receipt["previous_revision"], ordered["revision"]);
    assert_eq!(receipt["status"], "deleted");
    assert!(
        receipt["revision"]
            .as_str()
            .expect("revision")
            .parse::<i64>()
            .expect("number")
            > revision(&ordered).parse::<i64>().expect("number")
    );
    assert_eq!(
        stranger
            .request(
                "GET",
                &format!("/api/voyages/{voyage}/uploads/{}", Uuid::now_v7()),
                None,
                None,
                None
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn rejection_receipts_preserve_the_status_and_code_after_a_lost_response(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let key = Uuid::now_v7();
    assert_eq!(
        client
            .request(
                "PATCH",
                &format!("/api/voyages/{}", id(&voyage)),
                Some(json!({"title":"Retained client edit"})),
                Some(key),
                Some("999")
            )
            .await
            .status(),
        StatusCode::PRECONDITION_FAILED
    );
    let receipt = body(
        client
            .request(
                "GET",
                &format!("/api/sync/operations/{key}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(receipt["status"], "rejected");
    assert_eq!(receipt["response_status"], 412);
    assert_eq!(receipt["code"], "revision_conflict");
}

#[sqlx::test]
async fn legacy_upload_recovery_does_not_invent_a_revision_predecessor(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let voyage_id = id(&voyage);
    let key = Uuid::now_v7();
    let bytes = client::jpeg();
    let accepted = body(client.upload(voyage_id, key, Some("draft"), &bytes).await).await;
    let receipt_path = format!("/api/voyages/{voyage_id}/uploads/{key}");
    let original = body(client.request("GET", &receipt_path, None, None, None).await).await;
    sqlx::query("UPDATE voyages SET title='Website changed while the phone was away' WHERE id=$1")
        .bind(voyage_id)
        .execute(&client.pool)
        .await
        .expect("website edit");
    assert_eq!(
        client
            .upload(voyage_id, key, Some("draft"), &bytes)
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        body(client.request("GET", &receipt_path, None, None, None).await).await,
        original
    );
    sqlx::query("DELETE FROM upload_receipts WHERE account_id=$1 AND voyage_id=$2")
        .bind(client.user_id)
        .bind(voyage_id)
        .execute(&client.pool)
        .await
        .expect("simulate pre-receipt upload");
    let recovered = body(client.upload(voyage_id, key, Some("draft"), &bytes).await).await;
    assert_eq!(recovered, accepted);
    let receipt = body(client.request("GET", &receipt_path, None, None, None).await).await;
    assert_eq!(receipt["status"], "accepted");
    assert!(receipt.get("previous_revision").is_none());
    assert_eq!(receipt["photo_id"], accepted["photos"][0]["id"]);
    let cancellation = Uuid::now_v7();
    assert_eq!(
        client
            .request("DELETE", &receipt_path, None, Some(cancellation), None)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let first = body(client.request("GET", &receipt_path, None, None, None).await).await;
    assert_eq!(
        client
            .request("DELETE", &receipt_path, None, Some(Uuid::now_v7()), None)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let repeated = body(client.request("GET", &receipt_path, None, None, None).await).await;
    assert_eq!(
        repeated["revision"], first["revision"],
        "repeated cancellation does not mutate content twice"
    );
    assert_eq!(repeated["previous_revision"], first["revision"]);
}

#[sqlx::test]
async fn malformed_operation_keys_and_non_text_revision_headers_never_reach_mutations(
    pool: PgPool,
) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    for key in [
        axum::http::HeaderValue::from_static("not-a-uuid"),
        axum::http::HeaderValue::from_static(""),
        axum::http::HeaderValue::from_bytes(&[0xff]).expect("opaque header"),
    ] {
        let request = Request::builder()
            .method("PATCH")
            .uri(format!("/api/voyages/{}", id(&voyage)))
            .header(header::AUTHORIZATION, format!("Bearer {}", client.token))
            .header(header::CONTENT_TYPE, "application/json")
            .header("Idempotency-Key", key)
            .header(header::IF_MATCH, "\"v:1\"")
            .body(Body::from("{\"title\":\"Must not apply\"}"))
            .expect("request");
        assert_eq!(
            client
                .router
                .clone()
                .oneshot(request)
                .await
                .expect("router")
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/voyages/{}", id(&voyage)))
        .header(header::AUTHORIZATION, format!("Bearer {}", client.token))
        .header(header::CONTENT_TYPE, "application/json")
        .header("Idempotency-Key", Uuid::now_v7().to_string())
        .header(
            header::IF_MATCH,
            axum::http::HeaderValue::from_bytes(&[0xff]).expect("opaque revision"),
        )
        .body(Body::from("{\"title\":\"Must not apply\"}"))
        .expect("request");
    assert_eq!(
        client
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router")
            .status(),
        StatusCode::BAD_REQUEST
    );
    let current = body(
        client
            .request(
                "GET",
                &format!("/api/voyages/{}", id(&voyage)),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(current["title"], voyage["title"]);
}

#[sqlx::test]
async fn native_upload_pixel_limits_reject_before_photo_or_receipt_acceptance(pool: PgPool) {
    let client = Client::configured(pool, |config| config.photos.max_megapixels = 0).await;
    let voyage = client.create().await;
    let voyage_id = id(&voyage);
    let key = Uuid::now_v7();
    let response = client
        .upload(voyage_id, key, Some("draft"), &client::jpeg())
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let receipt = body(
        client
            .request(
                "GET",
                &format!("/api/voyages/{voyage_id}/uploads/{key}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    assert_eq!(receipt["status"], "not_seen");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM photos WHERE voyage_id=$1")
        .bind(voyage_id)
        .fetch_one(&client.pool)
        .await
        .expect("photo count");
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn an_incomplete_saved_receipt_never_recreates_an_already_committed_resource(pool: PgPool) {
    let client = Client::new(pool).await;
    let key = Uuid::now_v7();
    let payload =
        json!({"title":"One committed draft","description":"Receipt recovery","granularity":"km1"});
    assert_eq!(
        client
            .request(
                "POST",
                "/api/voyages",
                Some(payload.clone()),
                Some(key),
                None
            )
            .await
            .status(),
        StatusCode::CREATED
    );
    sqlx::query("UPDATE native_operations SET state='in_progress',response_status=NULL WHERE account_id=$1 AND id=$2")
        .bind(client.user_id).bind(key).execute(&client.pool).await.expect("simulate incomplete historical receipt");
    assert_eq!(
        client
            .request("POST", "/api/voyages", Some(payload), Some(key), None)
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM voyages WHERE owner_id=$1")
        .bind(client.user_id)
        .fetch_one(&client.pool)
        .await
        .expect("voyages");
    assert_eq!(count, 1);
    assert_eq!(
        body(
            client
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
        "in_progress"
    );
}
