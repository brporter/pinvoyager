//! Dependency failures must remain retryable without leaking SQL or committing partial writes.
#[path = "support/native_client.rs"]
mod client;

use axum::http::StatusCode;
use client::{Client, body, id, revision};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test]
async fn unavailable_voyage_storage_never_becomes_an_empty_or_successful_response(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let voyage_id = id(&voyage);
    sqlx::query("ALTER TABLE voyages RENAME TO temporarily_unavailable_voyages")
        .execute(&client.pool)
        .await
        .expect("simulate storage outage");
    for (method, path, payload) in [
        ("GET", "/api/voyages".into(), None),
        ("GET", format!("/api/voyages/{voyage_id}"), None),
        ("GET", format!("/api/voyages/{voyage_id}/map"), None),
        (
            "GET",
            format!("/api/voyages/{voyage_id}/offline-manifest"),
            None,
        ),
        ("GET", format!("/api/profiles/{}", client.user_id), None),
        (
            "GET",
            "/api/map?west=-180&south=-80&east=180&north=80&zoom=2".into(),
            None,
        ),
        (
            "POST",
            "/api/voyages".into(),
            Some(json!({"title":"Retried draft","description":"Retained","granularity":"km1"})),
        ),
        (
            "PATCH",
            format!("/api/voyages/{voyage_id}"),
            Some(json!({"title":"Retried edit"})),
        ),
        ("DELETE", format!("/api/voyages/{voyage_id}"), None),
        (
            "PUT",
            format!("/api/voyages/{voyage_id}/photo-order"),
            Some(json!({"photo_ids":[]})),
        ),
        (
            "DELETE",
            format!("/api/voyages/{voyage_id}/uploads/{}", Uuid::now_v7()),
            None,
        ),
    ] {
        let key = Uuid::now_v7();
        let response = client
            .request(method, &path, payload, Some(key), Some(revision(&voyage)))
            .await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{method} {path}"
        );
        let error = body(response).await;
        assert_eq!(error["code"], "internal");
        assert!(!error.to_string().contains("temporarily_unavailable"));
        if method != "GET" {
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
            assert_eq!(
                receipt["status"], "not_seen",
                "retry must retain the operation key"
            );
        }
    }
    sqlx::query("ALTER TABLE temporarily_unavailable_voyages RENAME TO voyages")
        .execute(&client.pool)
        .await
        .expect("restore storage");
    let recovered = body(
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
    assert_eq!(recovered, voyage);
}

#[sqlx::test]
async fn a_failed_voyage_write_rolls_back_its_receipt_and_can_retry_the_same_request(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let voyage_id = id(&voyage);
    sqlx::raw_sql("CREATE FUNCTION fail_test_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'private database diagnostics'; END $$; CREATE TRIGGER fail_test_write BEFORE INSERT OR UPDATE OR DELETE ON voyages FOR EACH ROW EXECUTE FUNCTION fail_test_write();")
        .execute(&client.pool).await.expect("simulate write failure");
    let create_key = Uuid::now_v7();
    let edit_key = Uuid::now_v7();
    let delete_key = Uuid::now_v7();
    let create = json!({"title":"Retried draft","description":"Retained","granularity":"km1"});
    for (method, path, payload, key) in [
        (
            "POST",
            "/api/voyages".into(),
            Some(create.clone()),
            create_key,
        ),
        (
            "PATCH",
            format!("/api/voyages/{voyage_id}"),
            Some(json!({"title":"Retried edit"})),
            edit_key,
        ),
        (
            "DELETE",
            format!("/api/voyages/{voyage_id}"),
            None,
            delete_key,
        ),
    ] {
        let response = client
            .request(method, &path, payload, Some(key), Some(revision(&voyage)))
            .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !body(response)
                .await
                .to_string()
                .contains("private database")
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
        assert_eq!(receipt["status"], "not_seen");
    }
    sqlx::query("DROP TRIGGER fail_test_write ON voyages")
        .execute(&client.pool)
        .await
        .expect("restore writes");
    let saved = body(
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
    assert_eq!(saved, voyage);
    assert_eq!(
        client
            .request(
                "POST",
                "/api/voyages",
                Some(create),
                Some(create_key),
                Some(revision(&voyage))
            )
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        client
            .request(
                "PATCH",
                &format!("/api/voyages/{voyage_id}"),
                Some(json!({"title":"Retried edit"})),
                Some(edit_key),
                Some(revision(&voyage))
            )
            .await
            .status(),
        StatusCode::OK
    );
}

#[sqlx::test]
async fn unavailable_upload_receipts_or_publication_review_never_accept_an_untracked_photo(
    pool: PgPool,
) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let voyage_id = id(&voyage);
    for (disable, restore) in [
        (
            "ALTER TABLE upload_receipts RENAME TO unavailable_records",
            "ALTER TABLE unavailable_records RENAME TO upload_receipts",
        ),
        (
            "ALTER TABLE publication_intents RENAME TO unavailable_records",
            "ALTER TABLE unavailable_records RENAME TO publication_intents",
        ),
    ] {
        sqlx::query(disable)
            .execute(&client.pool)
            .await
            .expect("simulate unavailable ledger");
        let key = Uuid::now_v7();
        let response = client
            .upload(voyage_id, key, Some("draft"), &client::jpeg())
            .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !body(response)
                .await
                .to_string()
                .contains("unavailable_records")
        );
        let photos: i64 = sqlx::query_scalar("SELECT count(*) FROM photos WHERE voyage_id=$1")
            .bind(voyage_id)
            .fetch_one(&client.pool)
            .await
            .expect("photo count");
        assert_eq!(
            photos, 0,
            "a failed receipt/review lookup must roll back ingestion"
        );
        sqlx::query(restore)
            .execute(&client.pool)
            .await
            .expect("restore ledger");
    }
    let key = Uuid::now_v7();
    assert_eq!(
        client
            .upload(voyage_id, key, Some("draft"), &client::jpeg())
            .await
            .status(),
        StatusCode::CREATED
    );
}

#[sqlx::test]
async fn committing_a_receipt_is_atomic_with_its_resource_and_failed_lookup_never_means_not_seen(
    pool: PgPool,
) {
    let client = Client::new(pool).await;
    sqlx::raw_sql("CREATE FUNCTION fail_receipt_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'private receipt diagnostics'; END $$; CREATE TRIGGER fail_receipt_commit BEFORE UPDATE ON native_operations FOR EACH ROW EXECUTE FUNCTION fail_receipt_commit();")
        .execute(&client.pool).await.expect("simulate failed receipt checkpoint");
    let key = Uuid::now_v7();
    let payload = json!({"title":"Atomic receipt","description":"Saved with its receipt","granularity":"km1"});
    let response = client
        .request(
            "POST",
            "/api/voyages",
            Some(payload.clone()),
            Some(key),
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body(response).await.to_string().contains("private receipt"));
    let voyages: i64 = sqlx::query_scalar("SELECT count(*) FROM voyages WHERE owner_id=$1")
        .bind(client.user_id)
        .fetch_one(&client.pool)
        .await
        .expect("voyages");
    assert_eq!(voyages, 0);
    sqlx::query("DROP TRIGGER fail_receipt_commit ON native_operations")
        .execute(&client.pool)
        .await
        .expect("restore receipts");
    let receipt_path = format!("/api/sync/operations/{key}");
    assert_eq!(
        body(client.request("GET", &receipt_path, None, None, None).await).await["status"],
        "not_seen"
    );
    assert_eq!(
        client
            .request("POST", "/api/voyages", Some(payload), Some(key), None)
            .await
            .status(),
        StatusCode::CREATED
    );
    sqlx::query("ALTER TABLE native_operations RENAME TO unavailable_operations")
        .execute(&client.pool)
        .await
        .expect("simulate receipt read outage");
    assert_eq!(
        client
            .request("GET", &receipt_path, None, None, None)
            .await
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let response = client.request("POST","/api/voyages",Some(json!({"title":"No receipt, no write","description":"Must retry later","granularity":"km1"})),Some(Uuid::now_v7()),None).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM voyages WHERE owner_id=$1")
        .bind(client.user_id)
        .fetch_one(&client.pool)
        .await
        .expect("voyages remain");
    assert_eq!(count, 1);
    sqlx::query("ALTER TABLE unavailable_operations RENAME TO native_operations")
        .execute(&client.pool)
        .await
        .expect("restore receipt reads");
    assert_eq!(
        body(client.request("GET", &receipt_path, None, None, None).await).await["status"],
        "applied"
    );
}

#[sqlx::test]
async fn failed_block_filtering_does_not_fall_back_to_unfiltered_public_results(pool: PgPool) {
    let reader = Client::new(pool.clone()).await;
    let owner = Client::new(pool).await;
    let voyage = owner.create().await;
    let voyage_id = id(&voyage);
    sqlx::query("UPDATE voyages SET published=true,published_at=now() WHERE id=$1")
        .bind(voyage_id)
        .execute(&reader.pool)
        .await
        .expect("publish");
    sqlx::query("ALTER TABLE account_blocks RENAME TO unavailable_block_preferences")
        .execute(&reader.pool)
        .await
        .expect("simulate filtering outage");
    for path in [
        "/api/voyages".into(),
        "/api/map?west=-180&south=-80&east=180&north=80&zoom=2".into(),
        format!("/api/voyages/{voyage_id}"),
        format!("/api/voyages/{voyage_id}/map"),
        format!("/api/profiles/{}", owner.user_id),
        "/api/me/blocks".into(),
    ] {
        let response = reader.request("GET", &path, None, None, None).await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{path}"
        );
        assert!(
            !body(response)
                .await
                .to_string()
                .contains("unavailable_block")
        );
    }
    sqlx::query("ALTER TABLE unavailable_block_preferences RENAME TO account_blocks")
        .execute(&reader.pool)
        .await
        .expect("restore filtering");
    assert_eq!(
        reader
            .request(
                "GET",
                &format!("/api/voyages/{voyage_id}"),
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
async fn missing_photo_storage_cannot_look_like_empty_membership_or_authorized_images(
    pool: PgPool,
) {
    use pinvoyager_api::photos::worker::{self, VariantJob};
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let voyage_id = id(&voyage);
    let uploaded = body(
        client
            .upload(voyage_id, Uuid::now_v7(), Some("draft"), &client::jpeg())
            .await,
    )
    .await;
    let photo: Uuid = uploaded["photos"][0]["id"]
        .as_str()
        .expect("photo")
        .parse()
        .expect("UUID");
    worker::process(
        &client.pool,
        &client.config,
        &VariantJob::new(photo).with_voyage(voyage_id),
    )
    .await
    .expect("ready photo");
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
    sqlx::query("ALTER TABLE photos RENAME TO unavailable_photo_records")
        .execute(&client.pool)
        .await
        .expect("simulate metadata outage");
    for (method, path, payload) in [
        ("GET", format!("/api/voyages/{voyage_id}"), None),
        ("GET", format!("/api/voyages/{voyage_id}/map"), None),
        ("GET", format!("/api/voyages/{voyage_id}/photos"), None),
        (
            "GET",
            format!("/api/voyages/{voyage_id}/offline-manifest"),
            None,
        ),
        ("GET", format!("/api/photos/{photo}"), None),
        ("GET", format!("/api/photos/{photo}/thumb"), None),
        ("GET", format!("/api/photos/{photo}/offline/display"), None),
        ("GET", "/api/voyages?mine=true".into(), None),
        (
            "DELETE",
            format!("/api/voyages/{voyage_id}/photos/{photo}"),
            None,
        ),
        (
            "PUT",
            format!("/api/voyages/{voyage_id}/photo-order"),
            Some(json!({"photo_ids":[photo]})),
        ),
    ] {
        let response = client
            .request(
                method,
                &path,
                payload,
                Some(Uuid::now_v7()),
                Some(revision(&current)),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{method} {path}"
        );
        assert!(
            !body(response)
                .await
                .to_string()
                .contains("unavailable_photo")
        );
    }
    sqlx::query("ALTER TABLE unavailable_photo_records RENAME TO photos")
        .execute(&client.pool)
        .await
        .expect("restore metadata");
    let recovered = body(
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
    assert_eq!(recovered["photos"].as_array().expect("photos").len(), 1);
    assert_eq!(recovered["revision"], current["revision"]);
}

#[sqlx::test]
async fn failed_block_writes_keep_the_previous_choice_and_allow_the_same_key_to_retry(
    pool: PgPool,
) {
    let reader = Client::new(pool.clone()).await;
    let target = Client::new(pool).await;
    sqlx::query("CREATE FUNCTION reject_block_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'private block diagnostics'; END $$")
        .execute(&reader.pool).await.expect("failure function");
    let path = format!("/api/me/blocks/{}", target.user_id);
    for method in ["PUT", "DELETE"] {
        let trigger = if method == "PUT" {
            "CREATE TRIGGER reject_block_write BEFORE INSERT ON account_blocks FOR EACH ROW EXECUTE FUNCTION reject_block_write()"
        } else {
            "CREATE TRIGGER reject_block_write BEFORE DELETE ON account_blocks FOR EACH ROW EXECUTE FUNCTION reject_block_write()"
        };
        sqlx::query(trigger)
            .execute(&reader.pool)
            .await
            .expect("simulate block write failure");
        let key = Uuid::now_v7();
        let response = reader.request(method, &path, None, Some(key), None).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!body(response).await.to_string().contains("private block"));
        let blocks = body(
            reader
                .request("GET", "/api/me/blocks", None, None, None)
                .await,
        )
        .await;
        assert_eq!(
            blocks["user_ids"].as_array().expect("blocks").len(),
            usize::from(method == "DELETE")
        );
        let receipt = body(
            reader
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
        assert_eq!(receipt["status"], "not_seen");
        sqlx::query("DROP TRIGGER reject_block_write ON account_blocks")
            .execute(&reader.pool)
            .await
            .expect("restore writes");
        assert_eq!(
            reader
                .request(method, &path, None, Some(key), None)
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
    }
}
