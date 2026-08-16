# Running locally

## Prerequisites

- **Docker must be running.** PostgreSQL comes from Compose; the integration
  tests need it too.
- Rust toolchain matching `rust-version` in the root `Cargo.toml`.
- `sqlx-cli`, for migrations and the offline query cache:

  ```sh
  cargo install sqlx-cli --no-default-features --features rustls,postgres
  ```

## First run

```sh
docker compose up -d          # PostgreSQL only
export DATABASE_URL='postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager'
sqlx migrate run --source crates/api/migrations
cargo run -p pinvoyager-api
```

The committed `config.toml` already holds development values matching the
Compose database, so no `.env` is needed for a host-side run. Copy one when you
want to change something without editing a committed file:

```sh
cp .env.example .env
set -a; source .env; set +a   # environment outranks config.toml
```

Then:

```sh
curl localhost:8080/health    # liveness  — 200 even with the database down
curl localhost:8080/ready     # readiness — 503 if the database is unreachable
```

The service also applies migrations at startup, so `sqlx migrate run` is only
needed when you want the schema in place without booting the service. Phase 8
moves migrations to a deploy step.

## Day to day

```sh
docker compose up -d          # database up
docker compose ps             # `db` should read (healthy)
docker compose logs -f db
docker compose down           # stop; the pgdata volume survives
docker compose down -v        # stop and DESTROY the data
psql "$DATABASE_URL"          # shell into the database
```

`POSTGRES_USER` / `POSTGRES_PASSWORD` / `POSTGRES_DB` only take effect on the
**first** boot of a fresh volume. Changing them later does nothing until
`docker compose down -v`.

## Watching for blocked runtime threads

D8's rule is that nothing blocking may run on a runtime worker thread. That
class of bug does not fail a test — it shows up as tail latency under
concurrency — so `tokio-console` is how you actually see it.

```sh
RUSTFLAGS="--cfg tokio_unstable" cargo run --features tokio-console
tokio-console                 # in a second terminal
```

**Both parts are required.** The feature flag installs the console layer, but
tokio only emits task instrumentation under `--cfg tokio_unstable`; with the
flag alone the console connects and shows nothing.

The cfg is deliberately not set in `.cargo/config.toml`, because that would
apply it to every build including the release binary in the container image.

## Tests

```sh
cargo test                    # needs the database up
cargo clippy --all-targets -- -D warnings
cargo fmt --check

# Sends a real SIGTERM to the test process, so it is #[ignore]d by default
# and must run alone.
cargo test --lib -- --ignored sigterm --test-threads=1
```

## Queries and the offline cache

Queries are checked at compile time. After adding or changing one, regenerate
the committed `.sqlx` cache against a migrated database:

```sh
cargo sqlx prepare --workspace -- --all-targets
```

Skipping this breaks the container build, which compiles with `SQLX_OFFLINE=true`
and has no database to check against.

## Container

```sh
docker build -t pinvoyager-api .
docker compose --profile api up --build    # database + service together
```

The service in Compose reaches the database at host `db`, not `localhost`, and
waits for its healthcheck before starting — otherwise it races initdb and fails
its first migration.

Notes on the image:

- It ships **no `config.toml`**. That file is a development file pointing at a
  localhost database; baking it in would give a container a fallback that
  quietly hides a missing `PINVOYAGER__DATABASE__URL`. In a container the
  environment is the only configuration source.
- Runs as a non-root user (`app`, uid 10001). `/data` is the one writable
  directory it owns; phase 4 uploads go there.
- Binds `0.0.0.0:8080`. A loopback bind inside a container is unreachable from
  outside it, so the image sets `PINVOYAGER__SERVER__BIND` itself. Overriding it
  means also overriding the image's `HEALTHCHECK`, which is hardcoded to that
  port.
- Dependencies compile in a layer keyed on the manifests alone, so a source-only
  change does not recompile every crate. Touching `Cargo.toml` or `Cargo.lock`
  does — that is expected and takes a while.

```sh
docker run --rm -p 8080:8080 \
  -e PINVOYAGER__DATABASE__URL='postgres://pinvoyager:pinvoyager@host.docker.internal:5432/pinvoyager' \
  -e PINVOYAGER__DATABASE__MAX_CONNECTIONS=10 \
  -e PINVOYAGER__DATABASE__MIN_CONNECTIONS=1 \
  -e PINVOYAGER__DATABASE__ACQUIRE_TIMEOUT_SECS=5 \
  pinvoyager-api
```

## Troubleshooting

| Symptom | Cause |
|---|---|
| Startup fails naming a setting | It has no default and no value. See `.env.example`. |
| `port is already allocated` | Something else holds 5432. Set `POSTGRES_PORT` in `.env` and update both URLs. |
| Connection refused from the container | Used `localhost` where the container needs `db` (Compose) or `host.docker.internal` (plain `docker run`). |
| `db` never reaches `healthy` | `docker compose logs db` — usually a half-initialised volume. `docker compose down -v`. |
| Container build fails on a `query!` macro | The `.sqlx` cache is stale. Rerun `cargo sqlx prepare`. |
