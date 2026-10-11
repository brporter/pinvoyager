/** Illustrative location precision over the shared St. Louis basemap. */
import { GRANULARITY_PRECISION } from "../components/granularity";
import type { Granularity } from "../api/types";
import { MapArea, Pin } from "../components/MapPrimitives";
import { MapCanvas, MapMarker } from "./MapCanvas";
import { areaDiameterPx, zoomForPrecision } from "./geo";

export interface PrecisionPreviewProps {
  precision: Granularity | null;
}

const LOCATION = { latitude: 38.627, longitude: -90.1994 };

export default function PrecisionPreview({ precision }: PrecisionPreviewProps) {
  const halfCellM = precision === null ? null : GRANULARITY_PRECISION[precision];
  const extentM = precision === null ? 10000 : (halfCellM ?? 100);
  const latitudeSpan = extentM / 111320;
  const longitudeSpan = latitudeSpan / Math.cos((LOCATION.latitude * Math.PI) / 180);
  const bounds = {
    west: LOCATION.longitude - longitudeSpan,
    east: LOCATION.longitude + longitudeSpan,
    south: LOCATION.latitude - latitudeSpan,
    north: LOCATION.latitude + latitudeSpan,
  };
  return (
    <MapCanvas
      label="Location precision example — St. Louis, Missouri"
      fitTo={bounds}
      maxFitZoom={halfCellM === null ? 14 : zoomForPrecision(LOCATION.latitude, halfCellM, 128)}
      className="pv-gran-preview"
      style={{ height: 280 }}
    >
      {({ zoom }) =>
        precision === null ? null : (
          <MapMarker coordinate={LOCATION}>
            {halfCellM === null ? (
              <Pin label="Example photo location in St. Louis" />
            ) : (
              <MapArea
                left={0}
                top={0}
                size={areaDiameterPx(halfCellM, LOCATION.latitude, zoom)}
                label="Example location area around St. Louis"
              />
            )}
          </MapMarker>
        )
      }
    </MapCanvas>
  );
}
