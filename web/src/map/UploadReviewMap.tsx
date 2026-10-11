/** Upload review: exact owner-only coordinates over the shared basemap. */
import { useMemo, type CSSProperties } from "react";

import { Pin } from "../components";
import { MapCanvas, MapMarker } from "./MapCanvas";
import { boundsOf } from "./geo";
import type { VoyagePin } from "./VoyageMap";

export interface UploadReviewMapProps {
  pins: readonly VoyagePin[];
  label: string;
  style: CSSProperties;
}

export default function UploadReviewMap({ pins, label, style }: UploadReviewMapProps) {
  const bounds = useMemo(() => boundsOf(pins.map((pin) => pin.coordinate)), [pins]);
  return (
    <MapCanvas label={label} fitTo={bounds} maxFitZoom={16} style={style}>
      {() =>
        pins.map((pin) => (
          <MapMarker key={pin.id} coordinate={pin.coordinate}>
            <Pin label={pin.label} />
          </MapMarker>
        ))
      }
    </MapCanvas>
  );
}
