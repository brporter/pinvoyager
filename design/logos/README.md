# PinVoyager logo concepts

The current iOS and website icon is the lens-and-push-pin artwork exported from
`PinVoyager.icon` into `PinVoyager Exports/`. The native `AppIcon.appiconset` and
`BrandIcon.imageset` in `ios/PinVoyager/DesignSystem/Assets.xcassets` contain the
default and dark exports. Keep those copies aligned when replacing artwork.
The website uses small PNG derivatives of `BrandIcon.imageset` for its header,
favicons, and iPhone home-screen icon. Regenerate them from the repository root
on macOS after changing the native artwork:

```sh
sips -z 64 64 'ios/PinVoyager/DesignSystem/Assets.xcassets/BrandIcon.imageset/PinVoyager-iOS-Default-1024@1x.png' --out web/public/brand/pinvoyager-light.png
sips -z 64 64 'ios/PinVoyager/DesignSystem/Assets.xcassets/BrandIcon.imageset/PinVoyager-iOS-Dark-1024@1x.png' --out web/public/brand/pinvoyager-dark.png
sips -z 180 180 'ios/PinVoyager/DesignSystem/Assets.xcassets/BrandIcon.imageset/PinVoyager-iOS-Default-1024@1x.png' --out web/public/apple-touch-icon.png
```

The following studies preserve the earlier design exploration.

The [lens and push pin experiment](lens-pin/README.md) explores six separate
SVG layers: sky, stars, background mountain, foreground mountain, camera lens,
and a visible orange push pin. The [daytime/nighttime comparison](lens-pin/appearances.svg)
adds a green daytime scene with a rayed sun and a purple nighttime scene with
stars and a crescent moon. Daytime has six independent vector source layers;
nighttime has seven. The experiment includes
an assembled icon and a review sheet without replacing any existing artwork.

The [iOS Liquid Glass icon proposal](ios-liquid/README.md) applies a new material
treatment to the original Horizon Pin geometry, with default, dark, and clear
studies and separate vector source layers.

Open [the comparison sheet](concepts.svg) to compare three original, single-color
SVG concepts ([PNG preview](concepts-preview.png)), including 24 px samples and white-on-teal versions. These are
earlier proposals; the lens-and-push-pin artwork now supplies the site mark.

| Concept | Idea | Best use |
| --- | --- | --- |
| [Horizon pin](horizon-pin.svg) | A location pin containing mountains and a sun: photographs tied to places. | Earlier website mark. |
| [Voyager compass](voyager-compass.svg) | An open compass ring around a bold V-shaped direction marker. | Travel apparel and a broader exploration identity. |
| [Waypoint P](waypoint-p.svg) | A continuous route draws a P, with a solid origin and a ring waypoint. | Compact app icons, sleeve prints, and an understated monogram. |

All three masters have a 128 × 128 viewBox, transparent backgrounds, and one ink
color (`#0c615b`, the existing deep teal). The marks contain paths/circles rather
than fonts, filters, embedded bitmaps, or external dependencies. They can render
in one solid color on the web or in print. Change the root `color` to recolor a
standalone SVG; inline SVG can inherit the surrounding CSS color. The comparison
sheet's wordmark uses system typography for illustration, not final outlined
lettering.

For a T-shirt, start with a mark about 75 mm wide, with white ink on a dark shirt
or deep teal/black on a light shirt. Ask the printer to verify the smallest
negative spaces at the chosen size and expand strokes to outlines if their
process requires it. Small embroidery needs its own simplified proof. These are
visual concepts, not trademark-clearance results.

The comparison sheet was rendered with macOS Quick Look and visually inspected.
SVG structure was checked for a viewBox, accessible names, and no raster content.
Actual garment printing and embroidery have not been proofed.
