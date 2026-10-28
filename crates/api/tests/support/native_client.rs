//! Native API client fixture; authentication itself is tested through OIDC in native_auth.
#![allow(dead_code)]
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use pinvoyager_api::{
    Config,
    auth::{ProviderRegistry, session},
    db::repo::users,
    http,
    photos::worker::{VariantJob, VariantQueue},
    state::AppState,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::mpsc;
use tower::ServiceExt as _;
use uuid::Uuid;

pub struct Client {
    pub config: Arc<Config>,
    pub variants: VariantQueue,
    pub router: Router,
    pub pool: PgPool,
    pub user_id: Uuid,
    pub token: String,
    pub root: PathBuf,
    _queue: mpsc::Receiver<VariantJob>,
}

impl Client {
    pub async fn new(pool: PgPool) -> Self {
        Self::configured(pool, |_| {}).await
    }

    pub async fn configured(pool: PgPool, configure: impl FnOnce(&mut Config) + Send) -> Self {
        let root = std::env::temp_dir().join(format!("pv-native-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&root).expect("storage");
        let mut config: Config=serde_json::from_value(json!({
            "database":{"url":"postgres://unused","max_connections":5,"min_connections":1,"acquire_timeout_secs":5},
            "auth":{"base_url":"https://pinvoyager.test","providers":{}},
            "photos":{"storage_root":root,"upload_rate_per_minute":10000}
        })).expect("config");
        configure(&mut config);
        let name = format!("native-{}", &Uuid::now_v7().simple().to_string()[..16]);
        let user = users::create(&pool, Some(&name), None)
            .await
            .expect("account");
        let token = session::create(&pool, user.id, &config.auth.session)
            .await
            .expect("session");
        sqlx::query("INSERT INTO native_sessions(token_hash) VALUES($1)")
            .bind(token.hash())
            .execute(&pool)
            .await
            .expect("native scope");
        let (sender, queue) = mpsc::channel(64);
        let variants = VariantQueue::from_sender(sender);
        let config = Arc::new(config);
        let router = http::router(AppState::new(
            pool.clone(),
            config.clone(),
            ProviderRegistry::empty(),
            variants.clone(),
        ));
        Self {
            config,
            variants,
            router,
            pool,
            user_id: user.id,
            token: token.expose().to_owned(),
            root,
            _queue: queue,
        }
    }

    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        key: Option<Uuid>,
        revision: Option<&str>,
    ) -> axum::response::Response {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.token));
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key.to_string());
        }
        if let Some(revision) = revision {
            request = request.header(header::IF_MATCH, format!("\"v:{revision}\""));
        }
        let body = if let Some(body) = body {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        } else {
            Body::empty()
        };
        self.router
            .clone()
            .oneshot(request.body(body).expect("request"))
            .await
            .expect("router")
    }

    pub async fn create(&self) -> Value {
        let response = self
            .request(
                "POST",
                "/api/voyages",
                Some(json!({"title":"Winter","description":"On foot","granularity":"km1"})),
                Some(Uuid::now_v7()),
                None,
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        body(response).await
    }

    pub async fn upload(
        &self,
        voyage: Uuid,
        key: Uuid,
        visibility: Option<&str>,
        bytes: &[u8],
    ) -> axum::response::Response {
        let mut multipart=b"--native\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n".to_vec();
        multipart.extend_from_slice(bytes);
        multipart.extend_from_slice(b"\r\n--native--\r\n");
        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/api/voyages/{voyage}/photos"))
            .header(header::AUTHORIZATION, format!("Bearer {}", self.token))
            .header("Idempotency-Key", key.to_string())
            .header(header::CONTENT_TYPE, "multipart/form-data; boundary=native")
            .header(header::CONTENT_LENGTH, multipart.len());
        if let Some(visibility) = visibility {
            request = request.header("X-PinVoyager-Upload-Visibility", visibility);
        }
        self.router
            .clone()
            .oneshot(request.body(Body::from(multipart)).expect("upload"))
            .await
            .expect("router")
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub async fn body(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("JSON response")
}

pub fn id(value: &Value) -> Uuid {
    value["id"].as_str().expect("id").parse().expect("UUID")
}
pub fn revision(value: &Value) -> &str {
    value["revision"].as_str().expect("owner content revision")
}

pub fn jpeg() -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(4, 4, image::Rgb([20, 60, 80]))
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .expect("JPEG");
    bytes.into_inner()
}
