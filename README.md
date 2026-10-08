# PinVoyager

A photo travelogue site. Users sign in with a social provider, upload photos, and group
them into **voyages** shown on an interactive map. The app reads locations from photo EXIF
data and lets each user choose the precision shown to the public.

Draft voyages are visible only to their owners. Published voyages are public.

## Status

The Rust API and React app are in the repository. See the [phase index](docs/plans/README.md)
for work completed and work still in progress. See [running locally](docs/running.md) to start
the app.

## Repository layout

| Path | Contents |
|---|---|
| `docs/decisions.md` | Locked architectural decisions. Read this before proposing anything that contradicts one. |
| `docs/plans/` | One document per phase. The unit of work. |
| `docs/designs/` | Mermaid diagrams of routes and data flows, based on the current code. |
| `design/` | Design system source — tokens, component previews, screens, flow diagrams. |
| `crates/api/` | Rust API, database migrations, and integration tests. |
| `web/` | React app, API client, maps, and component tests. |

## How work is organized

Work proceeds in numbered phases. Each phase has a document in `docs/plans/` stating its
goal, non-goals, numbered targets, and the acceptance criteria that prove each target is
met. See `docs/plans/README.md` for the index and the working agreement.

The short version: edit a phase document until its targets say what you actually want,
then hand that phase over for execution.

## Design system

`design/tokens.css` is the single source of truth for color, type, spacing, radii,
elevation, and motion. The frontend imports this file directly rather than restating its
values — that is deliberate, and changing a token should change the app.

Preview pages under `design/foundations`, `design/components`, `design/screens`, and
`design/flows` are self-contained HTML: no build step, no network requests. Open
`design/index.html` in a browser to browse them all locally. They are also mirrored to a
Claude Design project for review.

## Stack

- **Backend** — Rust: axum, tokio, sqlx, PostgreSQL
- **Frontend** — React, TypeScript, Vite, MapLibre GL JS
- **Map tiles** — OpenStreetMap raster tiles during development; self-hosted Protomaps
  `.pmtiles` remain planned for release
- **Auth** — OIDC/OAuth2 against Google, Microsoft, Apple, and Facebook. The app stores no
  user passwords or long-lived provider tokens.

The create voyage flow collects a Voyage Name and photos together. Every photo
belongs to one voyage; drafts have prominent publish actions on detail and edit
pages. See [voyage-owned photos](docs/voyage-owned-photos.md).
