# genechart

A command-line tool that reads a [GEDCOM 5.5.1](https://gedcom.io/) genealogical file and generates a family-tree chart as text, SVG, or PDF.

**Version**: v0.7.0

## Installation

```sh
# Install from source
cargo install --path .

# Or build a release binary in target/release/
cargo build --release
```

Requires a stable Rust toolchain (Rust 2024 edition).

## Usage

```
genechart [OPTIONS] [GEDCOM_FILE]
```

### Options

| Flag | Description |
|---|---|
| `-r` / `--root <ID>` | Root individual ID (default: first individual in file) |
| `-g <N>` / `--generations <N>` / `--gen <N>` | Number of generations to show |
| `--dir <DIRECTION>` | Chart direction: `descendants`, `ancestors` = `pedigree`, `forest` |
| `--type <TYPE>` | Layout algorithm: `simple`, `fan`, `boxed_couples`, `fancy`, `boxes` |
| `--text` | Output as plain text |
| `--svg` | Output as SVG |
| `--pdf` | Output as PDF |
| `-o` / `--output <FILE>` | Output file (extension infers type if no `--text`/`--svg`/`--pdf` flag) |
| `--merge <GEDCOM> <ALIAS>` | Merge a further GEDCOM file using an alias file (repeatable; order matters) |
| `--pref '<key name="val">'` | Override any preference inline (TOML syntax, repeatable) |
| `--pref` | Bare `--pref` (no value): dump merged preferences to stdout and exit |
| `--prpref` | Print the fully-resolved preferences as TOML and exit; combine with `-o` to see type inference |
| `--preff <FILE>` | Load an explicit TOML preferences file (repeatable; files apply in order, a later file overriding an earlier one) |
| `--trace [COMPONENT]` | Print structured diagnostics to stderr; bare `--trace` traces all |
| `-h` / `--help` | Show help |
| `--version` | Show version |

### Examples

```sh
# Generate a 4-generation descendant chart as SVG
genechart family.ged -r I1 -g 4 --svg -o chart.svg

# Generate a pedigree fan chart as PDF (type inferred from extension)
genechart family.ged -r I1 --type fan -o chart.pdf

# Ancestor chart, plain text, 3 generations
genechart family.ged -r I1 --dir ancestors -g 3 --text

# Boxed couples layout as SVG
genechart family.ged -r I1 --type boxed_couples -o chart.svg

# Dump merged preferences (useful for debugging)
genechart family.ged --pref

# Print resolved preferences for SVG output (no chart generated)
genechart family.ged -r I1 --prpref -o chart.svg

# Trace how preferences are resolved across all sources
genechart family.ged -r I1 -g 5 --trace prefs 2>&1 | head

# Use a shared preferences file for a project-specific style
genechart family.ged --preff ~/projects/genealogy/style.toml

# Merge a second GEDCOM (paternal ancestry) into the main file
genechart maternal.ged -r I1 --merge paternal.ged mat_to_pat_aliases.txt --dir ancestors -o chart.svg
```

## Layout Types

### simple

Text-like layout with indented generations. Suitable for terminal output or simple SVG/PDF charts. Supports descendants, ancestors, and forest directions.

In **forest** mode every individual is in scope and each disconnected family tree is rendered as an independent sub-chart below the previous one (largest tree first). Sub-trees whose children were already shown in a prior tree are replaced by `...` to avoid repetition. With `show.last_gen_spouses = true`, redundant spouse-only trees (the couple already appeared in a larger tree and has no new children) are suppressed entirely.

```sh
genechart family.ged -r I1 --type simple -o chart.svg
genechart family.ged --dir forest --pref 'show.last_gen_spouses = true' --text
```

Configuration: `[layout.simple]` — `indent` (columns per generation), `vert_spacing` (lines between generations).

When `show.notes = true`, GEDCOM `NOTE` text is rendered as additional indented rows below each individual (simple layout only; descendants, ancestors, and forest directions). When `show.notes_html = true`, HTML anchor tags inside notes (`<a href="…">text</a>`) are rendered as clickable hyperlinks in SVG and PDF output; other HTML tags have their content shown as plain text.

### boxed_couples

Recursive box-placement algorithm. Each individual or couple gets a box; children are placed below their parents with envelope-based spacing to avoid overlaps. Supports descendants and ancestors directions.

```sh
genechart family.ged -r I1 --type boxed_couples -o chart.svg
```

Configuration: `[layout.boxed_couples]` — `box_width`, `box_height`, `gap_width`, `gap_height`, `box_width_2_spouses`.

**Realistic tree branches (experimental):** when `layout.root_pos = "bottom"` (the default), you can replace the straight connector lines with organic-looking tree branches via `output.style.realistic_tree.enabled = true`. Four rendering styles are available:

| Style | Description |
|---|---|
| `"tapered"` (default) | Filled closed Bézier paths; branch width decreases globally from root to tips |
| `"stroke"` | Layered stroked S-curve Bézier paths with opacity-based taper |
| `"filter"` | Thick rounded paths with a white highlight for a cylindrical 3D look |
| `"ink"` | Hand-drawn coherent tree, modelled on a hand-drawn reference: a flared trunk that shows below the root box, organic buttress roots, a continuous flat-topped open-ellipse leaf canopy, and each child connected by a single continuous tapered branch that runs along the main limb and turns up under its box. Trunk and limbs carry short white bark scratches with a lit side |

```sh
# Tapered style with medium leaf density
genechart family.ged -r I1 --type boxed_couples \
  --pref 'output.style.realistic_tree.enabled = true' \
  --pref 'output.style.realistic_tree.style = "tapered"' \
  -o chart.svg

# ink style — hand-drawn tree look
genechart family.ged -r I1 --type boxed_couples \
  --pref 'output.style.realistic_tree.enabled = true' \
  --pref 'output.style.realistic_tree.style = "ink"' \
  -o chart.svg
```

Configuration: `[output.style.realistic_tree]` — `enabled` (bool), `style` (`"tapered"` | `"stroke"` | `"filter"` | `"ink"`), `trunk_color` (hex), `leaf_color` (hex), `leaf_density` (`"none"` | `"low"` | `"medium"` | `"high"`).

### fan

Half-circle pedigree fan (180°). Places ancestors in concentric rings, with the root at the center. Ancestors-only.

```sh
genechart family.ged -r I1 --type fan -o chart.svg
```

Configuration: `[layout.fan]` — `ring_height` (radial thickness of each inner ring), `ring_gap` (gap between rings), `outer_ring_height` (radial thickness of outer radial-text rings), `radial_gen` (generation index at which to switch to radial text; 0 = all radial).

### fancy

Cascading layout (SVG/PDF only). Supports both descendants and ancestors directions. In descendants mode, each direct descendant is left-aligned by generation at a fixed horizontal distance, with spouses listed below each individual and children branching to the right via curved connectors. In ancestors mode, the root is at the left and ancestors grow rightward, one column per generation.

```sh
genechart family.ged -r I1 --type fancy -o chart.svg
genechart family.ged -r I1 --type fancy --dir ancestors -o chart.svg
```

Configuration: `[layout.fancy]` — `gen_width` (horizontal distance between successive generations; ignored when `compact = true`), `child_gap` (vertical gap between a person's last spouse and their first child), `anc_gap` (vertical breathing room around each individual in ancestors mode), `compact` (default `true`: children are stacked below the spouse column rather than placed `gen_width` to the right, producing a much narrower chart).

### boxes

One individual per box (SVG/PDF only). No marriage data is shown. Supports both descendants and ancestors directions. Consanguineous individuals are placed at every position they appear; a double border is drawn when `show.duplicated_individual = true`.

**Ancestors direction:** the root is at one edge and parents grow outward one box per generation. Father boxes are placed to the left, mother boxes to the right. A horizontal connector bar links each individual to their parents.

**Descendants direction:** the root is at one edge and each individual has their own box. Spouses are placed to the right of the individual, slightly lower (`couple_y_offset`). Children of each spouse are placed below that spouse, centered under it.

```sh
genechart family.ged -r I1 --type boxes --dir ancestors -o chart.svg
genechart family.ged -r I1 --type boxes -o chart.svg
```

Configuration: `[layout.boxes]` — `box_width`, `box_height`, `gap_width`, `gap_height`, `couple_y_offset` (vertical offset between individual and spouse box tops, descendants only).

**Photos:** the `boxes` layout optionally displays a photo at the top of each box, above the name. Enable with `show.photo = true` and configure the `[photos]` section. Photos are not supported in other layout types.

## Output Formats

### Text

Plain text with column-aligned names, dates, and dot leaders. Default output format.

```sh
genechart family.ged -r I1 --text
```

### SVG

Vector graphics output. Supports boxes, connectors, and configurable fonts. Every SVG element carries a `class=` attribute so the chart can be restyled with a `<style>` block or external CSS without editing the source TOML:

| Element | Class(es) |
|---|---|
| Individual/couple boxes (`<rect>`) | `box`; two-spouse boxes also get `double` |
| Connectors (`<line>`, `<path>`) | `connector` |
| Fan wedges (`<path>`) | `wedge` |
| Row-rule underlines (`<line>`) | `row_rule` |
| Note bars (`<line>`) | `note_bar` |
| Note hyperlinks (`<a>`) | `note_link` |
| Highlight backgrounds (`<rect>`) | `highlight_rect` |
| Photo images (`<image>`) | `photo` |
| Realistic tree layer (`<g>`) | `realistic-tree` |
| Realistic tree branch paths/lines | `tree-branch` |
| Realistic tree leaf shapes | `tree-leaf` |
| Text elements | `indi_name`, `spouse_name`, `indi_birth`, `indi_death`, `indi_marriage`, `indi_id`, `gen_num`, `note_text`; highlighted text also adds `highlighted` |

```sh
genechart family.ged -r I1 --svg -o chart.svg
```

### PDF

Generated via SVG conversion. Supports poster tiling (multi-page output for large charts).

```sh
genechart family.ged -r I1 --pdf -o chart.pdf
```

Poster tiling: configure `[output.poster]` — `rows`, `columns`, `overlap_mm`, `alignment_lines`.

Generate a 2x2 poster from a sample GEDCOM file:

```sh
# Tiled 2x2 poster via command-line overrides
genechart family.ged -r I1 --type boxed_couples --pdf -o chart.pdf \
  --pref 'output.poster.rows = 2' \
  --pref 'output.poster.columns = 2' \
  --pref 'output.poster.overlap_mm = 5.0'

# Or via a preferences file
genechart family.ged -r I1 --type boxed_couples --pdf -o chart.pdf \
  --preff poster_style.toml
```

where `poster_style.toml` contains:

```toml
[output.poster]
rows = 2
columns = 2
overlap_mm = 5.0
alignment_lines = true
```

## Preferences File

Preferences are read from (lowest to highest priority):

1. Installation-directory defaults (`defaults.toml`)
2. User home (`~/.genechart.toml`)
3. Directory TOML (`genechart.toml` in the same directory as the GEDCOM file)
4. File TOML (same basename as the GEDCOM file, e.g. `family.toml` for `family.ged`)
5. `--preff <FILE>` — explicit preferences file(s); may be given more than once and are applied in command-line order, so a later `--preff` overrides conflicting preferences from an earlier one
6. `--pref` command-line overrides

Unknown preference keys in `--pref` are a hard error (the command aborts with a message naming the bad key). Unknown keys in TOML config files produce a warning and are ignored.

### Full TOML Example

```toml
[files]
gedcom = "{gedcom}"
highlights = ""
merge = []          # further GEDCOM files to merge: ["paternal.ged", "maternal.ged", ...]
merge_aliases = []  # alias files, one per merge entry: ["aliases_pat.txt", "aliases_mat.txt", ...]

[scope]
root = ""
generations = 4
direction = "descendants"

[show]
generation_num = true
sex = true
birth = true
death = true
marriage = true
notes = false           # simple layout: display GEDCOM NOTE text below each individual
notes_html = false      # notes only: render <a href="..."> as clickable hyperlinks (SVG/PDF)
last_gen_spouses = false
id = false
duplicated_individual = false
photo = false

[photos]
directory = "photos"    # relative to GEDCOM file
index = ""              # "" = ID-based filenames (e.g. I1.jpg); non-empty = path to index file
embedded = false        # true: base64 data URI; false: relative path; PDF always embeds
width = 100.0           # canvas units
height = 100.0          # canvas units
margin = 2.0            # space on all four sides of the photo within the box
scale = "crop"          # "fit" | "crop" | "none"
box_resize = true       # grow box height by (height + 2*margin) to fit the photo
downsample = 72.0       # max DPI for embedded images; 0.0 = no downsampling

[format]
individual = "{firstname} {lastname} {sex}"
birth = "* {date:%d %b %Y}, {location}"
death = "× {date:%d %b %Y}, {location}"
marriage = "⚭ {date:%d %b %Y}, {location}"
date_qualifiers = "compact"  # "none" | "gedcom" | "compact"
no_name = ""                # Placeholder for individuals with no name in the GEDCOM file; empty = omit the name line
[layout]
type = "simple"
root_pos = "bottom"

[layout.simple]
indent = 3
vert_spacing = 0

[layout.boxed_couples]
box_width = 240.0
box_height = 140.0
spouse_sep_height = 30.0
gap_width = 40.0
gap_height = 80.0
box_width_2_spouses = 520.0

[layout.fan]
ring_height = 90
ring_gap = 10
outer_ring_height = 200
radial_gen = 3

[layout.fancy]
gen_width = 300.0      # ignored when compact = true
child_gap = 10.0
anc_gap = 10.0
compact = true         # stack children below spouse (narrower chart)

[layout.boxes]
box_width = 240.0
box_height = 80.0
gap_width = 40.0
gap_height = 80.0
couple_y_offset = 20.0

[output]
type = "text"
path = ""
noclobber = false

[output.paper]
size = "A4"           # "A0"–"A5", "letter", "custom"
orientation = "portrait"  # ignored when size = "custom"

[output.paper.custom]
width = 0.0   # mm — set both > 0 to activate
height = 0.0  # mm

[output.poster]
rows = 1
columns = 1
overlap_mm = 0.0
alignment_lines = true

[output.style]
dot_leaders = true

[output.style.spacing]
title = 12.0         # Vertical space (canvas units) between the title text and the chart
copyright = 12.0     # Vertical space (canvas units) between the copyright text and the chart

[output.style.fonts]
names = "Georgia 14"
dates = "Arial 10"
title = "Georgia 24"
copyright = "Arial 8"

[output.text]
title = "{gedcom}"
copyright = ""
```

Format strings use `{key}` placeholders: `{firstname}`, `{lastname}`, `{sex}`, `{date}`, `{location}`. See [Date Formatting](#date-formatting) below for date pattern syntax.

## Date Formatting

Dates (birth, death, marriage) have two independent controls:

### Date format pattern

Use `{date:FORMAT}` in a format template, where `FORMAT` is a strftime-like pattern:

| Code | Meaning | Example (`1 JAN 1812`) |
|------|---------|------------------------|
| `%d` | Day, zero-padded | `01` |
| `%e` | Day, no padding | `1` |
| `%m` | Month number, zero-padded | `01` |
| `%b` | Abbreviated month name | `Jan` |
| `%B` | Full month name | `January` |
| `%Y` | 4-digit year | `1812` |
| `%y` | 2-digit year | `12` |

Missing components (e.g. a GEDCOM date that has only a year) are silently omitted so `{date:%d %b %Y}` on `"1812"` produces `"1812"`, not `"  1812"`.

Plain `{date}` (without a colon) passes the raw GEDCOM string through — useful with `date_qualifiers = "gedcom"`.

### Date qualifiers (`format.date_qualifiers`)

Controls how GEDCOM date qualifier tokens (ABT, BEF, AFT, BET…AND, FROM…TO) are rendered:

| Value | Behaviour |
|-------|-----------|
| `"compact"` *(default)* | Translate to compact symbols (see table below) |
| `"none"` | Strip all qualifiers; for date ranges, show only the first date |
| `"gedcom"` | Pass through the raw GEDCOM string unchanged |

Compact symbol mapping:

| GEDCOM qualifier | Compact output |
|-----------------|----------------|
| `ABT`, `CAL`, `EST` | `~date` |
| `BEF` | `<date` |
| `AFT` | `>date` |
| `BET … AND …`, `FROM … TO …` | `date1-date2` |

**Corner case — same-range deduplication**: if both dates in a range format to the same string (e.g. `{date:%Y}` applied to `"BET APR 1880 AND JUL 1880"` yields `1880` for both), the qualifier is dropped and the date is shown once.

### Examples

```toml
[format]
# Default: full date, compact qualifiers
birth = "* {date:%d %b %Y}, {location}"
date_qualifiers = "compact"
```
| GEDCOM date | Output |
|-------------|--------|
| `1 JAN 1812` | `* 01 Jan 1812` |
| `ABT 1850` | `* ~1850` |
| `BEF 1900` | `* <1900` |
| `AFT 1800` | `* >1800` |
| `BET APR 1880 AND JUL 1890` | `* Apr 1880-Jul 1890` |
| `BET APR 1880 AND JUL 1880` | `* Apr 1880` *(deduplication)* |

```toml
# Year-only, no qualifiers
birth = "* {date:%Y}, {location}"
date_qualifiers = "none"
```
| GEDCOM date | Output |
|-------------|--------|
| `ABT 1850` | `* 1850` |
| `BET 1880 AND 1890` | `* 1880` *(first date only)* |

```toml
# Raw GEDCOM pass-through
birth = "* {date}, {location}"
date_qualifiers = "gedcom"
```
| GEDCOM date | Output |
|-------------|--------|
| `ABT 1850` | `* ABT 1850` |
| `BET APR 1880 AND JUL 1880` | `* BET APR 1880 AND JUL 1880` |
## Highlights File

The `files.highlights` preference points to a plain-text file that marks individuals for visual emphasis in the chart. Each line has the format:

```
ID [name...] [# comment]
```

- `ID` — the GEDCOM individual ID (without `@` delimiters), e.g. `I1`
- `name` — optional, for documentation purposes only
- `# comment` — optional, ignored by the parser

Example `highlights.txt`:

```
I1 John Smith # root ancestor
I5 Jane Doe # married 1843
I12 Paul Smith # emigrated 1900
```

Highlighted individuals are visually distinguished in SVG/PDF output. They are rendered in a different text color, configurable via `output.style.text.highlights.color`, and with a different background color, configurable via `output.style.text.highlights.background_color`. The text backend supports two fallback modes, controlled by `output.style.text.highlights.fallback`: when set to `"uppercase"`, the highlighted individual's name is capitalized; when set to any other value (e.g. `"->"`), that literal string is prepended to the left of the line (even before the ID column, if shown), and all content on that line is shifted right to make room.

## Multi-GEDCOM Merge

When genealogical data is split across several GEDCOM files (e.g. paternal and maternal ancestries maintained separately), genechart can merge them into a single chart.

### How it works

Each further GEDCOM is parsed separately and merged into the main GEDCOM's `Genrep` structure. An *alias file* identifies individuals that appear in both the main and a further GEDCOM (with different IDs in each), so they are treated as a single person. All other IDs from the further GEDCOM are disambiguated by inserting a prefix letter (`B` for the 2nd file, `C` for the 3rd, etc.) after the first character of the ID — e.g. `I45` → `IB45`.

When an aliased individual exists in both GEDCOMs, the main GEDCOM's record wins for all non-empty fields; gaps are filled from the further GEDCOM; family links (`FAMS`/`FAMC`) and notes are always unioned.

### Alias file format

```
# main_id  further_id  optional name (ignored)
I1  I99  John Smith
F3  F201
```

- First field: ID in the **main** GEDCOM (without `@`)
- Second field: ID in the **further** GEDCOM (without `@`)
- `#` comments and blank lines are ignored
- Family IDs (`F…`) may also be aliased

### Specifying further GEDCOMs

**CLI** (repeatable; order matters):

```sh
genechart main.ged --merge second.ged aliases2.txt --merge third.ged aliases3.txt
```

Paths are resolved relative to `main.ged`'s directory.

**Preferences** (`genechart.toml` or `--pref`):

```toml
[files]
merge         = ["second.ged", "third.ged"]
merge_aliases = ["aliases2.txt", "aliases3.txt"]
```

`merge_aliases` must have at least as many entries as `merge`. CLI `--merge` takes precedence over preference-based merge pairs when both are present.

Maximum 25 further GEDCOM files (prefix letters B–Z).

### Example

```sh
# Pedigree chart combining paternal and maternal ancestry files
genechart maternal.ged -r I1 \
  --merge paternal.ged mat_to_pat_aliases.txt \
  --dir ancestors --type fan -o chart.svg
```

## Photos

Photos are supported in the `boxes` layout only. Enable with `show.photo = true`.

### Photo discovery

Photos are resolved relative to the GEDCOM file's directory:

- **ID-based (default):** `<photos.directory>/<individual_id>.<ext>`, where `ext` is tried as `jpg`, `jpeg`, `png` (both cases). For example, individual `I1` maps to `photos/I1.jpg`.
- **Index file:** set `photos.index` to a path (relative to the GEDCOM directory) of a plain-text file. Each line has the format `ID filename [anything...]`; `#` starts a comment; blank lines are ignored.

  ```
  # Kennedy family photos
  I0  jfk.jpg
  I52 jackie.jpg      # married 1953
  I53 caroline.jpg
  ```

### Scaling

| `scale` value | Behaviour |
|---|---|
| `"crop"` *(default)* | Fill the box exactly, cropping the centre |
| `"fit"` | Scale to fit within the box, preserving aspect ratio |
| `"none"` | No scaling; use the original image dimensions |

### Embedding vs linking

| Setting | Behaviour |
|---|---|
| `embedded = false` *(default)* | The SVG `href` is a path relative to the SVG output file |
| `embedded = true` | The image is base64-encoded into the SVG as a data URI |
| PDF output | Always embeds regardless of `embedded` |

### Downsampling

`photos.downsample` caps the resolution of embedded images. The maximum pixel dimensions are computed as `width * downsample / 96` × `height * downsample / 96`. Set to `0.0` to disable.

### Box sizing

When `box_resize = true` (default), all boxes in the chart grow taller by `height + 2 * margin` to accommodate the photo. When `box_resize = false`, the photo is skipped for boxes where it would not fit alongside at least one line of text.

### Example

```sh
genechart family.ged -r I1 --type boxes \
  --pref 'show.photo = true' \
  --pref 'photos.directory = "photos"' \
  --pref 'photos.embedded = true' \
  --pref 'photos.scale = "crop"' \
  -o chart.svg
```

## Building

```sh
cargo build             # debug
cargo build --release   # release
cargo test              # run all tests

cargo build --features lua   # include the experimental Lua plugin system
cargo test  --features lua   # run the plugin tests too
```

## Lua Plugins (experimental)

genechart can run small **Lua scripts** that post-process each record as it is
parsed — for example to fold a non-standard GEDCOM tag into a name, or to
normalise place strings. This is **opt-in at build time** behind the `lua` Cargo
feature (it pulls in [`mlua`](https://crates.io/crates/mlua) with a vendored,
MIT-licensed Lua 5.4 that is compiled from source, so the build needs a C
compiler):

```sh
cargo build --release --features lua
```

A binary built **without** the feature will refuse to run if a plugin is
configured (rather than silently ignore it).

### Configuration

```toml
[plugins.parse]
indi = "scripts/nickname.lua"  # runs on_individual(ind) for every individual
fam  = ""                      # runs on_family(fam) for every family
all  = "scripts/usa.lua"       # runs on_individual AND on_family, BEFORE indi/fam
```

Each value is a path to a Lua script (relative to the current directory; empty =
disabled). For a given record the `all` script runs first, then the
type-specific (`indi`/`fam`) script, which sees any edits `all` made.

As a convenience, the **`--plugin-parse <FILE>`** command-line option is a
shorthand for `plugins.parse.all` (and overrides any `all` value from a
preferences file). It is handy for a single catch-all script; separate `indi`/
`fam` scripts are better grouped in a preferences file (see `--preff`):

```sh
genechart family.ged --plugin-parse scripts/usa.lua
```

### Writing a script

A script defines named callback functions. The `indi` script defines
`on_individual`, the `fam` script defines `on_family`, and the `all` script
defines both. Each callback receives a table describing the record and returns
either `nil` (no change) or a table of fields to change.

Record table (individual): `id`, `given`, `surname`, `sex`, `living`,
`alt_name`, `relig_name`, `notes` (array), `fams`/`famc` (arrays, read-only),
`birth`/`death` (`{date=, place=}` or nil), and `unparsed` — an array of
`{level, tag, value}` for every GEDCOM line the parser did not map to a field
(this is how you reach tags like `NICK`). Family table: `id`, `husband`, `wife`,
`children` (read-only), `marriage` (`{date=, place=}`), `relig_marr`, `notes`,
`unparsed`.

Returnable (changeable) fields are **text/scalar and event fields only**:
`given`, `surname`, `sex`, `living`, `alt_name`, `relig_name`, `notes`, and
`birth`/`death`/`marriage` (`{date=, place=}`, merged into the existing event).
Structural fields (`id`, `fams`, `famc`, `husband`, `wife`, `children`) are
read-only and ignored if returned (with a warning when `diagnostics.warnings`).

`print(...)` from a script writes to genechart's **stdout** — handy for progress
messages, but note it will interleave with text-chart output sent to stdout, so
prefer `-o file` for the chart when a script prints. A script that fails to
load/compile is a fatal error; a runtime error in a callback is reported and
that record is left unchanged.

### Example — append a nickname to the given name

`scripts/nickname.lua` (see `tests/fixtures/plugins/nickname.lua`):

```lua
local targets = { I1 = true }  -- limit to these individual ids

function on_individual(ind)
  if not targets[ind.id] or not ind.given then return end
  for _, u in ipairs(ind.unparsed) do
    if u.tag == "NICK" and u.value ~= "" then
      return { given = ind.given .. ' "' .. u.value .. '"' }
    end
  end
end
```

```sh
genechart family.ged --text \
  --pref 'plugins.parse.indi = "scripts/nickname.lua"'
# 1. Robert "Bob" Smith ...
```

### Example — normalise US places

`scripts/usa.lua` (see `tests/fixtures/plugins/usa.lua`) appends `", USA"` to any
birth/death/marriage place ending in a US two-letter state code, for both
individuals and families. Run it as the `all` plugin:

```sh
genechart family.ged --text --pref 'plugins.parse.all = "scripts/usa.lua"'
# ... × 01 Jan 1970, Chicago, IL, USA  ⚭ 02 Feb 1925, Reno, NV, USA
```

The plugin system is designed to grow — later hooks (e.g. at layout or
SVG-output time) can reuse the same machinery.

> **License note:** the `lua` feature statically links Lua 5.4, which is
> distributed under the MIT license. Retain Lua's copyright notice when
> redistributing a binary built with `--features lua`.

## Placement Debug Logging (`bc_debug`)

### Why this matters

The `boxed_couples` layout placement algorithm is one of the most complex parts of the
codebase, and placement bugs are among the hardest to diagnose. The algorithm operates
in three recursive passes over a tree that can easily span hundreds of nodes and a dozen
generations:

1. **`place_descendants`** — a single left-to-right, depth-first traversal that assigns
   every individual's initial position using a right-envelope constraint system.
2. **`compact_pass`** — a top-down sweep that closes excess gaps between siblings by
   shifting left-packed subtrees rightward.
3. **`recenter_pass`** — a bottom-up sweep that re-centres parents over their children
   after compaction has moved those children.
4. **`fix_overlaps_pass`** — a final per-generation scan that corrects any cross-family
   overlaps introduced by the interaction between the three passes above.

The root difficulty is that each pass reads positions left by the previous one. A
subtle error in pass 1 (say, a miscalculated right-envelope depth) is silently amplified
by passes 2 and 3, so a node that was initially placed by 65 units can end up 65 units
too far right after recenter — while its sibling-in-a-different-subtree was placed
against the *pre-recenter* position, causing the two to overlap.

The situation is further complicated by **multi-spouse boxes**. An individual with one
in-scope spouse gets a normal-width box; two spouses get a wider `box_w2` box with a
different connector offset; three spouses get an even wider `box_w3` box where the parent
is centred over the *median* of the second spouse's children rather than aligned to the
first child's connector. Each box type feeds a different centering formula in every pass,
and the formulas interact across generation levels through the `global_right` tracking
array and the `fill_env_from_global` mechanism. A GEDCOM file with only single-spouse
couples can mask a formula error that only surfaces when a three-spouse box appears
somewhere in the subtree — and even then, only when the subtree is wide enough for
the compact and recenter passes to produce a cascading shift.

Because these interactions are determined entirely by the runtime data in the GEDCOM
file, **it is essentially impossible to reproduce a placement bug by inspecting the code
alone**. The only reliable way to diagnose it is to observe the actual sequence of
placement decisions — which node was placed where, by which pass, and what prior
position it had before it was moved.

### Enabling the log

Compile with the `bc_debug` Cargo feature. This is a compile-time flag; **no logging
code is present in a normal build**, so there is zero runtime cost unless the feature
is explicitly enabled.

```sh
# Run with debug logging enabled; log goes to /tmp/bc_debug.log by default
BC_DEBUG_LOG=/path/to/my.log cargo run --features bc_debug -- family.ged \
  --preff family.toml --type boxed_couples -o chart.svg
```

The `BC_DEBUG_LOG` environment variable controls the output file path. If unset, the
log is written to `/tmp/bc_debug.log`. A confirmation line is printed to stderr when the
feature is active:

```
bc_debug: logging to /tmp/bc_debug.log
```

### Log format

The log is a CSV file with one header row followed by one row per placement event:

```
op,id,x_before,x_after,dx,generation,source
PLACE,I1511,,7585.0,7585.0,2,src/layout/boxed_couples.rs:1069
SHIFT,I1580,7290.0,7420.0,130.0,4,src/layout/boxed_couples.rs:407/compact
RECENTER,I1513,7585.0,7650.0,65.0,3,src/layout/boxed_couples.rs:612
SHIFT,I1501,7805.0,7870.0,65.0,2,src/layout/boxed_couples.rs:407/fix_overlap
```

| Column | Meaning |
|--------|---------|
| `op` | Operation type: `PLACE`, `SHIFT`, or `RECENTER` |
| `id` | Individual ID (e.g. `I1511`) |
| `x_before` | x coordinate before the operation (empty for `PLACE`) |
| `x_after` | x coordinate after the operation |
| `dx` | Change in x (empty for `PLACE`) |
| `generation` | Generation depth from the root (0 = root) |
| `source` | Source file and line, plus caller context for `SHIFT` |

The `source` field for `SHIFT` events appends a slash-separated context label that
identifies which caller triggered the shift:

| Context label | Cause |
|---|---|
| `place/align` | `place_descendants` shifted a child subtree so the parent can sit at `x_default` |
| `compact` | `compact_siblings` closed an excess gap between siblings |
| `fix_overlap` | `fix_overlaps_pass` resolved a cross-family overlap |

### Diagnostic workflow

A typical session for diagnosing a placement overlap:

```sh
# 1. Generate the chart with logging
BC_DEBUG_LOG=/tmp/bug.log cargo run --features bc_debug -- family.ged \
  --preff family.toml -o /tmp/chart.svg

# 2. Identify overlapping individuals
python3 .claude/util/analyze_svg.py /tmp/chart.svg --overlaps

# 3. Trace the placement history of both individuals
grep "I1511\|I1501" /tmp/bug.log

# 4. Follow the sequence of operations to find the root cause
#    Look for a RECENTER that moves a node rightward after its right
#    sibling was already PLACED at a position that assumed the old x.
```

The combination of the debug log (which shows *when* and *how much* each node moved)
and the SVG analysis script (which shows the *final* positions and any overlaps) lets
you pinpoint the exact pass and the exact node that caused the problem without
re-running any code.

## License

MIT — see `LICENSE`.

## Author's Note
This is my first attempt at a non-trivial project that is almost completely vibe-coded. I am not proficient in Rust, which I can read but can't (yet) write fluently. I am aware that the LLM-generated code is baroque and often more complicated than necessary. However, using AI allowed me to get the job done, the alternative being no project at all, as I don't have the time to hand-code it myself.

For this project, I used mostly *Claude Sonnet 4.6*, with some tasks delegated to *Haiku 4.5* for more efficient use of the token budget. I also used *qwen3.6:27b-coding-nvfp4* running on Ollama locally on my MacBook Pro M3 (40-core GPU with 400GB/s memory bandwidth) with 36GB of RAM. The local model ran surprisingly well, but it is orders of magnitude slower than Claude and sometimes gets stuck because of context rot. Qwen also had problems with the Edit tool, so I had to vibe-code alternative file-editing tools. In either case, I ran the models from *Claude Code* launched in a terminal inside Visual Studio Code.

During the course of the project I improved my prompting. My initial prompts were more terse and attempted to tackle larger problems. Over time I learned to make smaller incremental changes and to provide detailed descriptions, often supplemented with bug duplication instructions. It is an ongoing journey of learning to interact with non-human intelligence.

*A.B.*
