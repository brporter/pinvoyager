# Test coverage plan

## Current baseline

Measurements below come from the latest frontend and backend test runs. The
frontend command was
`npm run test -- --coverage --coverage.reportOnFailure --maxWorkers=1` in
`web/`; the backend command was
`DATABASE_URL='postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager' cargo coverage -- --test-threads=1`.
Both suites passed on 2026-09-29. The backend coverage command exits with
status 1 because its configured 100% thresholds remain unmet. The latest
serial run passed 336 library unit tests, 1 main-binary test, and 59
integration tests (all integration suites; zero ignored). It covered 3,511 of
3,596 lines, 491 of 510 functions, and 4,854 of 5,126 regions.

| Area and scope | Statements / regions | Lines | Branches | Functions | Test result |
|---|---:|---:|---:|---:|---|
| Frontend production files (Vitest V8) | 100% statements | 100% | 100% | 100% | 443 passed, 39 files (2026-09-28) |
| Backend production source (cargo-llvm-cov 0.9.1, stable) | 94.69% regions | 97.64% | Not collected | 96.27% | 336 library unit tests, 1 binary test, 59 integration tests passed; 0 ignored (2026-09-29) |

Frontend tests cover the app root mount and retry callback outcomes in
`web/src/__tests__/main.test.tsx`, all `AccountMenu` lines, functions, and
branches in `web/src/shell/__tests__/AccountMenu.test.tsx`, and the hidden,
focused, and blurred `SkipLink` states. `DevSignIn`'s request, callback, error,
pending, and disabled-input cases are covered in
`web/src/auth/__tests__/DevSignIn.test.tsx`. API endpoint, fetch error, and
upload transport behavior is covered in
`web/src/api/__tests__/client.test.ts` and `clientUpload.test.ts`. Query keys,
retry policy, pagination, read states, mutation behavior, and cache changes are
covered in `web/src/api/__tests__/hooks.test.tsx`. The latest run passed all
443 tests across 39 files. Sign-in rendering and provider redirects are covered
in `web/src/auth/__tests__/SignIn.test.tsx`; `SignIn.tsx` now has 100% statement,
line, function, and branch coverage. `useAuth`'s provider and outside-provider
outcomes are covered in `web/src/auth/__tests__/useAuth.test.tsx`. Callback
retry, cancellation, provider restart, and conflict paths are covered by
`web/src/auth/__tests__/AuthCallback.test.tsx`; authentication now has 100%
statement, line, and function coverage. Upload
selection, success and failure rows, progress transitions, cancellation, retry,
review, new-voyage creation, attachment filtering, and object URL cleanup are
exercised in `web/src/screens/__tests__/Upload.test.tsx`; `Upload.tsx` now has
full statement, line, function, and branch coverage.
Settings loading and recovery, profile saves, local precision, provider safety,
and deletion outcomes are tested in `web/src/screens/__tests__/Settings.test.tsx`;
`Settings.tsx` has full statement, line, function, and branch coverage. Voyage edit
missing/error/loading, empty-photo, save-success, and delete-confirmation states
are tested in `web/src/screens/__tests__/VoyageEdit.test.tsx`; `VoyageEdit.tsx`
now has full statement, line, function, and branch coverage.
`VoyageDetail.tsx` now has full statement, line, function, and branch coverage
after tests for unpublished voyages, invalid and missing capture dates, plural
unplaced-photo counts, generic read failures, reduced motion, sheet key handling,
and both drag snap positions. `Discover.tsx` has 100% statement, line, and function
coverage and complete branch coverage after tests for desktop layout, requested
skeleton counts, and API error references. `Settings.tsx` branch coverage
is now complete after testing empty, malformed, null, and wrongly typed API
error bodies, blank profile names, provider unlink states, and relative dates.
An unreachable fallback for the only linked provider was removed because the
surrounding size check guarantees a provider key. The latest frontend run has
100% statements (1,392/1,392), lines (1,266/1,266), functions (478/478), and
branches (1,268/1,268) across the configured production scope.
Drag entry, nested leave tracking, unsupported and unknown drag types, empty
dragovers, and disabled handling in `Dropzone` now have 100% line and function
coverage. Profile pagination and the desktop join date are exercised in
`web/src/screens/__tests__/Profile.test.tsx`; `Profile.tsx` has full statement,
line, and function coverage. `MapCanvas` bounds updates and marker handling are
covered by `web/src/map/__tests__/MapCanvas.test.tsx`; selected and missing photo
paths in `VoyageMap` are tested in `web/src/map/__tests__/VoyageMap.test.tsx`.
The connected top navigation is tested for desktop and mobile and all auth
states in `web/src/shell/__tests__/TopNav.test.tsx`; responsive app and auth
shell branches are covered in `web/src/shell/__tests__/Shells.test.tsx`. Shell
branches now have complete coverage. The `Toast` info icon,
default tone, and polite role are tested in
`web/src/components/__tests__/feedback.test.tsx`; all component statements and
lines and branches are now covered. Additional component cases cover empty
button content, keyboard keys outside the activation set, custom/dark map
attribution, and empty drop payloads. Tests also verify emoji initials.
The measurement includes every production `.ts` and `.tsx` file under
`web/src/`, including files not imported by tests, and excludes test files,
`__tests__`, fixtures, mocks, setup, declarations, and other test-only helpers.
The backend report follows moving inline unit-test modules and the shared
photo fixtures into `*_tests.rs` files, which do not appear in the report. The
earlier raw test-build report was 81.61% regions, 82.61% lines, and 81.60%
functions, but included test implementation code. The measured production
coverage is now 97.64% lines, 96.27% functions, and 94.69% regions. The
`auth/attempt.rs`, `config.rs`, and `telemetry.rs` production modules now each
have 100% lines,
functions, and regions. Auth-attempt tests cover wrong-length binding hashes,
database errors, and a corrupt provider row in an isolated test database;
config tests cover defaults, successful loading, and error formatting.
`crates/api/src/error_tests.rs` checks all HTTP error categories, caller-safe
messages, database retry mapping, hidden internal details, and nested sources.
The tests install an enabled tracing subscriber and emit both error and
rejection events. cargo-llvm-cov still reports three logging-field source
lines and seven regions in `error.rs` uncovered. Investigate the macro source
mapping and, if needed, restructure logging so instrumentation maps back to the
executed production expressions.
`auth/extract.rs` now has 100% line, function, and region coverage after testing
database lookup failures for both required and optional authentication. The
tests use a lazy pool with a short connection timeout. `http/health.rs` now also
has full line, function, and region coverage with a successful database-backed
readiness response. `crates/api/src/db/mod_tests.rs` covers successful
PostgreSQL connections and
readiness queries plus connection and readiness failures. This brings
`db/mod.rs` to 100% lines, functions, and regions. Apple key parsing,
startup probing, secret-source minting, and invalid-key discovery are covered
in `crates/api/src/auth/providers/apple_tests.rs`; Apple line coverage is now
89.92%. Apple discovery, form-post authorization, and exchange are tested
against the local OIDC provider. `auth/dev.rs` now has 100% function
coverage and 100% lines after
direct handler tests for disabled access, invalid requests, account creation,
and returning identity lookup in `crates/api/src/auth/dev_tests.rs`. No test
modules or shared photo fixtures appear in the report. Rust branch
instrumentation requires nightly, so this project stays on stable Rust and
does not report a backend branch percentage. Backend tests should exercise
both outcomes of meaningful runtime conditionals. SIGTERM and shutdown-drain
tests run serially as part of the backend coverage command (`docs/running.md`).

`crates/api/src/main_tests.rs` runs startup through configuration, telemetry,
PostgreSQL, migrations, providers, and worker creation, then verifies that an
occupied listener produces a failure exit code. `main.rs` now has 83.67% line,
100% function, and 79.41% region coverage; startup success and some later
failure paths remain uncovered.

Worker tests now exercise dispatch shutdown, recovery query errors, empty and
non-empty recovery queues, and a closed recovery queue in
`crates/api/src/photos/worker_tests.rs`. Worker line coverage increased from
58.99% to 96.40%; full-suite recovery behavior remains in
`crates/api/tests/photos_recovery.rs`.
Storage tests now exercise directory read errors, parent-directory creation
errors, missing temporary files, filesystem removal errors, and best-effort
temporary cleanup in `crates/api/src/photos/storage_tests.rs`. Storage line
coverage increased from 73.27% to 96.36%, and all storage functions are
covered. Tests also force the copy path and verify the cross-device fallback.

OIDC tests cover issuer spelling and scheme validation, authorization
scope/extra parameters, adapter exchange, and successful JWKS refresh in
`crates/api/src/auth/providers/oidc_tests.rs`; OIDC line coverage is 91.76%.
D1 documents the HTTP(S)-issuer requirement.
`crates/api/tests/auth_flow.rs` now covers linking a new provider, repeating a
link for the same account, rejecting an identity owned by another account,
preserving both accounts after conflict, and returning-sign-in behavior.
`http/auth.rs` line coverage is 95.99%. A SQLx test in
`crates/api/src/http/auth_tests.rs` verifies PostgreSQL unique-violation mapping.

`http/mod.rs` now has a testable shutdown future: `serve` still uses the OS
signal handler, while `serve_with_shutdown` permits deterministic graceful
shutdown tests. Serial tests send SIGINT and SIGTERM and exercise a stalled
request's drain deadline. Its line coverage is 89.71%.

Worker tests cover every function, including queued-job processing and permit
release.

The latest backend run covered 3,596 executable lines and missed 85, covered
4,854 of 5,126 regions, and executed 491 of 510 functions. It passed 336
library unit tests, one binary test, and 59 integration tests, including
seven router-level auth flow tests in `crates/api/tests/auth_flow.rs`; no tests
were ignored.
The backend measurements came from `cargo-llvm-cov 0.9.1`. Its command and
thresholds are configured in `.cargo/config.toml` as `cargo coverage`.

The current uncovered backend scope is 85 lines, 19 functions, and 272
regions. Uncovered production lines are in `auth/providers/facebook.rs` (150),
`http/auth.rs` (398, 573, 634), `http/photos.rs` (214–215, 256, 998, 1094,
1105), `http/voyages.rs` (903), `main.rs` (56, 77), `photos/decode.rs`
(165, 179–180), `photos/exif.rs` (71, 143, 152–153, 168, 174, 205, 208, 213,
218, 226), `photos/storage.rs` (142, 202–203, 272), and `photos/worker.rs`
(161, 193, 197, 235, 337).
Missed functions and regions cluster in provider error handling, request
handlers, and external-library or OS failure paths. The logging refactor
precomputes tracing fields in `auth/provider.rs`, `auth/providers/oidc.rs`,
`http/mod.rs`, and `main.rs`; it raised line coverage by 0.11 percentage
points and region coverage by 0.13 points. Use the annotated `cargo llvm-cov
report --show-missing-lines` after each complete run to update this list.
`error.rs` now formats error and source-chain strings before passing them to
tracing; all its lines and functions are covered. A non-finite map-bound test exercises
the Decimal fallback in `crates/api/src/http/voyages_tests.rs`;
`http/voyages.rs` now has 99.42% line and 100% function coverage. Facebook's
malformed authorization-base test covers its URL error conversion; Apple tests
cover an invalid signing key and the startup panic conversion. Image serving
tests cover invalid variants and a filesystem read error. A shared
`observe_signal` helper makes SIGINT and SIGTERM setup-error behavior
deterministic; tests cover both failures and the normal signal paths. Three
database tests now set and compare timestamps using PostgreSQL's clock, which
avoids failures when the database host and test host clocks differ. SQL trigger
tests exercise both unique-violation race paths in `http/auth.rs`. The
discovery clustering loop now accepts its statically paired `f64` coordinates
directly; the query selects `latitude IS NOT NULL`, and the table constraint
requires latitude and longitude to be null together, so its former `None`
branch could not occur for a `DiscoverPhoto` row.

## Plan to reach 100%

Each item starts with tests. Run the new tests against the current behavior and
confirm they fail for the expected missing case before changing implementation
or coverage configuration. Then implement the smallest change that makes them
pass, rerun the focused tests and full suite, and record the new coverage. Do
not add coverage exclusions to make a threshold pass; justify any unavoidable
instrumentation exclusion here.

### 1. Coverage scope and thresholds

- **Frontend scope is configured:** `web/vitest.config.ts` includes every
  production `.ts` and `.tsx` file under `web/src/`, including unimported
  modules, and excludes test files and helpers. Statement, line, function, and
  branch thresholds are 100%. The latest full-scope run passed 443 tests and
  measured 100% statements, lines, functions, and branches.
  `DevSignIn.tsx` now has full coverage across all four measures.
  `hooks.ts` and `client.ts` now have 100% statement, line, function, and
  branch coverage. The API client and hook files are complete within the
  frontend scope.
- **Backend thresholds are configured:** `.cargo/config.toml` defines `cargo
  coverage` with 100% line, function, and region thresholds on stable Rust.
  Inline unit tests and shared photo fixtures live in `*_tests.rs` files. The
  test-only multipart source variant was removed; parser tests now deliver
  fragmented bodies through the normal body stream. The
  production coverage is 97.64% lines, 96.27% functions, and 94.69% regions.
  Coverage work remains. Branch percentages are unavailable. Tests should
  exercise both outcomes of meaningful runtime conditionals. The service
  runtime is Tokio's default multi-thread runtime from `crates/api/src/main.rs`.
  Measure both default builds and the optional `tokio-console` feature in
  separate coverage runs; verify that the dev-login route is absent from the
  release artifact using the check in `docs/running.md`. Linux is the Docker
  deployment target and macOS is a documented local development target; both
  use the Unix shutdown handler in `crates/api/src/http/mod.rs`. The non-Unix
  shutdown fallback is outside the current declared build matrix and has not
  been checked.
- Run both complete suites in CI. Threshold failures mean coverage work remains.

### 2. Raise frontend coverage

Address the largest gaps first across the declared production scope:

1. **API client and query hooks:** `web/src/api/hooks.ts` and
   `web/src/api/client.ts` now have 100% statement, line, function, and branch
   coverage after testing query outcomes, pagination, transport errors,
   optional values, cache invalidation, and mutation results. Tests are in
   `web/src/api/__tests__/client.test.ts`,
   `clientUpload.test.ts`, and `contract.test.ts`.
2. **Screens:** `Upload.tsx`, `Settings.tsx`, `VoyageEdit.tsx`, and
   `VoyageDetail.tsx` now each have 100% statement, line, function, and branch
   coverage. Tests cover upload progress, errors, retry, destination choices,
   location edge cases, profile saves, local precision, provider safety,
   publication changes, sheet interaction, and deletion outcomes.
3. **Authentication:** `AuthCallback.tsx` now has 100% statement, line, and
   function coverage after adding callback, cancellation, retry, and provider
   restart tests. `SignIn.tsx` and `DevSignIn.tsx` also have full statement,
   line, function, and branch coverage from tests in `web/src/auth/__tests__/`.
   `DevSignIn.tsx` coverage comes from
   `web/src/auth/__tests__/DevSignIn.test.tsx`. `main.tsx` and `AccountMenu.tsx`
   now have complete line, statement, function, and branch
   coverage from `web/src/__tests__/main.test.tsx` and
   `web/src/shell/__tests__/AccountMenu.test.tsx`. `SkipLink`'s hidden,
   focused, and blurred states are covered in
   `web/src/shell/__tests__/SkipLink.test.tsx`.
4. **Remaining uncovered branches:** use the coverage report to work through
   uncovered branches and functions in all other frontend modules. Keep
   fixtures and API types synchronized when tests expose contract mismatches.

### 3. Raise backend coverage

Start with modules with the largest unvisited or uncovered areas in the current
report:

1. **Request authentication and routing:** `crates/api/src/http/auth.rs`
   (95.99% lines), `http/mod.rs` (89.71%), `http/photos.rs` (98.36%), and
   `http/voyages.rs` (98.64%). Add focused tests
   for every route outcome, provider callback error, account-linking result,
   cookie/session condition, timeout, and shutdown path. Unit tests now cover
   callback browser outcomes (success, missing provider, cancellation, conflict,
   generic failure, and filtered request references), cookie attributes,
   duration clamping, provider parsing, generic auth errors, redirect header
   validation, and form-field lookup in `crates/api/src/http/auth_tests.rs`.
   `crates/api/tests/auth_flow.rs` now exercises GET start/callback/current-user,
   callback error outcomes, form POST callback success, and unknown providers.
   These tests found and fixed binding-cookie verification ordering and failed
   callback cookie removal. Continue with deterministic router-level tests for
   the remaining uncovered route outcomes.
2. **Provider paths:** `crates/api/src/auth/providers/apple.rs` (89.92% lines),
   `crates/api/src/auth/providers/oidc.rs` (91.76%), and
   `crates/api/src/auth/provider.rs` (98.11% lines). Add deterministic provider
   fakes for success, invalid responses, missing fields, discovery/JWKS
   failures, and configuration combinations; do not require live provider
   credentials. Facebook tests verify the provider kind, Graph API version,
   redirect parameters, state, PKCE challenge, minimal scope, token exchange
   form, bearer-token profile request, response mapping, and HTTP/JSON failure
   paths, including transport failure on code exchange. Remaining provider
   coverage gaps are identified in the report sweep.
3. **Runtime and configuration:** `crates/api/src/main.rs` (83.67% lines),
   `crates/api/src/http/mod.rs` (89.71%), `crates/api/src/db/mod.rs` (100% lines/functions/regions),
   `crates/api/src/error.rs` (93.33% lines), and `crates/api/src/auth/dev.rs` (100% lines and functions).
   `db/mod.rs` has 100% line, function, and region coverage. `telemetry.rs` and `config.rs` now have 100% line,
   function, and region coverage. Extract small testable constructors where
   necessary, then cover valid and invalid config, startup/shutdown wiring,
   development-auth behavior. `auth/dev.rs` is 100% function covered; its
   remaining uncovered source line is the provider field in its warning event.
   Telemetry filter selection, subscriber install
   failure, and JSON/pretty formatter construction now have full coverage in
   `crates/api/src/telemetry_tests.rs`.
4. **Photo worker and remaining branches:**
   `crates/api/src/photos/worker.rs` (96.40% lines), `photos/storage.rs`
   (96.36%), `photos/decode.rs` (94.12%), `photos/validate.rs` (97.01%), and
   `photos/variants.rs` (98.00%). `worker_tests.rs` covers derivation, corrupt
   input, atomic replacement and path errors, queue closure, and backpressure.
   Database-backed cases in `crates/api/tests/photos_recovery.rs` cover a
   deleted photo, an already ready photo, successful generation and status
   update, and failure marking. Cover dispatch/recovery scheduling,
   storage/decode errors, boundary input, and cleanup. The
   `photos.rs` and `voyages.rs` repositories have 100% line and function
   coverage; fill the remaining uncovered regions in
   `db/repo/photos.rs` (96.72% regions) and `db/repo/voyages.rs` (95.45%).
5. **Sweep the whole report:** cover all remaining lines, regions, functions,
   and branches in `crates/api/src/`. Keep tests deterministic and use the
   existing PostgreSQL integration setup for SQL and transaction behavior.

### 4. Enforce the target

- Run frontend tests with coverage and the configured four 100% thresholds
  (`--maxWorkers=1` avoids worker startup and timeout failures in this environment).
- Run `cargo coverage -- --test-threads=1` with PostgreSQL running; require its line, function,
  and region thresholds to pass.
- Keep coverage collection on stable Rust. The CI check should run frontend
  coverage with its four 100% thresholds and `cargo coverage -- --test-threads=1` with its three
  100% thresholds. Review tests for both outcomes of each meaningful backend
  conditional because backend branch percentages are not available on stable.
- Add both coverage commands to CI, then make a feature complete only when the
  tests, applicable thresholds, and feature documentation pass review.

## Related commands and code

- Backend command: `.cargo/config.toml` (`coverage` alias); prerequisites:
  `docs/running.md`, “Tests” and “First run”.
- Frontend tests: `web/package.json` (`test` script),
  `web/vitest.config.ts`, and tests under `web/src/**/__tests__/`.
- Backend tests: `crates/api/src/**` unit tests and `crates/api/tests/`
  integration tests.
