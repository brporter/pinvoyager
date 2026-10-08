# Upload and first-visit image regressions

This suite protects the D11 creation flow and the asynchronous image pipeline.
Production behavior is documented in `docs/voyage-owned-photos.md` and
`docs/designs/photo-pipeline.md`. These tests supplement the full suite; they do
not replace its coverage requirements.

## Tests and expected outcomes

| Scenario | Location and assertions |
| --- | --- |
| Details before files | `web/src/screens/__tests__/Upload.test.tsx`: required name/description, whitespace validation, next-step explanation, trimmed creation payload, no picker or bytes while creation is pending, preserved details after failure. |
| Duplicate or keyboard submission | `Upload.test.tsx`: double-click Next during deferred creation produces one draft; Enter from the name field submits details once. |
| Picker wiring and per-file requests | `Upload.test.tsx`: clicking choose files calls the input's click handler; two selected files produce exactly two scoped uploads. `web/src/components/__tests__/upload.test.tsx` also covers keyboard activation and drag events. |
| First visit after upload | `web/src/screens/__tests__/VoyageUploadFlow.test.tsx`: real `Upload`, `VoyageDetail`, API clients, hooks, QueryClient and routes; mock only fetch/XHR transports and the shared WebGL substitute. Create a draft, upload three files, review, navigate, then return readiness separately. Assert one, two, then three image elements with the correct URLs, no remaining skeletons, and no further polling. |
| Polling lifecycle | `web/src/api/__tests__/hooks.test.tsx`, `voyage image processing`: fake timers and controlled responses check staggered completion, terminal ready/failed statuses, stopping on unmount, recovery after a temporary request failure, and isolation from a late response for the previous voyage. |
| Partial batches | `Upload.test.tsx`: successes remain visible beside failures, retry only failed files, cancellation affects the active file, and no-GPS photos remain in their voyage. |
| Real HEIC processing | `crates/api/src/http/photos_tests.rs`: committed native HEIC fixture passes ingestion, queues work, starts pending, generates all variants, becomes ready, and serves readable WebPs. Truncated HEIC returns 422 `unprocessable_image`, creates no row, and queues no job. Corruption before conversion marks the row failed and serves no original. No fixture-generation skip can hide a missing decoder. |

The route test checks DOM image elements and their URLs; it does not claim that
jsdom decodes images or lays out the page like a browser. The backend tests
independently decode the actual served variant bytes.

## Run locally and in CI

Start PostgreSQL and install the native dependencies in `docs/running.md`.

```sh
npm --prefix web run test:regression
npm --prefix web run test -- --coverage
npm --prefix web run typecheck
npm --prefix web run lint
npm --prefix web run build

cargo test -p pinvoyager-api --lib heic_ -- --test-threads=1
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo coverage -- --test-threads=1
```

Run the full frontend checks for each change. Run backend tests and coverage for
image-processing changes. Use the existing CI provider to invoke these commands;
this repository does not currently contain a `.github/` workflow.

If the development API already occupies port 8080, run backend library coverage
under the normal environment first, then remaining workspace coverage using
port 18080 and skipping the configuration test already measured under its normal
environment:

```sh
cargo llvm-cov --workspace --lib --no-report -- --test-threads=1
PINVOYAGER__SERVER__BIND=127.0.0.1:18080 cargo llvm-cov --workspace --no-clean -- --test-threads=1 --skip config::tests::load_reads_the_repository_configuration_and_default_values
cargo llvm-cov report --fail-under-lines 100 --fail-under-functions 100 --fail-under-regions 100
```

## Prove regression sensitivity

Write each regression before changing production behavior. Confirm that it fails
for the reported outcome, then implement the fix and rerun it. When the fix already
exists, temporarily restore the specific broken behavior and restore the source
immediately after the focused test run.

For this execution, disabling `useVoyage` polling caused three failures: staggered
completion, temporary-failure recovery, and the first-visit route test. Eleven
other tests passed. Restoring polling made these regressions pass. No production
behavior changes were needed for this test expansion.

## Browser checks still required

Localhost browser automation is blocked by the saved browser preference. No
browser or headless-browser workaround was used. Screenshot comparisons and
actual browser image decoding were not executed or claimed as verified.

When browser testing is available, run this smoke sequence at desktop and narrow
mobile widths:

1. On step 1, verify name and description spacing, wrapping, and the next-step
   guidance. Save screenshots for reviewed baselines.
2. Continue; verify the drop target fits the viewport and the picker opens.
3. Upload a mixed JPEG/HEIC batch by picker and a separate batch by drag/drop.
4. Open the draft immediately. Without reloading, wait until every valid image
   has `complete === true` and `naturalWidth > 0`; confirm no loading animations
   remain and compare screenshots with the reviewed baselines.
5. Exercise one failed upload, retry, and narrow-screen action wrapping.

Do not approve screenshot baselines without inspecting them. Keep browser smoke
requests on the local API; do not bulk-fetch OSM tiles for test fixtures.

## Execution results

Measured on 6 October 2026:

| Check | Result |
| --- | --- |
| Focused frontend (`npm --prefix web run test:regression`) | 60 tests pass across four files. |
| Full frontend with V8 coverage | 455 tests pass across 40 files; statements, branches, functions, and lines all 100%. No new exclusions. |
| Type checking, web lint, production build | Pass; existing Fast Refresh/config-loader and bundle-size warnings remain. |
| Backend library coverage run | All 346 library tests pass, including all three new HEIC cases. |
| Remaining workspace coverage run | 345 repeated library tests pass, with the normal-environment configuration test already measured; startup test and all 62 integration tests pass. |
| Rust doctests (`cargo test -p pinvoyager-api --doc`) | One doctest passes; 410 unique backend tests pass in total. |
| Backend coverage thresholds | Fail: lines 98.11% (3633/3703), functions 96.35% (501/520), regions 95.05% (5007/5268). The existing deficit remains. |
| Rust format and diff whitespace | Pass. |
| Rust Clippy with warnings denied | Fails with 64 existing errors, including test unwraps, duplicate test-module paths, and an unused EXIF parameter. |
| Browser screenshots and native browser decoding | Not run: saved preference blocks localhost browser automation. |
| Running servers | API `/ready` succeeds; Vite `/upload` returns 200. |

Full frontend command:
`npm --prefix web run test -- --coverage --coverage.reportsDirectory=/tmp/pinvoyager-regression-coverage`.
Backend commands are the split coverage sequence above. The threshold report exits
1, as required when coverage is below 100%. No production runtime code needed
changes for this test expansion. No SQL queries were changed, so offline metadata
regeneration was unnecessary. Browser verification, the backend coverage deficit,
and existing Clippy errors remain unfinished.
