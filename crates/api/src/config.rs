//! Typed configuration.
//!
//! Precedence, lowest to highest: built-in defaults → `config.toml` →
//! `PINVOYAGER__`-prefixed environment variables (double underscore separates
//! nested keys, e.g. `PINVOYAGER__DATABASE__MAX_CONNECTIONS`).

use std::net::SocketAddr;
use std::path::PathBuf;

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use secrecy::SecretString;
use serde::Deserialize;
use url::Url;

/// Configuration file, looked up relative to the working directory. figment
/// also searches parent directories, so `cargo run -p pinvoyager-api` from
/// inside `crates/api` still finds the repository-root file.
const CONFIG_FILE: &str = "config.toml";

/// Environment prefix. The separator is doubled so that a single underscore
/// stays available inside a key name: `PINVOYAGER__DATABASE__MAX_CONNECTIONS`
/// is unambiguously `database.max_connections`.
const ENV_PREFIX: &str = "PINVOYAGER__";
const ENV_SEPARATOR: &str = "__";

// Defaults for individual database settings; the database URL is required.
const DEFAULT_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_MIN_CONNECTIONS: u32 = 1;
/// Default seconds to wait for a free database connection before returning an error.
const DEFAULT_ACQUIRE_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    #[serde(default)]
    pub telemetry: TelemetryConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub photos: PhotosConfig,
}

/// Photo storage and upload limits.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct PhotosConfig {
    /// Root of the on-disk store. Originals and variants live in sibling
    /// subdirectories beneath it.
    ///
    /// Nothing is ever served from here as a static file (D7) — every read goes
    /// through a handler that sets its own `Content-Type` and security headers.
    pub storage_root: PathBuf,

    /// Maximum upload request size in bytes, including multipart framing.
    /// Rejects an oversized declared length before reading; also enforced while streaming.
    pub max_upload_bytes: u64,

    /// Maximum decoded pixel count in decimal megapixels. Checked from image
    /// headers before decoding the full pixel buffer.
    pub max_megapixels: u32,

    /// Uploads permitted per user per minute.
    pub upload_rate_per_minute: u32,

    /// Maximum queued variant jobs. Uploads wait for capacity before saving a photo.
    pub variant_queue_depth: usize,

    /// Maximum seconds an upload waits for image-processing capacity.
    pub variant_queue_wait_secs: u64,

    /// Maximum concurrent image conversions on the blocking pool, independent
    /// of the number of queued jobs.
    pub variant_concurrency: usize,

    /// Quality for the lossy WebP variants, 0–100.
    pub webp_quality: f32,
}

/// Authentication configuration.
#[derive(Debug, Deserialize)]
pub struct AuthConfig {
    /// Public base URL used to construct provider callback URIs. Register the
    /// resulting callback URI with each configured provider.
    pub base_url: Url,
    #[serde(default)]
    pub session: SessionConfig,
    /// Every provider is optional. The service starts with any subset configured.
    /// An unconfigured provider's endpoints return 404.
    #[serde(default)]
    pub providers: ProvidersConfig,

    /// Enables development sign-in without provider verification.
    /// The route is compiled only with debug assertions and defaults to disabled.
    /// When enabled, callers can impersonate provider identities and startup logs a warning.
    #[serde(default)]
    pub dev_login: bool,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    /// Sliding: each authenticated request extends the session by this much.
    pub idle_timeout_secs: u64,
    /// Maximum session lifetime in seconds, regardless of activity.
    pub absolute_timeout_secs: u64,
    pub cookie_name: String,
    /// Must be true anywhere but local plain-HTTP development.
    pub cookie_secure: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    pub google: Option<OidcProviderConfig>,
    pub microsoft: Option<OidcProviderConfig>,
    pub apple: Option<AppleProviderConfig>,
}

/// A standards-conformant OIDC provider: discovery document, JWKS, ID token.
#[derive(Debug, Deserialize)]
pub struct OidcProviderConfig {
    pub client_id: String,
    pub client_secret: SecretString,
    /// Issuer URL. Discovery is fetched from `{issuer}/.well-known/openid-configuration`.
    pub issuer: Url,
}

/// Apple is OIDC, but its client secret is not a static string — it is a JWT
/// you generate and sign yourself with an ES256 key downloaded from Apple, and
/// it expires (six months maximum). These are the inputs to generating it.
#[derive(Debug, Deserialize)]
pub struct AppleProviderConfig {
    /// The Services ID, not the App ID.
    pub client_id: String,
    pub team_id: String,
    /// Key ID of the .p8 signing key.
    pub key_id: String,
    /// Contents of the .p8 file, PEM-encoded PKCS#8.
    pub private_key_pem: SecretString,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    /// How long to let in-flight requests finish after a shutdown signal.
    pub shutdown_timeout_secs: u64,
    /// Deadline for requests other than photo uploads.
    pub request_timeout_secs: u64,
    /// Deadline for receiving, validating, and accepting a photo upload.
    pub upload_request_timeout_secs: u64,
    /// Maximum inactivity while receiving upload body frames.
    pub upload_body_idle_timeout_secs: u64,
}

/// Database connection settings. `SecretString` redacts the URL in Debug output.
#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub url: SecretString,
    pub max_connections: u32,
    pub min_connections: u32,
    /// Maximum seconds to wait for a free database connection.
    pub acquire_timeout_secs: u64,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct TelemetryConfig {
    /// `RUST_LOG`-style filter directive.
    pub log_level: String,
    /// JSON in production, pretty in development.
    pub json: bool,
    /// Full OTLP/HTTP traces URL. None keeps logging without trace export.
    pub otlp_endpoint: Option<String>,
    /// Resource name shown in the trace viewer.
    pub service_name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("configuration is invalid: {0}")]
    Invalid(String),
}

impl ConfigError {
    /// Include every configuration extraction error in the reported message.
    fn from_figment(err: figment::Error) -> Self {
        let messages: Vec<String> = err.into_iter().map(describe).collect();
        ConfigError::Invalid(messages.join("; "))
    }
}

/// Formats a configuration error with its file key, environment variable name,
/// and figment's explanation of the failure.
fn describe(err: figment::Error) -> String {
    let mut path = err.path.clone();
    if let figment::error::Kind::MissingField(field) = &err.kind {
        path.push(field.to_string());
    }

    let key = path.join(".");
    if key.is_empty() {
        return err.kind.to_string();
    }

    // Both spellings, because which one is wrong depends on where the value
    // was meant to come from, and the operator knows which they were editing.
    let env_var = format!(
        "{ENV_PREFIX}{}",
        key.replace('.', ENV_SEPARATOR).to_uppercase()
    );
    format!(
        "{} — set `{key}` in {CONFIG_FILE} or {env_var} in the environment",
        err.kind
    )
}

impl Config {
    /// Loads defaults, the configuration file, and environment overrides.
    /// Returns an error naming missing or malformed settings.
    pub fn load() -> Result<Self, ConfigError> {
        Self::from_figment(
            Figment::new()
                .merge(Toml::file(CONFIG_FILE))
                .merge(Env::prefixed(ENV_PREFIX).split(ENV_SEPARATOR)),
        )
    }

    /// Tests supply explicit sources; production supplies its file and environment.
    pub fn from_figment(sources: Figment) -> Result<Self, ConfigError> {
        Figment::new()
            .merge(Serialized::default(
                "database.max_connections",
                DEFAULT_MAX_CONNECTIONS,
            ))
            .merge(Serialized::default(
                "database.min_connections",
                DEFAULT_MIN_CONNECTIONS,
            ))
            .merge(Serialized::default(
                "database.acquire_timeout_secs",
                DEFAULT_ACQUIRE_TIMEOUT_SECS,
            ))
            .merge(sources)
            .extract()
            .map_err(ConfigError::from_figment)
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            // Loopback rather than 0.0.0.0: a development server should not
            // land on the local network by omission. Containers override this
            // with `PINVOYAGER__SERVER__BIND=0.0.0.0:8080`.
            bind: SocketAddr::from(([127, 0, 0, 1], 8080)),
            // Long enough for in-flight requests to finish, short enough that
            // a stuck request does not hold up a restart indefinitely.
            shutdown_timeout_secs: 30,
            request_timeout_secs: 30,
            upload_request_timeout_secs: 900,
            upload_body_idle_timeout_secs: 90,
        }
    }
}

impl Default for PhotosConfig {
    fn default() -> Self {
        Self {
            storage_root: PathBuf::from("data"),
            // 25 MiB maximum upload request size.
            max_upload_bytes: 25 * 1024 * 1024,
            // Reject images over 100 million pixels before full decoding.
            max_megapixels: 100,
            upload_rate_per_minute: 60,
            variant_queue_depth: 256,
            variant_queue_wait_secs: 10,
            // Leave headroom: the blocking pool also serves file I/O, and
            // saturating every core with image work makes the service
            // unresponsive in exactly the way D8 warns about.
            variant_concurrency: 4,
            // Default lossy WebP quality, clamped to 0–100 by the encoder.
            webp_quality: 82.0,
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            // Expire sessions after two weeks without activity.
            idle_timeout_secs: 14 * 24 * 60 * 60,
            // Expire sessions after ninety days even if they remain active.
            absolute_timeout_secs: 90 * 24 * 60 * 60,
            cookie_name: "pv_session".to_owned(),
            // Safe default. Local plain-HTTP development sets this false
            // explicitly, which is the right way round — forgetting to turn it
            // *on* in production is the dangerous direction.
            cookie_secure: true,
        }
    }
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            // Chatty about our own code, quiet about everything else — the
            // shape you want when the log is a terminal you are watching.
            log_level: "info,pinvoyager_api=debug".to_owned(),
            // Pretty by default because the default audience is a developer.
            // Deployments set `PINVOYAGER__TELEMETRY__JSON=true`.
            json: false,
            otlp_endpoint: None,
            service_name: "pinvoyager-api".to_owned(),
        }
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
