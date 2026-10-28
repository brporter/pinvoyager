#[path = "support/native_client.rs"]
mod client;
#[path = "support/contract_fixtures.rs"]
mod fixtures;
use axum::http::StatusCode;
use client::{Client, body, id, revision};
use fixtures::write_fixture;
use pinvoyager_api::{
    photos::worker::{self, VariantJob},
    voyages::publication,
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
#[sqlx::test]
async fn native_contracts_are_captured_from_the_real_router(pool: PgPool) {
    let client = Client::new(pool).await;
    let caps = body(
        client
            .request("GET", "/api/client-capabilities", None, None, None)
            .await,
    )
    .await;
    assert_eq!(caps["native_protocol_versions"], json!([1]));
    write_fixture("native_capabilities", &caps.to_string());
    let operation = Uuid::now_v7();
    let response = client
        .request(
            "POST",
            "/api/voyages",
            Some(json!({"title":"Winter","description":"On foot","granularity":"km1"})),
            Some(operation),
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let voyage = body(response).await;
    let voyage_id = id(&voyage);
    let receipt = body(
        client
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
    write_fixture("native_operation_applied", &receipt.to_string());
    let key = Uuid::now_v7();
    let upload = body(
        client
            .upload(voyage_id, key, Some("draft"), &client::jpeg())
            .await,
    )
    .await;
    let photo: Uuid = upload["photos"][0]["id"]
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
    write_fixture("native_upload_accepted", &receipt.to_string());
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
    let digest = publication::metadata_digest("Winter", Some("On foot"), "km1").expect("digest");
    let review = json!({"metadata_digest":digest,"expires_at":(chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339(),"photos":[{"photo_id":photo}]});
    let response = client
        .request(
            "POST",
            &format!("/api/voyages/{voyage_id}/publication-intents"),
            Some(review),
            Some(Uuid::now_v7()),
            Some(revision(&current)),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let intent = body(response).await;
    let intent_id = id(&intent);
    write_fixture("native_publication_processing", &intent.to_string());
    worker::process(
        &client.pool,
        &client.config,
        &VariantJob::new(photo).with_voyage(voyage_id),
    )
    .await
    .expect("convert");
    let status = body(
        client
            .request(
                "GET",
                &format!("/api/voyages/{voyage_id}/publication-intents/{intent_id}"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    write_fixture("native_publication_published", &status.to_string());
    let manifest = body(
        client
            .request(
                "GET",
                &format!("/api/voyages/{voyage_id}/offline-manifest"),
                None,
                None,
                None,
            )
            .await,
    )
    .await;
    write_fixture("native_offline_manifest", &manifest.to_string());
    let title = "雪 \"⛄️\" / \\ path";
    let description = "Line\nTab\t\u{2028}\u{2029} 🗺️";
    let canonical =
        serde_json::to_string(&json!([title, description, "exact"])).expect("canonical");
    write_fixture("native_metadata_digest",&json!({"title":title,"description":description,"granularity":"exact","canonical_utf8":canonical,"sha256":publication::metadata_digest(title,Some(description),"exact").expect("digest")}).to_string());
}

#[sqlx::test]
async fn native_rejection_receipt_retains_the_original_conflict_status(pool: PgPool) {
    let client = Client::new(pool).await;
    let voyage = client.create().await;
    let key = Uuid::now_v7();
    assert_eq!(
        client
            .request(
                "PATCH",
                &format!("/api/voyages/{}", id(&voyage)),
                Some(json!({"title":"Retained edit"})),
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
    assert_eq!(receipt["response_status"], 412);
    write_fixture("native_operation_rejected", &receipt.to_string());
}
