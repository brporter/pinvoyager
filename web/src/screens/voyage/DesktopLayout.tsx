import { NoPhotos } from "./VoyageStates";
import type { CSSProperties } from "react";
import { useMemo } from "react";

import { PhotoGrid } from "../../components";
import { LazyVoyageMap } from "../../map";

import {
  RAIL_WIDTH,
  fillHeight,
  groupByCaptureDate,
  pinsFor,
  placedPhotos,
  type LayoutProps,
} from "./model";
import { SelectablePhoto, UnplacedSection, VoyageHeading } from "./VoyageChrome";

export function DesktopLayout({
  voyage,
  selection,
  onSelectPin,
  onOpenGallery,
  onSelectPhoto,
  registerPhoto,
}: LayoutProps) {
  const selectedPhotoId = selection?.photoId ?? null;
  const placed = placedPhotos(voyage);
  const unplaced = voyage.photos.filter((photo) => !photo.placed);
  const pins = useMemo(() => pinsFor(voyage), [voyage]);

  return (
    <div
      style={{
        ...fillHeight(true),
        display: "grid",
        gridTemplateColumns: `1fr ${RAIL_WIDTH}px`,
      }}
    >
      <div style={{ position: "relative", minWidth: 0 }}>
        <LazyVoyageMap
          label={`Map of ${voyage.title}`}
          pins={pins}
          precisionM={voyage.precision_m}
          selection={selection}
          onSelectPin={onSelectPin}
          onOpenGallery={onOpenGallery}
          style={{ position: "absolute", inset: 0 }}
        />
      </div>

      <aside
        aria-label="Photos in this voyage"
        style={{
          borderLeft: "1px solid var(--pv-border)",
          background: "var(--pv-bg)",
          display: "flex",
          flexDirection: "column",
          minHeight: 0,
        }}
      >
        <div
          style={{
            flex: "none",
            padding: "var(--pv-space-4)",
            borderBottom: "1px solid var(--pv-border)",
          }}
        >
          <VoyageHeading voyage={voyage} />
        </div>

        <div style={{ overflowY: "auto", padding: "var(--pv-space-4)" }}>
          {voyage.photos.length === 0 ? (
            <NoPhotos voyageId={voyage.id} />
          ) : (
            <>
              {groupByCaptureDate(placed).map((group) => (
                <section key={group.key} style={{ marginBottom: "var(--pv-space-5)" }}>
                  <h3 style={DATE_HEADING}>{group.label}</h3>
                  {/* Two columns, as the design draws the rail: the grid's
                      150px `auto-fill` minimum resolves to exactly two inside
                      a 420px rail, so the column count is not restated here. */}
                  <PhotoGrid label={`Photos from ${group.label}`}>
                    {group.photos.map((photo) => (
                      <SelectablePhoto
                        key={photo.id}
                        photo={photo}
                        selected={photo.id === selectedPhotoId}
                        onSelect={onSelectPhoto}
                        registerPhoto={registerPhoto}
                      />
                    ))}
                  </PhotoGrid>
                </section>
              ))}

              <UnplacedSection
                photos={unplaced}
                selectedPhotoId={selectedPhotoId}
                onSelect={onSelectPhoto}
                registerPhoto={registerPhoto}
              />
            </>
          )}
        </div>
      </aside>
    </div>
  );
}

const DATE_HEADING: CSSProperties = {
  margin: "0 0 var(--pv-space-3)",
  fontSize: "var(--pv-text-xs)",
  color: "var(--pv-text-subtle)",
  textTransform: "uppercase",
  letterSpacing: "var(--pv-tracking-wide)",
  fontWeight: "var(--pv-weight-semibold)",
};

/* ==========================================================================
   Mobile — full-bleed map, photos in a sheet over it
   ========================================================================== */

/** The sheet's two rest positions, from the design: peek shows the filmstrip,
 *  expanded shows the selected photograph above it. */
