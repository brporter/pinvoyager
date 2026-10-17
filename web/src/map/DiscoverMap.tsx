/** Server-clustered discovery with visible query status and selectable destinations. */
import { useState } from "react";
import { Link, useNavigate } from "react-router";

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
  const [selected, setSelected] = useState<string | null>(null);

  // The zoom is rounded before it becomes part of the question. A fractional
  // zoom would make every frame of a pinch a different cache key and a
  // different request, for an answer that changes only when the grouping does.
  // MapCanvas renders its children only after the map loads and its initial
  // bounds have been committed (see MapCanvas tests). A null window cannot
  // reach this child, so the query always has a concrete viewport.
  const query = useDiscoverMap({ ...view.window!, zoom: Math.round(view.zoom) });

  const clusters = query.data?.clusters ?? [];
  const selectedCluster = clusters.find((cluster) => cluster.photo_id === selected);

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
              onClick={() => {
                if (view.zoom >= MAX_ZOOM) setSelected(cluster.photo_id);
                else
                  view.moveTo(cluster.coordinate, {
                    zoom: Math.min(MAX_ZOOM, view.zoom + CLUSTER_EXPAND_STEPS),
                  });
              }}
            />
          </MapMarker>
        ),
      )}

      {/* Show when the API omitted results after reaching its response limit. */}
      {query.isError && (
        <div className="pv-map-notice" role="alert">
          Map photographs could not be loaded.{" "}
          <button className="pv-btn pv-btn--secondary" onClick={() => void query.refetch()}>
            Retry map
          </button>
        </div>
      )}
      {selectedCluster && (
        <div className="pv-map-notice" aria-label="Voyages at this location">
          <p>Voyages at this location</p>
          <ul>
            {selectedCluster.voyages.map((voyage) => (
              <li key={voyage.id}>
                <Link to={`/voyages/${voyage.id}`}>{voyage.title}</Link>
              </li>
            ))}
          </ul>
          <button className="pv-btn pv-btn--ghost" onClick={() => setSelected(null)}>
            Close voyage list
          </button>
        </div>
      )}
      {query.data?.truncated === true && (
        <div className="pv-map-notice" role="status">
          There are more photographs here than the map can show. Zoom in to see them all.
        </div>
      )}
    </>
  );
}
