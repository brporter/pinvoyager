# Lens and push pin experiment

An independent icon study on `design/icon`: overlapping mountain silhouettes
span the icon behind a stylized camera lens. A round-headed map pin marks a
visited place inside the lens, and five small stars give the sky definition.

The rounded, overlapping peaks take inspiration from the mountains in
[Apple’s Image Playground icon](https://developer.apple.com/assets/elements/icons/image-playground/image-playground-256x256_2x.png),
viewed on 2026-10-28. These are newly drawn vector paths using solid fills. The taller distant mountain sits on the right;
the nearer mountain sits on the left. Apple’s bubble, gradients, and material effects are not part of this study.
The nighttime crescent is a separately drawn vector path.

This color study uses light lavender for the distant mountain and deeper
purple for the nearer mountain, preserving their difference in brightness.
The lens uses black and warm greys. The cream sky, orange pin head, and neutral
colors come from `design/tokens.css`; the two purples are local artwork colors
and do not add runtime UI tokens.

| Element | Color |
| --- | --- |
| Sky | Cream `#ffedd5` |
| Background mountain | Light lavender `#bca7d4` |
| Foreground mountain and stars | Muted purple `#8b6ba8` |
| Lens barrel | Near-black `#1b1817` |
| Inner lens rim | Grey `#6e6963` |
| Upper-left lens reflection | Light grey `#bcb8b3` |
| Lower-right lens reflection | Off-white `#fafaf9` |
| Pin shaft | Dark grey `#57524d` |
| Pin head | Orange `#d95c09` |

Open the [daytime/nighttime comparison](appearances.svg), the
[daytime icon](light/icon.svg), or the [nighttime icon](dark/icon.svg).
The earlier cream-sky purple study remains in [icon.svg](icon.svg), with its
[layer review sheet](preview.svg).
The existing Horizon Pin SVG/PNG, earlier icon proposals, web assets, and native
app icon remain unchanged. This experiment is not imported by either app.

## Light and dark scene variations

These versions depict different lighting in the scene. The daytime version
uses green mountains and a sun with eight short rays. The nighttime version
uses purple mountains, stars, and a crescent moon. Both preserve the same mountain paths, lens geometry, map pin
placement, and shared 1024 × 1024 canvas. Distant mountains stay lighter than
nearer mountains in both palettes.

| Element | Light / daytime | Dark / nighttime |
| --- | --- | --- |
| Sky | Pale blue `#deedf6` | Deep navy `#182039` |
| Sky detail | Amber sun and rays `#eeb447`; no stars | Pale stars `#eee9fa`, cream crescent `#fff1c9`; no sun |
| Background mountain | Light sage `#a5b9b3` | Muted lavender `#a38dbf` |
| Foreground mountain | Deeper green `#617f68` | Deeper purple `#6f588c` |
| Lens barrel / inner rim | Near-black `#1b1817` / grey `#6e6963` | Graphite `#6e707c` / pale grey `#adb0bc` |
| Upper / lower reflections | Light grey `#bcb8b3` / off-white `#fafaf9` | Cool grey `#b9bcc7` / off-white `#f5f4f2` |
| Pin shaft / head | Grey `#57524d` / orange `#d95c09` | Silver-grey `#e1dfdc` / orange `#f27318` |

The lighter nighttime lens rim and shaft keep those shapes visible against
the dark sky and purple terrain. The sun has a 26-unit disc at `(628, 264)` and eight short rounded rays,
reaching 50 units from its center including stroke caps. The crescent is one
filled path translated to `(644, 268)`, with a transparent opening formed by
two arcs. Neither uses masks or sky-colored patches. Both celestial symbols
are clear of the pin head and inner lens rim. One nighttime star moves to
`(512, 244)` to leave room for the moon. Both versions use solid colors without
gradients or baked lighting effects.

The daytime version has six independent vector layers; nighttime has seven.
Stack the following rows from back to front, skipping the empty daytime moon
row:

| Layer | Light / daytime | Dark / nighttime |
| --- | --- | --- |
| Sky | [Sky](light/layers/sky.svg) | [Sky](dark/layers/sky.svg) |
| Sun or stars | [Sun and rays](light/layers/sun.svg) | [Stars](dark/layers/stars.svg) |
| Moon | — | [Crescent moon](dark/layers/moon.svg) |
| Background mountain | [Background mountain](light/layers/background-mountain.svg) | [Background mountain](dark/layers/background-mountain.svg) |
| Foreground mountain | [Foreground mountain](light/layers/foreground-mountain.svg) | [Foreground mountain](dark/layers/foreground-mountain.svg) |
| Lens | [Lens](light/layers/lens.svg) | [Lens](dark/layers/lens.svg) |
| Push pin | [Push pin](light/layers/push-pin.svg) | [Push pin](dark/layers/push-pin.svg) |

Use the complete layer set from the same appearance together. Their sky layers
are full-bleed and opaque; all other layers have transparent backgrounds. Each
appearance’s `icon.svg` embeds its source groups. These studies do not add
runtime theme switching or modify the native app’s existing icon package.

## Original study’s separate SVG layers

Stack these files in this order, back to front:

| File | Group | Content |
| --- | --- | --- |
| [sky.svg](layers/sky.svg) | `sky` | Solid warm cream background filling the full canvas. |
| [stars.svg](layers/stars.svg) | `stars` | Five small four-point purple stars on a transparent canvas. |
| [background-mountain.svg](layers/background-mountain.svg) | `background-mountain` | Light lavender distant mountain, with the taller rounded peak on the right. |
| [foreground-mountain.svg](layers/foreground-mountain.svg) | `foreground-mountain` | Deeper purple nearer mountain, with the rounded peak on the left. |
| [lens.svg](layers/lens.svg) | `lens` | Bold outer barrel, thin inner rim, and two curved glass reflections. |
| [push-pin.svg](layers/push-pin.svg) | `push-pin` | Round orange head and pointed grey shaft, entirely inside the lens opening. |

All six layers have a `0 0 1024 1024` viewBox and 1024 × 1024 dimensions.
Import them at the same origin and size; do not crop individual files to their
visible bounds. Only the sky layer is opaque across the full canvas. The
stars, both mountains, lens, and push pin have transparent backgrounds. Each file opens
on its own without another file.

Both lens circles are centered at `(512, 512)`. The outer barrel has a radius
of 380 and a 44-unit stroke, for an outer diameter of 804 units. The inner rim
has a radius of 334 and a 12-unit stroke, leaving an opening radius of 328.
Two rounded arcs near radius 300 suggest glass reflections. The upper-left arc
is light grey; the shorter lower-right arc is off-white.

Both mountain paths extend from x=0 to x=1024 behind the lens without clipping.
Their peaks use quadratic curves. Stack the lighter background mountain below
the darker foreground mountain. Each mountain is one path in its own SVG, so
their depth and materials can be adjusted independently. The split preserves
the original path coordinates and placement; this color study updates only
the fills and strokes. Neither mountain has
snowcaps, contour lines, textures, gradients, or shadows. The stars use five filled paths,
spaced away from the lens barrel, reflections, and pin head. They remain a
separate layer so their appearance and placement can be adjusted independently.

The `push-pin` group translates to `(528, 472)` and rotates 24 degrees. Its
orange head is a circle with radius 88; its grey shaft narrows to a point in
the mountain land area. Both shapes fit inside the lens opening.

The assembled icon has fourteen visible shapes: one sky rectangle, five stars,
two mountain paths, two lens circles, two reflection arcs, and two pin shapes.
The source layers and assembled icon are square and unmasked. Only the review
sheet applies rounded corners.

`icon.svg` contains the same groups as the source layers in stacking order,
without external references. Keep its corresponding groups aligned when
editing the layer files. `preview.svg` embeds copies for the assembled,
separate-layer, and small-size studies; update those copies after revising
the artwork. The sheet uses system fonts for labels; the icon has no fonts.

## Sources for later Icon Composer work

All artwork in this experiment is vector SVG. The layer files contain only
rectangles, circles, paths, groups, and accessible names. The lens retains
editable vector strokes. There are no embedded images, raster effects, masks,
filters, remote resources, or fonts in the icon sources.

Use `layers/sky.svg` as the full-bleed background. Above it, stack
`layers/stars.svg`, `layers/background-mountain.svg`,
`layers/foreground-mountain.svg`, `layers/lens.svg`, and
`layers/push-pin.svg` as separate foreground sources. Keep their shared 1024 × 1024 placement. The
source layers have no rounded enclosure or baked shadows. Use `icon.svg` as an
assembled reference; do not stack it together with its source layers. Raster
renders used for visual inspection are kept outside this source folder.
These experiment layers have not been imported or checked in Icon Composer
by the agent.

## Verification and limits

All twenty-four SVG files were parsed as XML and checked for accessible titles,
unique IDs, resolved local references, and the absence of raster images,
scripts, gradients, filters, and external dependencies. The assembled groups
were compared with the separate sources. Layer order, fourteen visible shapes,
two mountain files, five stars, and the pin bounds were checked. The color
study was compared with the previous assembled icon: geometry, stroke widths,
transforms, shape order, and placement remain unchanged. Only fill and stroke
colors changed. The background mountain remains lighter than the foreground.

The original review sheet was rendered at 1600 × 1120 and the appearance
comparison at 1600 × 1160 in headless Chrome. Both were visually inspected,
including 96, 60, 40, and 29 pixel samples. The appearance sources were checked
for six daytime layers, seven nighttime layers, eight sun rays, a separate
crescent path, no stars in daytime, no sun at night, a darker night sky,
and a lighter distant mountain in both scenes. Geometry and placement of the
five shared layers match the original; the sun rays and moon are clear of
the lens rim and pin head. The nighttime stars are clear of the moon. The two mountains,
lens, and orange pin remain distinguishable in the small samples. Stars, sun rays, the thin inner rim, reflections, and needle lose definition
at the smallest size.
These are static browser previews. Native materials, platform masking, and
device appearances have not been checked. No application code changed.
Application tests and frontend/backend coverage were not run for this artwork
experiment.
