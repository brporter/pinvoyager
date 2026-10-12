# Production deployment

Pinvoyager runs on `shell.betaporter.dev` at `https://pinvoyager.com`.
The shared Caddy service terminates TLS. `www.pinvoyager.com` redirects to the
canonical domain while preserving the request path and query.

## Images and updates

`.github/workflows/deploy.yml` runs on pushes to `main` and manual dispatches.
It runs the reusable `checks.yml` workflow (including the owner-authorized 95%
coverage thresholds) before publishing two Linux amd64
images to GHCR, each tagged with the commit SHA and `latest`:

- `ghcr.io/brporter/pinvoyager-api`: the release Rust API from `Dockerfile`,
  running as uid 10001 with the native HEIC decoder and WebP encoder.
- `ghcr.io/brporter/pinvoyager-web`: the frontend built by
  `deploy/Dockerfile.web`, served by nginx as its unprivileged `nginx` user.

Set the GitHub repository variable `VITE_TILE_URL` to the production raster tile
URL template before building. The frontend image refuses to build without it.
The owner authorized `https://tile.openstreetmap.org/{z}/{x}/{y}.png` for the
initial deployment on 2026-10-11 as an exception to the development-only rule.
The browser fetches tiles directly; preserve the visible attribution required
by the chosen provider. The current UI displays OpenStreetMap attribution.
See [tile behavior](osm-tile-policy.md) and constraint D4.

Both GHCR packages must be public so the existing VM Watchtower can pull them
anonymously. GitHub Actions pushes with its automatic `GITHUB_TOKEN`; no SSH or
registry secret is required by the workflow. Watchtower notices new `latest`
images within approximately five minutes and recreates the labeled application
containers. PostgreSQL has no Watchtower label. API and frontend updates happen
independently; endpoint changes must remain compatible during the transition.

## VM configuration

`deploy/vm/docker-compose.yml` defines the stack under `/opt/pinvoyager`.
The external Docker network `web` connects the two application containers to
shared Caddy under aliases `pinvoyager-api` and `pinvoyager-web`. PostgreSQL is
on the stack's private default network. No Pinvoyager port is published on the
host. Named volumes `pinvoyager_pgdata` and `pinvoyager_uploads` persist database
state and image files across container updates.

Create `/opt/pinvoyager/.env` with permissions 600, using
`deploy/vm/.env.example` as the setting reference. Generate a URL-safe database
password using `openssl rand -hex 32`; never commit it. The API receives optional
provider credentials from this file. Register callback URLs such as
`https://pinvoyager.com/auth/google/callback` in the provider console. Without
provider credentials the public site starts, but users cannot sign in. Secure
session cookies are enabled and development login is disabled; the latter is
also absent from the release binary.

Copy the production Compose file to `/opt/pinvoyager/docker-compose.yml`, then:

```sh
cd /opt/pinvoyager
sudo docker compose pull
sudo docker compose up -d --wait
sudo docker compose ps
```

Append the sites from `deploy/vm/Caddyfile` to `/opt/shared/Caddyfile`. Validate
before reloading the running proxy:

```sh
sudo docker exec shared-caddy-1 caddy validate --config /etc/caddy/Caddyfile
sudo docker exec shared-caddy-1 caddy reload --config /etc/caddy/Caddyfile
```

Caddy forwards `/api`, `/api/*`, `/auth`, `/auth/*`, `/health`, and `/ready` to
the API without stripping their prefixes. `/auth/callback` is the frontend
sign-in completion screen and goes to nginx before the API matcher. Other paths
also go to nginx. nginx serves
SPA deep links through `index.html`, prevents caching that document, and caches
hashed `/assets/` files for one year. Missing assets return 404.

`main.rs::run_service` applies embedded migrations at API startup. The API
healthcheck probes `/health`; `/ready` separately checks PostgreSQL. On stop,
the API allows 30 seconds for HTTP requests and a separate 30 seconds for image
workers; Compose allows 75 seconds before terminating the process.

## Verification and rollback

Run `python3 deploy/smoke.py` from a machine with public network access. It
checks valid HTTPS, the frontend, SPA deep links, public voyage JSON, health,
database readiness, and the canonical `www` redirect. The initial test before
deployment fails because the domain has no configured TLS site.

Inspect logs with `sudo docker compose logs --tail=100 api frontend db` from
`/opt/pinvoyager`. To roll back an image, replace its `latest` tag in Compose
with a previously published commit SHA and run `sudo docker compose up -d`.
Remove its Watchtower label while pinning the image. Database migrations are
not automatically reversed: verify schema compatibility before rollback.

Back up PostgreSQL and the uploads volume together. The deployment does not
configure scheduled backups. Never run `docker compose down -v` in production;
it deletes both persistent volumes.

## Configure sign-in providers

The requested providers are Google, Microsoft, and Apple. Credentials stay in
`/opt/pinvoyager/.env`; they are not GitHub repository secrets and are not baked
into images. After adding complete provider settings, run
`sudo docker compose up -d api` from `/opt/pinvoyager` and check API startup logs.
A provider whose discovery fails is disabled independently. Verify each
`/auth/{provider}/start` redirects to the provider, then complete a real sign-in
in a browser. Credentials alone do not prove that the callback succeeds.

### Google

In Google Cloud's Google Auth Platform, create/select a project, configure
branding and audience, and create an OAuth client of type **Web application**.
Register `https://pinvoyager.com/auth/google/callback` as an authorized redirect
URI. For an external app still in testing, add your account as a test user;
publish the consent configuration when ready for general access. Save the
client ID and secret in the VM environment:

```dotenv
PINVOYAGER__AUTH__PROVIDERS__GOOGLE__CLIENT_ID=your-client-id
PINVOYAGER__AUTH__PROVIDERS__GOOGLE__CLIENT_SECRET=your-client-secret
PINVOYAGER__AUTH__PROVIDERS__GOOGLE__ISSUER=https://accounts.google.com
```

Source: [Google OpenID Connect setup](https://developers.google.com/identity/openid-connect/openid-connect).

### Microsoft

In Microsoft Entra's App registrations, create a registration and add a **Web**
redirect URI: `https://pinvoyager.com/auth/microsoft/callback`. Under Certificates
& secrets, create a client secret and save its **value**, not its ID. Save the
Application (client) ID and Directory (tenant) ID.

The current `auth/providers/oidc.rs::OidcCore::discover` requires the discovered
issuer to equal the configured issuer exactly. Use a specific tenant's issuer:

```dotenv
PINVOYAGER__AUTH__PROVIDERS__MICROSOFT__CLIENT_ID=your-application-id
PINVOYAGER__AUTH__PROVIDERS__MICROSOFT__CLIENT_SECRET=your-secret-value
PINVOYAGER__AUTH__PROVIDERS__MICROSOFT__ISSUER=https://login.microsoftonline.com/your-tenant-id/v2.0
```

This configures sign-in for that tenant. Do not use `common` or `organizations`
with the existing exact-issuer implementation; broad organizational/personal
Microsoft account support needs additional provider implementation and tests.
Choose the tenant's supported account type in the registration for the initial
configuration.

Source: [Microsoft web redirect configuration](https://learn.microsoft.com/en-us/entra/identity-platform/how-to-add-redirect-uri).

### Apple

In Apple Developer Certificates, Identifiers & Profiles, enable Sign in with
Apple on a primary App ID. Create a Services ID for the website, enable Sign in
with Apple, and associate it with that primary App ID. Register domain
`pinvoyager.com` and return URL `https://pinvoyager.com/auth/apple/callback`.
Create a Sign in with Apple private key associated with the primary App ID,
download the `.p8` file, and record its Key ID and your Team ID.

```dotenv
PINVOYAGER__AUTH__PROVIDERS__APPLE__CLIENT_ID=your-services-id
PINVOYAGER__AUTH__PROVIDERS__APPLE__TEAM_ID=your-team-id
PINVOYAGER__AUTH__PROVIDERS__APPLE__KEY_ID=your-key-id
PINVOYAGER__AUTH__PROVIDERS__APPLE__PRIVATE_KEY_PEM='-----BEGIN PRIVATE KEY-----
your-key-contents
-----END PRIVATE KEY-----'
```

Compose supports a multiline single-quoted value in `.env`. The API's
`auth/providers/apple.rs::AppleProvider` signs its own expiring client secrets
from this key; do not create a static Apple client-secret value.

Sources: [Apple web configuration](https://developer.apple.com/help/account/capabilities/configure-sign-in-with-apple-for-the-web),
[Apple private key creation](https://developer.apple.com/help/account/capabilities/create-a-sign-in-with-apple-private-key).
