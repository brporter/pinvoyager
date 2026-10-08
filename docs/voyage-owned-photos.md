# Voyage creation and photo ownership

Decision D11 in `docs/decisions.md` replaces the former photo library. Every photo
record belongs to exactly one voyage, including drafts and photos without GPS.

## User flow

All New voyage links lead to `/upload`. `web/src/screens/Upload.tsx` (`Upload`)
shows required **Voyage Name** and **Voyage Description** fields in the first step.
The screen explicitly says, “You will be able to upload your photos at the next
step.” **Next: upload photos** trims both fields, rejects blank values, and
creates a private draft before opening the second step. File selection is not
available until creation succeeds. A creation failure preserves the entered
details and allows retry without sending photo bytes.

The second step shows the drop target and file picker. Dropping or choosing files
starts uploading immediately into that draft. **View voyage** also allows the
owner to leave this step with an empty draft. Existing-voyage upload links skip
the details step because their destination already exists.

Uploads run sequentially, with a result and retry action per file. Accepted
photos remain in their voyage even if another file fails or lacks GPS. Review
shows the batch and offers **View voyage**; it offers no library or destination
selection. Initial uploads append in submission order; the editor can set an
explicit order. Adding photos from `VoyageEdit` links to `/upload?voyage={id}`,
which starts uploading dropped or chosen files immediately without creating another draft.

`VoyageDetail` (`PublishVoyage`) shows a large primary **Publish voyage** button
for the owner of a draft on both desktop and mobile. `VoyageEdit` puts the same
primary action in its page header. Publishing remains a deliberate PATCH;
creating or uploading never publishes automatically. Failed publication displays
an error and leaves the draft available for editing. Published voyages retain
the editor's unpublish action. D6 and D7 still govern visibility and coordinates.

These are deliberate changes from `design/screens/upload-flow.html`: name and
description precede photo selection, review has no library assignment, and
publication is prominent on draft detail and edit pages. Existing shared design
tokens and Button/Field/Dropzone components provide the styling. The details
fields use the 24 px section spacing token. See
`docs/designs/navigation.md` for the source-based spacing review.

## API and database

| Operation | Behavior |
| --- | --- |
| `POST /api/voyages/{id}/photos` | Stream multipart uploads directly into an owned voyage. Missing or foreign voyages return 404 before reading bytes; anonymous callers return 401. |
| `GET /api/voyages/{id}/photos` | Paginated owner-only exact photo metadata for that voyage. Missing or foreign voyages return 404. |
| `GET /api/photos/{id}` | Owner-only exact metadata for an existing voyage-owned photo. |
| `GET /api/photos/{id}/{variant}` | Re-encoded, metadata-stripped variants; draft photos remain owner-only, published photos are publicly viewable. |
| `DELETE /api/voyages/{id}/photos/{photo_id}` | Delete that voyage's photo record, rather than detach it into a library. |
| `DELETE /api/photos/{id}` | Owner-only deletion of a voyage-owned photo record. |
| `DELETE /api/voyages/{id}` | Delete the voyage and all its photo records. |
| `GET` or `POST /api/photos` | 404; the independent library and unscoped upload routes do not exist. |
| JSON `POST /api/voyages/{id}/photos` with `photo_ids` | 400; the old later-attachment API does not exist. |

The implementation is in `crates/api/src/http/photos.rs` (`upload_to_voyage`,
`owned_voyage`, `ingest_part`, `list`, `destroy`) and
`crates/api/src/http/voyages.rs` (`detach`, `destroy`). The API client requires
`UploadOptions.voyageId`; `useUploadPhotos` invalidates the destination voyage's
detail, map, list, and photo metadata caches.

Migration `20261006000001_photos_belong_to_voyages.sql` adds required
`photos.voyage_id` and `photos.position`. The composite foreign key requires the
photo and voyage to have the same owner and cascades voyage deletion to photos.
The old `voyage_photos` table becomes a view over those columns, so existing
map/discovery queries cannot create independent membership. The repository's
`move_photos` supports internal reassignment of already voyage-owned records for
fixtures; no HTTP endpoint exposes it.

Uploads lock the voyage and recheck ownership before committing the original
and photo row. Uploads and deletion share content-hash advisory locks through
`crates/api/src/db/repo/photos.rs` (`lock_content`). Deletion checks remaining
references before removing files, preserving content shared by other records.
Filesystem cleanup failures are logged; the deletion still removes records.
A pre-existing worker race can leave variant files if generation finishes after
a photo was deleted; it cannot recreate a photo record or library membership.

## Existing data

The migration preserves each existing voyage membership. A photo previously
shared by multiple voyages becomes separate records referring to the same
stored bytes. Unattached photos move into a private **Recovered photos** draft
for their owner. Applying the migration does not publish any voyage or delete
existing photo content. Deploy the updated API with the migration: the former
API's unscoped inserts are incompatible with the required voyage column.

## Validation

Tests first demonstrated the missing name/upload flow and prominent publish
buttons, the old upload URL, accepted unattached database inserts, and absent
ownership migration. After implementation, the corresponding tests pass.

- `web/src/screens/__tests__/Upload.test.tsx`: required name, no bytes before
  draft creation, create failure/retry, existing destinations, partial batches,
  no-GPS photos, cancellation, review navigation, and preview cleanup.
- `VoyageDetail.test.tsx` and `VoyageEdit.test.tsx`: prominent publication,
  desktop/mobile ownership, publish errors, precision, and editing behavior.
- `web/src/api/__tests__/clientUpload.test.ts` and `hooks.test.tsx`: scoped
  upload URL, transport results, and cache changes.
- `crates/api/tests/photo_ownership.rs`: mandatory ownership, matching owners,
  and migration of both library and multiply-associated legacy records.
- `crates/api/src/http/photos_tests.rs`: scoped upload/list authorization,
  direct association, vanished destinations, absent library routes, and storage.
- `crates/api/src/http/voyages_tests.rs`: removal and voyage deletion, including
  preservation and eventual deletion of shared content.
- Existing repository, public-access, recovery, granularity, and API contract
  tests continue to exercise the new ownership model.

Full frontend coverage command:
`npm --prefix web run test -- --coverage --coverage.reportsDirectory=/tmp/pinvoyager-voyage-coverage`.
It passes 447 tests with 100% statements, branches, functions, and lines, without
adding exclusions. The web build, type checking, web lint (existing warnings), Rust build,
`cargo fmt --check`, and `cargo sqlx prepare --workspace -- --all-targets` pass.
`SQLX_OFFLINE=true cargo build` also passes with regenerated metadata.

The backend suite passes 407 tests: 343 library tests, one binary startup test,
62 integration tests, and one doctest. To leave the running API on port 8080,
the binary test uses `PINVOYAGER__SERVER__BIND=127.0.0.1:18080`; the configuration
fixture is tested under the normal environment. Coverage combines a normal
`cargo llvm-cov --workspace --lib --no-report` run with a `--no-clean` workspace
run on port 18080 that skips the already-tested configuration fixture.

Stable backend coverage is **98.11% lines, 96.35% functions, and 94.99% regions**.
`cargo llvm-cov report --summary-only --fail-under-lines 100
--fail-under-functions 100 --fail-under-regions 100` fails those thresholds.
The existing backend coverage deficit remains, including uncovered error paths
in changed handlers; no exclusions were added. `cargo clippy --all-targets --
-D warnings` also still fails with the existing duplicate-module, unused-variable,
needless-borrow, and test `unwrap` errors (64 errors in the library test target).
**The feature is not marked complete under the repository's coverage requirement.**

A live request check through Vite on port 5173 passes named draft creation,
scoped upload, private draft visibility, preservation of an unplaced photo,
variant generation, explicit publication, anonymous access to the published
variant, photo and voyage deletion, and absent library routes. Automated visual
browser inspection was not run; component tests cover desktop and mobile UI.


### Two-step creation and image readiness — 6 October 2026

`Upload` starts new voyages at `details`, creates the draft through its `create`
mutation, and then opens `choose`. Existing voyage destinations start at `choose`.
`Upload.onFiles` starts `runBatch` immediately for dropped or chosen files. Tests
in `Upload.test.tsx` cover required and whitespace-only details, trimmed API data,
pending creation, failure/retry, no picker before creation, both upload controls,
existing voyages, and viewing an empty draft.

Image ingestion returns before background variant generation completes.
`web/src/api/hooks.ts` (`useVoyage`) now refreshes once per second while any
photo has `variant_status: pending`, so detail and edit screens replace loading
animations without a page refresh. Polling stops when every variant is ready or
failed and is inactive for missing query data. The hook regression in
`web/src/api/__tests__/hooks.test.tsx` (`voyage image processing`) exercises a
pending response, a ready/failed response, and cessation of further requests.
Previously, this query never refreshed after the worker finished.

Tests were written first: the details suite had 17 expected failures and 11 passes;
the readiness regression failed because the query stayed pending. Validation
results for the final implementation are recorded below.

Final validation: 449 frontend tests pass with 100% statements, branches,
functions, and lines, without new exclusions. Command:
`npm --prefix web run test -- --coverage --coverage.reportsDirectory=/tmp/pinvoyager-details-coverage`.
Type checking, lint, production build, and `git diff --check` pass. Existing Fast
Refresh lint and bundle-size build warnings remain. API readiness and the Vite
upload route respond successfully. Browser interaction was not checked because
the saved browser preference blocks localhost automation. Backend code did not
change; backend tests and coverage were not rerun. The previously recorded
backend coverage shortfall remains unfinished.

## Regression suite expansion

`docs/upload-regression-tests.md` records the additional tests for keyboard and
duplicate creation submission, picker activation, the real upload-to-detail
route transition, polling lifecycle, and native HEIC upload/variant failures.
Run the focused frontend cases with `npm --prefix web run test:regression`.
Its execution results supersede the earlier test counts above.
