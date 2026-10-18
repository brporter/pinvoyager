# Voyage creation and photo ownership

Every photo record belongs to one voyage with the same owner. The database
requires that association at insertion. There is no standalone photo library or
later attachment endpoint. Photos without GPS remain in their voyage.

## Creation and upload

[`Upload`](../web/src/screens/Upload.tsx) starts with required Voyage Name, Voyage
Description, and Location precision fields. Precision defaults to ~1 km for every new
voyage. The four choices are Exact, ~1 km, ~10 km, and ~50 km. Continuing trims the text
and creates a private draft before showing the file picker. Failure preserves the fields
and sends no photo bytes.

`http/voyages.rs::create` also defaults omitted API precision to `km1`. The
`20261017000001_default_voyage_granularity.sql` migration sets the same default
for database inserts that omit the column. Explicit choices and existing voyages
keep their selected precision. `http/voyages_tests.rs::create_update_and_delete_handlers_apply_owner_changes`,
`db/repo/voyages_tests.rs::omitted_granularity_defaults_to_one_kilometer`, and
`web/src/screens/__tests__/Upload.test.tsx` cover these defaults and explicit choices.

Choosing or dropping files starts sequential uploads into that draft. Accepted
photos remain even when another file fails. `web/src/screens/Upload.tsx::ProcessStep`
shows one batch progress bar, a processed-file count, and the current filename with
its upload or location-reading status. The bar advances when each request succeeds
or fails, including cancellation; transferring all bytes does not finish a file.
Queued and accepted photos do not get individual rows. Failures retain per-file
retry and removal controls inside a keyboard-focusable region limited to 18rem in
height (`design/components.css::.pv-upload-errors`). This keeps hundreds of selected
or failed files from extending the page. The active file can be canceled while
uploading or waiting for acceptance. Original-image previews are never loaded.
`web/src/screens/__tests__/Upload.test.tsx` covers 300 queued files, 100 failures,
current-file changes, progress, cancellation, retry, and review. Adding
to an existing voyage uses `/upload?voyage={id}` and skips draft creation.
Photos append in submission order. The API supports reordering, but the current
editor photo grid has no reorder or removal controls.

Review displays the accepted batch and its exact owner coordinates on `UploadReviewMap`.
View voyage opens the destination voyage. Creating the draft writes its ID into the URL,
so reload resumes that draft. Files and progress remain in component memory. The batch
hook aborts active requests and stops queued files on unmount. Stable per-file UUID
retry keys prevent uncertain responses from duplicating accepted photos.

## Publication and editing

[`PublishVoyage`](../web/src/screens/voyage/VoyageChrome.tsx) presents a
Publish voyage action to the draft owner on desktop and mobile. Creation and
upload never publish automatically. Publication changes visibility independently
of variant processing, so a published voyage can contain pending or failed photos.

[`VoyageEdit::EditForm`](../web/src/screens/VoyageEdit.tsx) provides Discard,
Publish/Unpublish, and Save. The back action and Discard return without saving. Save
preserves publication state. Publish sends the current title, description, precision,
and `published: true` in one update; the API validates the effective values while
holding the voyage row lock through the update. The row lock prevents concurrent edits
from changing those values between validation and the update.

The voyage view's Share action opens a dialog with a public URL and copy action,
or prompts its owner to publish. See [sharing](voyage-sharing.md).

## Precision controls and review map

`GranularitySelector` uses `PrecisionPreview`, centered on St. Louis at 38.627,
-90.1994. No example marker appears before a choice. Exact draws a pin; coarse options
use per-axis half-widths of 500, 5,000, and 25,000 meters. The preview circle uses the
cell diagonal. The preview has the shared tiles, controls, and attribution. See [map
data flow](architecture/map-data-flow.md) for projection and how the maps represent
precision. Cards, voyage headings, photo captions, and gallery omit precision labels;
creation and editing retain the controls. Public coordinates still use the stored
precision.

`LazyUploadReviewMap` loads the review map when review is displayed. The details
step can already load MapLibre through the precision preview. Review fits located
photos with maximum zoom 16; no locations leaves the world view. Photos missing
coordinates remain counted beneath the map. Markers do not wait for WebP readiness,
and review does not fetch original images or processed previews.

## Card previews and processing

Voyage summaries expose `cover_url` and `photos_pending`. The cover is the first
ready photo in position/ID order; pending and failed images are skipped. An
unplaced photo can be a cover. `covers_for_voyages` fetches this information for a
whole page in one query. Cards on discovery, My voyages, and named profiles share
`components/VoyageCards.tsx::VoyageCard`.

List/profile hooks poll every second while returned summaries report pending photos; the
detail hook polls on photo statuses. Polling stops when no returned photos are pending
or when the component unmounts. The first ready cover can change when an earlier photo
finishes. Variant requests authorize at the API and require private cache revalidation,
including after unpublishing. See [image processing and serving](photo-pipeline.md).

## API, schema, and migration

| Endpoint | Behavior |
| --- | --- |
| `POST /api/voyages` | Create an owned draft; the UI supplies name, description, and precision. |
| `GET/POST /api/voyages/{id}/photos` | Owner-only metadata listing and upload; unavailable/foreign destination returns 404. |
| `PATCH /api/voyages/{id}/photos/order` | Reorder an owned voyage's photos. |
| `DELETE /api/voyages/{id}/photos/{photo}` | Delete the photo record from that owned voyage. |
| `DELETE /api/photos/{id}` | Delete an owned photo. |
| `DELETE /api/voyages/{id}` | Delete the owned voyage and cascade its photo records. |

`photos.voyage_id` is non-null; a composite foreign key enforces matching voyage
and photo ownership. `voyage_photos` is a compatibility view over `photos`.
Physical files use content hashes and can be shared by separate records. Deletion
records cleanup in the row-deletion transaction and returns 204 after commit.
Worker maintenance removes files in the background; the request does not wait
for filesystem work. Cleanup locks the content hash and rechecks references; failures remain
queued for retry. A failed database commit leaves referenced files intact.

[`20261006000001_photos_belong_to_voyages.sql`](../crates/api/migrations/20261006000001_photos_belong_to_voyages.sql)
converts old multiple memberships into distinct records sharing stored bytes and
places unassociated photos in private Recovered photos voyages. It does not publish
those voyages. The updated schema rejects inserts without a voyage.

Implementation lives in `http/photos.rs`, `http/voyages.rs`, and `db/repo/voyages.rs`
under `crates/api/src/`. `photo_ownership.rs` and `photos_public_access.rs` cover
ownership/visibility. Frontend upload, edit, profile, and readiness suites are
listed in [testing](testing.md).
