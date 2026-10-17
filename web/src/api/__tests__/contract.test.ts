/**
 * Checks API response types against committed router-response fixtures.
 *
 * Assignments detect missing or incompatible fields, ExtraKeys detects added
 * fields, and separate checks validate enum spellings widened by JSON imports.
 * The runtime test invokes tsc because Vitest alone strips types.
 *
 * To regenerate fixtures from the repository root:
 * DATABASE_URL=postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager \
 *   PINVOYAGER_UPDATE_FIXTURES=1 cargo test --test api_fixtures
 */

import { execFileSync } from "node:child_process";
import { describe, expect, it } from "vitest";

import errorBadRequest from "../__fixtures__/error_bad_request.json";
import errorNotFound from "../__fixtures__/error_not_found.json";
import errorUnauthorized from "../__fixtures__/error_unauthorized.json";
import errorValidation from "../__fixtures__/error_validation.json";
import publicProfile from "../__fixtures__/public_profile.json";
import me from "../__fixtures__/me.json";
import photo from "../__fixtures__/photo.json";
import photoList from "../__fixtures__/photo_list.json";
import photoUpload from "../__fixtures__/photo_upload.json";
import discoverMap from "../__fixtures__/discover_map.json";
import voyagePhotos from "../__fixtures__/voyage_photos.json";
import voyageCreated from "../__fixtures__/voyage_created.json";
import voyageDetail from "../__fixtures__/voyage_detail.json";
import voyageList from "../__fixtures__/voyage_list.json";
import voyageListMine from "../__fixtures__/voyage_list_mine.json";
import voyageMap from "../__fixtures__/voyage_map.json";
import voyageReordered from "../__fixtures__/voyage_reordered.json";
import voyageUpdated from "../__fixtures__/voyage_updated.json";

import type { ApiErrorBody } from "../client";
import type {
  ApiError,
  DiscoverMap,
  Granularity,
  LinkedProvider,
  Me,
  PublicProfilePage,
  Photo,
  PhotoList,
  UploadResult,
  VariantStatus,
  Voyage,
  VoyageList,
  VoyageMap,
} from "../types";

// ---------------------------------------------------------------------------
// The two type-level tools
// ---------------------------------------------------------------------------

/**
 * An API type as it survives a trip through a `.json` import.
 *
 * TypeScript widens the literals in a JSON module: `"km10"` arrives as `string`
 * and `5000` as `number`. So a fixture cannot be assigned to a type whose field
 * is `"exact" | "km1" | "km10" | "km50"`, and the assignment has to be against a
 * widened form instead.
 *
 * Everything drift is actually made of survives that widening — which keys
 * exist, whether each may be null, whether it is an object, an array or a
 * scalar. The literal unions are the one thing it loses, and they are checked
 * separately below.
 */
type FromJson<T> = T extends string
  ? string
  : T extends number
    ? number
    : T extends boolean
      ? boolean
      : T extends readonly (infer E)[]
        ? FromJson<E>[]
        : T extends object
          ? { [K in keyof T]: FromJson<T[K]> }
          : T;

/**
 * The dotted paths of every key in `Actual` that `Expected` does not declare,
 * as a union of string literals, or `never` when there are none.
 *
 * This is the half that catches a field the *server added*. Assignment cannot:
 * excess-property checking is a property of fresh object literals, and a JSON
 * import is a value with a type, not a literal at the assignment site.
 */
type ExtraKeys<Actual, Expected> = Actual extends readonly (infer A)[]
  ? Expected extends readonly (infer E)[]
    ? ExtraKeys<A, E>
    : never
  : Actual extends object
    ? Expected extends object
      ? {
          [K in Extract<keyof Actual, string>]: K extends keyof Expected
            ? Nested<K, ExtraKeys<Actual[K], Expected[K]>>
            : K;
        }[Extract<keyof Actual, string>]
      : never
    : never;

type Nested<K extends string, Rest> = Rest extends string ? `${K}.${Rest}` : never;

/**
 * Fails to compile unless `Extra` is `never`.
 *
 * The failure reads `Type '"nickname"' does not satisfy the constraint
 * 'never'`, which names the field the server grew.
 */
type NoExtraFields<Extra extends never> = Extra;

/** True only when two types are identical, not merely mutually assignable. */
type Identical<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;

type Assert<T extends true> = T;

// ---------------------------------------------------------------------------
// 1. Every field the types declare is present, with a compatible type
// ---------------------------------------------------------------------------

/**
 * The assignment the contract rests on.
 *
 * Each property is a captured response body being assigned to the type that
 * claims to describe it. A field the server dropped or renamed makes the
 * corresponding line fail here, naming the missing property.
 *
 * Exported so that `noUnusedLocals` does not delete the entire check.
 */
export const assigned: {
  me: FromJson<Me>;
  publicProfile: FromJson<PublicProfilePage>;
  photo: FromJson<Photo>;
  photoList: FromJson<PhotoList>;
  photoUpload: FromJson<UploadResult>;
  voyageCreated: FromJson<Voyage>;
  voyagePhotos: FromJson<Voyage>;
  voyageReordered: FromJson<Voyage>;
  voyageUpdated: FromJson<Voyage>;
  voyageDetail: FromJson<Voyage>;
  voyageMap: FromJson<VoyageMap>;
  discoverMap: FromJson<DiscoverMap>;
  voyageList: FromJson<VoyageList>;
  voyageListMine: FromJson<VoyageList>;
  errorNotFound: FromJson<ApiError>;
  errorUnauthorized: FromJson<ApiError>;
  errorBadRequest: FromJson<ApiError>;
  // Check this validation fixture against the transport error-body type,
  // which accepts arbitrary codes and optional field errors.
  errorValidation: ApiErrorBody;
} = {
  me,
  publicProfile,
  photo,
  photoList,
  photoUpload,
  voyageCreated,
  voyagePhotos,
  voyageReordered,
  voyageUpdated,
  voyageDetail,
  voyageMap,
  discoverMap,
  voyageList,
  voyageListMine,
  errorNotFound,
  errorUnauthorized,
  errorBadRequest,
  errorValidation,
};

// ---------------------------------------------------------------------------
// 2. …and the server sent nothing the types do not declare
// ---------------------------------------------------------------------------

/**
 * One entry per fixture. Each resolves to `never` while the shapes agree, and
 * to the dotted path of the offending field the moment they do not — at which
 * point the entry fails its `extends never` constraint and the build stops.
 */
export type ContractHasNoExtraFields = [
  NoExtraFields<ExtraKeys<typeof me, Me>>,
  NoExtraFields<ExtraKeys<typeof publicProfile, PublicProfilePage>>,
  NoExtraFields<ExtraKeys<typeof photo, Photo>>,
  NoExtraFields<ExtraKeys<typeof photoList, PhotoList>>,
  NoExtraFields<ExtraKeys<typeof photoUpload, UploadResult>>,
  NoExtraFields<ExtraKeys<typeof voyageCreated, Voyage>>,
  NoExtraFields<ExtraKeys<typeof voyagePhotos, Voyage>>,
  NoExtraFields<ExtraKeys<typeof voyageReordered, Voyage>>,
  NoExtraFields<ExtraKeys<typeof voyageUpdated, Voyage>>,
  NoExtraFields<ExtraKeys<typeof voyageDetail, Voyage>>,
  NoExtraFields<ExtraKeys<typeof voyageMap, VoyageMap>>,
  NoExtraFields<ExtraKeys<typeof voyageList, VoyageList>>,
  NoExtraFields<ExtraKeys<typeof voyageListMine, VoyageList>>,
  NoExtraFields<ExtraKeys<typeof errorNotFound, ApiError>>,
  NoExtraFields<ExtraKeys<typeof errorUnauthorized, ApiError>>,
  NoExtraFields<ExtraKeys<typeof errorBadRequest, ApiError>>,
  NoExtraFields<ExtraKeys<typeof errorValidation, ApiErrorBody>>,
];

// ---------------------------------------------------------------------------
// 3. The enum spellings, which the widening in (1) throws away
// ---------------------------------------------------------------------------

const GRANULARITIES = ["exact", "km1", "km10", "km50"] as const;
const VARIANT_STATUSES = ["pending", "ready", "failed"] as const;
const ORIGINAL_FORMATS = ["jpeg", "png", "webp", "heic"] as const;
const PROVIDERS = ["google", "microsoft", "apple"] as const;

/**
 * The arrays above are the same sets as the unions in `types.ts`.
 *
 * Asserted rather than assumed, so that a spelling added to `types.ts` without
 * being added here — which would leave the runtime check below quietly
 * rejecting a legitimate value — is a compile error instead.
 */
export type ContractEnumsMatch = [
  Assert<Identical<(typeof GRANULARITIES)[number], Granularity>>,
  Assert<Identical<(typeof VARIANT_STATUSES)[number], VariantStatus>>,
  Assert<Identical<(typeof ORIGINAL_FORMATS)[number], Photo["original_format"]>>,
  Assert<Identical<(typeof PROVIDERS)[number], LinkedProvider["provider"]>>,
];

// ---------------------------------------------------------------------------
// The runtime half
// ---------------------------------------------------------------------------

describe("the captured fixtures", () => {
  it("use only the enum spellings the types allow", () => {
    for (const voyage of [voyageCreated, voyageDetail, voyageUpdated]) {
      expect(GRANULARITIES).toContain(voyage.granularity);
      for (const p of voyage.photos) expect(VARIANT_STATUSES).toContain(p.variant_status);
    }

    expect(GRANULARITIES).toContain(voyageMap.granularity);
    for (const summary of [...voyageList.voyages, ...voyageListMine.voyages]) {
      expect(GRANULARITIES).toContain(summary.granularity);
    }

    for (const p of [photo, ...photoList.photos, ...photoUpload.photos]) {
      expect(ORIGINAL_FORMATS).toContain(p.original_format);
      expect(VARIANT_STATUSES).toContain(p.variant_status);
    }

    for (const identity of me.identities) expect(PROVIDERS).toContain(identity.provider);

    for (const error of [errorNotFound, errorUnauthorized, errorBadRequest]) {
      expect(error.code).toMatch(/^[a-z_]+$/);
    }
  });

  /**
   * The assignments above are only as good as what the fixtures happen to
   * contain. A capture in which every photograph had coordinates would type
   * check perfectly and would never have exercised `coordinate: null`, so the
   * day the server stopped sending the key at all, nothing here would notice.
   *
   * These assertions are on the *fixture generator*, not on the API: they say
   * the captured world is varied enough for the type check above to be worth
   * running.
   */
  it("cover the nullable cases, or the type check proves less than it looks", () => {
    expect(voyageDetail.photos.some((p) => p.coordinate === null)).toBe(true);
    expect(voyageDetail.photos.some((p) => p.coordinate !== null)).toBe(true);
    expect(voyageDetail.photos.some((p) => p.captured_at === null)).toBe(true);

    expect(photoList.photos.some((p) => p.latitude === null)).toBe(true);
    expect(photoList.photos.some((p) => p.latitude !== null)).toBe(true);

    expect(voyageListMine.voyages.some((v) => v.description === null)).toBe(true);
    expect(voyageListMine.voyages.some((v) => v.published_at === null)).toBe(true);
    expect(voyageList.voyages.some((v) => v.published_at !== null)).toBe(true);

    expect(voyageMap.pins.length).toBeGreaterThan(0);
    expect(voyageMap.unplaced.length).toBeGreaterThan(0);
    expect(me.identities.length).toBeGreaterThan(0);
    expect(me.user.avatar_url).toBeNull();

    // Counted after rounding: two photographs a few hundred meters apart are
    // one place at ~10 km. If these were equal the fixture would not
    // distinguish the two counts, and a client that confused them would pass.
    expect(voyageDetail.placed_count).toBe(2);
    expect(voyageDetail.place_count).toBe(1);
  });

  /**
   * Runs the contract type checks with tsc using the API-specific tsconfig.
   * Vitest does not type-check these assignments itself.
   */
  it("still fit src/api/types.ts", () => {
    // Relative to the working directory, which is the `web/` package: that is
    // what `npm run test` sets and what vitest resolves its own root against.
    // Not derived from `import.meta.url`, because vitest rewrites that to a URL
    // rooted at the project rather than at the filesystem, so its `pathname` is
    // `/src/api/...` and resolves to nothing.
    const project = "src/api/tsconfig.contract.json";
    const tsc = "node_modules/typescript/bin/tsc";

    try {
      execFileSync("node", [tsc, "-p", project, "--noEmit"], { encoding: "utf8" });
    } catch (error) {
      const failure = error as { stdout?: string; stderr?: string };
      const diagnostics = [failure.stdout, failure.stderr, String(error)]
        .filter((part) => part !== undefined && part !== "")
        .join("\n");
      throw new Error(
        "The captured API responses no longer fit the types in src/api/types.ts.\n\n" +
          "Either the server changed and types.ts must follow, or the fixtures are stale — " +
          "regenerate them with:\n" +
          "  DATABASE_URL=postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager \\\n" +
          "    cargo test --test api_fixtures\n\n" +
          diagnostics,
      );
    }
  }, 120_000);
});
