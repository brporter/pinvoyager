# syntax=docker/dockerfile:1

# PinVoyager API multi-stage build. The builder compiles both release binaries;
# the runtime includes the API, operator tool, native libraries, certificates, and curl.
# Both stages use the same Debian codename for runtime library compatibility.
ARG RUST_VERSION=1.94
ARG DEBIAN_CODENAME=trixie

# --------------------------------------------------------------------------
# builder
# --------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-slim-${DEBIAN_CODENAME} AS builder

# Native image libraries (D9). `image` has no HEIC decoder at all — not a
# feature flag, the decoder does not exist — so HEIC support means libheif, and
# lossy WebP means libwebp because the pure-Rust encoder is lossless-only.
# pkg-config is how the -sys crates find both.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libheif-dev \
        libwebp-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Compile checked SQL queries using committed .sqlx metadata without a live database.
# Regenerate the metadata after query changes and commit it.
ARG SQLX_OFFLINE=true
ENV SQLX_OFFLINE=${SQLX_OFFLINE}
# Forging handlers and grants must be absent from deployed binaries.
ENV CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=false

# --- dependency layer -----------------------------------------------------
# Copy only the manifests, stand up a placeholder crate with the same targets
# as the real one (a library and two binaries), and build it. Every third-party
# crate compiles here, into a layer keyed on Cargo.toml/Cargo.lock alone — so
# editing a .rs file below reuses this layer instead of recompiling the world.
COPY Cargo.toml Cargo.lock ./
COPY crates/api/Cargo.toml crates/api/Cargo.toml
RUN mkdir -p crates/api/src/bin \
    && echo 'fn main() {}' > crates/api/src/main.rs \
    && : > crates/api/src/lib.rs \
    && echo 'fn main() {}' > crates/api/src/bin/pinvoyager-admin.rs \
    && cargo build --release --locked \
    && rm -rf crates/api/src

# --- application layer ----------------------------------------------------
# Copy source and offline query metadata; .dockerignore excludes artifacts and secrets.
COPY . .

# Ensure the migrations directory exists for compile-time embedding.
RUN mkdir -p crates/api/migrations

# Docker preserves source mtimes from the build context, so the real sources
# can look *older* than the placeholder build's fingerprint and cargo will
# happily decide the crate is fresh — shipping a stub binary. Discard the
# placeholder fingerprint and stamp the sources so the rebuild is guaranteed.
RUN rm -rf target/release/.fingerprint/pinvoyager-api-* \
    && touch crates/api/src/lib.rs crates/api/src/main.rs \
    && cargo build --release --locked --bins

# --------------------------------------------------------------------------
# runtime
# --------------------------------------------------------------------------
FROM debian:${DEBIAN_CODENAME}-slim AS runtime

# ca-certificates: sqlx is built with rustls (tls-rustls-ring) and verifies the
#   PostgreSQL server certificate against the system trust store. A slim base
#   ships none, and the failure only appears against a TLS-requiring database —
#   i.e. in production, not locally.
# curl: only present for HEALTHCHECK below.
# libheif1 / libwebp7: the RUNTIME halves of the build dependencies above. The
#   binary is dynamically linked against both, so without them it does not start
#   at all — and the error is a loader message, not anything mentioning images.
#   The runtime packages are deliberately not the -dev ones: headers and static
#   archives are build-time only and would just be extra surface.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        libheif1 \
        libheif-plugin-libde265 \
        libwebp7 \
    && rm -rf /var/lib/apt/lists/*

# The service runs as an unprivileged user with a writable upload directory.
RUN groupadd --system --gid 10001 app \
    && useradd --system --uid 10001 --gid app --home-dir /app --shell /usr/sbin/nologin app

WORKDIR /app

# Binary stays root-owned and read-only to `app`: the service can execute it
# but cannot rewrite it.
COPY --from=builder --chown=root:root \
    /app/target/release/pinvoyager-api /usr/local/bin/pinvoyager-api

COPY --from=builder --chown=root:root \
    /app/target/release/pinvoyager-admin /usr/local/bin/pinvoyager-admin

# Migrations are embedded in the binary and applied at startup.

# Upload root (D7). Owned by `app` because it is the one directory the service
# writes to. Compose mounts the persistent uploads volume here.
RUN mkdir -p /data && chown app:app /data

USER app

# Containers must bind the wildcard address; a loopback default would make the
# service unreachable from outside the container and the mistake looks like a
# networking problem rather than a config one. Everything else comes from the
# environment — see .env.example for the full set.
ENV PINVOYAGER__SERVER__BIND=0.0.0.0:8080
ENV PINVOYAGER__PHOTOS__STORAGE_ROOT=/data
EXPOSE 8080

# Liveness only — /health deliberately does not touch the database, so a
# database blip does not get the container killed. Readiness (/ready) is the
# orchestrator's job, not Docker's.
# Note: hardcoded to the port above. Override PINVOYAGER__SERVER__BIND and this
# check must be overridden with it.
HEALTHCHECK --interval=15s --timeout=3s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/health || exit 1

# Run the API as PID 1 so it receives SIGINT/SIGTERM and starts shutdown.
ENTRYPOINT ["/usr/local/bin/pinvoyager-api"]
