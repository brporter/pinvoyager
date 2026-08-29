/**
 * TypeScript shapes for every API response.
 *
 * CONTRACT — these mirror the `Serialize` structs in the Rust service. Each one
 * names the Rust type it corresponds to so the pair can be found from either
 * side.
 *
 * ## How drift is caught
 *
 * These types are written by hand, which means nothing stops them from
 * disagreeing with the server. The defense is a pair of tests rather than code
 * generation:
 *
 *  1. A Rust integration test drives each endpoint and writes the real response
 *     body to `src/api/__fixtures__/<name>.json`.
 *  2. A TypeScript test imports each fixture and assigns it to the type below.
 *     A field the server added, removed, or renamed makes that assignment fail
 *     to compile.
 *
 * The fixtures are captured from the running service, so they are the actual
 * wire format rather than somebody's description of it. Both tests must exist
 * for this to work — the TypeScript half alone would only check that the types
 * match themselves.
 */

/** Rust: `voyages::Coordinate`. Fields are private in Rust; this is its
 *  serialized form.
 *
 *  `precision_m` is absent — not null — when the voyage is at exact precision.
 *  When present it is the radius in meters that the map should draw as an area
 *  instead of a pin, because a sharp pin at the center of a rounded coordinate
 *  claims accuracy the value does not have. */
export interface Coordinate {
  latitude: number;
  longitude: number;
  precision_m?: number;
}

/** The four precision settings. Matches the database CHECK constraint and
 *  `voyages::Granularity`. */
export type Granularity = "exact" | "km1" | "km10" | "km50";

/** Whether a photo's derived images have been generated yet. A photo is usable
 *  in the interface before its variants exist, so `pending` needs a real
 *  treatment rather than being assumed away. */
export type VariantStatus = "pending" | "ready" | "failed";

// ---------------------------------------------------------------------------
// Voyages — Rust: `http::voyages`
// ---------------------------------------------------------------------------

/** Rust: `VoyagePhotoResponse`. */
export interface VoyagePhoto {
  id: string;
  position: number;
  captured_at: string | null;
  width: number;
  height: number;
  variant_status: VariantStatus;
  thumbnail_url: string;
  display_url: string;
  full_url: string;
  /** Null when the photograph carries no location. That is an ordinary
   *  outcome — screenshots, scans, and any camera with location services off —
   *  and such photos must still be shown, not filtered out. */
  coordinate: Coordinate | null;
  placed: boolean;
}

/** Rust: `VoyageResponse`. The full read of one voyage. */
export interface Voyage {
  id: string;
  owner_id: string;
  title: string;
  description: string | null;
  published: boolean;
  granularity: Granularity;
  /** Server-supplied label — "Exact", "~1 km", "~10 km", "~50 km". Rendered
   *  rather than derived on the client so the three places it appears cannot
   *  disagree. */
  granularity_label: string;
  precision_m: number | null;
  created_at: string;
  published_at: string | null;
  photo_count: number;
  placed_count: number;
  /** Distinct locations after rounding, which is the "12 places" beside
   *  "48 photos". Counted after rounding on purpose: counting before would
   *  tell a viewer how many separate spots exist inside one cell. */
  place_count: number;
  unplaced_count: number;
  photos: VoyagePhoto[];
}

/** Rust: `VoyageSummary`. Carries no location of any kind. */
export interface VoyageSummary {
  id: string;
  owner_id: string;
  title: string;
  description: string | null;
  published: boolean;
  granularity: Granularity;
  granularity_label: string;
  precision_m: number | null;
  created_at: string;
  published_at: string | null;
}

/** Rust: `voyages::ListResponse`. */
export interface VoyageList {
  voyages: VoyageSummary[];
  /** Opaque cursor for the next page, or null at the end. Pass it back as the
   *  `before` query parameter. Do not parse it. */
  next_before: string | null;
}

/** Rust: `MapPin`. */
export interface MapPin {
  photo_id: string;
  coordinate: Coordinate;
  thumbnail_url: string;
}

/** Rust: `MapUnplaced`. `placed` is always false. */
export interface MapUnplaced {
  photo_id: string;
  thumbnail_url: string;
  placed: boolean;
}

/** Rust: `MapResponse`. Deliberately leaner than `Voyage` — the map fetches
 *  this for every pin on screen, so anything it will not draw is weight paid
 *  per pin. */
export interface VoyageMap {
  voyage_id: string;
  granularity: Granularity;
  granularity_label: string;
  precision_m: number | null;
  pins: MapPin[];
  unplaced_count: number;
  unplaced: MapUnplaced[];
}

/** Rust: `DiscoverCluster`. One marker on the discovery map.
 *
 *  `coordinate` is a real photograph's published position rather than a
 *  computed center of the group. That is forced by the server's own types —
 *  `Coordinate` can only be built by rounding through a granularity, and an
 *  average of several photographs is not the output of that — and it has a
 *  useful consequence here: every position this endpoint returns is one some
 *  voyage already publishes. */
export interface MapCluster {
  coordinate: Coordinate;
  /** How many photographs the marker stands for, the named one included. */
  count: number;
  /** The representative photograph, and the voyage to follow it to. */
  photo_id: string;
  voyage_id: string;
  thumbnail_url: string;
}

/** Rust: `DiscoverMapResponse`. Every published voyage's photographs inside a
 *  rectangle, grouped for the zoom that was asked for.
 *
 *  Clustering happens on the server here, unlike on a single voyage's map: the
 *  rectangle at a world view covers every published photograph there is, and no
 *  client can be sent them all to group for itself. */
export interface DiscoverMap {
  clusters: MapCluster[];
  /** True when there were more photographs in the rectangle than one request
   *  will consider and some were left out. Stated rather than hidden — a map
   *  that silently drops half a continent looks exactly like a map of a
   *  continent with fewer photographs in it. */
  truncated: boolean;
}

// ---------------------------------------------------------------------------
// Photos — Rust: `http::photos`
// ---------------------------------------------------------------------------

/** Rust: `PhotoResponse`.
 *
 *  Note the bare `latitude`/`longitude`: this endpoint is owner-only and
 *  returns EXACT coordinates. It is a user's own library, not the public
 *  surface. Never render these on a public page — anything a stranger can see
 *  must come from a `Coordinate` on a voyage, which has already been rounded to
 *  that voyage's precision. */
export interface Photo {
  id: string;
  width: number;
  height: number;
  byte_size: number;
  original_format: "jpeg" | "png" | "webp" | "heic";
  captured_at: string | null;
  latitude: number | null;
  longitude: number | null;
  placed: boolean;
  variant_status: VariantStatus;
  created_at: string;
}

/** Rust: `photos::ListResponse`. */
export interface PhotoList {
  photos: Photo[];
  next_before: string | null;
}

/** Rust: `UploadResponse`. */
export interface UploadResult {
  photos: Photo[];
}

// ---------------------------------------------------------------------------
// Session — Rust: `http::auth`
// ---------------------------------------------------------------------------

/** Rust: `MeUser`.
 *
 *  Both name fields are nullable and that is not an edge case. Providers differ
 *  in what they return, and Apple supplies a name only on the very first
 *  authorization — so an account can legitimately have neither a name nor a
 *  picture, and every place that shows a user has to cope. */
export interface CurrentUser {
  id: string;
  display_name: string | null;
  avatar_url: string | null;
  created_at: string;
}

/** Rust: `LinkedProvider`. */
export interface LinkedProvider {
  provider: "google" | "microsoft" | "apple" | "facebook";
  linked_at: string;
  last_seen_at: string;
}

/** Rust: `MeResponse`. */
export interface Me {
  user: CurrentUser;
  identities: LinkedProvider[];
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/** Every error body the service produces, from `AppError::into_response`.
 *
 *  `code` is stable and meant to be branched on; `message` is prose for a
 *  person and may change. For a 5xx the message is deliberately generic and the
 *  real cause is only in the server log, retrievable by the `x-request-id`
 *  response header — which is the reference the design's error screen asks the
 *  user to quote. */
export interface ApiError {
  code:
    // From `AppError::status_and_code` in crates/api/src/error.rs
    | "not_found"
    | "bad_request"
    | "unauthorized"
    | "forbidden"
    | "conflict"
    | "service_unavailable"
    | "internal"
    // From the handler-local rejection type in crates/api/src/http/voyages.rs
    | "validation_failed"
    // From the handler-local rejection type in crates/api/src/http/photos.rs
    | "too_large"
    | "not_an_image"
    | "unprocessable_image"
    | "rate_limited"
    | "variants_pending";
  message: string;
  /** Present on `validation_failed`. This is what lets the voyage edit screen
   *  put "A title is required before publishing" beside the title input rather
   *  than in a banner at the top of the page. */
  errors?: FieldError[];
}

export interface FieldError {
  field: string;
  message: string;
}
