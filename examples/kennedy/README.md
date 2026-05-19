# Kennedy Family Example

## Source and attribution

`kennedy.ged` is copied from the [findmypast/gedcom-samples](https://github.com/findmypast/gedcom-samples)
repository. That repository has no license file; this copy is included for educational and
demonstration purposes only. All genealogical data is historical and publicly known.

## Scope

70 individuals, 19 families, 5 generations from patriarch Patrick Kennedy (b. 1823, Ireland)
through the grandchildren of Joseph Kennedy Sr.

Notable IDs used in the examples:

| ID  | Person                  |
|-----|-------------------------|
| I46 | Patrick Kennedy (1823)  |
| I1  | Joseph Kennedy Sr.      |
| I0  | John Fitzgerald Kennedy |
| I21 | Robert Francis Kennedy  |
| I39 | Edward Moore Kennedy    |

## Auto-loaded preferences

`kennedy.toml` has the same basename as `kennedy.ged`, so genechart loads it automatically.
It sets the default root (Joseph Sr.), generation depth, title, copyright, fonts, and
highlights file. `generate.sh` overrides individual settings via `--pref` and other CLI flags,
demonstrating that TOML defaults and CLI overrides work together.

## Output files

### `kennedy_simple.txt` — plain text, 5 generations from patriarch

```
genechart kennedy.ged --root I46 --gen 5 --text
```

Full descendant tree starting from Patrick Kennedy. Shows the text layout with the title set
via `--pref 'output.text.title = "The Kennedy Family"'`.

### `kennedy_boxed_couples.svg` — boxed couples, 4 generations from Joseph Sr.

```
genechart kennedy.ged --root I1 --gen 4 --type boxed_couples --svg
```

Each couple is rendered in a paired box with Unicode box-drawing borders. Spouse names and
life dates appear side by side; children branch downward. Highlighted individuals (JFK, RFK,
Ted Kennedy) appear in a distinct colour.

### `kennedy_fan.svg` — ancestor fan, 4 generations from JFK

```
genechart kennedy.ged --root I0 --dir ancestors --type fan --gen 4 --svg
```

Half-circle pedigree fan: JFK at the centre, parents in the inner ring, grandparents in the
next, and great-grandparents at the outer rim. Direction is `ancestors`, so the chart traces
lineage upward rather than descendants downward.

### `kennedy_fancy.svg` — fancy descendants, 3 generations from Joseph Sr.

```
genechart kennedy.ged --root I1 --gen 3 --type fancy --svg
```

Cascading descendants layout (SVG/PDF only). Each generation is offset diagonally, giving a
distinctive staircase appearance. Highlighted individuals (JFK, RFK, Ted) are visually
distinct from their siblings.

### `kennedy_boxes_photos.svg` — boxes layout with photos, 3 generations from Joseph Sr.

```
genechart kennedy.ged --preff kennedy_boxes_photos.toml --svg
```

One box per individual, root at the top, descendants growing downward. Each box whose
individual has a photo shows it centered above the name and dates. Settings are in
`kennedy_boxes_photos.toml` (boxes layout, `root_pos = "top"`, embedded photos, crop scale,
80×80 canvas units).

Six Wikipedia portraits are included in `photos/` and mapped to individual IDs via
`photos.index`:

| File | Individual |
|------|-----------|
| `Joseph_P._Kennedy,_Sr._1938_(cropped).jpg` | I1 — Joseph Kennedy Sr. |
| `Rose_Kennedy_1967.JPG` | I2 — Rose Fitzgerald Kennedy |
| `John_F._Kennedy,_White_House_color_photo_portrait.jpeg` | I0 — John Fitzgerald Kennedy |
| `Mrs_Kennedy_in_the_Diplomatic_Reception_Room_cropped.jpeg` | I52 — Jacqueline Bouvier Kennedy |
| `Robert_F._Kennedy.jpg` | I21 — Robert Francis Kennedy |
| `Ethel_Kennedy_Smile_1968_(cropped).jpg` | I22 — Ethel Skakel Kennedy |

## Highlights

`kennedy_highlights.txt` marks three brothers for visual emphasis:

```
I0   John Fitzgerald Kennedy
I21  Robert Francis Kennedy
I39  Edward Moore Kennedy
```

In SVG outputs these individuals are rendered with a highlight colour. In the plain-text
output (`kennedy_simple.txt`) highlights are not rendered visually but the file is still
loaded (demonstrating that the feature does not break text mode).
