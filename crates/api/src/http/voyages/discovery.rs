//! Public map membership is decided only after applying each voyage's precision.
use super::{Rejection, place_pair, stored_granularity, variant_url};
use crate::{db::repo::voyages as repo, photos::VariantKind, state::AppState, voyages::Coordinate};
use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// The discovery map
// ---------------------------------------------------------------------------

/// What the discovery map asks for: a rectangle and how far in the camera is.
#[derive(Debug, Deserialize)]
pub(super) struct DiscoverMapQuery {
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    /// Web Mercator zoom. Decides how many pixels apart two photographs are,
    /// and therefore whether they are drawn as one marker or two.
    zoom: f64,
}

/// A group of photographs the discovery map draws as one marker.
#[derive(Debug, Serialize)]
struct DiscoverCluster {
    /// Representative photo's already-rounded coordinate, not an average of
    /// the group's positions.
    coordinate: Coordinate,
    /// How many photographs the marker stands for, the representative
    /// included.
    count: usize,
    /// The representative: the lowest photo id in the group, so the choice is
    /// deterministic and the same rectangle draws the same markers twice
    /// running.
    photo_id: Uuid,
    /// Which published voyage the representative belongs to, so a marker can be
    /// followed to something.
    voyage_id: Uuid,
    thumbnail_url: String,
    /// Distinct destinations remain selectable when coincident pins cannot separate.
    voyages: Vec<DiscoverVoyage>,
}

#[derive(Debug, Serialize)]
struct DiscoverVoyage {
    id: Uuid,
    title: String,
}

#[derive(Debug, Serialize)]
pub(super) struct DiscoverMapResponse {
    clusters: Vec<DiscoverCluster>,
    /// True if more than `DISCOVER_MAP_LIMIT` photos matched the viewport
    /// after rounding and some were omitted.
    truncated: bool,
}

/// Maximum matching photos grouped in one response. Additional matches set
/// the response's `truncated` flag.
const DISCOVER_MAP_LIMIT: i64 = 20_000;

/// How far apart, in screen pixels, two photographs must be to get their own
/// markers.
///
/// The 44px touch target from `design/tokens.css`, the same figure the browser
/// clusters a single voyage by. Two markers closer than one touch target cannot
/// be tapped apart, so leaving them separate produces taps that appear to do
/// nothing.
const DISCOVER_CLUSTER_CELL_PX: f64 = 44.0;

/// Returns clusters of published photo locations in the requested viewport.
///
/// Rounds each photo using its voyage's granularity before checking viewport
/// membership, applying the response cap, or clustering. No session is required.
/// Invalid bounds or zoom return a validation error; database failures propagate.
pub(super) async fn discover_map(
    State(state): State<AppState>,
    crate::auth::MaybeUser(user): crate::auth::MaybeUser,
    Query(query): Query<DiscoverMapQuery>,
) -> Result<Json<DiscoverMapResponse>, Rejection> {
    let bbox = Bbox::parse(&query)?;

    let mut rows = Vec::new();
    let mut after = None;
    // Page by identity, then round and filter before applying the response cap.
    // This bounds memory without making public membership depend on hidden GPS.
    'scan: loop {
        let page = repo::published_photo_page_visible(
            &state.pool,
            after,
            1000,
            user.as_ref().map(|user| user.user_id),
        )
        .await?;
        let count = page.len();
        for row in page {
            after = Some(row.photo_id);
            let point = place_pair(
                stored_granularity(&row.granularity),
                row.latitude,
                row.longitude,
            );
            let longitude = point.longitude();
            let in_longitude = if bbox.wraps {
                longitude >= bbox.west || longitude <= bbox.east
            } else {
                longitude >= bbox.west && longitude <= bbox.east
            };
            if point.latitude() >= bbox.south && point.latitude() <= bbox.north && in_longitude {
                rows.push(row);
                if rows.len() as i64 > DISCOVER_MAP_LIMIT {
                    break 'scan;
                }
            }
        }
        if count < 1000 {
            break;
        }
    }

    let truncated = rows.len() as i64 > DISCOVER_MAP_LIMIT;
    let considered = &rows[..rows.len().min(DISCOVER_MAP_LIMIT as usize)];

    if truncated {
        tracing::warn!(
            limit = DISCOVER_MAP_LIMIT,
            zoom = bbox.zoom,
            "discovery map hit its row cap; some photographs were left out"
        );
    }

    Ok(Json(DiscoverMapResponse {
        clusters: cluster_for_discovery(considered, bbox.zoom),
        truncated,
    }))
}

/// A validated rectangle.
struct Bbox {
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    zoom: f64,
    /// True when the rectangle crosses the antimeridian, so `west` is
    /// numerically greater than `east`.
    wraps: bool,
}

const BAD_BOUNDS: &str = "That is not a rectangle we can look in.";
const BAD_ZOOM: &str = "That is not a zoom level we recognize.";

/// The zoom levels the map can be at. Zero is the whole world in one tile; the
/// tile source stops at 19 and the client allows one blurry level past it.
const MAX_QUERY_ZOOM: f64 = 20.0;

impl Bbox {
    fn parse(query: &DiscoverMapQuery) -> Result<Self, Rejection> {
        // Reject non-finite bounds and zoom before projection or range comparisons.
        for value in [query.west, query.south, query.east, query.north] {
            if !value.is_finite() {
                return Err(Rejection::field("bounds", BAD_BOUNDS));
            }
        }

        if !query.zoom.is_finite() || query.zoom < 0.0 || query.zoom > MAX_QUERY_ZOOM {
            return Err(Rejection::field("zoom", BAD_ZOOM));
        }

        if !(-90.0..=90.0).contains(&query.south)
            || !(-90.0..=90.0).contains(&query.north)
            || !(-180.0..=180.0).contains(&query.west)
            || !(-180.0..=180.0).contains(&query.east)
            || query.south > query.north
        {
            return Err(Rejection::field("bounds", BAD_BOUNDS));
        }

        Ok(Bbox {
            west: query.west,
            south: query.south,
            east: query.east,
            north: query.north,
            zoom: query.zoom,
            // West greater than east is how a rectangle crossing the
            // antimeridian arrives — from Fiji to Samoa the numbers run 177 to
            // -172. It is not an error and it is not an empty rectangle.
            wraps: query.west > query.east,
        })
    }
}

/// Groups rounded positions into a fixed world-pixel grid at the requested zoom.
///
/// Each cell uses one photo as representative. The grid is independent of the
/// viewport, so panning does not move its cell boundaries. Photos on opposite
/// sides of a boundary can remain separate even if their markers overlap.
fn cluster_for_discovery(rows: &[repo::DiscoverPhoto], zoom: f64) -> Vec<DiscoverCluster> {
    // Insertion-ordered, so the output follows the input's `ORDER BY photo_id`
    // and the response is byte-identical for an unchanged rectangle.
    let mut cells: Vec<((i64, i64), DiscoverCluster)> = Vec::new();
    let mut index: HashMap<(i64, i64), usize> = HashMap::new();
    let mut destinations: HashMap<(i64, i64), BTreeMap<Uuid, String>> = HashMap::new();

    for row in rows {
        let granularity = stored_granularity(&row.granularity);

        // Rounded before it is grouped. The grid never sees an exact
        // coordinate from a voyage that does not publish one.
        // This query selects only placed photos; `DiscoverPhoto` stores both
        // coordinates as `f64` and the database check rejects half-placed rows.
        let coordinate = place_pair(granularity, row.latitude, row.longitude);

        let (x, y) = world_pixels(coordinate.latitude(), coordinate.longitude(), zoom);
        let key = (
            (x / DISCOVER_CLUSTER_CELL_PX).floor() as i64,
            (y / DISCOVER_CLUSTER_CELL_PX).floor() as i64,
        );

        destinations
            .entry(key)
            .or_default()
            .entry(row.voyage_id)
            .or_insert_with(|| row.voyage_title.clone());
        match index.get(&key) {
            Some(&at) => cells[at].1.count += 1,
            None => {
                index.insert(key, cells.len());
                cells.push((
                    key,
                    DiscoverCluster {
                        coordinate,
                        count: 1,
                        photo_id: row.photo_id,
                        voyage_id: row.voyage_id,
                        thumbnail_url: variant_url(row.photo_id, VariantKind::Thumb),
                        voyages: Vec::new(),
                    },
                ));
            }
        }
    }

    cells
        .into_iter()
        .map(|(key, mut cluster)| {
            cluster.voyages = destinations
                .remove(&key)
                .unwrap_or_default()
                .into_iter()
                .map(|(id, title)| DiscoverVoyage { id, title })
                .collect();
            cluster
        })
        .collect()
}

/// Web Mercator, in world pixels at `zoom`.
///
/// The whole world is `512 * 2^zoom` pixels across, which is the same
/// convention MapLibre uses, so a distance computed here is the distance the
/// browser will draw. Written out rather than shared with the frontend because
/// the two are separate programs; both are checked against the same published
/// constants.
fn world_pixels(latitude: f64, longitude: f64, zoom: f64) -> (f64, f64) {
    /// The latitude Web Mercator stops at. Past it the projection runs to
    /// infinity, and an infinite pixel coordinate has no grid cell.
    const MAX_MERCATOR_LATITUDE: f64 = 85.051_129;

    let scale = 512.0 * 2f64.powf(zoom);
    let clamped = latitude.clamp(-MAX_MERCATOR_LATITUDE, MAX_MERCATOR_LATITUDE);
    let sin = clamped.to_radians().sin();

    (
        ((longitude + 180.0) / 360.0) * scale,
        (0.5 - ((1.0 + sin) / (1.0 - sin)).ln() / (4.0 * std::f64::consts::PI)) * scale,
    )
}
