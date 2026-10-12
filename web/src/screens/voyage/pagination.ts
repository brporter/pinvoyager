interface PhotoGroup<T> {
  key: string;
  label: string;
  photos: T[];
}

/** Pack two-column rows, repeating the heading when a date spans pages. */
export function paginatePhotoGroups<T>(
  groups: PhotoGroup<T>[],
  height: number,
  photoSize: number,
  headingHeight: number,
  rowGap: number,
  sectionGap: number,
): PhotoGroup<T>[][] {
  if (height < photoSize + headingHeight) return [];
  const pages: PhotoGroup<T>[][] = [];
  let page: PhotoGroup<T>[] = [];
  let used = 0;
  for (const group of groups) {
    for (let index = 0; index < group.photos.length; index += 2) {
      let last = page.at(-1);
      let cost =
        photoSize + (last?.key === group.key ? rowGap : headingHeight + (last ? sectionGap : 0));
      if (used + cost > height) {
        pages.push(page);
        page = [];
        used = 0;
        last = undefined;
        cost = photoSize + headingHeight;
      }
      if (last?.key === group.key) last.photos.push(...group.photos.slice(index, index + 2));
      else page.push({ ...group, photos: group.photos.slice(index, index + 2) });
      used += cost;
    }
  }
  if (page.length) pages.push(page);
  return pages;
}
