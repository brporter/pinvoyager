import { NoPhotos } from "./VoyageStates";
import type { PointerEvent as ReactPointerEvent } from "react";
import { useCallback, useMemo, useRef, useState } from "react";
import { Link } from "react-router";

import { useAuth } from "../../auth/useAuth";
import { Badge, Button, CloseIcon, Filmstrip, Photo } from "../../components";
import { LazyVoyageMap } from "../../map";
import { prefersReducedMotion } from "../../shell/motion";

import {
  CHIP,
  capturedAtLabel,
  coordinateLabel,
  countLabel,
  fillHeight,
  photoAlt,
  pinsFor,
  placedPhotos,
  type LayoutProps,
} from "./model";
import {
  BackIcon,
  PublishVoyage,
  SelectablePhoto,
  ShareVoyage,
  UnplacedSection,
} from "./VoyageChrome";
const SHEET_PEEK = 168;
const SHEET_EXPANDED = 300;

export function MobileLayout({
  voyage,
  selection,
  onSelectPin,
  onOpenGallery,
  onSelectPhoto,
  onClearSelection,
  registerPhoto,
}: LayoutProps) {
  const { user } = useAuth();
  const owner = user !== null && user.id === voyage.owner_id;
  const selectedPhotoId = selection?.photoId ?? null;
  const placed = placedPhotos(voyage);
  const unplaced = voyage.photos.filter((photo) => !photo.placed);
  const selected = voyage.photos.find((photo) => photo.id === selectedPhotoId) ?? null;
  const pins = useMemo(() => pinsFor(voyage), [voyage]);

  const [height, setHeight] = useState<number>(SHEET_PEEK);
  const [dragging, setDragging] = useState(false);
  const drag = useRef<{ startY: number; startHeight: number } | null>(null);

  // A pin selection expands the sheet: the point of selecting a pin is to see
  // the photograph, and the peek height has room for the strip only.
  const expandAndSelect = useCallback(
    (photoId: string) => {
      setHeight(SHEET_EXPANDED);
      onSelectPin(photoId);
    },
    [onSelectPin],
  );

  const toggleSheet = useCallback(() => {
    setHeight((current) => (current >= SHEET_EXPANDED ? SHEET_PEEK : SHEET_EXPANDED));
  }, []);

  const onGripPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    drag.current = { startY: event.clientY, startHeight: height };
    setDragging(true);
    event.currentTarget.setPointerCapture?.(event.pointerId);
  };

  const onGripPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const start = drag.current;
    if (start === null) return;
    // Dragging up grows the sheet, so the delta is inverted.
    const next = start.startHeight + (start.startY - event.clientY);
    setHeight(Math.min(SHEET_EXPANDED, Math.max(SHEET_PEEK, next)));
  };

  const endDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (drag.current === null) return;
    drag.current = null;
    setDragging(false);
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    // Two rest positions, so a released drag snaps to the nearer of them.
    setHeight((current) =>
      current - SHEET_PEEK < (SHEET_EXPANDED - SHEET_PEEK) / 2 ? SHEET_PEEK : SHEET_EXPANDED,
    );
  };

  const expanded = height >= SHEET_EXPANDED;

  return (
    <div style={{ ...fillHeight(false), position: "relative", overflow: "hidden" }}>
      <LazyVoyageMap
        label={`Map of ${voyage.title}`}
        pins={pins}
        precisionM={voyage.precision_m}
        selection={selection}
        onSelectPin={expandAndSelect}
        onOpenGallery={onOpenGallery}
        // Attribution is a license requirement, so it rides above the sheet
        // rather than sliding underneath it and disappearing.
        attributionOffset={height}
        style={{ position: "absolute", inset: 0 }}
      />

      {/* Floats over the map rather than taking a band above it, so the map
          keeps the full height of a small screen. */}
      <div
        style={{
          position: "relative",
          zIndex: 2,
          display: "flex",
          alignItems: "center",
          gap: "var(--pv-space-2)",
          padding: "var(--pv-space-3)",
        }}
      >
        <Link
          to="/"
          className="pv-btn pv-btn--icon"
          aria-label="Back to discover"
          style={{ ...CHIP, padding: "0 var(--pv-space-2)" }}
        >
          <BackIcon />
        </Link>

        <div style={{ ...CHIP, flex: 1, minWidth: 0 }}>
          <h1
            className="pv-sm"
            style={{
              margin: 0,
              fontWeight: "var(--pv-weight-semibold)",
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
          >
            {voyage.title}
          </h1>
        </div>
        <ShareVoyage voyage={voyage} />
      </div>

      <div
        className="pv-sheet"
        style={{
          zIndex: 3,
          height,
          // Two reasons to drop the design system's height transition. The
          // height is rewritten on every pointer move during a drag, and
          // animating each of those makes the sheet lag the finger; and a
          // 300px slide is exactly the motion someone asking for reduced
          // motion is asking not to see.
          transition: dragging || prefersReducedMotion() ? "none" : undefined,
        }}
      >
        <div
          className="pv-sheet-grip"
          role="button"
          tabIndex={0}
          aria-expanded={expanded}
          aria-label={expanded ? "Collapse photo list" : "Expand photo list"}
          // `.pv-sheet-grip` is 28px in the design system, below the 44px
          // floor. Raised here rather than in the stylesheet, which this
          // screen does not own.
          style={{ minHeight: "var(--pv-touch-min)", touchAction: "none" }}
          onPointerDown={onGripPointerDown}
          onPointerMove={onGripPointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              toggleSheet();
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setHeight(SHEET_EXPANDED);
            } else if (event.key === "ArrowDown") {
              event.preventDefault();
              setHeight(SHEET_PEEK);
            }
          }}
        />

        <div className="pv-sheet-head">
          {selected === null ? (
            <>
              <div>
                <div className="pv-sm" style={{ fontWeight: "var(--pv-weight-semibold)" }}>
                  {countLabel(voyage.photo_count, "photo")}
                </div>
                <div className="pv-xs pv-subtle">{countLabel(voyage.place_count, "place")}</div>
              </div>
              <Badge tone={voyage.published ? "public" : "draft"}>
                {voyage.published ? "Public" : "Draft"}
              </Badge>
            </>
          ) : (
            <>
              <div>
                <div className="pv-sm" style={{ fontWeight: "var(--pv-weight-semibold)" }}>
                  {capturedAtLabel(selected) ?? "Capture time unknown"}
                </div>
                <div className="pv-xs pv-subtle">
                  {coordinateLabel(selected.coordinate) ?? "No location"}
                </div>
              </div>
              <Button
                variant="ghost"
                size="sm"
                iconOnly
                aria-label="Close selection"
                onClick={onClearSelection}
              >
                <CloseIcon size={14} />
              </Button>
            </>
          )}
        </div>
        {owner && !voyage.published && (
          <div style={{ paddingInline: "var(--pv-space-4)" }}>
            <PublishVoyage voyageId={voyage.id} />
          </div>
        )}

        <div className="pv-sheet-body">
          {voyage.photos.length === 0 ? (
            <NoPhotos voyageId={voyage.id} />
          ) : (
            <>
              {selected !== null && (
                <div style={{ marginBottom: "var(--pv-space-3)" }}>
                  <Photo
                    src={selected.display_url}
                    alt={photoAlt(selected)}
                    loading={selected.variant_status === "pending"}
                  />
                </div>
              )}

              <Filmstrip label="Photos in this voyage">
                {placed.map((photo) => (
                  <SelectablePhoto
                    key={photo.id}
                    photo={photo}
                    selected={photo.id === selectedPhotoId}
                    onSelect={onSelectPhoto}
                    registerPhoto={registerPhoto}
                  />
                ))}
              </Filmstrip>

              <UnplacedSection
                photos={unplaced}
                selectedPhotoId={selectedPhotoId}
                onSelect={onSelectPhoto}
                registerPhoto={registerPhoto}
              />
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/** The pill the floating top bar's controls sit in, from the design's
 *  `.m-chip`. */
