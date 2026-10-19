# Legal policy drafts

PinVoyager provides public `/terms` and `/privacy` pages and links to both from
`SiteFooter` in the ordinary and authentication layouts. `App` registers these
routes outside `SessionRoutes` and `AuthProvider`, so the documents do not need
an API response, sign-in, or display-name setup. On mobile, the ordinary footer
precedes the bottom navigation in document flow; it is not a fixed overlay.
The legal pages use `AuthShell` with a centered, readable text column and shared
CSS tokens. Long pages and the footer scroll normally.

## Policy contents and provenance

`web/src/screens/Terms.tsx::Terms` adapts the user-content sections of the
[App.net terms template](https://github.com/appdotnet/template-terms-of-service/blob/master/terms_template.md).
It covers account eligibility, ownership and a service-limited hosting license,
public sharing, prohibited material, removal at any time and for any reason,
removal of objectionable content without warning at the operator's sole
discretion, account suspension, referrals of content deemed illegal to the
appropriate authorities, complaints, hosted-content responsibility, service
availability, and applicable-law limits on disclaimers. Referral language does
not claim that the site continuously monitors uploads or automatically reports
them. Mandatory notices and review rights remain subject to applicable law.

`web/src/screens/Privacy.tsx::Privacy` adapts the
[App.net privacy template](https://github.com/appdotnet/template-terms-of-service/blob/master/privacy_template.md)
to the service's actual data flows: provider subjects, profile information,
sessions, originals and re-encoded variants, exact EXIF metadata, public
precision choices, moderation access, cookies, direct tile requests, operational
diagnostics, deletion queues, shared storage, backups, and privacy requests.
The text distinguishes stripping served images from retaining original files
and extracted metadata. It does not promise immediate erasure or a self-service
account-deletion control.

Both drafts identify Bryan Porter, personally, in Missouri, United States of
America, as operator and link `inquiries@bryanporter.com` for contact. The terms
specify Missouri and applicable United States federal law, subject to mandatory
consumer protections. Accounts and uploads are intended for people aged 18 or
older; this is a proposed policy restriction, not an implemented age-verification
flow.

`web/src/screens/LegalPage.tsx::LegalPage` provides attribution, links to the
source templates and [CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/),
and identifies the documents as modified versions. The adapted policy text is
available under that license; this does not license user photos, logos, or the
rest of the application. The source templates are not a representation that
these drafts satisfy every applicable law.

## Current implementation references

- `crates/api/src/auth/sign_in.rs::sign_in` and
  `crates/api/src/db/repo/identities.rs::create`: provider identity/account storage.
- `crates/api/src/auth/session.rs::lookup_and_touch`: server-side sessions.
- `crates/api/src/http/photos.rs` and `crates/api/src/photos/`: owner metadata,
  upload storage, metadata-free variants, and background cleanup. See
  [image pipeline](photo-pipeline.md) for the current lifecycle.
- `crates/api/src/voyages/granularity.rs::Granularity::apply`: public precision.
- `crates/api/src/http/admin.rs`: account moderation and draft metadata access.
  See [site administration](site-administration.md).
- `web/src/map/MapCanvas.tsx`: direct map tile requests. See
  [tile requests](osm-tile-policy.md).

## Details required before the policies take effect

The pages are explicitly **drafts for review**, dated October 18, 2026, with no
effective date yet. Confirm the following before treating them as final:

- Log, trace, and backup retention periods and the actual hosting locations,
  infrastructure providers, and international transfer safeguards. These are
  marked in the privacy draft because repository code cannot establish them.
- The effective date and legal review of the operator's jurisdiction, audience,
  complaint process, consumer protections, privacy rights, and content-reporting
  obligations. Confirm any required postal address or copyright agent details.
- A working process to receive complaints and privacy/deletion requests at the
  supplied mailbox, preserve evidence lawfully, and refer suspected illegal
  content to the appropriate authorities. Publishing terms does not implement
  that operational process or register a DMCA agent.

No automatic reporting, continuous moderation, policy acceptance recording,
age verification, or new data collection is implemented by this change.
No production deployment is part of this work.

## Verification

`web/src/screens/__tests__/Legal.test.tsx` checks public routes with a failing
session API, draft status, key removal/referral and data-handling disclosures,
source attribution, and navigation between documents. The absence of any API
call on these routes also establishes that accounts awaiting display-name
setup cannot block the documents. `web/src/shell/__tests__/Shells.test.tsx`
checks both links in both layouts at mobile and desktop widths.

Before implementation, the focused command failed seven new cases because the
routes, disclosures, and footers were missing; the two existing shell cases
passed. After implementation all nine focused cases passed:

```sh
npm --prefix web run test -- src/screens/__tests__/Legal.test.tsx src/shell/__tests__/Shells.test.tsx
```

See [testing](testing.md) for full coverage commands and thresholds.

Final local checks on October 18, 2026:

| Command | Result |
| --- | --- |
| `npm --prefix web run typecheck` | Passed. |
| `npm --prefix web run lint` | Passed without warnings after fixing test array keys. |
| `npm --prefix web run format:check` | Passed. |
| `npm --prefix web run build` | Passed; Vite still warns about the large map bundle. |
| `npm --prefix web run test -- --coverage` | 564 tests passed; statements 99.93%, branches 99.71%, functions 100%, lines 100%. |
| `SQLX_OFFLINE=true DATABASE_URL=postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager cargo coverage -- --test-threads=1` | 487 passed, one opt-in performance test ignored; lines 98.85%, functions 98.62%, regions 95.67%. |
| `git diff --check` | Passed. |

The first backend attempt failed because sandbox restrictions denied database
connections and local test sockets. Repeating the same coverage command with
local network permission passed. No backend source changed. The legal layout
and footer-to-privacy navigation were also checked in Chrome at desktop width;
mobile shell links were checked by automated tests, not a mobile browser.
Actual garment printing was not checked.
