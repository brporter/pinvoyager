# Remaining Apple sign-in work

## Apple credential lifecycle (review finding 4)

- [ ] Implement Apple authorization-state and revocation handling. The iOS app
  currently creates an independent PinVoyager session after provider verification;
  it does not check `ASAuthorizationAppleIDProvider.getCredentialState`, observe
  `credentialRevokedNotification`, or process Apple server notifications.
- [ ] Decide how to securely retain and validate Apple refresh tokens. Current
  constraint D1 prohibits long-lived provider token storage; change that constraint,
  storage/security documentation, and privacy disclosures together if retention is
  adopted. Keep tokens encrypted, restrict access, and remove them on account deletion.
- [ ] Validate signed Apple server notifications, including issuer, audience, expiry,
  and replay protection. Define what happens to Apple identities and PinVoyager
  sessions after `consent-revoked` and `account-deleted`, including accounts with
  other linked providers. Do not infer that email forwarding changes delete accounts.
- [ ] Test credential states, duplicate/forged notifications, Apple Account deletion,
  session invalidation, offline recovery, and accounts with multiple providers.
  Permanent Apple Account deletion does not produce the native revocation notification;
  credential-state checks must cover that case.

References: `ios/PinVoyager/Features/AppleAuthorization.swift::SystemAppleAuthorization`,
`crates/api/src/auth/providers/oidc.rs::OidcCore::exchange_with_revocation`,
`crates/api/src/auth/session.rs::lookup_and_touch`, and
[`docs/architecture/constraints.md`](docs/architecture/constraints.md) D1.
Apple guidance: [TN3194](https://developer.apple.com/documentation/technotes/tn3194-handling-account-deletions-and-revoking-tokens-for-sign-in-with-apple)
and [processing account changes](https://developer.apple.com/documentation/signinwithapple/processing-changes-for-sign-in-with-apple-accounts).

## Account deletion without fresh Apple authorization (review finding 5)

- [ ] Allow a verified owner to delete their PinVoyager account when a linked Apple
  account cannot complete fresh authorization. Currently `DeleteAccountView.providers`
  offers only Apple when Apple is linked, and the API requires a matching Apple
  revocation proof before preparing deletion.
- [ ] Separate proof of PinVoyager account ownership from Apple token revocation.
  Preserve same-account verification, purpose-bound proofs, explicit confirmation,
  durable deletion recovery, and protections against deleting another user's account.
- [ ] Use securely retained Apple tokens for revocation if the lifecycle work above
  adopts storage. If no usable token or authorization code exists, follow Apple's
  documented fallback: fulfill deletion and explain how to revoke Apple access manually.
  Do not make obtaining fresh Apple credentials a permanent prerequisite for deletion.
- [ ] Test loss/deletion of the Apple Account, another linked provider proving ownership,
  missing tokens, revocation failures, canceled confirmation, retries, and local cleanup.

References: `ios/PinVoyager/Features/AccountView.swift::DeleteAccountView`,
`crates/api/src/http/account_deletions.rs::prepare`,
`crates/api/src/auth/providers/apple.rs::AppleProvider::exchange_native_apple`, and
[`docs/native-api.md`](docs/native-api.md#account-deletion-and-recovery).
Apple guidance: [account deletion](https://developer.apple.com/help/app-review/guideline-reference/5-1-1-account-deletion)
and [TN3194's missing-token fallback](https://developer.apple.com/documentation/technotes/tn3194-handling-account-deletions-and-revoking-tokens-for-sign-in-with-apple).

These items remain deferred; native Apple authorization and branded provider buttons
alone do not implement them.
