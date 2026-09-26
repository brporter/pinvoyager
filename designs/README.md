# Diagrams

Mermaid diagrams of how a person moves through PinVoyager and how data moves
through it. Derived from the code as it stands, not from the phase plans — where
a plan and the code disagree, these follow the code and say so.

Not to be confused with `design/` (singular), which is the design system:
tokens, component previews, and screen mock-ups.

| Document | What it shows |
|---|---|
| [navigation.md](navigation.md) | Every route, who can reach it, and every link and redirect between them |
| [auth-flow.md](auth-flow.md) | Sign-in and provider-linking, browser to provider to session cookie |
| [data-flow.md](data-flow.md) | A request from React through the API client to Postgres, and the cache that sits in the middle |
| [photo-pipeline.md](photo-pipeline.md) | Upload, validation gates, variant generation, and serving |
| [map-data-flow.md](map-data-flow.md) | The two maps, and the granularity boundary that rounds every coordinate |

## Source of truth for each diagram

| Diagram | Files read |
|---|---|
| Routes | `web/src/App.tsx`, `web/src/shell/destinations.ts`, `web/src/auth/RequireAuth.tsx` |
| Auth | `crates/api/src/http/auth.rs`, `web/src/auth/*` |
| Data flow | `web/src/api/client.ts`, `web/src/api/hooks.ts`, `crates/api/src/http/*` |
| Photos | `crates/api/src/http/photos.rs`, `crates/api/src/photos/*` |
| Maps | `crates/api/src/http/voyages.rs`, `crates/api/src/voyages/granularity.rs`, `web/src/map/*` |
