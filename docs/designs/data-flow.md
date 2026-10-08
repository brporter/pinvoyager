# Data flow

## The layers

```mermaid
graph TD
    subgraph Browser
        Screens["Screens<br/>Discover, VoyageDetail, Profile,<br/>Upload, VoyageEdit, Settings"]
        Hooks["api/hooks.ts<br/>TanStack Query — cache, retry, invalidation"]
        Client["api/client.ts<br/>fetch wrapper + XHR upload"]
        Cache[("Query cache<br/>keyed by keys.*")]
    end

    subgraph Service["axum service"]
        MW["http::middleware<br/>request id, timeout, tracing"]
        Auth["http::auth<br/>/auth/*, /api/me"]
        Photos["http::photos<br/>/api/voyages/{id}/photos,<br/>/api/photos/*"]
        Voyages["http::voyages<br/>/api/voyages/*, /api/map"]
        Gran["voyages::granularity<br/>Coordinate — private fields,<br/>one constructor"]
        Worker["photos::worker<br/>bounded queue + semaphore"]
    end

    DB[("PostgreSQL<br/>users, identities, sessions,<br/>auth_attempts, photos, voyages")]
    Disk[("Storage root<br/>originals/ and variants/,<br/>content-addressed")]

    Screens --> Hooks
    Hooks <--> Cache
    Hooks --> Client
    Client -->|"same-origin, cookies always sent"| MW
    MW --> Auth
    MW --> Photos
    MW --> Voyages
    Auth --> DB
    Photos --> DB
    Photos --> Disk
    Voyages --> DB
    Voyages --> Gran
    Photos -->|enqueue| Worker
    Worker --> DB
    Worker --> Disk
```

In development the Vite dev server proxies `/api` and `/auth` to the service, so
both halves share one origin and the session cookie works. That is why `BASE` in
`web/src/api/client.ts:146` is the empty string.

## One read, end to end

```mermaid
sequenceDiagram
    autonumber
    participant Screen as VoyageDetail
    participant Hook as useVoyage(id)
    participant Cache as Query cache
    participant Client as getVoyage
    participant Router as axum
    participant Repo as db::repo::voyages
    participant PG as Postgres

    Screen->>Hook: render with :id from the URL
    Hook->>Cache: read keys.voyages.detail(id)
    alt fresh
        Cache-->>Screen: cached voyage, no request
    else stale or missing
        Hook->>Client: getVoyage(id, { signal })
        Client->>Router: GET /api/voyages/{id}<br/>credentials: same-origin
        Router->>Router: MaybeUser — a session if there is one
        Router->>Repo: visible(pool, user, id)
        Repo->>PG: SELECT voyage
        PG-->>Repo: row
        Repo-->>Router: published, or owned by this user
        Router->>Router: place() every photo through<br/>Granularity::apply
        Router-->>Client: 200 JSON + x-request-id
        Client-->>Hook: typed Voyage
        Hook->>Cache: store under keys.voyages.detail(id)
        Cache-->>Screen: voyage
    end
```

Every response carries `x-request-id`, a UUID v7, set by the outermost layer so
even a 404 gets one. The client keeps it on every error object, because for a
5xx the body says nothing useful on purpose and the real cause is findable in
the server log only by that header.

## Errors as they reach a screen

```mermaid
graph TD
    R["fetch"] -->|"never produced a response"| NE["NetworkError<br/>offline, DNS, reset, abort"]
    R -->|"2xx"| OK["typed response from api/types.ts"]
    R -->|"401"| U["UnauthorizedError<br/>subclass — 'sign in', not 'broken'"]
    R -->|"other non-2xx"| AE["ApiRequestError<br/>status, code, message,<br/>requestId, fieldErrors, retryAfterSeconds"]

    AE -->|"422"| F["fieldErrors[] → shown beside the input"]
    AE -->|"429, or 409 variants_pending"| RA["retryAfterSeconds from Retry-After"]
    AE -->|"5xx"| Ref["error screen: quote the reference id"]
```

`code` is typed as `string`, not a union, because the handler-local `Rejection`
types add codes beyond the seven `AppError` produces — `validation_failed`,
`too_large`, `not_an_image`, `unprocessable_image`, `rate_limited`,
`variants_pending`. A client that threw on an unknown code would turn a new
server-side rejection into a blank screen.

Retry policy (`web/src/api/hooks.ts:115`): anything below 500 is not retried. A
404 does not become a 200 on the third attempt, and retrying costs the user the
latency of every attempt before the designed empty state appears.

## Cache keys and what invalidates what

```mermaid
graph LR
    subgraph Keys["keys — prefix-sharing tuples"]
        me["me"]
        vall["voyages"]
        vlist["voyages · list · params"]
        vdet["voyages · detail · id"]
        vmap["voyages · map · id"]
        dmap["discover-map · west,south,east,north,zoom"]
        pall["photos"]
        plist["photos · list · params"]
        pdet["photos · detail · id"]
    end

    vall --- vlist
    vall --- vdet
    vall --- vmap
    pall --- plist
    pall --- pdet

    subgraph Mutations
        create["useCreateVoyage"]
        update["useUpdateVoyage"]
        del["useDeleteVoyage"]
        detach["useDetachPhoto"]
        reorder["useReorderPhotos"]
        up["useUploadPhotos"]
        delp["useDeletePhoto"]
        out["useSignOut"]
    end

    create --> vall
    update --> vall
    del --> vall
    detach --> vall
    reorder --> vall
    up --> pall
    up --> vdet
    up --> vmap
    up --> vlist
    delp --> pall
    out --> me
```

Keys are tuples rather than strings so that
`invalidateQueries({ queryKey: keys.voyages.all })` reaches the feed, every
detail read and every map at once. A flat `"voyage-detail-<id>"` scheme cannot
express that, and the invalidation lists then drift from the routes.

`discoverMap` sits deliberately outside `voyages`, even though it draws
voyages. Publishing one voyage invalidates `['voyages']`, and every rectangle of
the world map a user had panned across would be thrown away with it — while the
answer they are looking at changed by at most one marker.

## The endpoint surface

```mermaid
graph LR
    subgraph Public["No session required"]
        P1["GET /health"]
        P2["GET /ready"]
        P3["GET /api/voyages/{id}"]
        P4["GET /api/voyages/{id}/map"]
        P5["GET /api/map"]
        P6["GET /api/photos/{id}/{variant}"]
        P7["GET /auth/{provider}/start"]
        P8["GET and POST /auth/{provider}/callback"]
    end

    subgraph Session["CurrentUser extractor — no anonymous path"]
        S1["GET /api/me"]
        S2["POST /auth/logout"]
        S3["POST /auth/link/{provider}"]
        S4["GET /api/voyages — your own"]
        S5["POST /api/voyages"]
        S6["PATCH and DELETE /api/voyages/{id}"]
        S7["POST /api/voyages/{id}/photos"]
        S8["PATCH /api/voyages/{id}/photos/order"]
        S9["DELETE /api/voyages/{id}/photos/{photo}"]
        S10["POST /api/voyages/{id}/photos — voyage upload"]
        S11["GET /api/voyages/{id}/photos — owner metadata"]
        S12["GET and DELETE /api/photos/{id}"]
    end

    subgraph Dev["debug_assertions only"]
        D1["POST /auth/dev/login"]
    end
```

`GET /api/photos/{id}/{variant}` takes an optional session: the owner sees their
own drafts, everyone else sees it only if it belongs to a published voyage. A
photo that is neither answers 404, not 403 — telling a caller that a photo
exists but is not theirs is an enumeration oracle.

`/auth/dev/login` is compiled out of a release binary. `debug_assertions` is off
in the release profile and the container builds `--release`, so the route is
absent rather than disabled, and the handler additionally refuses unless
`auth.dev_login` is set.

## Shutdown

```mermaid
sequenceDiagram
    participant OS
    participant Serve as http::serve
    participant Axum as axum::serve
    participant Clock as drain_timeout

    OS->>Serve: SIGTERM or SIGINT
    Serve->>Axum: begin graceful shutdown
    Serve->>Clock: oneshot fires — the drain clock starts now
    par
        Axum-->>Serve: every in-flight request finished
    and
        Clock-->>Serve: shutdown_timeout_secs elapsed
    end
    Note over Serve: whichever wins. On timeout the server<br/>future is dropped, closing remaining<br/>connections, and a warning is logged.
```

The clock starts at the signal, never at startup — `drain_timeout` waits on the
oneshot first. Without that, a long-running server would shoot itself after
`shutdown_timeout_secs` of uptime. If the sender is dropped without signaling
(the server stopped for its own reasons) the clock never resolves, so it cannot
race and swallow the server's own result.
