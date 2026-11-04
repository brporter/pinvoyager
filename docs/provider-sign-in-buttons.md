# Provider sign-in buttons

The iOS sign-in and provider-linking sheet uses one layout for Apple, Google, and
Microsoft. `ios/PinVoyager/DesignSystem/ProviderSignInButton.swift::ProviderSignInButton`
renders every provider with **Sign in with [provider]**, a white background, an
outline, and aligned logo and title columns. `SignInView` in
`ios/PinVoyager/Features/AccountView.swift` orders Apple, Google, and Microsoft
independently of the server's provider order. It displays only configured providers.

Changing the button appearance does not change authentication:
`AuthenticationPresentation` still sends Apple through `SystemAppleAuthorization`
and Google/Microsoft through `BrowserAuthentication`. Busy, offline, and server
capability checks continue to disable the buttons.

## Guidelines and differences

Reviewed November 3, 2026 against the primary provider references:

| Aspect | Apple | Google | Microsoft | Decision |
| --- | --- | --- | --- | --- |
| Wording | Allows Sign in with Apple, Sign up with Apple, or Continue with Apple. | Allows Sign in with Google, Sign up with Google, or Continue with Google. | Specifies Sign in with Microsoft; permits the shorter Sign in when space is limited. | Use Sign in with for all three. Continue with is not a common option in the published guidance. |
| Alignment | Explicitly permits custom buttons to align logos with other providers. | The iOS diagram specifies 16-point leading padding and a 12-point logo/title gap. | Its recommended diagram also places the logo before the title with a 12-pixel gap. | Use a leading logo column and aligned title starts. |
| Artwork/color | Requires supplied Apple artwork; logo and title must be black or white. | Requires the standard full-color Google G. | Requires an unchanged Microsoft logo; provides the full-color symbol. | Preserve brand-specific logo colors; use the providers' light text colors on white. Identical monochrome logos would violate Google's guidance. |
| Typography | Prefers the system font and specifies native button proportions: 19-point title at 44-point height. | Specifies Google Sans Medium. The text and older iOS illustration differ on the font family; use the current written Google Sans instruction. | Its reference diagram recommends Segoe UI, weight 600, 15 pixels for a 41-pixel button. | Keep Apple at system 19-point medium, Google at Google Sans 17-point medium, and Microsoft at native system 17-point semibold. Segoe UI is a reference recommendation; it is not bundled with the iOS app. Do not claim identical typography or a pixel-for-pixel copy of every reference. |
| Insets/border | Supplied logo padding must remain intact; custom bezel/corner radius are allowed. The title needs at least 8% of the button width clear to its trailing edge. | The iOS diagram uses 16-point side insets; its light outline is #747775. | Its reference recommends 12-pixel side insets and a #8C8C8C light outline. | Adopt Google's 16-point insets and outline for the common shell. This adapts Microsoft's recommended metrics; its logo is unchanged. |

There is no conflict that prevents common wording, alignment, button dimensions,
spacing, or a white outlined design. Exact brand colors and typography differ;
Microsoft's reference insets also differ from Google's iOS diagram. The shared
layout retains required brand details and explicitly adapts those reference metrics.

Sources:

- [Apple Human Interface Guidelines: Sign in with Apple](https://developer.apple.com/design/human-interface-guidelines/sign-in-with-apple#Creating-a-custom-Sign-in-with-Apple-button)
- [Apple Design Resources](https://developer.apple.com/design/resources/)
- [Google Identity branding guidelines](https://developers.google.com/identity/branding-guidelines)
- [Google iOS spacing illustration](https://developers.google.com/static/identity/images/branding_guide_ios_spec.png)
- [Microsoft identity platform branding guidelines](https://learn.microsoft.com/en-us/entra/identity-platform/howto-add-branding-in-apps)
- [Microsoft recommended dimensions and typography](https://learn.microsoft.com/en-us/entra/identity-platform/media/howto-add-branding-in-apps/sign-in-with-microsoft-redlines.png)

## Rendering and artwork

All three buttons have equal available width, 44-point height, 8-point corner
radius, and a one-point #747775 border. `SignInView` spaces them 12 points apart.
The common content layout uses 16-point side insets, a 20-point logo column, and a
12-point gap before the title. The fill remains white in light and dark appearance.
Apple's title is black, Google's is #1F1F1F, and Microsoft's is #5E5E5E. Titles are
single-line and scale down when necessary; VoiceOver receives the full title.
The Apple title has more than the required 8% trailing margin at supported iPhone
and iPad widths. Logos are decorative to accessibility, so their brand names are
not announced twice.

`AppleSignIn.imageset/apple.svg` is the unchanged **Left-aligned / Black / Medium**
SVG from Apple's Logo-Sign-in-with-Apple artwork bundle, obtained after the project
owner accepted Apple's artwork license. Its complete 31×44 canvas is rendered at
31×44 points, centered in the shared logo column. Its supplied padding is not cropped
or altered; no vertical padding is added. Apple supplies multiple logo sizes so the
visible glyph can match other providers without stretching it. The accompanying
license is retained in `ios/PinVoyager/Resources/ProviderArtwork/Apple-Artwork-License.rtf`.

Google and Microsoft artwork and the Google Sans license are described in
[iOS setup](ios-running.md#native-apple-sign-in-setup). No SF Symbol, text character,
or third-party substitute is used for Apple's logo. Apple's native authorization
sheet remains system-provided; only the app's entry button is customized.

## Checks

`BrandAssetTests::testProviderLogosLoadAsFullColorOriginalArtwork` verifies all three
bundled logos and the Google font. `AccountFlowTests::testAccountNameMethodsBlockingExportAndHelp`
checks common wording, equal button dimensions, aligned title starts, 12-point row
gaps, and dispatch of the Apple action. The fixture deliberately supplies providers
in reverse order. The test retains a screenshot for visual review.

The new checks first failed because the Apple asset and Sign in with titles were
missing. They pass after the shared button implementation. The focused run also
includes `AuthenticationPresentationTests`, which verifies native Apple dispatch.

November 3 verification: the focused artwork/authentication/UI run passed, then
`DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer ios/Scripts/test.sh`
passed 322 iPhone tests and the iPad guest-navigation test without retries.
Production Swift coverage was 19,093 / 19,977 executable lines (95.57%), with no
new exclusions. Results are retained in
`ios/TestResults/20261103-125519.xcresult` and
`ios/TestResults/20261103-125519-ipad.xcresult`; the summary is
`ios/TestResults/20261103-125519-coverage-swift.json`.
Touched Swift formatting and `git diff --check` passed. The retained dark-appearance
screenshot was visually checked; it is also copied to
`ios/TestResults/unified-sign-in-buttons-dark.png`.

Backend and web production code did not change in this button revision. Their
preceding coverage measurements remain 98.68% lines / 98.92% functions / 95.12%
regions for stable Rust, and 99.10% statements / 98.88% branches / 98.32% functions /
99.34% lines for the web app. Those suites were not rerun for this iOS-only revision.
