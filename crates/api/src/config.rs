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

// Defaults for the `database` section. These cannot live in a `Default` impl
// the way the other sections' do, because `DatabaseConfig` deliberately has no
// `Default` — `url` is required and there is no honest value for it. Defaulting
// the section as a whole would invent one.
const DEFAULT_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_MIN_CONNECTIONS: u32 = 1;
/// D8: finite, and short. This is the delay before pool exhaustion becomes a
/// 503; longer values just queue more requests behind an already-saturated pool.
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

    /// Largest accepted upload. Rejected before the body is read, so an
    /// oversized request costs a header parse rather than a disk write.
    pub max_upload_bytes: u64,

    /// Largest accepted decoded image, in megapixels.
    ///
    /// This is the decompression-bomb guard and it is checked from the header
    /// alone, before any full decode: a 50,000 × 50,000 PNG is a few hundred
    /// kilobytes on disk and about 10 GB in memory. A byte-size limit does not
    /// catch it; only a pixel-count limit does.
    pub max_megapixels: u32,

    /// Uploads permitted per user per minute.
    pub upload_rate_per_minute: u32,

    /// How many variant-generation jobs may queue before the producer blocks.
    ///
    /// Bounded on purpose: an unbounded channel converts a burst of uploads
    /// into unbounded memory, and backpressure that shows up as a slow upload
    /// is far better than an out-of-memory kill.
    pub variant_queue_depth: usize,

    /// How many images may be processed concurrently.
    ///
    /// Distinct from the queue depth, and the number that actually protects the
    /// runtime: image work is CPU-bound (D8) and runs on the blocking pool, so
    /// letting it run unboundedly wide starves everything else on the machine.
    pub variant_concurrency: usize,

    /// Quality for the lossy WebP variants, 0–100.
    pub webp_quality: f32,
}

/// Authentication configuration.
#[derive(Debug, Deserialize)]
pub struct AuthConfig {
    /// Public base URL of this service, used to build provider redirect URIs.
    /// Must match what is registered in each provider's console exactly —
    /// a trailing-slash mismatch is rejected by every provider with an
    /// unhelpful error.
    pub base_url: Url,
    #[serde(default)]
    pub session: SessionConfig,
    /// Every provider is optional. The service starts with any subset configured.
    /// An unconfigured provider's endpoints return 404.
    #[serde(default)]
    pub providers: ProvidersConfig,

    /// Enables `POST /auth/dev/login`, which forges a sign-in without any
    /// provider round trip.
    ///
    /// **This is a complete authentication bypass.** Anyone who can reach the
    /// endpoint can become any account. It exists so the rest of the product
    /// can be built and tested before provider credentials are in
    /// hand, and it is defended in three independent layers:
    ///
    /// 1. The handler and its route are behind `#[cfg(debug_assertions)]`, so
    ///    they are **not compiled into a release binary at all**. The container
    ///    image builds `--release`, so the code does not exist there. This is
    ///    the guarantee that matters; the other two are for someone running a
    ///    debug build somewhere unexpected.
    /// 2. This flag defaults to `false`, so a debug build still refuses unless
    ///    it is turned on deliberately.
    /// 3. Startup logs a loud warning whenever it is active.
    #[serde(default)]
    pub dev_login: bool,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    /// Sliding: each authenticated request extends the session by this much.
    pub idle_timeout_secs: u64,
    /// Hard ceiling regardless of activity. A sliding timeout alone means a
    /// stolen cookie stays valid indefinitely so long as it is used.
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
    /// Per-request timeout, including uploads and queue backpressure.
    pub request_timeout_secs: u64,
}

/// `SecretString` has no `Debug` that reveals its contents, which is what
/// satisfies the "printing the config does not reveal a secret" criterion.
/// Do not replace it with `String` for convenience.
#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub url: SecretString,
    pub max_connections: u32,
    pub min_connections: u32,
    /// D8: never unset. Without it, pool exhaustion hangs requests forever
    /// instead of surfacing as a 503.
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
    /// A figment error is a chain — one extraction can fail on several keys at
    /// once — and reporting only the first sends an operator round the loop
    /// once per mistake. Report all of them.
    fn from_figment(err: figment::Error) -> Self {
        let messages: Vec<String> = err.into_iter().map(describe).collect();
        ConfigError::Invalid(messages.join("; "))
    }
}

/// Renders one figment error so that it names the setting the operator has to
/// go and fix.
///
/// figment's own `Display` is kept for the *reason* (`missing field \`url\``,
/// `invalid type: found string, expected u32`), but not for the key: it prints
/// the path of the value that failed to deserialize, which for a missing field
/// is the enclosing table (`database`, not `database.url`), profile-prefixed,
/// and tagged with the source location of whichever provider happened to
/// supply the parent — for a missing required key that is the defaults
/// provider, which is exactly the wrong place to send someone looking.
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
    /// Loads configuration, failing loudly on anything missing or malformed.
    ///
    /// A missing required setting must produce an error naming the setting.
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
        }
    }
}

impl Default for PhotosConfig {
    fn default() -> Self {
        Self {
            storage_root: PathBuf::from("data"),
            // 25 MB. Comfortably above a modern phone's HEIC or JPEG, and well
            // below what a 200-megapixel medium-format raw would need — which
            // is fine, because raw is a declared non-goal.
            max_upload_bytes: 25 * 1024 * 1024,
            // 100 MP. Above any consumer camera and far below the pixel counts
            // that make a decode bomb interesting.
            max_megapixels: 100,
            upload_rate_per_minute: 60,
            variant_queue_depth: 256,
            // Leave headroom: the blocking pool also serves file I/O, and
            // saturating every core with image work makes the service
            // unresponsive in exactly the way D8 warns about.
            variant_concurrency: 4,
            // 82 is the usual sweet spot for photographic WebP — visually
            // indistinguishable from the source at normal viewing sizes, and
            // roughly a third of an equivalent JPEG.
            webp_quality: 82.0,
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            // Sliding: two weeks of inactivity ends the session. Generous,
            // because this is a photo-browsing site and being signed out is
            // pure friction — there is no money and no admin surface behind
            // the cookie.
            idle_timeout_secs: 14 * 24 * 60 * 60,
            // But not unbounded. A sliding timeout on its own means a stolen
            // cookie lives forever provided it keeps being used; the absolute
            // ceiling is what eventually forces a fresh sign-in.
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
