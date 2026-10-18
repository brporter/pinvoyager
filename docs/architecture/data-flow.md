# Request and application data flow

```mermaid
flowchart LR
    Screen[React screen] --> Hook[React Query hook]
    Hook --> Client[Typed fetch or XHR client]
    Client --> Router[axum router and middleware]
    Router --> Handler[Validation and authorization]
    Handler --> Repo[Async SQL repository]
    Repo --> DB[(PostgreSQL)]
    Handler --> Response[Presentation response]
    Response --> Cache[React Query cache]
    Cache --> Screen
```

[`api/client.ts`](../../web/src/api/client.ts) sends JSON requests and throws
`ApiRequestError` for API failures. Uploads use XHR for transfer progress.
[`api/types.ts`](../../web/src/api/types.ts) contains handwritten response types;
backend-produced fixtures and frontend contract tests check representative shapes.
`client.ts::parseErrorText` handles error JSON for both fetch and XHR; fetch awaits
body reading before calling that parser. There is no OpenAPI generation step.

[`api/hooks.ts`](../../web/src/api/hooks.ts) owns query keys and ordinary mutations.
Voyage reads include photo presentation data. Detail and list/profile queries poll
every second while their returned data reports pending photos. Worker completion
does not push directly to the browser. Successful logout clears cached account data.
A failed logout remains visible in the account menu with a retry action; it does
not navigate away. See the [authentication reference](auth-flow.md).

Voyage mutations update cached detail data and invalidate affected lists, discovery, and
public-profile queries. Upload uses the shared creation hook, keeps the draft ID in the
URL, and retains per-file retry keys in its batch hook. Files and progress remain in
browser memory. Reloading resumes the draft, but the user must select files again. The
query client and individual hooks share `retryOnlyTransient`.

`web/src/screens/Upload.tsx::Upload` remounts `UploadFlow` when navigation selects
another voyage or starts a new creation flow. Unmounting aborts active uploads and
prevents queued files from starting; details, progress, destination, and trace context
reset together. A late draft-creation response cannot redirect an abandoned flow.
The server may already have saved that draft. Recording a draft created by the current
flow updates the URL while preserving the flow and its creation/upload trace.
`web/src/screens/__tests__/Upload.test.tsx` checks destination changes and late replies;
`VoyageUploadFlow.test.tsx` checks the trace handoff and upload/review lifecycle.

On the server, [`http::router`](../../crates/api/src/http/mod.rs) composes routes and
middleware for request IDs, tracing, timeouts, and body limits. Handlers enforce
ownership/publication and construct responses. SQL operations live in `db/repo`, with
transaction-specific queries also in handlers. After reserving queue capacity, the
upload handler writes the original file and commits the photo row. The filesystem write
and database commit are separate operations. See the [image
pipeline](../photo-pipeline.md).

Exact photo coordinates are stored in the database. Owner photo metadata exposes them;
public voyage responses round coordinates to the chosen granularity. Named-profile
queries filter the owner and published state before pagination. See
[architecture](../architecture.md) for the route table and data model.
