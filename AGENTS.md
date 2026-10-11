# Agent guide

## Communication

- Answer the request directly. During longer work, give brief updates when you find something important, change direction, or reach a milestone.
- State what you observed, what you inferred, and what remains uncertain. Do not present an assumption as a fact.
- Ask a question only when the answer would change the result. Give the relevant context and clear options. Continue work that does not depend on the answer.
- When reporting work, state what changed, why, what you checked, and what remains unfinished. Say when you did not run a check.
- If work is blocked, name the blocker and the specific input or action needed.

## Writing

- Use plain English and active voice. Name who does what.
- Be concise without sacrificing clarity or useful detail. Use additional words when they make behavior easier to understand. Do not restate the question, add filler, give praise, or apologize.
- Use familiar technical terms when they help, but do not compress behavior into jargon or abstract labels that the reader must decode. Do not use phrases such as “load-bearing.”
- When describing behavior, name the component and explain what triggers it, what it does, and any relevant order, conditions, limits, or timeouts. State these details directly instead of making the reader infer them from a label.
- For example: “When the server receives SIGINT or SIGTERM, it stops accepting new HTTP requests and lets requests already being processed continue until they finish or the configured shutdown timeout expires.”
- Apply this standard to chat responses, project documentation, plans, and code comments. Assume technical familiarity where appropriate; explain the system's behavior rather than unnecessarily defining familiar terms such as SIGINT and SIGTERM.
- Avoid the term “gate”; describe checks, thresholds, and requirements in plain language.
- When explaining code, show the smallest useful snippet or example and cite the file and line numbers. Explain the result in plain English.
- In project docs and plans, use concrete outcomes and checks. For example: “An upload request over the configured 25 MiB limit returns 413.”
- Use references for claims about existing code. Keep examples and references current when the code changes.

## Start here

- Read `docs/architecture.md` and `docs/architecture/constraints.md` before changing architecture or product behavior. Keep the affected functional documentation aligned with the implementation.
- Use `README.md` for the repository overview and `docs/running.md` for local setup.

## Repository map

- `crates/api/`: Rust API (axum, Tokio, sqlx), migrations, and integration tests.
- `web/`: React and TypeScript app, API client, screens, map, and component tests.
- `design/`: shared runtime CSS. The app imports `tokens.css` and `components.css` through `web/src/styles/index.css`.
- `docs/`: architecture, current feature behavior, and operating instructions. `docs/documentation-inventory.md` indexes these references.
- `docs/architecture/`: implementation constraints and current route/data-flow diagrams.
- `.sqlx/`: committed offline query metadata. Regenerate it after changing checked SQL queries.

## Product and implementation constraints

- Published voyages are public; draft voyages are owner-only and return 404 to other callers. Serve only re-encoded image variants with metadata stripped, never original uploads. Apply a voyage's chosen granularity to coordinates in public responses; owner-only photo metadata intentionally returns exact coordinates. See D6 and D7.
- Authentication uses provider identities and server-side sessions. Do not add password storage or merge identities by email. See D1–D3.
- Keep database access async and move CPU-heavy image work and blocking file operations off Tokio worker threads. See D8.
- MapLibre currently uses OpenStreetMap raster tiles for development. Keep attribution visible and do not use that tile server for a release. See D4a and D4.
- A voyage's map groups pins in the browser; discovery groups them on the server after applying each voyage's granularity. See D10.
- Use the existing components and shared CSS tokens when changing UI. Document the resulting behavior in the relevant functional reference.

## Local commands

- Follow `docs/running.md` for native image dependencies, PostgreSQL setup, environment configuration, and API startup. The usual API command is `cargo run -p pinvoyager-api` after starting the database with `docker compose up -d`.
- In `web/`, run `npm install` if dependencies are missing, then `npm run dev`. Vite serves the app on port 5173 and proxies API and auth requests to port 8080.
- Rust checks: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` (the tests need PostgreSQL and the native tools listed in `docs/running.md`).
- Web checks from `web/`: `npm run typecheck`, `npm run lint`, `npm run test`, and `npm run build`.
- After changing sqlx checked queries, run `cargo sqlx prepare --workspace -- --all-targets` against a migrated database and commit the resulting `.sqlx/` changes.

## Working in this repository

- Preserve unrelated changes in the worktree. Check status before editing; do not overwrite another person's in-progress work.
- Keep API response shapes and `web/src/api/` types and fixtures in sync when changing endpoints.
- Report the behavior actually checked and any remaining failures or blockers. Do not claim completion solely because code is written.

## Feature delivery requirements

- Maintain **100% code coverage** for both frontend and backend using stable toolchains. Frontend coverage must be 100% for statements, lines, functions, and branches across production code. Backend coverage must be 100% for lines, functions, and regions. Rust branch percentages require nightly instrumentation and are not part of the measured threshold; instead, write tests for both outcomes of each meaningful conditional. Do not exclude code from coverage just to satisfy thresholds; document any technically unavoidable instrumentation exclusion in `docs/` and report it.
- Start every feature by implementing its tests. Run those tests before writing the feature implementation and verify that they fail for the expected missing behavior. Then implement the feature and rerun the tests to verify they pass. Include the red and green results in the handoff.
- Fully document every feature in the appropriate files under `docs/`. Cover its behavior, relevant decisions or constraints, and how it fits the system. Include code references using repository paths and symbol names so readers can find the implementation and tests. Update existing documentation when it already covers the feature; add a document when needed.
- Do not mark a feature complete unless its tests pass, both frontend and backend remain at the required coverage, and its documentation is complete. Report coverage commands and results, or identify the specific tooling or environment blocker when coverage could not be measured.
