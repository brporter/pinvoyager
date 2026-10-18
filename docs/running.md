# Running locally

## Native prerequisites

The API uses libheif to decode HEIC images and libwebp to encode WebP variants. The
tests use exiftool to check that generated variants contain no original GPS, timestamp,
or camera metadata:

```sh
brew install libheif webp pkg-config exiftool     # macOS
# Debian/Ubuntu: libheif-dev libwebp-dev pkg-config libimage-exiftool-perl
```

- **libheif 1.17 or newer**, with an HEVC decoder plugin, is required for HEIC
  support. See `crates/api/src/photos/decode.rs::decode_heic` and D9 in
  [implementation constraints](architecture/constraints.md).
- **libwebp** supplies the lossy encoder used by
  `crates/api/src/photos/variants.rs::encode_variant`.
- **exiftool** is required by the metadata-removal tests in
  `crates/api/src/photos/variants_tests.rs`. Those tests fail if it is unavailable.

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
docker compose up -d          # PostgreSQL + Collector + Jaeger
export DATABASE_URL='postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager'
sqlx migrate run --source crates/api/migrations
cargo run -p pinvoyager-api
```

The committed `config.toml` already holds development values matching the Compose
database, so no `.env` is needed for a host-side run. Copy one when you want to override
settings locally:

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
needed when you want the schema in place without booting the service.

## Day to day

```sh
docker compose up -d          # development dependencies up
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

Debug builds can expose `/auth/dev/login` to create a session without provider
credentials. The committed `config.toml` enables it.

```sh
curl -i -X POST http://127.0.0.1:8080/auth/dev/login \
     -H 'content-type: application/json' \
     -d '{"subject":"alice@example.test"}'
```

The response sets a real session cookie; every authenticated endpoint then works
normally. Re-running with the same `subject` signs in to the same account rather
than creating a second one, exactly as a returning provider sign-in does.
New accounts have no display name until they choose one in the app. The legacy
`display_name` login field is accepted but ignored. Save a name using the
authenticated `PATCH /api/me` endpoint. The name migration preserves valid unique
handles and clears invalid, reserved, or duplicate legacy labels; those users
will see the same prompt on their next visit. See [display names](display-names.md).

The development provider stores `subject` as the account's provider identifier. The
example uses an email-shaped string to make that identifier easy to remember.
To test administration, select **Grant site administrator privileges** in
**Forge a session**, or add `"grant_admin":true` to the request above. The checkbox
starts unchecked. A true value promotes new or returning accounts; false or omitted
values preserve existing roles. Suspended accounts remain blocked.

Both sign-in paths check suspension, update identity activity, and create a session
in one transaction. The debug-only grant also commits in that transaction; see the
[shared pipeline](architecture/auth-flow.md#shared-sign-in-pipeline).

`#[cfg(debug_assertions)]` in
`crates/api/src/http/auth.rs::routes` omits it from release builds. To check that the
route string is absent from the release binary:

```sh
SQLX_OFFLINE=true cargo build --release
strings target/release/pinvoyager-api | grep -c '/auth/dev/login'   # must print 0
```

## Reviewing image-processing traces

`docker compose up -d` also starts an OpenTelemetry Collector and Jaeger.
The committed API configuration exports to the Collector on port 4318.
After creating a voyage and uploading photos, open [Jaeger](http://localhost:16686)
and select `pinvoyager-api` and `POST /api/voyages`. Expand each
`photo.process <photo UUID>` branch to compare queue wait, decode, resize,
encode, and write times. See [tracing](tracing.md) for the full walkthrough,
configuration, trace shape, and limitations.

## Watching for blocked runtime threads

D8 requires CPU-heavy image work and blocking file operations to run off Tokio worker
threads. Blocking a worker can delay other requests under concurrent load. Use
`tokio-console` to inspect task activity; passing unit tests alone does not establish
that runtime workers remain responsive.

```sh
RUSTFLAGS="--cfg tokio_unstable" cargo run --features tokio-console
tokio-console                 # in a second terminal
```

Both flags are required. The `tokio-console` feature installs the console layer; `--cfg
tokio_unstable` enables Tokio task instrumentation. Without the latter, the console has
no task data to display. Set it for this command only so ordinary builds do not include
that instrumentation.

## Tests

```sh
cargo test                    # needs the database up
cargo clippy --all-targets -- -D warnings
cargo fmt --check

# Backend coverage (requires cargo-llvm-cov and PostgreSQL)
# Use the same serial execution setting as the coverage check in CI.
cargo coverage -- --test-threads=1
```

Install the coverage subcommand once with `cargo install cargo-llvm-cov --locked`.
`cargo coverage` instruments all workspace tests and fails unless line,
function, and region coverage each reach 95%. Start PostgreSQL and set
`DATABASE_URL` as described under [First run](#first-run) first. See the
[test reference](testing.md) for coverage scope and test-environment limitations.

The backend command uses stable Rust and checks lines, functions, and regions.
Rust branch instrumentation requires nightly, so this project does not collect
branch percentages. Tests should cover both outcomes of each meaningful
conditional instead.

### On-demand photo performance regression

The large-image worker benchmark is ignored by normal test runs. It measures
the `photo.resize` and `photo.encode` OpenTelemetry spans using 12- and
24-megapixel JPEG, HEIC, and RGBA PNG inputs. See
[photo performance](photo-performance.md) for the unoptimized/optimized commands,
JSON reports, and saved-baseline regression checks.

### Web upload regression checks

Run `npm --prefix web run test:regression` for the upload controls, creation
screen, polling lifecycle, and first-visit route integration. Run
`npm --prefix web run test -- --coverage`, type checking, lint, and build for
full frontend validation. See [testing](testing.md) for native HEIC suites and the limits of browser substitutes.

## Queries and the offline cache

Queries are checked at compile time. After adding or changing one, regenerate
the committed `.sqlx` cache against a migrated database:

```sh
cargo sqlx prepare --workspace -- --all-targets
```

Skipping this breaks the container build, which compiles with `SQLX_OFFLINE=true`
and has no database to check against.

## Container

For the shared production VM, see [production deployment](deployment.md).

The Dockerfile uses Debian trixie and the libheif 1.17 API feature set supported
by its packaged native library. The runtime installs the HEVC decoder plugin.
The API runs as uid 10001, stores uploads at `/data`, and Compose mounts the
persistent `uploads` volume there.

```sh
docker build -t pinvoyager-api .
docker compose --profile api up --build    # database + service together
```

The API container connects to PostgreSQL at host `db`. Compose waits for the database
healthcheck to pass before starting the API so startup migrations can connect to an
initialized database.

Notes on the image:

- The image omits the development `config.toml`. Set
  `PINVOYAGER__DATABASE__URL` in the environment; the development database URL
  is not available as a fallback.
- The API runs as a non-root user (`app`, uid 10001). `/data` is the one writable
  directory it owns. The image sets the storage root to `/data`, and Compose
  mounts its named `uploads` volume there. Compose supplies the required
  authentication callback origin through `PINVOYAGER__AUTH__BASE_URL`, defaulting
  to `http://localhost:8080`. Set it to the externally reachable API origin when
  using another port or a proxy.
- The API binds `0.0.0.0:8080` so callers outside the container can reach it.
  The image sets `PINVOYAGER__SERVER__BIND` and a `HEALTHCHECK` using port 8080.
  Update the healthcheck if you change the bind port.
- Docker caches dependency compilation in a layer based on the Cargo manifests
  and lockfile. Changing application source reuses that layer; changing a
  manifest or `Cargo.lock` rebuilds it.

```sh
docker run --rm -p 8080:8080 \
  -e PINVOYAGER__DATABASE__URL='postgres://pinvoyager:pinvoyager@host.docker.internal:5432/pinvoyager' \
  -e PINVOYAGER__PHOTOS__STORAGE_ROOT=/data \
  -v pinvoyager-uploads:/data \
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
| `db` never reaches `healthy` | Inspect `docker compose logs db` and the existing volume before changing database storage. |
| Container build fails on a `query!` macro | The `.sqlx` cache is stale. Rerun `cargo sqlx prepare`. |

## First site administrator

Use the Rust operator tool against an existing account:

```sh
cargo run -p pinvoyager-api --bin pinvoyager-admin -- YOUR-ACCOUNT-UUID
# On the production host, from the Compose directory:
docker compose exec api pinvoyager-admin YOUR-ACCOUNT-UUID
```

It uses the API database configuration and reports a grant, an existing role, or a
missing-account error. Reload the app after success. See
[site administration](site-administration.md#bootstrap) for exit status,
suspension behavior, and development-only forging. Further promotions are available
at `/admin`.

## IDE launch configuration

`.run/Run pinvoyager-api.run.xml` runs `cargo run --package pinvoyager-api --bin
pinvoyager-api` without startup arguments. The API rejects the retired
`--initial-admin-email` flag. Use the development sign-in administrator checkbox
or the `pinvoyager-admin UUID` operator tool described above to grant the role.
