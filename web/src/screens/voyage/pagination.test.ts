import { describe, expect, it } from "vitest";
import { paginatePhotoGroups } from "./pagination";

const groups = [
  { key: "a", label: "First day", photos: [1, 2, 3, 4, 5] },
  { key: "b", label: "Second day", photos: [6, 7] },
];

describe("paginatePhotoGroups", () => {
  it("packs complete two-photo rows, including headings and spacing", () => {
    expect(paginatePhotoGroups(groups, 276, 100, 28, 12, 20)).toEqual([
      [{ ...groups[0], photos: [1, 2, 3, 4] }],
      [{ ...groups[0], photos: [5] }, groups[1]],
    ]);
  });
  it("repeats the date heading when a group continues on the next page", () => {
    expect(paginatePhotoGroups(groups, 128, 100, 28, 12, 20)).toEqual([
      [{ ...groups[0], photos: [1, 2] }],
      [{ ...groups[0], photos: [3, 4] }],
      [{ ...groups[0], photos: [5] }],
      [groups[1]],
    ]);
  });
  it("returns no pages when there are no photos or no room for a row", () => {
    expect(paginatePhotoGroups([], 250, 100, 28, 12, 20)).toEqual([]);
    expect(paginatePhotoGroups(groups, 127, 100, 28, 12, 20)).toEqual([]);
  });
});
