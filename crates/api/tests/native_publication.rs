#[path = "support/native_client.rs"]
mod client;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use client::{Client, body, id, revision};
use pinvoyager_api::photos::worker::{self, VariantJob};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

fn manifest(voyage: &Value, photos: Vec<Value>) -> Value {
    let metadata = serde_json::to_vec(&json!([
        voyage["title"],
        voyage["description"],
        voyage["granularity"]
    ]))
    .expect("metadata");
    json!({"metadata_digest":hex::encode(Sha256::digest(metadata)),"expires_at":(Utc::now()+Duration::hours(1)).to_rfc3339(),"photos":photos})
}

async fn current(client: &Client, voyage: Uuid) -> Value {
    body(
        client
            .request("GET", &format!("/api/voyages/{voyage}"), None, None, None)
            .await,
    )
    .await
}

async fn register(client: &Client, voyage: &Value, photos: Vec<Value>) -> Value {
    let response = client
        .request(
            "POST",
            &format!("/api/voyages/{}/publication-intents", id(voyage)),
            Some(manifest(voyage, photos)),
            Some(Uuid::now_v7()),
            Some(revision(voyage)),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    body(response).await
}

async fn status(client: &Client, voyage: Uuid, intent: Uuid) -> Value {
    body(
        client
            .request(
                "GET",
                &format!("/api/voyages/{voyage}/publication-intents/{intent}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await
}

#[sqlx::test]
async fn the_real_image_worker_publishes_only_after_every_reviewed_variant_is_ready(pool: PgPool) {
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
    let voyage = current(&client, voyage_id).await;
    let intent = register(&client, &voyage, vec![json!({"photo_id":photo})]).await;
    assert_eq!(intent["status"], "processing");
    assert_eq!(current(&client, voyage_id).await["published"], false);
    worker::process(
        &client.pool,
        &client.config,
        &VariantJob::new(photo).with_voyage(voyage_id),
    )
    .await
    .expect("conversion");
    assert_eq!(current(&client, voyage_id).await["published"], true);
    assert_eq!(
        status(&client, voyage_id, id(&intent)).await["status"],
        "published"
    );
}

#[sqlx::test]
async fn future_uploads_may_arrive_out_of_order_but_publish_the_frozen_order(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let voyage_id = id(&voyage);
    let bytes = client::jpeg();
    let hash = hex::encode(Sha256::digest(&bytes));
    let first = Uuid::now_v7();
    let second = Uuid::now_v7();
    let intent = register(
        &client,
        &voyage,
        vec![
            json!({"upload_key":first,"content_hash":hash}),
            json!({"upload_key":second,"content_hash":hash}),
        ],
    )
    .await;
    assert_eq!(intent["status"], "waiting_for_uploads");
    let second_response = client
        .upload(voyage_id, second, Some("draft"), &bytes)
        .await;
    assert_eq!(second_response.status(), StatusCode::CREATED);
    let second_photo = body(second_response).await["photos"][0]["id"]
        .as_str()
        .expect("id")
        .parse::<Uuid>()
        .expect("UUID");
    worker::process(
        &client.pool,
        &client.config,
        &VariantJob::new(second_photo).with_voyage(voyage_id),
    )
    .await
    .expect("second ready");
    assert_eq!(current(&client, voyage_id).await["published"], false);
    let first_response = body(client.upload(voyage_id, first, Some("draft"), &bytes).await).await;
    let first_photo = first_response["photos"][0]["id"]
        .as_str()
        .expect("id")
        .parse::<Uuid>()
        .expect("UUID");
    worker::process(
        &client.pool,
        &client.config,
        &VariantJob::new(first_photo).with_voyage(voyage_id),
    )
    .await
    .expect("first ready");
    let published = current(&client, voyage_id).await;
    assert_eq!(published["published"], true);
    assert_eq!(
        published["photos"]
            .as_array()
            .expect("photos")
            .iter()
            .map(|photo| photo["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(first_photo), json!(second_photo)]
    );
    assert_eq!(
        status(&client, voyage_id, id(&intent)).await["status"],
        "published"
    );
}

#[sqlx::test]
async fn cancellation_expiry_failure_and_suspension_prevent_publication(pool: PgPool) {
    let client = Client::new(pool).await;
    for reason in ["cancel", "expire", "failure", "suspend"] {
        let voyage = client.create().await;
        let voyage_id = id(&voyage);
        let photo_response = body(
            client
                .upload(voyage_id, Uuid::now_v7(), Some("draft"), &client::jpeg())
                .await,
        )
        .await;
        let photo = photo_response["photos"][0]["id"]
            .as_str()
            .expect("photo")
            .parse::<Uuid>()
            .expect("UUID");
        let voyage = current(&client, voyage_id).await;
        let intent = register(&client, &voyage, vec![json!({"photo_id":photo})]).await;
        if reason == "cancel" {
            let canceled = client
                .request(
                    "DELETE",
                    &format!(
                        "/api/voyages/{voyage_id}/publication-intents/{}",
                        id(&intent)
                    ),
                    None,
                    Some(Uuid::now_v7()),
                    Some(revision(&voyage)),
                )
                .await;
            assert_eq!(canceled.status(), StatusCode::OK);
            assert_eq!(body(canceled).await["status"], "canceled");
        } else if reason == "expire" {
            sqlx::query(
                "UPDATE publication_intents SET expires_at=now()-interval '1 second' WHERE id=$1",
            )
            .bind(id(&intent))
            .execute(&client.pool)
            .await
            .expect("expire");
        } else if reason == "failure" {
            sqlx::query("UPDATE photos SET variant_status='failed' WHERE id=$1")
                .bind(photo)
                .execute(&client.pool)
                .await
                .expect("failed conversion");
        } else {
            sqlx::query("UPDATE users SET suspended=true WHERE id=$1")
                .bind(client.user_id)
                .execute(&client.pool)
                .await
                .expect("suspend");
        }
        if reason != "failure" {
            worker::process(
                &client.pool,
                &client.config,
                &VariantJob::new(photo).with_voyage(voyage_id),
            )
            .await
            .expect("worker");
        }
        if reason == "suspend" {
            sqlx::query("UPDATE users SET suspended=false WHERE id=$1")
                .bind(client.user_id)
                .execute(&client.pool)
                .await
                .expect("restore");
        }
        let result = status(&client, voyage_id, id(&intent)).await;
        assert_eq!(
            result["status"],
            if reason == "cancel" {
                "canceled"
            } else if reason == "expire" {
                "expired"
            } else {
                "needs_review"
            }
        );
        assert_eq!(current(&client, voyage_id).await["published"], false);
    }
}

#[sqlx::test]
async fn unexpected_mutations_invalidate_consent_and_wrong_bytes_cannot_satisfy_it(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let voyage_id = id(&voyage);
    let bytes = client::jpeg();
    let key = Uuid::now_v7();
    let intent = register(
        &client,
        &voyage,
        vec![json!({"upload_key":key,"content_hash":"00".repeat(32)})],
    )
    .await;
    assert_eq!(
        client
            .upload(voyage_id, key, Some("draft"), &bytes)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        status(&client, voyage_id, id(&intent)).await["status"],
        "waiting_for_uploads"
    );
    let unrelated = client
        .upload(voyage_id, Uuid::now_v7(), Some("draft"), &bytes)
        .await;
    assert_eq!(unrelated.status(), StatusCode::CREATED);
    assert_eq!(
        status(&client, voyage_id, id(&intent)).await["status"],
        "needs_review"
    );
    assert_eq!(current(&client, voyage_id).await["published"], false);
}

#[sqlx::test]
async fn an_explicit_web_unpublish_cannot_be_undone_by_old_mobile_consent(pool: PgPool) {
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
    let voyage = current(&client, voyage_id).await;
    let intent = register(&client, &voyage, vec![json!({"photo_id":photo})]).await;
    let browser = pinvoyager_api::auth::session::create(
        &client.pool,
        client.user_id,
        &client.config.auth.session,
    )
    .await
    .expect("browser");
    let request = axum::http::Request::builder()
        .method("PATCH")
        .uri(format!("/api/voyages/{voyage_id}"))
        .header("Cookie", format!("pv_session={}", browser.expose()))
        .header("content-type", "application/json")
        .body(axum::body::Body::from("{\"published\":false}"))
        .expect("request");
    use tower::ServiceExt as _;
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
    worker::process(
        &client.pool,
        &client.config,
        &VariantJob::new(photo).with_voyage(voyage_id),
    )
    .await
    .expect("ready");
    assert_eq!(
        status(&client, voyage_id, id(&intent)).await["status"],
        "needs_review"
    );
    assert_eq!(current(&client, voyage_id).await["published"], false);
}

#[sqlx::test]
async fn publication_rejects_an_unnamed_account_and_damaged_accepted_receipts(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let key = Uuid::now_v7();
    let bytes = client::jpeg();
    let hash = hex::encode(Sha256::digest(&bytes));
    sqlx::query("UPDATE users SET display_name=NULL WHERE id=$1")
        .bind(client.user_id)
        .execute(&client.pool)
        .await
        .expect("name");
    let response = client
        .request(
            "POST",
            &format!("/api/voyages/{}/publication-intents", id(&voyage)),
            Some(manifest(
                &voyage,
                vec![json!({"upload_key":key,"content_hash":hash})],
            )),
            Some(Uuid::now_v7()),
            Some(revision(&voyage)),
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    sqlx::query("UPDATE users SET display_name='native-valid' WHERE id=$1")
        .bind(client.user_id)
        .execute(&client.pool)
        .await
        .expect("name");
    sqlx::query("INSERT INTO upload_receipts(account_id,voyage_id,upload_key,state,content_hash,revision) VALUES($1,$2,$3,'accepted',$4,1)").bind(client.user_id).bind(id(&voyage)).bind(format!("{key}:0")).bind(&hash).execute(&client.pool).await.expect("damaged receipt");
    let response = client
        .request(
            "POST",
            &format!("/api/voyages/{}/publication-intents", id(&voyage)),
            Some(manifest(
                &voyage,
                vec![json!({"upload_key":key,"content_hash":hash})],
            )),
            Some(Uuid::now_v7()),
            Some(revision(&voyage)),
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(current(&client, id(&voyage)).await["published"], false);
}

#[sqlx::test]
async fn website_can_inspect_and_cancel_armed_mobile_publication(pool: PgPool) {
    let client = Client::new(pool.clone()).await;
    let stranger = Client::new(pool).await;
    let voyage = client.create().await;
    let key = Uuid::now_v7();
    let hash = hex::encode(Sha256::digest(client::jpeg()));
    let intent = register(
        &client,
        &voyage,
        vec![json!({"upload_key":key,"content_hash":hash})],
    )
    .await;
    let path = format!("/api/voyages/{}/publication-intents", id(&voyage));
    assert_eq!(
        stranger
            .request("GET", &path, None, None, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let response = client.request("GET", &path, None, None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let list = body(response).await;
    assert_eq!(list["intents"][0]["id"], intent["id"]);
    assert_eq!(list["intents"][0]["waiting_count"], 1);
    assert_eq!(
        client
            .request(
                "DELETE",
                &format!("{path}/{}", id(&intent)),
                None,
                Some(Uuid::now_v7()),
                None
            )
            .await
            .status(),
        StatusCode::OK
    );
    let list = body(client.request("GET", &path, None, None, None).await).await;
    assert_eq!(list["intents"], json!([]));
}

#[sqlx::test]
async fn canceling_a_missing_reviewed_upload_invalidates_consent_in_the_same_transaction(
    pool: PgPool,
) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let key = Uuid::now_v7();
    let intent = register(
        &client,
        &voyage,
        vec![json!({"upload_key":key,"content_hash":hex::encode(Sha256::digest(client::jpeg()))})],
    )
    .await;
    assert_eq!(
        client
            .request(
                "DELETE",
                &format!("/api/voyages/{}/uploads/{key}", id(&voyage)),
                None,
                Some(Uuid::now_v7()),
                Some(revision(&voyage))
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let status: String = sqlx::query_scalar("SELECT status FROM publication_intents WHERE id=$1")
        .bind(id(&intent))
        .fetch_one(&client.pool)
        .await
        .expect("intent");
    assert_eq!(status, "needs_review");
}

#[sqlx::test]
async fn invalid_frozen_manifests_never_arm_publication(pool: PgPool) {
    let client = Client::new(pool.clone()).await;
    let stranger = Client::new(pool).await;
    let voyage = client.create().await;
    let key = Uuid::now_v7();
    let valid = json!({"upload_key":key,"content_hash":"ab".repeat(32)});
    let path = format!("/api/voyages/{}/publication-intents", id(&voyage));
    let mut cases = vec![
        (manifest(&voyage, vec![]), StatusCode::BAD_REQUEST),
        (
            manifest(&voyage, vec![valid.clone(); 10_001]),
            StatusCode::BAD_REQUEST,
        ),
        (
            manifest(&voyage, vec![valid.clone(), valid.clone()]),
            StatusCode::BAD_REQUEST,
        ),
        (
            manifest(&voyage, vec![json!({"photo_id":Uuid::now_v7()})]),
            StatusCode::BAD_REQUEST,
        ),
        (
            manifest(
                &voyage,
                vec![json!({"upload_key":key,"content_hash":"x".repeat(64)})],
            ),
            StatusCode::BAD_REQUEST,
        ),
        (
            manifest(&voyage, vec![json!({"upload_key":key,"content_hash":"ab"})]),
            StatusCode::BAD_REQUEST,
        ),
    ];
    let mut expired = manifest(&voyage, vec![valid.clone()]);
    expired["expires_at"] = json!((Utc::now() - Duration::minutes(1)).to_rfc3339());
    cases.push((expired, StatusCode::BAD_REQUEST));
    let mut stale = manifest(&voyage, vec![valid.clone()]);
    stale["metadata_digest"] = json!("00".repeat(32));
    cases.push((stale, StatusCode::PRECONDITION_FAILED));
    for (review, expected) in cases {
        let response = client
            .request(
                "POST",
                &path,
                Some(review),
                Some(Uuid::now_v7()),
                Some(revision(&voyage)),
            )
            .await;
        assert_eq!(response.status(), expected);
        assert_eq!(current(&client, id(&voyage)).await["published"], false);
    }
    assert_eq!(
        stranger
            .request(
                "POST",
                &path,
                Some(manifest(&voyage, vec![valid.clone()])),
                Some(Uuid::now_v7()),
                Some(revision(&voyage))
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .request(
                "POST",
                &path,
                Some(manifest(&voyage, vec![valid.clone()])),
                Some(Uuid::now_v7()),
                Some("99")
            )
            .await
            .status(),
        StatusCode::PRECONDITION_FAILED
    );
    let mut capped = manifest(&voyage, vec![valid.clone()]);
    capped["expires_at"] = json!((Utc::now() + Duration::days(30)).to_rfc3339());
    let response = client
        .request(
            "POST",
            &path,
            Some(capped),
            Some(Uuid::now_v7()),
            Some(revision(&voyage)),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let intent = body(response).await;
    let deadline =
        chrono::DateTime::parse_from_rfc3339(intent["expires_at"].as_str().expect("deadline"))
            .expect("date");
    assert!(deadline <= Utc::now() + Duration::days(7));
    assert_eq!(
        client
            .request(
                "POST",
                &path,
                Some(manifest(&voyage, vec![valid])),
                Some(Uuid::now_v7()),
                Some(revision(&voyage))
            )
            .await
            .status(),
        StatusCode::CONFLICT
    );
    for target in [Uuid::now_v7(), id(&intent)] {
        assert_eq!(
            stranger
                .request("GET", &format!("{path}/{target}"), None, None, None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            stranger
                .request(
                    "DELETE",
                    &format!("{path}/{target}"),
                    None,
                    Some(Uuid::now_v7()),
                    None
                )
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        client
            .request(
                "GET",
                &format!("{path}/{}", Uuid::now_v7()),
                None,
                None,
                None
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .request(
                "DELETE",
                &format!("{path}/{}", Uuid::now_v7()),
                None,
                Some(Uuid::now_v7()),
                None
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn restart_sweep_expires_every_page_and_ignores_missing_photos_and_voyages(pool: PgPool) {
    use pinvoyager_api::voyages::publication;
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let mut tx = client.pool.begin().await.expect("transaction");
    for _ in 0..105 {
        let voyage_id = Uuid::now_v7();
        sqlx::query("INSERT INTO voyages(id,owner_id,title,description,granularity) VALUES($1,$2,'Restart','Reviewed','km1')")
            .bind(voyage_id).bind(client.user_id).execute(&mut *tx).await.expect("voyage");
        sqlx::query("INSERT INTO publication_intents(id,owner_id,voyage_id,metadata_digest,photos,tracked_revision,expires_at,status) VALUES($1,$2,$3,$4,'[]',1,now()-interval '1 second','waiting_for_uploads')")
            .bind(Uuid::now_v7()).bind(client.user_id).bind(voyage_id).bind("00".repeat(32)).execute(&mut *tx).await.expect("expired intent");
    }
    tx.commit().await.expect("commit");
    publication::sweep(&client.pool)
        .await
        .expect("restart sweep");
    let expired: i64 =
        sqlx::query_scalar("SELECT count(*) FROM publication_intents WHERE status='expired'")
            .fetch_one(&client.pool)
            .await
            .expect("count");
    assert_eq!(expired, 105);
    publication::finalize(&client.pool, Uuid::now_v7())
        .await
        .expect("deleted voyage");
    publication::photo_changed(&client.pool, Uuid::now_v7())
        .await
        .expect("deleted photo");
    assert_eq!(current(&client, id(&voyage)).await["published"], false);
}

#[sqlx::test]
async fn restart_recovery_rejects_changed_or_unreadable_consent_without_blocking_other_voyages(
    pool: PgPool,
) {
    let client = Client::new(pool).await;
    for fault in ["digest", "revision", "manifest", "photo_hash"] {
        let voyage = client.create().await;
        let voyage_id = id(&voyage);
        let key = Uuid::now_v7();
        let bytes = client::jpeg();
        let hash = hex::encode(Sha256::digest(&bytes));
        let uploaded = body(client.upload(voyage_id, key, Some("draft"), &bytes).await).await;
        let photo: Uuid = uploaded["photos"][0]["id"]
            .as_str()
            .expect("photo")
            .parse()
            .expect("UUID");
        let voyage = current(&client, voyage_id).await;
        let intent = register(
            &client,
            &voyage,
            vec![json!({"upload_key":key,"content_hash":hash})],
        )
        .await;
        let intent_id = id(&intent);
        match fault {
            "digest" => {
                sqlx::query("UPDATE publication_intents SET metadata_digest=$2 WHERE id=$1")
                    .bind(intent_id)
                    .bind("00".repeat(32))
                    .execute(&client.pool)
                    .await
                    .expect("damaged digest");
            }
            "revision" => {
                sqlx::query("UPDATE publication_intents SET tracked_revision=tracked_revision+1 WHERE id=$1").bind(intent_id).execute(&client.pool).await.expect("stale review");
            }
            "manifest" => {
                sqlx::query("UPDATE publication_intents SET photos=$2 WHERE id=$1")
                    .bind(intent_id)
                    .bind(json!([{"unknown":"unreadable stored review"}]))
                    .execute(&client.pool)
                    .await
                    .expect("damaged manifest");
            }
            _ => {
                sqlx::query("UPDATE upload_receipts SET content_hash=$3 WHERE voyage_id=$1 AND upload_key=$2").bind(voyage_id).bind(format!("{key}:0")).bind("00".repeat(32)).execute(&client.pool).await.expect("damaged receipt");
                sqlx::query("UPDATE publication_intents SET photos=$2 WHERE id=$1")
                    .bind(intent_id)
                    .bind(json!([{"upload_key":key,"content_hash":"00".repeat(32)}]))
                    .execute(&client.pool)
                    .await
                    .expect("damaged expected bytes");
            }
        }
        pinvoyager_api::voyages::publication::sweep(&client.pool)
            .await
            .expect("one unreadable review must not stop recovery for every voyage");
        let result = status(&client, voyage_id, intent_id).await;
        assert_eq!(result["status"], "needs_review", "{fault}");
        assert!(result["reason"].is_string());
        assert_eq!(current(&client, voyage_id).await["published"], false);
        assert_eq!(
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM photos WHERE id=$1")
                .bind(photo)
                .fetch_one(&client.pool)
                .await
                .expect("source remains"),
            photo
        );
    }
}

#[sqlx::test]
async fn publication_and_receipt_commit_failures_leave_the_review_private_and_retryable(
    pool: PgPool,
) {
    let client = Client::new(pool).await;
    for (table, event) in [
        ("publication_intents", "INSERT"),
        ("voyages", "UPDATE"),
        ("publication_intents", "UPDATE"),
    ] {
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
        let voyage = current(&client, voyage_id).await;
        let key = Uuid::now_v7();
        let payload = manifest(&voyage, vec![json!({"photo_id":photo})]);
        sqlx::query("CREATE OR REPLACE FUNCTION reject_review_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'private review diagnostics'; END $$")
            .execute(&client.pool).await.expect("failure function");
        let create_trigger = match (table, event) {
            ("publication_intents", "INSERT") => {
                "CREATE TRIGGER reject_review_write BEFORE INSERT ON publication_intents FOR EACH ROW EXECUTE FUNCTION reject_review_write()"
            }
            ("voyages", "UPDATE") => {
                "CREATE TRIGGER reject_review_write BEFORE UPDATE ON voyages FOR EACH ROW EXECUTE FUNCTION reject_review_write()"
            }
            _ => {
                "CREATE TRIGGER reject_review_write BEFORE UPDATE ON publication_intents FOR EACH ROW EXECUTE FUNCTION reject_review_write()"
            }
        };
        sqlx::query(create_trigger)
            .execute(&client.pool)
            .await
            .expect("simulate failed review write");
        let path = format!("/api/voyages/{voyage_id}/publication-intents");
        let response = client
            .request(
                "POST",
                &path,
                Some(payload.clone()),
                Some(key),
                Some(revision(&voyage)),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{table} {event}"
        );
        assert!(!body(response).await.to_string().contains("private review"));
        let drop_trigger = if table == "voyages" {
            "DROP TRIGGER reject_review_write ON voyages"
        } else {
            "DROP TRIGGER reject_review_write ON publication_intents"
        };
        sqlx::query(drop_trigger)
            .execute(&client.pool)
            .await
            .expect("restore review writes");
        assert_eq!(current(&client, voyage_id).await["published"], false);
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM publication_intents WHERE voyage_id=$1")
                .bind(voyage_id)
                .fetch_one(&client.pool)
                .await
                .expect("intent count");
        assert_eq!(count, 0);
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
        assert_eq!(
            client
                .request(
                    "POST",
                    &path,
                    Some(payload),
                    Some(key),
                    Some(revision(&voyage))
                )
                .await
                .status(),
            StatusCode::CREATED
        );
        assert_eq!(current(&client, voyage_id).await["published"], true);
    }
}

#[sqlx::test]
async fn cancellation_does_not_claim_to_have_checked_a_stale_content_revision(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let voyage_id = id(&voyage);
    let upload = Uuid::now_v7();
    let intent = register(
        &client,
        &voyage,
        vec![json!({"upload_key":upload,"content_hash":"aa".repeat(32)})],
    )
    .await;
    sqlx::query("UPDATE voyages SET title='An unseen website edit' WHERE id=$1")
        .bind(voyage_id)
        .execute(&client.pool)
        .await
        .expect("website edit");
    let key = Uuid::now_v7();
    let response = client
        .request(
            "DELETE",
            &format!(
                "/api/voyages/{voyage_id}/publication-intents/{}",
                id(&intent)
            ),
            None,
            Some(key),
            Some(revision(&voyage)),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
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
    assert!(
        receipt.get("previous_revision").is_none(),
        "cancellation accepts stale content revisions; it must not claim they were checked"
    );
    assert_eq!(receipt["publication_status"], "canceled");
}

#[sqlx::test]
async fn failed_cancellation_keeps_consent_armed_until_the_same_request_is_committed(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let voyage_id = id(&voyage);
    let intent = register(
        &client,
        &voyage,
        vec![json!({"upload_key":Uuid::now_v7(),"content_hash":"aa".repeat(32)})],
    )
    .await;
    let path = format!(
        "/api/voyages/{voyage_id}/publication-intents/{}",
        id(&intent)
    );
    sqlx::raw_sql("CREATE FUNCTION reject_cancellation_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'private cancellation diagnostics'; END $$; CREATE TRIGGER reject_cancellation_write BEFORE UPDATE ON publication_intents FOR EACH ROW EXECUTE FUNCTION reject_cancellation_write();")
        .execute(&client.pool).await.expect("simulate cancellation failure");
    let key = Uuid::now_v7();
    assert_eq!(
        client
            .request("DELETE", &path, None, Some(key), None)
            .await
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    sqlx::query("DROP TRIGGER reject_cancellation_write ON publication_intents")
        .execute(&client.pool)
        .await
        .expect("restore cancellation writes");
    assert_eq!(
        status(&client, voyage_id, id(&intent)).await["status"],
        "waiting_for_uploads"
    );
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
        "not_seen"
    );
    assert_eq!(
        client
            .request("DELETE", &path, None, Some(key), None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        status(&client, voyage_id, id(&intent)).await["status"],
        "canceled"
    );
}
