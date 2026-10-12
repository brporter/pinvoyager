import { useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";

import { PhotoGrid } from "../../components";
import { LazyVoyageMap } from "../../map";
import { groupByCaptureDate, pinsFor, placedPhotos, type LayoutProps } from "./model";
import { paginatePhotoGroups } from "./pagination";
import { SelectablePhoto, VoyageHeading } from "./VoyageChrome";
import { NoPhotos } from "./VoyageStates";

export function DesktopLayout({
  voyage,
  selection,
  onSelectPin,
  onOpenGallery,
  onSelectPhoto,
  registerPhoto,
}: LayoutProps) {
  const pins = useMemo(() => pinsFor(voyage), [voyage]);
  const area = useRef<HTMLDivElement>(null);
  const headingProbe = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({
    width: 388,
    height: 500,
    heading: 38,
    rowGap: 12,
    sectionGap: 20,
  });
  const [firstPhoto, setFirstPhoto] = useState<string | null>(null);

  useLayoutEffect(() => {
    const node = area.current!;
    const measure = () => {
      const rect = node.getBoundingClientRect();
      const probe = headingProbe.current!.getBoundingClientRect();
      const style = getComputedStyle(node);
      // A non-browser renderer has no layout. Keep the initial dimensions there.
      if (rect.width > 0) {
        setSize({
          width: rect.width,
          height: rect.height,
          heading: probe.height,
          rowGap: parseFloat(style.getPropertyValue("--voyage-row-gap")) || 12,
          sectionGap: parseFloat(style.getPropertyValue("--voyage-section-gap")) || 20,
        });
      }
    };
    measure();
    window.addEventListener("resize", measure);
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(node);
    observer?.observe(headingProbe.current!);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, []);

  const unplaced = voyage.photos.filter((photo) => !photo.placed);
  const groups = [
    ...groupByCaptureDate(placedPhotos(voyage)),
    ...(unplaced.length
      ? [{ key: "unplaced", label: "Photos with no location", photos: unplaced }]
      : []),
  ];
  // On short screens shrink the square thumbnails so at least one row fits.
  const photoSize = Math.max(
    0,
    Math.min((size.width - size.rowGap - 10) / 2, size.height - size.heading),
  );
  const pages = paginatePhotoGroups(
    groups,
    size.height,
    photoSize,
    size.heading,
    size.rowGap,
    size.sectionGap,
  );
  const requestedPage = pages.findIndex((page) =>
    page.some((group) => group.photos.some((photo) => photo.id === firstPhoto)),
  );
  const pageIndex = Math.max(0, requestedPage);
  const page = pages[pageIndex] ?? [];

  useLayoutEffect(() => {
    if (selection?.source === "pin") setFirstPhoto(selection.photoId);
  }, [selection]);

  function turnPage(index: number) {
    setFirstPhoto(pages[index][0].photos[0].id);
  }

  return (
    <div className="pv-voyage-desktop">
      <div className="pv-voyage-map-frame">
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
      <aside aria-label="Photos in this voyage" className="pv-voyage-rail">
        <div className="pv-voyage-rail-heading">
          <VoyageHeading voyage={voyage} />
        </div>
        <div className="pv-voyage-page-padding">
          <div
            ref={area}
            className="pv-voyage-page-area"
            style={{ "--voyage-photo-size": `${photoSize}px` } as CSSProperties}
          >
            <div ref={headingProbe} className="pv-voyage-heading-probe" aria-hidden="true">
              <h3 className="pv-voyage-date">Date</h3>
            </div>
            {voyage.photos.length === 0 ? (
              <NoPhotos voyageId={voyage.id} />
            ) : (
              page.map((group) => (
                <section key={group.key} className="pv-voyage-date-section">
                  <h3 className="pv-voyage-date">{group.label}</h3>
                  <PhotoGrid
                    label={group.key === "unplaced" ? group.label : `Photos from ${group.label}`}
                    className="pv-voyage-page-grid"
                  >
                    {group.photos.map((photo) => (
                      <SelectablePhoto
                        key={photo.id}
                        photo={photo}
                        selected={photo.id === selection?.photoId}
                        onSelect={onSelectPhoto}
                        registerPhoto={registerPhoto}
                      />
                    ))}
                  </PhotoGrid>
                </section>
              ))
            )}
          </div>
        </div>
        <nav aria-label="Photo pages" className="pv-voyage-pagination">
          {pageIndex > 0 ? (
            <a
              href={`#photos-page-${pageIndex}`}
              onClick={(event) => {
                event.preventDefault();
                turnPage(pageIndex - 1);
              }}
            >
              Previous
            </a>
          ) : (
            <span />
          )}
          <span className="pv-xs pv-subtle" aria-live="polite">
            {pages.length > 0 ? `Page ${pageIndex + 1} of ${pages.length}` : ""}
          </span>
          {pageIndex < pages.length - 1 ? (
            <a
              href={`#photos-page-${pageIndex + 2}`}
              onClick={(event) => {
                event.preventDefault();
                turnPage(pageIndex + 1);
              }}
            >
              More
            </a>
          ) : (
            <span />
          )}
        </nav>
      </aside>
    </div>
  );
}
