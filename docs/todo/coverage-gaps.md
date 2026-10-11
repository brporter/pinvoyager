# Backend coverage gaps

Analysis date: 2026-10-11. Measurements come from the backend coverage run on
2026-10-10. The report was regenerated for this analysis without rerunning tests.
These figures describe that working-tree snapshot; regenerate them after code
or test changes.

## Measured shortfall

The gaps are primarily failure paths within otherwise tested code, plus several
validation checks and tracing-related coverage entries.

| Metric | Covered / total | Coverage | Uncovered |
| --- | --- | --- | --- |
| Lines | 3,947 / 3,986 | 99.02% | 39 |
| Functions | 584 / 587 | 99.49% | 3 |
| Regions | 5,442 / 5,652 | 96.28% | 210 |

The run passed 440 backend tests and ignored one opt-in performance test. The
coverage command failed because the repository requires 100% for all three
metrics. See [testing](../testing.md) and [the coverage command](../../.cargo/config.toml).

Regions are executable ranges within source lines. For example, the success path
through `operation()?` can execute while its early error return remains
uncovered. Line, function, and region counts overlap and must not be added
together. Regions are not the nightly Rust branch-coverage metric.

## Gaps by area

| Area | Missing regions | What remains uncovered |
| --- | ---: | --- |
| HTTP handlers: `http/{auth,photos,profiles,voyages}.rs`, `http/voyages/discovery.rs` | 101 | Database failures during authentication, profile updates, uploads, and voyage mutations; some validation failures; a photo disappearing during deletion; cleanup failing after voyage deletion commits; one ownership rejection path. |
| Image processing and cleanup: `photos/{cleanup,decode,exif,storage,validate,worker}.rs` | 43 | Database failures in cleanup, failures propagating from final variant writes, certain malformed EXIF values, native decoder/metadata failures, and tracing fields. |
| Authentication and providers: `auth/{dev,provider}.rs`, `auth/providers/` | 37 | Provider initialization and secret-generation failures, database failures during development login, and provider logging fields. |
| HTTP server lifecycle: `http/mod.rs` | 14 | Remaining logging and generated async-code regions. Startup, bind failure, graceful shutdown, and the timeout for finishing active requests passed their tests in this run. |
| Database repositories: `db/repo/{photos,voyages}.rs` | 9 | Query failures and malformed timestamp/UUID components of pagination cursors. |
| Other infrastructure: `blocking.rs`, `error.rs`, `main.rs`, `telemetry.rs` | 6 | Migration failure during startup, telemetry initialization failure propagation, formatting failures while displaying an error chain, and blocking-task coverage entries. |
| **Total** | **210** | |

Paths in this table are relative to [`crates/api/src/`](../../crates/api/src/).
A missing failure return at one call site does not mean the underlying operation
or its general error conversion has no tests.

## Three unexecuted functions

All three closures convert failures into application errors. The surrounding operations
have tests.

| Location | Unexecuted behavior |
| --- | --- |
| [`auth/providers/apple.rs`](../../crates/api/src/auth/providers/apple.rs), `AppleProvider::discover` | Convert an invalid Apple issuer URL into `AuthError::Configuration`. |
| [`auth/providers/mod.rs`](../../crates/api/src/auth/providers/mod.rs), `provider_http_client` | Convert an HTTP client construction failure into `AuthError::Configuration`. |
| [`photos/validate.rs`](../../crates/api/src/photos/validate.rs), `probe_heic` | Convert failure to obtain the HEIC container's primary image into `PhotoError::Undecodable`. |

## Specific behavioral gaps

| Missing case | Implementation and existing coverage | Check needed |
| --- | --- | --- |
| A photo disappears between its initial lookup and deletion. | [`http/photos.rs`](../../crates/api/src/http/photos.rs), `destroy`, returns 404 when `delete_returning_orphaned` returns no row. The initial lookup's missing-photo case is separate. | Coordinate two deletion operations so the second reaches this return, and verify its 404 response. |
| Voyage deletion commits but immediate filesystem cleanup fails. | [`http/voyages.rs`](../../crates/api/src/http/voyages.rs), `destroy`, logs the failure and leaves cleanup for retry. [`photos/cleanup_tests.rs`](../../crates/api/src/photos/cleanup_tests.rs), `cleanup_failure_is_retained_and_a_later_pass_removes_it`, already tests the cleanup retry mechanism itself. | Cause cleanup to fail through the voyage handler; verify deletion still succeeds and the cleanup record remains available for retry. |
| A non-owner reaches the voyage ownership helper. | [`http/voyages.rs`](../../crates/api/src/http/voyages.rs), `owned`, has an uncovered owner-mismatch return. Other ownership checks already have tests. | Exercise a caller of this helper with a foreign voyage and verify 404 without mutation. |
| A cursor contains the separator but an invalid component. | [`db/repo/voyages.rs`](../../crates/api/src/db/repo/voyages.rs), `PublishedCursor::parse`, has uncovered timestamp and UUID parse failures. Successful tied-timestamp pagination is tested. | Test an invalid timestamp with a valid UUID, then a valid timestamp with an invalid UUID; verify rejection. |
| An intermediate database operation fails after earlier work succeeds. | The handler, repository, and cleanup modules above contain uncovered `?` error returns. | Exercise representative query and commit failures and verify the response, rollback, and retained file/cleanup state where applicable. |

## Tracing and instrumentation uncertainty

Some reported gaps require investigation before treating them as missing
behavioral tests. For example, LLVM marks variant tracing fields in
[`photos/worker.rs`](../../crates/api/src/photos/worker.rs), `encode` and
`write_variants`, as uncovered. However,
[`tests/image_tracing.rs`](../../crates/api/tests/image_tracing.rs),
`creation_upload_and_each_background_photo_export_one_correlated_trace`, explicitly
checks exported `photo.variant` attributes for `thumb`, `display`, and `full`.

That is evidence of a reporting discrepancy for those fields. It does not
establish that every tracing, async, or error-path entry is an instrumentation
artifact. Compare the affected compiled instances and report regions with the
behavioral assertions before deciding what remains untested.

No production coverage exclusions were added. Do not remove defensive error
handling or exclude production code merely to reach the threshold. Document any
technically unavoidable instrumentation exclusion with specific evidence.

## Reproducing the report

Follow [running locally](../running.md) for PostgreSQL and native image
dependencies, then run from the repository root:

```sh
DATABASE_URL='postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager' \
SQLX_OFFLINE=true cargo coverage -- --test-threads=1

cargo llvm-cov report --show-missing-lines
cargo llvm-cov report --json --output-path /tmp/pinvoyager-backend-coverage.json
cargo llvm-cov report --text --output-dir /tmp/pinvoyager-backend-coverage
```

The first command reruns tests and enforces the thresholds. The report commands
inspect the existing instrumentation data; they do not rerun tests. Coverage
reports alone do not prove concurrency, authorization, or privacy correctness;
retain the behavioral regression tests when closing these gaps.
