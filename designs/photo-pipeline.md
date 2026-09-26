# Photo pipeline

Upload → validate → store → extract EXIF → derive variants → serve.

Three rules govern all of it:

- **Variants carry no metadata.** Every voyage is public, so a served image is a
  public artifact. If a user picks coarse granularity and the served file still
  carries its original GPS EXIF, the setting is decorative.
- **Nothing is served from disk directly.** Every read goes through a handler
  that sets a server-determined `Content-Type`, `nosniff`, and a restrictive
  `Content-Disposition`. The failure being avoided is a stored XSS.
- **Image work never runs on a runtime worker thread.** Decode, resize, encode
  and hashing are CPU-bound and go through `crate::blocking::run_blocking`.

## Upload, one file

`POST /api/photos` is multipart and may carry many files, but the Upload screen
sends one per request so each file gets its own outcome.

```mermaid
sequenceDiagram
    autonumber
    participant UI as Upload screen
    participant XHR as uploadPhotos (XMLHttpRequest)
    participant H as POST /api/photos
    participant FS as Storage root
    participant PG as Postgres
    participant Q as VariantQueue

    UI->>XHR: one File
    Note over XHR: XHR, not fetch — fetch reports no<br/>upload progress in any shipping browser
    XHR->>H: multipart body, withCredentials
    XHR-->>UI: progress events → per-row bar

    H->>H: gate 1 — per-user rate token
    H->>H: gate 2 — declared Content-Length vs max_upload_bytes
    H->>FS: create tmp/{uuid}.part under the storage root
    loop each chunk
        H->>H: running byte count vs max_upload_bytes
        H->>H: sniff the first bytes → ImageFormat, or reject
        H->>FS: write chunk
    end
    H->>FS: flush and close

    rect rgb(240,240,240)
        Note over H: one run_blocking for the whole sequence
        H->>FS: probe_dimensions
        H->>H: check_pixel_budget vs max_megapixels
        H->>FS: SHA-256 of the file
        H->>FS: exif::extract — captured_at, lat, lon
    end

    H->>FS: commit_original — rename into originals/{aa}/{bb}/{hash}
    H->>PG: INSERT photos (owner, hash, size, w, h, format,<br/>captured_at, exact lat/lon, variant_status='pending')
    H->>Q: enqueue(VariantJob) — awaits when full
    H-->>XHR: 201 with the photo rows
    XHR-->>UI: row becomes "processing"
```

The temporary file lives under the storage root rather than in the system temp
directory, so committing an upload is a rename within one filesystem. A rename
across filesystems is a copy, which would double the write cost of every upload
and stop being atomic.

Coordinates are stored **exact**. Rounding at upload would discard the precision
irreversibly and make the granularity setting unchangeable afterwards.

Identical bytes deduplicate: `commit_original` is idempotent, so two rows can
share one file on disk.

## The validation gates, in order

```mermaid
graph TD
    A["request arrives"] --> B{"rate token available?"}
    B -->|no| R429["429 rate_limited + Retry-After"]
    B -->|yes| C{"declared length ≤ max_upload_bytes?"}
    C -->|no| R413["413 too_large"]
    C -->|"yes, or chunked with no length"| D["stream to a temp file"]
    D --> E{"running total ≤ max_upload_bytes?"}
    E -->|no| R413
    E -->|yes| F{"magic bytes name a known format?"}
    F -->|no| R415["415 not_an_image"]
    F -->|yes| G{"width × height ≤ max_megapixels?"}
    G -->|no| R422["422 unprocessable_image"]
    G -->|yes| H["hash, EXIF, commit, INSERT, enqueue"]
    H --> OK["201"]
```

Each gate is placed where it costs least. The rate check is first because it
costs nothing — a limiter that fires halfway through a body has already paid for
the transfer. The declared-length check gives a truthful client its 413
immediately; the running total catches a lying client and a chunked body, which
declares no length at all.

The format sniff runs the moment enough bytes exist to identify the file, before
the rest of the body is written and long before any decoder sees it. A `.jpg`
full of HTML lands there — that is the stored-XSS defense.

The pixel budget is the only check that catches a decompression bomb. A
50,000 × 50,000 PNG is a few hundred kilobytes on disk and about 10 GB decoded,
so a byte-size limit never will.

## Variant generation, off the request path

```mermaid
sequenceDiagram
    autonumber
    participant Q as mpsc channel, depth = variant_queue_depth
    participant D as dispatch loop
    participant S as Semaphore(variant_concurrency)
    participant PG as Postgres
    participant CPU as run_blocking
    participant FS as Storage root

    loop forever
        D->>S: acquire a permit FIRST
        Note over D,S: permit before receive, so the channel is only<br/>drained when there is capacity to act on it
        D->>Q: recv()
        Q-->>D: VariantJob { photo_id }
        D->>D: spawn a task
    end

    D->>PG: SELECT photo — connection released on return
    alt row gone
        D-->>D: deleted before processing, no-op
    else already 'ready'
        D-->>D: sweep and live enqueue named the same photo
    else
        rect rgb(240,240,240)
            Note over CPU: no pool connection held across any of this
            D->>CPU: decode the original once
            CPU->>CPU: apply EXIF orientation
            CPU->>CPU: resize to thumb, medium, large (Lanczos3)
            CPU->>CPU: encode each as WebP from the pixel buffer
            CPU->>FS: write variants/{aa}/{bb}/{hash}.{variant}.webp
        end
        D->>PG: UPDATE variant_status = 'ready'
    end
```

On failure the row is marked `failed` in a separate statement, not a
transaction: the work that failed is on the filesystem, so there is nothing to
roll back. If *that* write also fails — plausible, since a database outage is a
likely cause of the failure above — the row stays `pending` and the next startup
sweep retries it. A transient outage should not permanently poison a photo, and
a genuinely undecodable file fails again immediately.

The strip is structural rather than a step someone can forget: encoding from a
decoded `DynamicImage` means there is no metadata attached to carry forward. The
tests assert it with `exiftool` anyway, because "we re-encoded it" and "we
stripped it" are different claims and an encoder upgrade can quietly change
which one is true.

### Photo lifecycle

```mermaid
stateDiagram-v2
    [*] --> pending: INSERT at upload
    pending --> ready: variants written
    pending --> failed: decode or encode failed
    pending --> pending: process died — startup sweep requeues
    ready --> [*]: DELETE
    failed --> [*]: DELETE
```

Without the startup sweep a process that died mid-batch would strand photos
forever: the row exists, the library shows it, and nothing would ever generate
its variants because the only thing that enqueued it was the request that has
long since returned. The sweep is capped at 1,000 rows and logs when it hits the
cap.

## Serving a variant

```mermaid
graph TD
    A["GET /api/photos/{id}/{variant}"] --> B{"variant in {thumb, medium, large}?"}
    B -->|no| N404["404"]
    B -->|yes| C["load the photo row"]
    C -->|"no row"| N404
    C --> D{"caller owns it?"}
    D -->|yes| G["allowed"]
    D -->|no| E{"belongs to a published voyage?"}
    E -->|yes| G
    E -->|no| N404
    G --> F{"variant_status"}
    F -->|pending| C409["409 variants_pending + Retry-After"]
    F -->|failed| N404
    F -->|ready| H["read variants/{aa}/{bb}/{hash}.{variant}.webp"]
    H -->|"file missing"| N404W["404 + a warning in the log"]
    H --> I{"If-None-Match matches the ETag?"}
    I -->|yes| N304["304, validators only, no body"]
    I -->|no| N200["200 + image/webp, nosniff,<br/>Content-Disposition, ETag, Last-Modified"]
```

The closed set of three variant names is the path-traversal answer: `../` is not
one of them, so it 404s before any path is built. Nothing user-supplied ever
reaches the filesystem — paths are derived from a 64-character hex SHA-256,
sharded two levels, which cannot contain a separator, a dot-dot, or a null byte
no matter what the uploader called their file.

A pending variant answers 409, not 500. It does not exist *yet*, which is not a
server error.

`variant_body` buffers the whole file rather than streaming it. The code says so
(`crates/api/src/http/photos.rs:584-596`): streaming needs a nameable `Stream`
type this crate does not have. The buffering is bounded — a variant is a WebP
whose longest edge is at most 2560 px — and the original, which can be 25 MB, is
never served at all.

## Storage layout

```mermaid
graph LR
    Root["{storage_root}"] --> Tmp["tmp/{uuid}.part"]
    Root --> Orig["originals/{aa}/{bb}/{sha256}"]
    Root --> Var["variants/{aa}/{bb}/{sha256}.thumb.webp<br/>…medium.webp<br/>…large.webp"]
```

`originals` and `variants` are siblings rather than nested so a variant sweep
never has to walk past an original. Two shard levels of two hex characters give
65,536 leaf directories, which keeps any one of them small well past any volume
this service will see.
