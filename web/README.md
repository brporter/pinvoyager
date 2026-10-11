# PinVoyager web application

React and TypeScript application built with Vite. React Router handles navigation,
React Query caches API data, and MapLibre renders the maps. Shared styling comes
from `../design/tokens.css` and `../design/components.css`.

Run from this directory:

```sh
npm install
npm run dev
```

Vite serves port 5173 and proxies API/auth requests to the Rust API on port 8080.
See [running locally](../docs/running.md) for database, native image libraries,
authentication, and backend setup.

```sh
npm run format:check
npm run typecheck
npm run lint
npm run test -- --coverage
npm run build
```

See [architecture](../docs/architecture.md), [navigation](../docs/architecture/navigation.md),
and [testing](../docs/testing.md) for current behavior and test scope.
