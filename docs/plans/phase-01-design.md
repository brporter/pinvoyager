# Phase 1 — Design

**Status:** built — awaiting review
**Depends on:** nothing
**Owner:** Claude

All eight targets are built and pushed to the Claude Design project `PinVoyager`
(29 files). The phase is not complete until you have reviewed the screens and the open
questions below are answered — review is the remaining acceptance criterion.

## Goal

The visual and interaction language of PinVoyager exists as a reviewable artifact before any
application code is written. Tokens, components, screens, and flows are defined precisely
enough that phase 6 can implement a screen without asking a layout question, and phase 3 can
implement auth without inventing an error state.

## Non-goals

- No React, no build step, no framework. Previews are self-contained HTML and CSS.
- No real map tiles. Map previews use static representations; live MapLibre is phase 7.
- No copywriting pass. Placeholder text is fine where the words are not the point.
- No illustration or logo design.
- No place names anywhere — geocoding is deferred per D5.

## Targets

1. **`design/tokens.css`** — the single source of truth for color, type scale, spacing, radii,
   elevation, and motion, as CSS custom properties. Light and dark palettes. This is the
   literal file the frontend imports in phase 6, not a description of one.
2. **Foundations previews** — color (every text/background pairing labeled with its measured
   contrast ratio), typography, spacing/radii/elevation, motion.
3. **Component library** — one preview per component, showing default, hover, focus-visible,
   active, disabled, loading, and error states wherever each applies:
   buttons · provider sign-in buttons · form controls · navigation · photo primitives ·
   map primitives · upload · granularity selector · feedback and empty states.
4. **Provider sign-in buttons follow each provider's published branding rules** — Google and
   Apple both mandate specific wordmarks, minimum sizes, and permitted color variants.
   Non-compliance is grounds for rejection during provider app review, so this is a
   correctness target, not an aesthetic one.
5. **Screens**, each at 390 px and 1440 px: discover · voyage detail · photo lightbox ·
   upload flow · voyage create/edit · public profile · account settings · sign-in ·
   auth callback and conflict states · 404.
6. **Voyage detail resolves the map/photo layout problem** at both widths, and shows
   selection in both directions — pin to photo, and photo to pin.
7. **Flow diagrams** for auth, upload, and voyage lifecycle, annotated, with failure branches
   drawn — not just the happy path.
8. **The whole bundle is pushed to a Claude Design project** and browsable there, grouped
   into Foundations / Components / Screens / Flows.

## Acceptance criteria

- **T1** — `design/tokens.css` exists; every other design file consumes it via `var(--…)` and
  no preview hardcodes a hex value that duplicates a token.
- **T2** — Every foreground/background pairing shown in the color preview states its contrast
  ratio, and every pairing used for body or UI text meets WCAG AA (4.5:1, or 3:1 for large
  text). Pairings that fail are either removed or explicitly marked decorative-only.
- **T3/T4** — Open each component preview: every state listed above is visible and labeled.
  Provider buttons match the current published assets for each provider.
- **T5/T6** — Open each screen preview. At 390 px there is no horizontal page scroll. Voyage
  detail shows a distinct mobile treatment (full-bleed map, draggable photo sheet) and desktop
  treatment (side-by-side split), with selected states drawn in both.
- **T7** — Each flow diagram shows the failure branches named in the notes below, not only
  the success path.
- **T8** — `DesignSync list_files` returns the full path set, and the Design System pane
  renders cards under the four groups.
- **All** — Every preview renders standalone with no network requests. The design project
  enforces a strict CSP, so an external font or image reference will simply fail to load.

## Design references

This phase produces them. Everything under `design/`.

## Open questions

- **Brand direction.** The palette is currently a neutral, photography-first system: recessive
  desaturated chrome with a single accent reserved for interactive and map-pin states. If
  there is a real brand — a color, a name treatment, a feel — say so and it gets folded in.
- **Voyage ordering.** Photos on a voyage are shown in capture-time order. Manual reordering
  is drawn as available in voyage edit; if that should be chronological-only, that simplifies
  both the design and phase 5.
- **Granularity steps.** Currently exact / ~1 km / ~10 km / ~50 km. These are guesses at
  useful privacy tiers, not researched ones.

## Notes

**Failure branches the flow diagrams must cover.** These are where auth and upload actually
break, and leaving them undesigned means they get improvised in code:

- User cancels or denies consent at the provider
- Provider returns an email already attached to a different account (D3 — must never
  auto-merge; requires an explicit authenticated link action)
- Apple's name-only-on-first-authorization behavior
- Session expiry mid-upload
- Photo has no GPS EXIF at all — common for screenshots, scans, and location-services-off
  cameras. These need a visible home ("unplaced photos"), not silent omission.
- Upload rejected: not an image by magic bytes, too large, corrupt, decode-bomb dimensions

**EXIF stripping is user-visible.** Per D6, the granularity control states that served images
are re-encoded with metadata removed. The guarantee is the feature; if the UI does not say so,
the user cannot know the setting means anything.

**Two card-grouping mechanics.** Each preview file starts with a
`<!-- @dsCard group="…" -->` marker on its first line. That is what places it in the Design
System pane.
