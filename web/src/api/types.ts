/**
 * API response types corresponding to Rust serialization types.
 * Rust integration tests compare real router responses with committed fixtures.
 * Frontend contract tests check required fields, extra fields, and enum values
 * against those fixtures.
 */

/**
 * Serialized voyages::Coordinate. precision_m is absent for exact precision.
 * When present, it is the nominal per-axis half-width in meters; map circles
 * use sqrt(2) times this value as their approximate radius.
 */
export interface Coordinate {
  latitude: number;
  longitude: number;
  precision_m?: number;
}

/** Public precision names: exact, km1, km10, and km50. */
export type Granularity = "exact" | "km1" | "km10" | "km50";

/** Background variant generation status; pending photos may not have images yet. */
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
  /** Server-supplied display label for the selected precision. */
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
  /** First ready photo in voyage order, served as a metadata-stripped WebP. */
  cover_url: string | null;
  /** Refresh the list until background conversion has settled. */
  photos_pending: boolean;
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

/** Rust: MapResponse. Rounded pins and a separate list of unplaced photos. */
export interface VoyageMap {
  voyage_id: string;
  granularity: Granularity;
  granularity_label: string;
  precision_m: number | null;
  pins: MapPin[];
  unplaced_count: number;
  unplaced: MapUnplaced[];
}

/**
 * Rust: DiscoverCluster. One marker whose position is the representative
 * photo's published coordinate, with destinations for its member voyages.
 */
export interface MapCluster {
  voyages: { id: string; title: string }[];
  coordinate: Coordinate;
  /** How many photographs the marker stands for, the named one included. */
  count: number;
  /** The representative photograph, and the voyage to follow it to. */
  photo_id: string;
  voyage_id: string;
  thumbnail_url: string;
}

/**
 * Rust: DiscoverMapResponse. Published photo locations in a viewport,
 * grouped on the server for the requested zoom.
 */
export interface DiscoverMap {
  clusters: MapCluster[];
  /** True when matching photos exceeded the response limit and some were omitted. */
  truncated: boolean;
}

// ---------------------------------------------------------------------------
// Photos — Rust: `http::photos`
// ---------------------------------------------------------------------------

/**
 * Rust: PhotoResponse. Owner-only metadata with exact stored coordinates.
 * Public views use rounded voyage coordinates instead.
 */
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

/**
 * Rust: MeUser. display_name is null until the user chooses one; avatar_url
 * is optional provider-supplied data.
 */
export interface CurrentUser {
  is_site_admin: boolean;
  id: string;
  display_name: string | null;
  avatar_url: string | null;
  created_at: string;
}

/** Rust: `LinkedProvider`. */
export interface LinkedProvider {
  provider: "google" | "microsoft" | "apple";
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

/**
 * Application error body from AppError or handler-specific rejections.
 * Codes identify error categories; messages are display text. Server failures
 * use generic messages, with request IDs linking them to server logs. Extractor
 * and middleware failures may use other response shapes.
 */
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

/** Public profile by ID or name: public identity plus this owner's published page. */
export interface PublicProfilePage extends VoyageList {
  user: Omit<CurrentUser, "is_site_admin">;
}

/** Rust: http::admin. File bytes include originals and variants, deduplicated per account. */
export interface AdminUser {
  id: string;
  display_name: string | null;
  created_at: string;
  is_site_admin: boolean;
  suspended: boolean;
  photo_count: number;
  voyage_count: number;
  storage_bytes: number;
}
export interface AdminUserPage {
  users: AdminUser[];
  next_before: string | null;
}
export interface AdminUserDetail {
  user: AdminUser;
  voyages: { id: string; title: string; description: string | null; published: boolean }[];
  next_before: string | null;
}
export type AdminAction =
  | { action: "suspend" | "enable" | "delete" | "promote" | "unpublish_all" }
  | { action: "unpublish_voyage"; voyage_id: string };
