//! Exported spans, not just log messages: exercise the real request/queue/CPU path.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use pinvoyager_api::{
    AppState, Config,
    auth::{ProviderRegistry, session},
    db::repo::users,
    http,
    photos::worker,
};
use sqlx::PgPool;
use std::{io::Cursor, sync::Arc, time::Duration};
use tower::ServiceExt as _;
use tracing_subscriber::prelude::*;
use uuid::Uuid;

#[sqlx::test]
async fn creation_upload_and_each_background_photo_export_one_correlated_trace(pool: PgPool) {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_sampler(opentelemetry_sdk::trace::Sampler::ParentBased(Box::new(
            opentelemetry_sdk::trace::Sampler::AlwaysOn,
        )))
        .with_simple_exporter(exporter.clone())
        .build();
    tracing_subscriber::registry()
        .with(
            tracing_opentelemetry::layer()
                .with_tracer(provider.tracer("test"))
                .with_filter(pinvoyager_api::telemetry::SpanOnly),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::sink)
                .with_filter(tracing_subscriber::EnvFilter::new(
                    "info,pinvoyager_api=debug,tower_http=debug",
                )),
        )
        .try_init()
        .expect("isolated integration binary subscriber");
    let root = std::env::temp_dir().join(format!("pv-trace-{}", Uuid::now_v7()));
    let mut config = Config::load().expect("config");
    config.photos.storage_root = root.clone();
    let owner = users::create(&pool, Some("Trace test"), None)
        .await
        .expect("owner");
    let token = session::create(&pool, owner.id, &config.auth.session)
        .await
        .expect("session");
    let cookie = format!("{}={}", config.auth.session.cookie_name, token.expose());
    let config = Arc::new(config);
    let queue = worker::spawn(pool.clone(), config.clone());
    // The startup sweep can also find live uploads; let this existing one-shot
    // sweep finish so this test measures exactly the two live queue jobs.
    tokio::time::timeout(Duration::from_secs(5), async {
        while !exporter
            .get_finished_spans()
            .expect("spans")
            .iter()
            .any(|s| s.name == "photo.recover")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("startup recovery");
    let app = http::router(AppState::new(
        pool.clone(),
        config,
        ProviderRegistry::empty(),
        queue,
    ));
    let incoming = "00-11111111111111111111111111111111-2222222222222222-01";
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/voyages")
                .header("cookie", &cookie)
                .header("traceparent", incoming)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"title":"Trace test","description":"Two photos"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("create response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let parent = response
        .headers()
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or(incoming)
        .to_owned();
    let creation_has_context = response.headers().contains_key("traceparent");
    let voyage: serde_json::Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body"),
    )
    .expect("voyage");
    let id = voyage["id"].as_str().expect("id");
    let mut photo_ids = Vec::new();
    for color in [20, 200] {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            420,
            20,
            image::Rgb([color, 40, 80]),
        ));
        let mut png = Cursor::new(Vec::new());
        image
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("png");
        let mut body = b"--boundary\r\nContent-Disposition: form-data; name=\"files\"; filename=\"private-name.png\"\r\nContent-Type: image/png\r\n\r\n".to_vec();
        body.extend(png.into_inner());
        body.extend(b"\r\n--boundary--\r\n");
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/voyages/{id}/photos"))
                    .header("cookie", &cookie)
                    .header("traceparent", &parent)
                    .header("content-type", "multipart/form-data; boundary=boundary")
                    .body(Body::from(body))
                    .expect("upload"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::CREATED);
        let value: serde_json::Value = serde_json::from_slice(
            &to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body"),
        )
        .expect("json");
        photo_ids.push(value["photos"][0]["id"].as_str().expect("photo").to_owned());
    }
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM photos WHERE voyage_id=$1 AND variant_status='ready'",
            )
            .bind(Uuid::parse_str(id).expect("uuid"))
            .fetch_one(&pool)
            .await
            .expect("count");
            if count == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("same successful processing behavior");
    tokio::time::timeout(Duration::from_secs(5), async {
        while exporter
            .get_finished_spans()
            .expect("spans")
            .iter()
            .filter(|s| s.name.starts_with("photo.process "))
            .count()
            < 2
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("worker span completion after DB readiness");
    provider.force_flush().expect("flush");
    let spans = exporter.get_finished_spans().expect("exported spans");
    assert!(
        creation_has_context,
        "creation response must expose its traceparent"
    );
    assert!(
        spans.iter().any(|s| s.name == "voyage.create"),
        "creation span missing"
    );
    assert!(
        spans.iter().all(|s| s.events.is_empty()),
        "application log fields must not be exported as span events"
    );
    for photo_id in &photo_ids {
        let processing = spans
            .iter()
            .find(|s| s.name == format!("photo.process {photo_id}"))
            .expect("individually named processing span");
        assert_eq!(
            processing.span_context.trace_id().to_string(),
            "11111111111111111111111111111111"
        );
        assert!(
            processing
                .attributes
                .iter()
                .any(|a| a.key.as_str() == "photo.id" && a.value.to_string() == *photo_id)
        );
        assert!(
            processing
                .attributes
                .iter()
                .any(|a| a.key.as_str() == "voyage.id" && a.value.to_string() == id)
        );
        let child_names: Vec<_> = spans
            .iter()
            .filter(|s| s.parent_span_id == processing.span_context.span_id())
            .map(|s| s.name.as_ref())
            .collect();
        assert!(child_names.contains(&"photo.generate"));
        assert!(spans.iter().any(|s| {
            s.name == "photo.queue_wait"
                && s.attributes
                    .iter()
                    .any(|a| a.key.as_str() == "photo.id" && a.value.to_string() == *photo_id)
        }));
    }
    for stage in [
        "photo.decode",
        "photo.orient",
        "photo.variant",
        "photo.resize",
        "photo.encode",
        "photo.write",
        "photo.validate",
        "photo.hash",
        "photo.exif",
        "photo.store",
        "photo.enqueue",
    ] {
        assert!(spans.iter().any(|s| s.name == stage), "missing {stage}");
    }
    let variants: Vec<_> = spans.iter().filter(|s| s.name == "photo.variant").collect();
    assert_eq!(variants.len(), 6);
    for kind in ["thumb", "display", "full"] {
        assert_eq!(
            variants
                .iter()
                .filter(|s| s
                    .attributes
                    .iter()
                    .any(|a| a.key.as_str() == "photo.variant" && a.value.to_string() == kind))
                .count(),
            2
        );
    }
    assert!(
        spans
            .iter()
            .all(|s| !format!("{:?}", s.attributes).contains("private-name")),
        "filenames must not be recorded"
    );
    // Propagation headers are diagnostic only: malformed parents do not reject requests.
    for invalid in [None, Some("invalid")] {
        let mut request = Request::builder().uri("/health");
        if let Some(value) = invalid {
            request = request.header("traceparent", value);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).expect("health request"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let returned = response
            .headers()
            .get("traceparent")
            .expect("new root context")
            .to_str()
            .expect("ascii");
        assert_ne!(&returned[3..35], "11111111111111111111111111111111");
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .header(
                    "traceparent",
                    "00-99999999999999999999999999999999-2222222222222222-00",
                )
                .body(Body::empty())
                .expect("health"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get("traceparent")
            .expect("context")
            .to_str()
            .expect("ascii")
            .ends_with("-00")
    );
    drop(response);
    assert!(
        !exporter
            .get_finished_spans()
            .expect("spans")
            .iter()
            .any(|s| s.span_context.trace_id().to_string() == "99999999999999999999999999999999")
    );

    // Preserve the existing failed-row behavior while annotating a corrupt conversion.
    let failed_id = Uuid::parse_str(&photo_ids[0]).expect("uuid");
    let photo = pinvoyager_api::db::repo::photos::find_by_id(&pool, failed_id)
        .await
        .expect("query")
        .expect("photo");
    let path = pinvoyager_api::photos::storage::original_path(
        &root,
        &pinvoyager_api::photos::storage::ContentHash::try_from(photo.content_hash)
            .expect("valid fixture hash"),
    );
    std::fs::write(path, b"corrupt").expect("corrupt original");
    pinvoyager_api::db::repo::photos::set_variant_status(&pool, failed_id, "pending")
        .await
        .expect("status");
    let mut failure_config = Config::load().expect("config");
    failure_config.photos.storage_root = root.clone();
    let job = worker::VariantJob::new(failed_id).with_voyage(Uuid::parse_str(id).expect("uuid"));
    assert!(worker::process(&pool, &failure_config, &job).await.is_err());
    drop(job);
    let failed = exporter
        .get_finished_spans()
        .expect("spans")
        .into_iter()
        .find(|s| {
            s.name == format!("photo.process {failed_id}")
                && matches!(s.status, opentelemetry::trace::Status::Error { .. })
        })
        .expect("failed span");
    assert!(
        failed
            .attributes
            .iter()
            .any(|a| a.key.as_str() == "photo.status" && a.value.to_string() == "failed")
    );
    assert_eq!(
        pinvoyager_api::db::repo::photos::find_by_id(&pool, failed_id)
            .await
            .expect("query")
            .expect("photo")
            .variant_status,
        "failed"
    );
    provider.shutdown().expect("shutdown");
    std::fs::remove_dir_all(root).expect("scratch cleanup");
}
