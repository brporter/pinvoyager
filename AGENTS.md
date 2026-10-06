# Agent guide

## Communication

- Describe checks, thresholds, and requirements in plain language; avoid vague labels.

## Start here

- Read `docs/decisions.md` before changing architecture or product behavior. It outranks the phase plans. Update it first if a settled decision needs to change.
- For phase work, read the relevant document in `docs/plans/` and the working agreement in `docs/plans/README.md`. Treat its numbered targets and acceptance criteria as the scope; resolve consequential open questions before implementation. Update phase status only when the acceptance criteria are met.
- Use `README.md` for the repository overview and `docs/running.md` for local setup.

## Repository map

- `crates/api/`: Rust API (axum, Tokio, sqlx), migrations, and integration tests.
- `web/`: React and TypeScript app, API client, screens, map, and component tests.
- `design/`: design system and HTML reference screens. `design/tokens.css` is the shared source for visual tokens; the app imports it through `web/src/styles/index.css`.
- `docs/plans/`: phased delivery plans. `docs/decisions.md` records binding product and architecture choices.
- `docs/designs/`: diagrams of current routes and data flows, derived from the code.
- `.sqlx/`: committed offline query metadata. Regenerate it after changing checked SQL queries.

## Product and implementation constraints

- Published voyages are public; draft voyages are owner-only and return 404 to other callers. Serve only re-encoded image variants with metadata stripped, never original uploads. Apply a voyage's chosen granularity to coordinates in public responses; owner-only photo metadata intentionally returns exact coordinates. See D6 and D7.
- Authentication uses provider identities and server-side sessions. Do not add password storage or merge identities by email. See D1–D3.
- Keep database access async and move CPU-heavy image work and blocking file operations off Tokio worker threads. See D8.
- MapLibre currently uses OpenStreetMap raster tiles for development. Keep attribution visible and do not use that tile server for a release. See D4a and D4.
- A voyage's map groups pins in the browser; discovery groups them on the server after applying each voyage's granularity. See D10.
- Use the existing design references and shared tokens when changing UI. Record deliberate deviations where they are introduced.

## Local commands

- Follow `docs/running.md` for native image dependencies, PostgreSQL setup, environment configuration, and API startup. The usual API command is `cargo run -p pinvoyager-api` after starting the database with `docker compose up -d`.
- In `web/`, run `npm install` if dependencies are missing, then `npm run dev`. Vite serves the app on port 5173 and proxies API and auth requests to port 8080.
- Rust checks: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` (the tests need PostgreSQL and the native tools listed in `docs/running.md`).
- Web checks from `web/`: `npm run typecheck`, `npm run lint`, `npm run test`, and `npm run build`.
- After changing sqlx checked queries, run `cargo sqlx prepare --workspace -- --all-targets` against a migrated database and commit the resulting `.sqlx/` changes.

## Working in this repository

- Preserve unrelated changes in the worktree. Check status before editing; do not overwrite another person's in-progress work.
- Keep API response shapes and `web/src/api/` types and fixtures in sync when changing endpoints.
- For phase delivery, report the acceptance criteria actually checked and any remaining failures or blocked targets. Do not mark a phase complete solely because its code is written.

## Feature delivery requirements

- Maintain **100% code coverage** for both frontend and backend using stable toolchains. Frontend coverage must be 100% for statements, lines, functions, and branches across production code. Backend coverage must be 100% for lines, functions, and regions. Rust branch percentages require nightly instrumentation and are not part of the measured threshold; instead, write tests for both outcomes of each meaningful conditional. Do not exclude code from coverage just to satisfy thresholds; document any technically unavoidable instrumentation exclusion in `docs/` and report it.
- Start every feature by implementing its tests. Run those tests before writing the feature implementation and verify that they fail for the expected missing behavior. Then implement the feature and rerun the tests to verify they pass. Include the red and green results in the handoff.
- Fully document every feature in the appropriate files under `docs/`. Cover its behavior, relevant decisions or constraints, and how it fits the system. Include code references using repository paths and symbol names so readers can find the implementation and tests. Update existing documentation when it already covers the feature; add a document when needed.
- Do not mark a feature or phase complete unless its tests pass, both frontend and backend remain at the required coverage, and its documentation is complete. Report coverage commands and results, or identify the specific tooling or environment blocker when coverage could not be measured.
