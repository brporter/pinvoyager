/**
 * Voyage detail — implements `design/screens/voyage-detail.html`.
 *
 * The core screen of the product and the only genuinely hard layout in it: a
 * map and a photo list both want to be the primary object, and on a phone they
 * cannot both be.
 *
 * ## Two layouts, not one that reflows
 *
 *  * **Desktop** — a `1fr 420px` split: map on the left, photo rail on the
 *    right, both primary because there is room for both to be.
 *  * **Mobile** — the map is the surface and the photos are a sheet drawn over
 *    it, with the top bar floating rather than occupying a band. A stacked
 *    map-above-list would give each half of a 390px screen and make both
 *    useless.
 *
 * They are chosen in JavaScript from `useIsDesktop`, not by CSS, for the reason
 * that hook already documents: the two treatments are different markup, and
 * rendering both to hide one would put two copies of every control in the
 * accessibility tree.
 *
 * ## Bidirectional selection
 *
 * One piece of state — `{ photoId, source }` — drives both sides. Choosing a
 * photo marks its pin; choosing a pin marks its photo *and* scrolls that photo
 * into view, which is why the source of the selection is recorded rather than
 * just the id: scrolling the rail underneath the user's own click would fight
 * them. This is the interaction the whole screen exists for.
 *
 * ## The map
 *
 * Phase 7 replaced the stand-in with MapLibre. What survived the swap is the
 * contract: the map is handed a list of pins and the current selection, and it
 * reports a photograph back when one of its pins is clicked. The layout was the
 * contract; the renderer was the implementation. See `src/map/`.
 *
 * The pins are built from the detail payload this screen already fetches rather
 * than from `GET /api/voyages/{id}/map`. That endpoint exists and is leaner,
 * but the map is only rendered once the detail read has resolved — the desktop
 * split and the mobile sheet both need the photo list to exist — so asking for
 * it as well would be a second round trip for coordinates already in hand. The
 * detail payload carries the same rounded `Coordinate`, the same `precision_m`
 * and, unlike the map payload, the capture times the pin labels are made of.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, PointerEvent as ReactPointerEvent } from "react";
import { Link, useParams } from "react-router";

import { isApiError } from "../api/client";
import { useVoyage } from "../api/hooks";
import type { Coordinate, Voyage, VoyagePhoto } from "../api/types";
import { useAuth } from "../auth/useAuth";
import {
  Badge,
  Button,
  EmptyState,
  Filmstrip,
  ImageIcon,
  Photo,
  PhotoGrid,
  Skeleton,
  WarningTriangleIcon,
  CloseIcon,
  ShareIcon,
} from "../components";
import { LazyVoyageMap, type MapSelection, type VoyagePin } from "../map";
import { prefersReducedMotion } from "../shell/motion";
import { useIsDesktop } from "../shell/useIsDesktop";

/* ==========================================================================
   Selection
   ========================================================================== */

/**
 * Which side of the screen asked for the selection.
 *
 * Both halves need it, for symmetrical reasons. A pin click has to scroll the
 * rail but must not move the camera; a photo click has to move the camera but
 * must not scroll the rail. In each case the thing the user just clicked is
 * already where they are looking, and moving it under them is disorienting.
 *
 * The type is the map module's, so the two halves cannot drift apart.
 */
type Selection = MapSelection;

/* ==========================================================================
   Screen
   ========================================================================== */

export function VoyageDetail() {
  const { id } = useParams<{ id: string }>();
  const query = useVoyage(id);
  const desktop = useIsDesktop();

  const [selection, setSelection] = useState<Selection | null>(null);

  /** Photo id → the element showing it, in whichever layout is mounted. Used
   *  only to scroll a pin's photograph into view. */
  const photoNodes = useRef(new Map<string, HTMLElement>());

  const registerPhoto = useCallback((photoId: string, node: HTMLElement | null) => {
    if (node === null) {
      photoNodes.current.delete(photoId);
    } else {
      photoNodes.current.set(photoId, node);
    }
  }, []);

  useEffect(() => {
    if (selection === null || selection.source !== "pin") return;

    const node = photoNodes.current.get(selection.photoId);
    // `scrollIntoView` is absent in jsdom and in any non-browser renderer, and
    // a missing scroll is not worth throwing over.
    node?.scrollIntoView?.({
      block: "nearest",
      inline: "nearest",
      behavior: prefersReducedMotion() ? "auto" : "smooth",
    });
  }, [selection]);

  const selectFromPin = useCallback((photoId: string) => {
    setSelection({ photoId, source: "pin" });
  }, []);

  const selectFromPhoto = useCallback((photoId: string) => {
    setSelection((current) =>
      current?.photoId === photoId ? null : { photoId, source: "photo" },
    );
  }, []);

  const clearSelection = useCallback(() => setSelection(null), []);

  if (id === undefined) {
    return <NotFound />;
  }

  if (query.isPending) {
    return <LoadingState desktop={desktop} />;
  }

  if (query.isError) {
    return <ErrorState error={query.error} onRetry={() => void query.refetch()} />;
  }

  const view: LayoutProps = {
    voyage: query.data,
    selection,
    onSelectPin: selectFromPin,
    onSelectPhoto: selectFromPhoto,
    onClearSelection: clearSelection,
    registerPhoto,
  };

  return desktop ? <DesktopLayout {...view} /> : <MobileLayout {...view} />;
}

/**
 * The box the screen has to fill.
 *
 * `height: 100%` is the intent: the shell's `<main>` is a grown flex item, and
 * the map and the rail divide its height between them. The `min-height` is the
 * same figure derived from the chrome's own tokens, and it is there because a
 * percentage height resolves to nothing if an ancestor's height is ever auto —
 * which for this screen would mean a map of zero pixels rather than a slightly
 * wrong one.
 */
function fillHeight(desktop: boolean): CSSProperties {
  return {
    height: "100%",
    minHeight: desktop
      ? "calc(100dvh - var(--pv-header-h))"
      : "calc(100dvh - var(--pv-header-h) - var(--pv-tabbar-h))",
  };
}

interface LayoutProps {
  voyage: Voyage;
  /** The whole selection, not just the id: the map needs to know which half of
   *  the screen asked, so that clicking a pin does not also re-centre on it. */
  selection: Selection | null;
  onSelectPin: (photoId: string) => void;
  onSelectPhoto: (photoId: string) => void;
  onClearSelection: () => void;
  registerPhoto: (photoId: string, node: HTMLElement | null) => void;
}

/**
 * The pins for a voyage, built from the photographs it already carries.
 *
 * Photographs with no coordinate are dropped here and picked up by
 * {@link UnplacedSection} instead — the map genuinely cannot draw them, and
 * they are accounted for in the rail rather than silently lost.
 *
 * The label is {@link photoAlt}, the same string the photograph carries in the
 * rail. That is deliberate: it is the only accessible name a pin has, and
 * sharing it is what lets a screen-reader user tell which pin belongs to which
 * photograph.
 */
function pinsFor(voyage: Voyage): VoyagePin[] {
  return voyage.photos
    .filter((photo): photo is VoyagePhoto & { coordinate: Coordinate } => photo.coordinate !== null)
    .map((photo) => ({
      id: photo.id,
      coordinate: photo.coordinate,
      label: photoAlt(photo, voyage),
    }));
}

/* ==========================================================================
   Desktop — map left, rail right
   ========================================================================== */

const RAIL_WIDTH = 420;

function DesktopLayout({
  voyage,
  selection,
  onSelectPin,
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
                        voyage={voyage}
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
                voyage={voyage}
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
const SHEET_PEEK = 168;
const SHEET_EXPANDED = 300;

function MobileLayout({
  voyage,
  selection,
  onSelectPin,
  onSelectPhoto,
  onClearSelection,
  registerPhoto,
}: LayoutProps) {
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
        // Attribution is a licence requirement, so it rides above the sheet
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

        <div style={{ ...CHIP, padding: "0 var(--pv-space-3)" }}>
          {/* The precision belongs in the header: without it a viewer reads
              the pins as exact. */}
          <Badge tone="accent">{voyage.granularity_label}</Badge>
        </div>
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
                <div className="pv-xs pv-subtle">
                  {countLabel(voyage.place_count, "place")} · {voyage.granularity_label}
                </div>
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
                  {coordinateLabel(selected.coordinate) ?? "No location"} ·{" "}
                  {voyage.granularity_label}
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

        <div className="pv-sheet-body">
          {voyage.photos.length === 0 ? (
            <NoPhotos voyageId={voyage.id} />
          ) : (
            <>
              {selected !== null && (
                <div style={{ marginBottom: "var(--pv-space-3)" }}>
                  <Photo
                    src={selected.display_url}
                    alt={photoAlt(selected, voyage)}
                    loading={selected.variant_status === "pending"}
                  />
                </div>
              )}

              <Filmstrip label="Photos in this voyage">
                {placed.map((photo) => (
                  <SelectablePhoto
                    key={photo.id}
                    photo={photo}
                    voyage={voyage}
                    selected={photo.id === selectedPhotoId}
                    onSelect={onSelectPhoto}
                    registerPhoto={registerPhoto}
                  />
                ))}
              </Filmstrip>

              <UnplacedSection
                photos={unplaced}
                voyage={voyage}
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
const CHIP: CSSProperties = {
  background: "var(--pv-bg)",
  borderRadius: "var(--pv-radius-full)",
  boxShadow: "var(--pv-shadow-2)",
  display: "flex",
  alignItems: "center",
  gap: "var(--pv-space-2)",
  padding: "var(--pv-space-2) var(--pv-space-3)",
  minHeight: "var(--pv-touch-min)",
};

/** Not in `components/icons` — the icon set there has no chevron. */
function BackIcon() {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="m15 18-6-6 6-6" />
    </svg>
  );
}

/* ==========================================================================
   Shared pieces
   ========================================================================== */

function VoyageHeading({ voyage }: { voyage: Voyage }) {
  const { user } = useAuth();
  const owner = user !== null && user.id === voyage.owner_id;

  return (
    <>
      <div className="pv-inline-2" style={{ marginBottom: "var(--pv-space-2)" }}>
        <Badge tone={voyage.published ? "public" : "draft"}>
          {voyage.published ? "Public" : "Draft"}
        </Badge>
        {/* The granularity badge sits in the header for the same reason the
            design puts it there: a viewer who does not know the precision
            reads the pins as exact. */}
        <Badge tone="accent">{voyage.granularity_label}</Badge>
        <span className="pv-spacer" />
        {owner && (
          <Link
            to={`/voyages/${voyage.id}/edit`}
            className="pv-btn pv-btn--secondary pv-btn--sm"
          >
            Edit
          </Link>
        )}
        <a
          className="pv-btn pv-btn--ghost pv-btn--icon pv-btn--sm"
          href={`/voyages/${voyage.id}`}
          aria-label="Link to this voyage"
        >
          <ShareIcon size={15} />
        </a>
      </div>

      <h1 className="pv-h2">{voyage.title}</h1>

      {voyage.description !== null && voyage.description !== "" && (
        <p className="pv-sm pv-muted" style={{ margin: "var(--pv-space-2) 0 var(--pv-space-3)" }}>
          {voyage.description}
        </p>
      )}

      <div className="pv-inline-2 pv-xs pv-subtle">
        <span>{countLabel(voyage.photo_count, "photo")}</span>
        <span aria-hidden="true">·</span>
        <span>{countLabel(voyage.place_count, "place")}</span>
      </div>
    </>
  );
}

interface SelectablePhotoProps {
  photo: VoyagePhoto;
  voyage: Voyage;
  selected: boolean;
  onSelect: (photoId: string) => void;
  registerPhoto: (photoId: string, node: HTMLElement | null) => void;
}

/**
 * A photograph in the rail or the strip.
 *
 * Wrapped in a `div` that holds the ref rather than reaching into `Photo`:
 * scrolling wants an element, `Photo` does not forward one, and the wrapper is
 * the same box either way.
 */
function SelectablePhoto({
  photo,
  voyage,
  selected,
  onSelect,
  registerPhoto,
}: SelectablePhotoProps) {
  return (
    <div ref={(node) => registerPhoto(photo.id, node)}>
      <Photo
        src={photo.thumbnail_url}
        alt={photoAlt(photo, voyage)}
        ratio="1:1"
        selected={selected}
        unplaced={!photo.placed}
        loading={photo.variant_status === "pending"}
        onSelect={() => onSelect(photo.id)}
        label={photoAlt(photo, voyage)}
        caption={
          selected ? (
            <span className="pv-xs">
              {[capturedTimeLabel(photo), voyage.granularity_label]
                .filter((part) => part !== null)
                .join(" · ")}
            </span>
          ) : undefined
        }
      />
    </div>
  );
}

interface UnplacedSectionProps {
  photos: readonly VoyagePhoto[];
  voyage: Voyage;
  selectedPhotoId: string | null;
  onSelect: (photoId: string) => void;
  registerPhoto: (photoId: string, node: HTMLElement | null) => void;
}

/**
 * Unplaced photographs, at the bottom of the rail.
 *
 * They are given a home rather than being dropped. A user who uploaded 54
 * photos and counts 48 on the map needs to be told where the other six went,
 * and "nowhere, silently" is the answer that makes the map look broken.
 */
function UnplacedSection({
  photos,
  voyage,
  selectedPhotoId,
  onSelect,
  registerPhoto,
}: UnplacedSectionProps) {
  const [open, setOpen] = useState(false);

  if (photos.length === 0) return null;

  return (
    <section style={{ marginTop: "var(--pv-space-6)" }}>
      <div
        className="pv-card"
        style={{
          padding: "var(--pv-space-3)",
          background: "var(--pv-warning-bg)",
          borderColor: "transparent",
        }}
      >
        <div className="pv-inline-2" style={{ justifyContent: "space-between" }}>
          <div>
            <div className="pv-sm" style={{ fontWeight: "var(--pv-weight-semibold)" }}>
              {photos.length === 1
                ? "1 photo has no location"
                : `${photos.length} photos have no location`}
            </div>
            <div className="pv-xs pv-muted">
              They are part of this voyage but not on the map.
            </div>
          </div>
          <Button
            variant="secondary"
            size="sm"
            aria-expanded={open}
            style={{ minHeight: "var(--pv-touch-min)" }}
            onClick={() => setOpen((current) => !current)}
          >
            {open ? "Hide" : "View"}
          </Button>
        </div>
      </div>

      {open && (
        <div style={{ marginTop: "var(--pv-space-3)" }}>
          <PhotoGrid label="Photos with no location">
            {photos.map((photo) => (
              <SelectablePhoto
                key={photo.id}
                photo={photo}
                voyage={voyage}
                selected={photo.id === selectedPhotoId}
                onSelect={onSelect}
                registerPhoto={registerPhoto}
              />
            ))}
          </PhotoGrid>
        </div>
      )}
    </section>
  );
}

/* ==========================================================================
   Loading, empty and error
   ========================================================================== */

function LoadingState({ desktop }: { desktop: boolean }) {
  const tiles = [0, 1, 2, 3];

  return (
    <div
      aria-busy="true"
      style={{
        ...fillHeight(desktop),
        display: "grid",
        gridTemplateColumns: desktop ? `1fr ${RAIL_WIDTH}px` : "1fr",
      }}
    >
      <span className="pv-sr">Loading this voyage…</span>
      <div className="pv-map" />
      {desktop && (
        <div
          style={{
            borderLeft: "1px solid var(--pv-border)",
            padding: "var(--pv-space-4)",
            display: "flex",
            flexDirection: "column",
            gap: "var(--pv-space-3)",
          }}
        >
          <Skeleton width="40%" height={12} />
          <Skeleton width="80%" height={24} />
          <Skeleton width="60%" height={12} />
          <PhotoGrid>
            {tiles.map((tile) => (
              <Skeleton key={tile} className="pv-photo pv-photo--square" />
            ))}
          </PhotoGrid>
        </div>
      )}
    </div>
  );
}

function ErrorState({ error, onRetry }: { error: Error; onRetry: () => void }) {
  const missing = isApiError(error) && error.status === 404;
  if (missing) return <NotFound />;

  const requestId = isApiError(error) ? error.requestId : null;

  return (
    <div className="pv-container" style={{ paddingBlock: "var(--pv-space-8)" }}>
      <EmptyState
        tone="danger"
        icon={<WarningTriangleIcon size={40} />}
        title="This voyage could not be loaded"
        action={
          <Button variant="secondary" onClick={onRetry} style={{ minHeight: "var(--pv-touch-min)" }}>
            Try again
          </Button>
        }
      >
        Something went wrong at our end.
        {requestId !== null && <> Quote reference {requestId} if you report it.</>}
      </EmptyState>
    </div>
  );
}

function NotFound() {
  return (
    <div className="pv-container" style={{ paddingBlock: "var(--pv-space-8)" }}>
      <EmptyState
        icon={<ImageIcon size={40} />}
        title="This voyage is not here"
        action={
          <Link className="pv-btn pv-btn--secondary" to="/">
            Discover voyages
          </Link>
        }
      >
        It may have been deleted, or it may never have been published.
      </EmptyState>
    </div>
  );
}

function NoPhotos({ voyageId }: { voyageId: string }) {
  return (
    <EmptyState
      icon={<ImageIcon size={40} />}
      title="No photographs yet"
      action={
        <Link className="pv-btn pv-btn--secondary" to={`/voyages/${voyageId}/edit`}>
          Add photos
        </Link>
      }
    >
      This voyage has nothing in it, so there is nothing to put on the map.
    </EmptyState>
  );
}

/* ==========================================================================
   Formatting
   ========================================================================== */

const DAY_FORMAT = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "long" });
const TIME_FORMAT = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });

function placedPhotos(voyage: Voyage): VoyagePhoto[] {
  return voyage.photos.filter((photo) => photo.placed);
}

function parseCaptured(photo: VoyagePhoto): Date | null {
  if (photo.captured_at === null) return null;
  const date = new Date(photo.captured_at);
  return Number.isNaN(date.getTime()) ? null : date;
}

interface DateGroup {
  key: string;
  label: string;
  photos: VoyagePhoto[];
}

/**
 * Groups by capture date, in the order the photographs arrive.
 *
 * Chronology is the only ordering a travelogue needs no explanation for, and
 * the API already returns photographs in voyage position order. Photographs
 * with no capture time land in one trailing group rather than being hidden —
 * the same principle as unplaced photographs.
 */
function groupByCaptureDate(photos: readonly VoyagePhoto[]): DateGroup[] {
  const groups: DateGroup[] = [];
  const byKey = new Map<string, DateGroup>();

  for (const photo of photos) {
    const captured = parseCaptured(photo);
    const key = captured === null ? "undated" : captured.toDateString();
    const label = captured === null ? "Date unknown" : DAY_FORMAT.format(captured);

    const existing = byKey.get(key);
    if (existing === undefined) {
      const group: DateGroup = { key, label, photos: [photo] };
      byKey.set(key, group);
      groups.push(group);
    } else {
      existing.photos.push(photo);
    }
  }

  // "Date unknown" belongs at the end, after everything that can be ordered.
  return [...groups.filter((g) => g.key !== "undated"), ...groups.filter((g) => g.key === "undated")];
}

function capturedTimeLabel(photo: VoyagePhoto): string | null {
  const captured = parseCaptured(photo);
  return captured === null ? null : TIME_FORMAT.format(captured);
}

function capturedAtLabel(photo: VoyagePhoto): string | null {
  const captured = parseCaptured(photo);
  return captured === null ? null : `${DAY_FORMAT.format(captured)}, ${TIME_FORMAT.format(captured)}`;
}

/** The rounded position, as the viewer would read it. Never the original EXIF:
 *  a voyage's coordinates have already been coarsened server-side. */
function coordinateLabel(coordinate: Coordinate | null): string | null {
  if (coordinate === null) return null;
  return `${coordinate.latitude.toFixed(2)}°, ${coordinate.longitude.toFixed(2)}°`;
}

/**
 * The accessible name of a photograph, which is also its pin's name.
 *
 * A pin is a graphic with no text, so this is the only name it has — "Photo
 * taken 14 February, 09:12" rather than "Pin". The two sides of the selection
 * deliberately share it, so a screen-reader user can tell which pin belongs to
 * which photograph.
 */
function photoAlt(photo: VoyagePhoto, voyage: Voyage): string {
  const when = capturedAtLabel(photo);
  const base = when === null ? `Photograph ${photo.position}` : `Photograph taken ${when}`;
  return photo.placed ? `${base}, within ${voyage.granularity_label}` : `${base}, no location`;
}

function countLabel(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}
