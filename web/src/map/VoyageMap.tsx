/**
 * A voyage drawn on the map: pins, clusters, granularity areas, and selection.
 *
 * Everything here is about photographs. Nothing here knows about MapLibre —
 * `MapCanvas.tsx` is the only file that does — so the pin behavior survives a
 * change of mapping library, and more immediately survives the swap from
 * openstreetmap.org's raster tiles to the self-hosted vector archive that
 * decision D4 describes.
 *
 * ## What is drawn
 *
 * One marker per cluster. A cluster of one is a pin; a cluster of several is a
 * numbered bubble at one of the design's three size tiers. When the voyage
 * publishes rounded coordinates, each marker also carries the circle that says
 * how rounded — that is target 10, and the reasoning is in
 * {@link GranularityArea}.
 *
 * ## Selection
 *
 * The camera moves only for a selection that came from the photo list. A
 * selection that came from clicking a pin does not move the camera, because the
 * pin the user just clicked is already under their pointer and sliding the map
 * out from under it is disorienting. This is the same `source` distinction the
 * voyage detail screen uses to decide whether to scroll the photo rail, applied
 * to the other half of the same interaction.
 */

import { useEffect, useMemo, type CSSProperties } from "react";

import { Cluster, MapArea, Pin } from "../components";
import { MapCanvas, MapMarker, type MapView } from "./MapCanvas";
import {
  areaDiameterPx,
  boundsOf,
  clusterPins,
  clusterTier,
  zoomForPrecision,
  type Clusterable,
  type MapCluster,
} from "./geo";
import { MAX_ZOOM } from "./style";

/* ==========================================================================
   Inputs
   ========================================================================== */

/** One photograph with a position. */
export interface VoyagePin extends Clusterable {
  /** The photograph's id. Also the cluster id when it seeds one. */
  id: string;
  coordinate: { latitude: number; longitude: number };
  /**
   * The accessible name — "Photograph taken 14 February, 09:12, within ~10 km".
   * A pin is a graphic with no text, so this is the only name it has, and it is
   * deliberately the same string the photograph carries in the rail so that a
   * screen-reader user can tell which pin belongs to which photograph.
   */
  label: string;
}

/** Which half of the screen asked for the current selection. */
export type MapSelectionSource = "pin" | "photo";

export interface MapSelection {
  photoId: string;
  source: MapSelectionSource;
}

export interface VoyageMapProps {
  pins: readonly VoyagePin[];
  /**
   * The radius, in meters, that the voyage's coordinates were rounded to —
   * `Coordinate.precision_m` from the API. Null when the voyage publishes exact
   * positions, which is the only case where a bare pin tells the truth.
   */
  precisionM: number | null;
  selection: MapSelection | null;
  onSelectPin: (photoId: string) => void;
  /** Opens the photos represented by a marker instead of expanding its cluster. */
  onOpenGallery?: (photoIds: string[]) => void;
  /** Names the map region. */
  label: string;
  /** Pixels to lift the attribution by, so the mobile sheet does not cover it. */
  attributionOffset?: number;
  style?: CSSProperties;
}

/* ==========================================================================
   Screen
   ========================================================================== */

/**
 * How much of the viewport a granularity circle should take up when the camera
 * is fitted to a voyage with only one location in it.
 *
 * Without a cap the camera fits a single point by zooming to the maximum, which
 * would show a named street corner for a position known only to within fifty
 * kilometers — the map would be making a claim the data cannot support, which
 * is the same defect target 10 is about, expressed as a camera rather than as a
 * marker.
 */
const FITTED_AREA_DIAMETER_PX = 320;

/** Never fit closer than this even at exact precision. A single photograph
 *  filling the screen with one building is disorienting; a neighborhood is
 *  legible. */
const MAX_FIT_ZOOM = 16;

export function VoyageMap({
  pins,
  precisionM,
  selection,
  onSelectPin,
  onOpenGallery,
  label,
  attributionOffset = 0,
  style,
}: VoyageMapProps) {
  const bounds = useMemo(() => boundsOf(pins.map((pin) => pin.coordinate)), [pins]);

  const maxFitZoom = useMemo(() => {
    if (precisionM === null || bounds === null) return MAX_FIT_ZOOM;

    const middle = (bounds.north + bounds.south) / 2;
    const precisionCap = zoomForPrecision(middle, precisionM, FITTED_AREA_DIAMETER_PX);
    return Math.min(MAX_FIT_ZOOM, precisionCap);
  }, [precisionM, bounds]);

  return (
    <MapCanvas
      label={label}
      fitTo={bounds}
      maxFitZoom={maxFitZoom}
      attributionOffset={attributionOffset}
      style={style}
    >
      {(view) => (
        <VoyageMarkers
          view={view}
          pins={pins}
          precisionM={precisionM}
          selection={selection}
          onSelectPin={onSelectPin}
          onOpenGallery={onOpenGallery}
        />
      )}
    </MapCanvas>
  );
}

export default VoyageMap;

/* ==========================================================================
   Markers
   ========================================================================== */

interface VoyageMarkersProps {
  view: MapView;
  pins: readonly VoyagePin[];
  precisionM: number | null;
  selection: MapSelection | null;
  onSelectPin: (photoId: string) => void;
  /** Opens the photos represented by a marker instead of expanding its cluster. */
  onOpenGallery?: (photoIds: string[]) => void;
}

/** How much closer a cluster click takes the camera. Two levels quarters the
 *  area a cluster covers, which is usually enough to break it apart without
 *  losing the surrounding context in one jump. */
const CLUSTER_EXPAND_STEPS = 2;

function VoyageMarkers({
  view,
  pins,
  precisionM,
  selection,
  onSelectPin,
  onOpenGallery,
}: VoyageMarkersProps) {
  const selectedId = selection?.photoId ?? null;

  /**
   * The selected pin never joins a cluster.
   *
   * Otherwise choosing a photograph in the rail could mark a pin that is not on
   * screen — swallowed into a bubble showing "9" — and the selection would
   * appear to have done nothing to the map at all. Pulling it out guarantees
   * that whatever is selected is visible as itself.
   */
  const { selectedPin, clusters } = useMemo(() => {
    const selected = pins.find((pin) => pin.id === selectedId) ?? null;
    const rest = selected === null ? pins : pins.filter((pin) => pin.id !== selected.id);

    return { selectedPin: selected, clusters: clusterPins(rest, view.zoom) };
  }, [pins, selectedId, view.zoom]);

  /**
   * Panning to a photograph chosen in the list.
   *
   * Only for `source === "photo"`. A pin click already put the map where the
   * user is looking, and re-centring on it would slide the marker out from
   * under the pointer that just clicked it.
   */
  const { moveTo } = view;
  useEffect(() => {
    if (selection === null || selection.source !== "photo") return;

    const pin = pins.find((candidate) => candidate.id === selection.photoId);
    if (pin === undefined) return;

    moveTo(pin.coordinate);
  }, [selection, pins, moveTo]);

  return (
    <>
      {clusters.map((cluster) => (
        <ClusterMarker
          key={cluster.id}
          cluster={cluster}
          precisionM={precisionM}
          zoom={view.zoom}
          onSelectPin={onSelectPin}
          onOpenGallery={onOpenGallery}
          onExpand={() =>
            view.moveTo(cluster.coordinate, {
              zoom: Math.min(MAX_ZOOM, view.zoom + CLUSTER_EXPAND_STEPS),
            })
          }
        />
      ))}

      {selectedPin !== null && (
        <MapMarker coordinate={selectedPin.coordinate}>
          <GranularityArea
            precisionM={precisionM}
            latitude={selectedPin.coordinate.latitude}
            zoom={view.zoom}
          />
          <Pin
            label={selectedPin.label}
            left={0}
            top={0}
            selected
            onClick={() => {
              onSelectPin(selectedPin.id);
              onOpenGallery?.([selectedPin.id]);
            }}
          />
        </MapMarker>
      )}
    </>
  );
}

interface ClusterMarkerProps {
  cluster: MapCluster<VoyagePin>;
  precisionM: number | null;
  zoom: number;
  onSelectPin: (photoId: string) => void;
  /** Opens the photos represented by a marker instead of expanding its cluster. */
  onOpenGallery?: (photoIds: string[]) => void;
  onExpand: () => void;
}

/**
 * One marker: a pin when it stands for a single photograph, a numbered bubble
 * when it stands for several.
 *
 * Both are buttons. With a gallery callback, the bubble opens its member
 * photos. Other map consumers expand the bubble by zooming. Both interactions
 * remain available to keyboard users through native button activation.
 */
function ClusterMarker({
  cluster,
  precisionM,
  zoom,
  onSelectPin,
  onOpenGallery,
  onExpand,
}: ClusterMarkerProps) {
  const single = cluster.members.length === 1 ? cluster.members[0] : null;

  return (
    <MapMarker coordinate={cluster.coordinate}>
      <GranularityArea precisionM={precisionM} latitude={cluster.coordinate.latitude} zoom={zoom} />

      {single === null ? (
        <Cluster
          count={cluster.members.length}
          left={0}
          top={0}
          size={clusterTier(cluster.members.length)}
          onClick={
            onOpenGallery
              ? () => onOpenGallery(cluster.members.map((member) => member.id))
              : onExpand
          }
        />
      ) : (
        <Pin
          label={single.label}
          left={0}
          top={0}
          onClick={() => {
            onSelectPin(single.id);
            onOpenGallery?.([single.id]);
          }}
        />
      )}
    </MapMarker>
  );
}

/**
 * Draw the cell's approximate enclosing circle behind its selectable marker.
 * The radius is sqrt(2) times the per-axis precision. Hide circles below a few
 * pixels because they convey no useful distance at that zoom.
 */
function GranularityArea({
  precisionM,
  latitude,
  zoom,
}: {
  precisionM: number | null;
  latitude: number;
  zoom: number;
}) {
  // A voyage at exact precision has no uncertainty to draw, and drawing one
  // would invent an imprecision the user did not ask for.
  if (precisionM === null) return null;

  const diameter = areaDiameterPx(precisionM, latitude, zoom);
  if (diameter < MIN_VISIBLE_AREA_PX) return null;

  return <MapArea left={0} top={0} size={Math.round(diameter)} />;
}

/** Below this the circle says nothing a reader can act on. */
const MIN_VISIBLE_AREA_PX = 12;
