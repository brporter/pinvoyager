/**
 * The world map on the discovery screen.
 *
 * Everything published, everywhere, grouped by the server. It is the same
 * primitives as the voyage map — the same pins, the same bubbles, the same
 * attribution — and a different data flow, for a reason worth being explicit
 * about:
 *
 *  * A **voyage** is a bounded collection. `GET /api/voyages/{id}/map` returns
 *    every pin in it in one response, and the browser groups them as the camera
 *    moves. No request is made for a pan or a zoom.
 *  * The **world** is not bounded. At a world view the rectangle covers every
 *    published photograph there is, so the grouping has to happen where the
 *    photographs are. `GET /api/map` takes the rectangle and the zoom and
 *    answers with markers.
 *
 * This is what the phase 7 goal means by "clustering is always server-side",
 * and here it genuinely has to be.
 *
 * ## What a marker does
 *
 * A marker standing for one photograph goes to its voyage. A marker standing
 * for several zooms in until it comes apart — there is nothing sensible to
 * navigate to when the photographs behind it belong to different people's
 * voyages, and picking one of them arbitrarily would be a link that goes
 * somewhere unrelated to what was clicked.
 *
 * ## A note on the design
 *
 * `design/screens/discover.html` draws no map. The phase 7 goal asks for one,
 * so this places it between the hero and the voyage grid using only components
 * the design system already defines. **Where it belongs, and how tall it is,
 * are open visual questions** that the design file should settle.
 */

import { useNavigate } from "react-router";

import { useDiscoverMap } from "../api/hooks";
import { Cluster, Pin } from "../components";
import { MapCanvas, MapMarker, type MapView } from "./MapCanvas";
import { clusterTier } from "./geo";
import { MAX_ZOOM } from "./style";

export interface DiscoverMapProps {
  /** Names the map region. A map is a graphic, and "map" alone does not
   *  distinguish this one from any other. */
  label: string;
  style?: React.CSSProperties;
}

export function DiscoverMap({ label, style }: DiscoverMapProps) {
  return (
    <MapCanvas label={label} style={style}>
      {(view) => <DiscoverMarkers view={view} />}
    </MapCanvas>
  );
}

export default DiscoverMap;

/** How much closer a bubble click takes the camera. */
const CLUSTER_EXPAND_STEPS = 2;

function DiscoverMarkers({ view }: { view: MapView }) {
  const navigate = useNavigate();

  // The zoom is rounded before it becomes part of the question. A fractional
  // zoom would make every frame of a pinch a different cache key and a
  // different request, for an answer that changes only when the grouping does.
  const query = useDiscoverMap(
    view.window === null ? null : { ...view.window, zoom: Math.round(view.zoom) },
  );

  const clusters = query.data?.clusters ?? [];

  return (
    <>
      {clusters.map((cluster) =>
        cluster.count === 1 ? (
          <MapMarker key={cluster.photo_id} coordinate={cluster.coordinate}>
            <Pin
              label="A photograph. Opens the voyage it belongs to."
              left={0}
              top={0}
              onClick={() => void navigate(`/voyages/${cluster.voyage_id}`)}
            />
          </MapMarker>
        ) : (
          <MapMarker key={cluster.photo_id} coordinate={cluster.coordinate}>
            <Cluster
              count={cluster.count}
              left={0}
              top={0}
              size={clusterTier(cluster.count)}
              onClick={() =>
                view.moveTo(cluster.coordinate, {
                  zoom: Math.min(MAX_ZOOM, view.zoom + CLUSTER_EXPAND_STEPS),
                })
              }
            />
          </MapMarker>
        ),
      )}

      {/* Truncation is stated rather than hidden. A map that silently dropped
          half a continent looks exactly like a map of a continent with fewer
          photographs on it, and a reader has no way to tell which they are
          looking at. */}
      {query.data?.truncated === true && (
        <div className="pv-sr" role="status">
          There are more photographs here than the map can show. Zoom in to see
          them all.
        </div>
      )}
    </>
  );
}
