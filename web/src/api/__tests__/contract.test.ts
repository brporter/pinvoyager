/**
 * Drift detection: the captured responses, checked against the hand-written
 * types.
 *
 * This is the second half of the scheme described at the top of `../types.ts`.
 * The first half is `crates/api/tests/api_fixtures.rs`, which drives the real
 * router against a real database and writes each response body into
 * `../__fixtures__/`. **Neither half is worth anything alone**: fixtures with no
 * types are just JSON, and types checked against types the same author wrote
 * only prove that person was consistent with themselves.
 *
 * To regenerate the fixtures, from the repository root:
 *
 * ```text
 * DATABASE_URL=postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager \
 *   cargo test --test api_fixtures
 * ```
 *
 * # How a change is caught
 *
 * Three mechanisms, because no one of them catches everything:
 *
 *  * **A removed or renamed field** fails the assignment in `assigned` below —
 *    the fixture no longer has a property the type requires.
 *  * **An added field** fails `ContractHasNoExtraFields`. Assignment alone does
 *    *not* catch this: TypeScript's excess-property check only fires on fresh
 *    object literals, and an imported JSON module is not one. `ExtraKeys` walks
 *    the two shapes and produces the dotted path of anything the server sent
 *    that the type does not declare.
 *  * **A changed set of enum spellings** fails `ContractEnumsMatch` at compile
 *    time and `it("...only spellings the types allow")` at runtime. The types
 *    declare these as unions of string literals, and TypeScript widens every
 *    literal in an imported `.json` file to `string`, so this one cannot be
 *    done by assignment.
 */

import { execFileSync } from "node:child_process";
import { describe, expect, it } from "vitest";

import errorForbidden from "../__fixtures__/error_forbidden.json";
import errorNotFound from "../__fixtures__/error_not_found.json";
import errorUnauthorized from "../__fixtures__/error_unauthorized.json";
import errorValidation from "../__fixtures__/error_validation.json";
import me from "../__fixtures__/me.json";
import photo from "../__fixtures__/photo.json";
import photoList from "../__fixtures__/photo_list.json";
import photoUpload from "../__fixtures__/photo_upload.json";
import discoverMap from "../__fixtures__/discover_map.json";
import voyageAttached from "../__fixtures__/voyage_attached.json";
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
  photo: FromJson<Photo>;
  photoList: FromJson<PhotoList>;
  photoUpload: FromJson<UploadResult>;
  voyageCreated: FromJson<Voyage>;
  voyageAttached: FromJson<Voyage>;
  voyageReordered: FromJson<Voyage>;
  voyageUpdated: FromJson<Voyage>;
  voyageDetail: FromJson<Voyage>;
  voyageMap: FromJson<VoyageMap>;
  discoverMap: FromJson<DiscoverMap>;
  voyageList: FromJson<VoyageList>;
  voyageListMine: FromJson<VoyageList>;
  errorNotFound: FromJson<ApiError>;
  errorUnauthorized: FromJson<ApiError>;
  errorForbidden: FromJson<ApiError>;
  // NOT `ApiError`, and the reason is a real gap rather than a convenience:
  // `types.ts` gives `ApiError.code` seven spellings — the ones `AppError`
  // produces — and no `errors` field. The two handler-local `Rejection` types
  // in `http::voyages` and `http::photos` produce six more (`validation_failed`,
  // `too_large`, `not_an_image`, `unprocessable_image`, `rate_limited`,
  // `variants_pending`), and the first of those also carries an `errors` array
  // so an edit form can put a message beside the input that failed. Until
  // `types.ts` widens, this fixture is checked against the client's own body
  // type, which is deliberately wider — see `client.ts`.
  errorValidation: ApiErrorBody;
} = {
  me,
  photo,
  photoList,
  photoUpload,
  voyageCreated,
  voyageAttached,
  voyageReordered,
  voyageUpdated,
  voyageDetail,
  voyageMap,
  discoverMap,
  voyageList,
  voyageListMine,
  errorNotFound,
  errorUnauthorized,
  errorForbidden,
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
  NoExtraFields<ExtraKeys<typeof photo, Photo>>,
  NoExtraFields<ExtraKeys<typeof photoList, PhotoList>>,
  NoExtraFields<ExtraKeys<typeof photoUpload, UploadResult>>,
  NoExtraFields<ExtraKeys<typeof voyageCreated, Voyage>>,
  NoExtraFields<ExtraKeys<typeof voyageAttached, Voyage>>,
  NoExtraFields<ExtraKeys<typeof voyageReordered, Voyage>>,
  NoExtraFields<ExtraKeys<typeof voyageUpdated, Voyage>>,
  NoExtraFields<ExtraKeys<typeof voyageDetail, Voyage>>,
  NoExtraFields<ExtraKeys<typeof voyageMap, VoyageMap>>,
  NoExtraFields<ExtraKeys<typeof voyageList, VoyageList>>,
  NoExtraFields<ExtraKeys<typeof voyageListMine, VoyageList>>,
  NoExtraFields<ExtraKeys<typeof errorNotFound, ApiError>>,
  NoExtraFields<ExtraKeys<typeof errorUnauthorized, ApiError>>,
  NoExtraFields<ExtraKeys<typeof errorForbidden, ApiError>>,
  NoExtraFields<ExtraKeys<typeof errorValidation, ApiErrorBody>>,
];

// ---------------------------------------------------------------------------
// 3. The enum spellings, which the widening in (1) throws away
// ---------------------------------------------------------------------------

const GRANULARITIES = ["exact", "km1", "km10", "km50"] as const;
const VARIANT_STATUSES = ["pending", "ready", "failed"] as const;
const ORIGINAL_FORMATS = ["jpeg", "png", "webp", "heic"] as const;
const PROVIDERS = ["google", "microsoft", "apple", "facebook"] as const;

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

    for (const error of [errorNotFound, errorUnauthorized, errorForbidden]) {
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
   * Runs the type checks above for real.
   *
   * Every mechanism in this file is a compile-time one, and `vitest` strips
   * types without checking them — so without this the whole contract would be
   * enforced only by whoever remembered to run `tsc`, which over a year is
   * nobody. Scoped to `src/api` by its own tsconfig so an unrelated screen
   * mid-edit cannot fail it; see `../tsconfig.contract.json`.
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
