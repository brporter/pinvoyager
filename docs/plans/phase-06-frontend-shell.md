# Phase 6 — Frontend shell

**Status:** not started
**Depends on:** phases 1, 3, 5
**Owner:** Claude

## Goal

A React application that implements the phase 1 design system, routes between every screen,
signs users in and out against the phase 3 endpoints, and renders real voyages from the
phase 5 API — with the map itself still a placeholder, filled in by phase 7.

## Non-goals

- No map. Phase 7 owns MapLibre entirely; this phase leaves a sized placeholder.
- No server-side rendering.
- No component framework — the design system is the component library.
- No new visual decisions. If a screen is ambiguous, the design file is fixed first.

## Targets

1. Vite + React + TypeScript, strict mode on.
2. `design/tokens.css` imported directly as the token source. Values are **not** transcribed
   into TypeScript or a theme object — editing a token changes the app, or the design system
   is decorative.
3. Component library implementing `design/components/`, matching the states drawn there,
   with the same names so a design file and its component are findable from each other.
4. Routing for every screen: discover, voyage detail, upload, voyage edit, profile, settings,
   sign-in, auth callback, 404.
5. Auth integration — provider buttons initiate the phase 3 flows, the session cookie carries
   the session, `/api/me` establishes current-user state, and logout clears it.
6. A typed API client whose types are derived from the backend rather than hand-written, so
   the two cannot drift silently.
7. Responsive shell: desktop top navigation, mobile bottom bar, at the phase 1 breakpoints.
8. Upload UI against the real phase 4 endpoint — dropzone, per-file progress, per-file errors,
   batch summary.
9. Real loading, empty, and error states from the design. Not spinners standing in for
   designed states.
10. Accessibility baseline: keyboard reachable throughout, visible focus, correct labels,
    landmark regions, and honored `prefers-reduced-motion`.

## Acceptance criteria

- **T2** — Change a color in `design/tokens.css`, reload, and the application changes. If it
  does not, the token layer is not wired.
- **T3** — Every component state drawn in phase 1 is reachable in the running app.
- **T5** — Sign in with each of the four providers; the session survives a page reload; logout
  returns to the signed-out state.
- **T7** — At 390 px there is no horizontal scroll on any route. Every touch target is at
  least 44 × 44 px.
- **T8** — Uploading a mixed batch (valid images, an oversized file, a non-image) shows a
  correct per-file outcome for each rather than failing the batch.
- **T10** — Keyboard-only traversal of every route reaches every control with visible focus.
  Axe reports no critical violations.

## Design references

Everything under `design/components/` and `design/screens/`.

## Open questions

- **State management.** TanStack Query alone is likely sufficient given the server owns
  nearly all state; a client store may be unnecessary.
- **API type generation.** Emitting an OpenAPI document from axum and generating a client, or
  hand-maintaining types. Generation costs backend annotation work but eliminates a whole
  class of drift.
- Whether upload should be resumable. It matters on mobile connections with large batches,
  and it is much cheaper to design in now than to retrofit.

## Notes

T2 is the load-bearing target. The reason phase 1 produced a real `tokens.css` rather than a
specification document is so this import is direct. Copying values into a TypeScript theme
object would make the design system a snapshot that immediately starts drifting.

The map placeholder should occupy the true dimensions from the design so the layout is
genuinely exercised at both breakpoints before phase 7 drops a real map into it.
