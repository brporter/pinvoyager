# syntax=docker/dockerfile:1

# PinVoyager API — multi-stage build.
#
#   builder  compiles a release binary against the full Rust toolchain
#   runtime  carries the binary, the migrations, and nothing else
#
# Both stages share a Debian codename so the glibc the binary is linked
# against is the glibc it runs against. Change DEBIAN_CODENAME in one place
# and they stay in lockstep. (If `rust:<version>-slim-bookworm` is not
# published for the pinned toolchain, switch both to `trixie`.)
ARG RUST_VERSION=1.94
ARG DEBIAN_CODENAME=bookworm

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

# sqlx's query macros are checked against the committed `.sqlx` offline cache
# rather than a live database, so the build needs neither network access nor a
# DATABASE_URL. Regenerate the cache with `cargo sqlx prepare --workspace`
# whenever a query changes, and commit it.
ARG SQLX_OFFLINE=true
ENV SQLX_OFFLINE=${SQLX_OFFLINE}

# --- dependency layer -----------------------------------------------------
# Copy only the manifests, stand up a placeholder crate with the same targets
# as the real one (a `[lib]` and a `[[bin]]`), and build it. Every third-party
# crate compiles here, into a layer keyed on Cargo.toml/Cargo.lock alone — so
# editing a .rs file below reuses this layer instead of recompiling the world.
COPY Cargo.toml Cargo.lock ./
COPY crates/api/Cargo.toml crates/api/Cargo.toml
RUN mkdir -p crates/api/src \
    && echo 'fn main() {}' > crates/api/src/main.rs \
    && : > crates/api/src/lib.rs \
    && cargo build --release --locked \
    && rm -rf crates/api/src

# --- application layer ----------------------------------------------------
# Whole-context copy rather than per-path COPYs: .dockerignore already keeps
# the context small, and paths that do not exist yet (`.sqlx`, `migrations/`)
# would otherwise turn a missing directory into a build failure.
COPY . .

# `sqlx::migrate!` resolves against crates/api/migrations. Ensure the
# directory exists so the runtime stage's COPY is unconditional.
RUN mkdir -p crates/api/migrations

# Docker preserves source mtimes from the build context, so the real sources
# can look *older* than the placeholder build's fingerprint and cargo will
# happily decide the crate is fresh — shipping a stub binary. Discard the
# placeholder fingerprint and stamp the sources so the rebuild is guaranteed.
RUN rm -rf target/release/.fingerprint/pinvoyager-api-* \
    && touch crates/api/src/lib.rs crates/api/src/main.rs \
    && cargo build --release --locked --bin pinvoyager-api

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
        libwebp7 \
    && rm -rf /var/lib/apt/lists/*

# Non-root from the start. Phase 4 writes user uploads to disk (D7); a process
# that never runs as root cannot be talked into writing outside what it owns.
RUN groupadd --system --gid 10001 app \
    && useradd --system --uid 10001 --gid app --home-dir /app --shell /usr/sbin/nologin app

WORKDIR /app

# Binary stays root-owned and read-only to `app`: the service can execute it
# but cannot rewrite it.
COPY --from=builder --chown=root:root \
    /app/target/release/pinvoyager-api /usr/local/bin/pinvoyager-api

# `sqlx::migrate!` embeds the migrations in the binary, so these are not needed
# to boot. They are here for phase 8, which moves migrations out of startup and
# into a deploy step that runs `sqlx migrate run` against this image.
COPY --from=builder --chown=root:root /app/crates/api/migrations /app/migrations

# Upload root (D7). Owned by `app` because it is the one directory the service
# is expected to write to. Mount a volume here when phase 4 lands.
RUN mkdir -p /data && chown app:app /data

USER app

# Containers must bind the wildcard address; a loopback default would make the
# service unreachable from outside the container and the mistake looks like a
# networking problem rather than a config one. Everything else comes from the
# environment — see .env.example for the full set.
ENV PINVOYAGER__SERVER__BIND=0.0.0.0:8080
EXPOSE 8080

# Liveness only — /health deliberately does not touch the database, so a
# database blip does not get the container killed. Readiness (/ready) is the
# orchestrator's job, not Docker's.
# Note: hardcoded to the port above. Override PINVOYAGER__SERVER__BIND and this
# check must be overridden with it.
HEALTHCHECK --interval=15s --timeout=3s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/health || exit 1

# Exec form, no init shim: the binary runs as PID 1 and installs its own
# SIGINT/SIGTERM handlers, which is what makes `docker stop` drain in-flight
# requests instead of severing them.
ENTRYPOINT ["/usr/local/bin/pinvoyager-api"]
