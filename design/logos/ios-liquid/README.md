# PinVoyager iOS icon proposal

Recommended direction: the original Horizon Pin in pale frosted glass over a
deep teal background. A narrow light edge and a soft shadow suggest material
and depth while keeping the pin, mountains, and sun readable.

Open the [comparison sheet](proposal.png) or its [editable SVG](proposal.svg).
The [default preview](default-preview.png), [dark preview](dark-preview.png),
and [clear study](clear-preview.png) are 1024 × 1024 PNGs with SVG counterparts.
These are static proposals; the clear study illustrates neutral material rather
than reproducing the system's wallpaper-dependent clear appearance. The app's
current icon has not been replaced.

## What stays the same

The source is [`design/logos/horizon-pin.svg`](../horizon-pin.svg), also named as
the native icon source in `docs/ios-app-design.md`, “Layout and interaction rules.”
The pin and mountain path data, circle position/radius, stroke widths, line caps,
and joins are copied directly. There is no new symbol, filled pin interior,
wordmark inside the icon, or orange accent. The negative spaces remain open.

All foreground elements share `translate(16 16) scale(.75)` in the original
128-unit coordinate space. This centers the original canvas in a 96-unit square
inside the 128-unit icon canvas without changing the relative positions of its
elements. The exported vector layers use 1024 × 1024 dimensions.

The background uses the existing teal primitives from `design/tokens.css`:
`#14988b`, `#0c615b`, and `#0c3f3c`. Foreground shading uses pale teal and white
to suggest frosted glass. The dark study subdues the same colors; the clear
study reduces saturation. Every appearance uses identical foreground geometry.

## Native authoring

Apple recommends separate vector artwork, full-bleed backgrounds, and
system-rendered material effects. See [App icons in the Human Interface
Guidelines](https://developer.apple.com/design/human-interface-guidelines/app-icons)
and [Icon Composer](https://developer.apple.com/icon-composer/).

1. Use `layers/horizon-pin.svg` as the combined foreground source. It contains
   the entire original mark with its placement applied. The individual
   `layers/pin.svg`, `layers/mountains.svg`, and `layers/sun.svg` are optional
   alternatives for separate material adjustments; do not also import the
   combined mark. These editable sources retain the original strokes. Outline
   those strokes in a vector editor before production import, as Apple advises.
2. Create a full-bleed teal gradient in Icon Composer. The unmasked,
   effect-free `layers/background.svg` records the proposed gradient if an
   imported background is preferred.
3. Keep the pin, mountains, and sun together at the same depth so the landscape
   remains aligned with the outline. Apply the glass treatment to the combined
   mark or its group. Start with high foreground opacity and restrained
   refraction; keep the small gaps and sun legible.
4. Let Icon Composer supply highlights, shadows, refraction, and the system's
   enclosure mask. The preview SVGs intentionally simulate these effects and
   corners for review; use the effect-free, unmasked sources in `layers/` for
   native authoring.
5. Review default, dark, clear light/dark, and tinted light/dark appearances on
   light, dark, and detailed wallpapers. Check at 60, 40, and 29 points on the
   target device scales before integrating the resulting `.icon` file.

## Verification and limits

The SVG XML was parsed, and the three elements in `layers/horizon-pin.svg` were
compared with the master: geometry and attributes match, with `currentColor`
changed to white. All nine SVG files are vector-only and contain no external
images or fonts. The sheet uses system typography for annotations.

The comparison sheet was rendered in headless Chrome at 1600 × 1080. Individual
1024 × 1024 previews were rendered with macOS Quick Look. The comparison sheet
and the default preview were visually inspected; the sheet includes default
samples at 60, 40, and 29 pixels. These pixel samples are visual checks rather
than device-scale validation. The clear study has lower contrast and needs
native appearance review before use.

No `.icon` package was authored, and native material rendering, device
appearance, motion, and wallpaper interactions have not been checked. No
application code or runtime assets changed. Application tests and coverage were
not run for this artwork proposal.
