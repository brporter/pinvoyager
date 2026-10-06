# Running locally

## Native prerequisites

Phase 4 links against two C libraries, and needs a third tool to verify its
central guarantee:

```sh
brew install libheif webp pkg-config exiftool     # macOS
# Debian/Ubuntu: libheif-dev libwebp-dev pkg-config libimage-exiftool-perl
```

- **libheif** — `image` has no HEIC decoder at all. It is not a feature flag;
  the decoder does not exist, largely because HEIC wraps HEVC. iPhones shoot
  HEIC by default, so this is not optional (D9).
- **libwebp** — the pure-Rust WebP encoder is lossless-only, and lossless WebP
  on a photograph is typically larger than the source JPEG, which would invert
  the reason for choosing WebP.
- **exiftool** — the test suite runs it against generated variants to prove no
  GPS, timestamp, or camera data survives. Without it that test cannot run, and
  that test is what makes the granularity feature real rather than cosmetic.

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

## Signing in without provider credentials

Debug builds expose a forged sign-in, so the rest of the product can be built
before four sets of OAuth credentials exist. `config.toml` enables it.

```sh
curl -i -X POST http://127.0.0.1:8080/auth/dev/login \
     -H 'content-type: application/json' \
     -d '{"subject":"alice@example.test","display_name":"Alice"}'
```

The response sets a real session cookie; every authenticated endpoint then works
normally. Re-running with the same `subject` signs in to the same account rather
than creating a second one, exactly as a returning provider sign-in does.

`subject` is stored as the **provider subject**, not as an email — this service
holds no email address anywhere. An email-shaped string is a memorable
handle for choosing which account you are.

**This is a complete authentication bypass.** It is absent from release builds
rather than disabled in them, because the route sits behind
`#[cfg(debug_assertions)]`. To verify that claim rather than trust it:

```sh
SQLX_OFFLINE=true cargo build --release
strings target/release/pinvoyager-api | grep -c '/auth/dev/login'   # must print 0
```

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

# Backend coverage (requires cargo-llvm-cov and PostgreSQL)
# Run serially because shutdown tests send signals to their test process.
cargo coverage -- --test-threads=1
```

Install the coverage subcommand once with `cargo install cargo-llvm-cov --locked`.
`cargo coverage` instruments all workspace tests and fails unless line,
function, and region coverage each reach 100%. Start PostgreSQL and set
`DATABASE_URL` as described under [First run](#first-run) first. See the
[coverage plan](coverage-plan.md) for the current baseline, branch coverage
limitation, and work needed to reach the thresholds.

The backend command uses stable Rust and checks lines, functions, and regions.
Rust branch instrumentation requires nightly, so this project does not collect
branch percentages. Tests should cover both outcomes of each meaningful
conditional instead.

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
| `db` never reaches `healthy` | `docker compose logs db` — usually a half-initialized volume. `docker compose down -v`. |
| Container build fails on a `query!` macro | The `.sqlx` cache is stale. Rerun `cargo sqlx prepare`. |
