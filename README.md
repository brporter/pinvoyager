# PinVoyager

A public photo-travelogue site. Sign in with a social provider, upload photos, and group
them into **voyages** rendered as pins on an interactive map. Pin locations come from EXIF
data in the photos themselves, at a granularity each user controls.

Every voyage is public. There is no private mode.

## Status

Pre-implementation. The repository currently holds the design system and the phase plans.
No application code exists yet.

## Repository layout

| Path | Contents |
|---|---|
| `docs/decisions.md` | Locked architectural decisions. Read this before proposing anything that contradicts one. |
| `docs/plans/` | One document per phase. The unit of work. |
| `design/` | Design system source — tokens, component previews, screens, flow diagrams. |

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

- **Backend** — C# / .NET 10: ASP.NET Core Minimal APIs, EF Core, Npgsql, PostgreSQL
- **Frontend** — React, TypeScript, Vite, MapLibre GL JS
- **Map tiles** — self-hosted Protomaps `.pmtiles`, OpenStreetMap data
- **Auth** — OIDC/OAuth2 against Google, Microsoft, Apple, and Facebook. No credentials of
  any kind are stored.
