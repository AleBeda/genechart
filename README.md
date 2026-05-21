# genechart

A command-line tool that reads a [GEDCOM 5.5.1](https://gedcom.io/) genealogical file and generates a family-tree chart as text, SVG, or PDF.

**Version**: v0.4.0

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
| `--pref '<key name="val">'` | Override any preference inline (TOML syntax, repeatable) |
| `--pref` | Bare `--pref` (no value): dump merged preferences to stdout and exit |
| `--prpref` | Print the fully-resolved preferences as TOML and exit; combine with `-o` to see type inference |
| `--preff <FILE>` | Load an explicit TOML preferences file |
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

When `show.notes = true`, GEDCOM `NOTE` text is rendered as additional indented rows below each individual (simple layout only; descendants, ancestors, and forest directions).

### boxed_couples

Recursive box-placement algorithm. Each individual or couple gets a box; children are placed below their parents with envelope-based spacing to avoid overlaps. Supports descendants and ancestors directions.

```sh
genechart family.ged -r I1 --type boxed_couples -o chart.svg
```

Configuration: `[layout.boxed_couples]` — `box_width`, `box_height`, `gap_width`, `gap_height`, `box_width_2_spouses`.

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

Configuration: `[layout.fancy]` — `gen_width` (horizontal distance between successive generations), `child_gap` (vertical gap between a person's last spouse and their first child), `anc_gap` (vertical breathing room around each individual in ancestors mode).

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

Vector graphics output. Supports boxes, connectors, and configurable fonts.

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
5. `--preff <FILE>` — an explicit preferences file
6. `--pref` command-line overrides

Unknown preference keys in `--pref` are a hard error (the command aborts with a message naming the bad key). Unknown keys in TOML config files produce a warning and are ignored.

### Full TOML Example

```toml
[files]
gedcom = "{gedcom}"
highlights = ""

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
gen_width = 300.0
child_gap = 10.0
anc_gap = 10.0

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
size = "A4"
orientation = "portrait"

[output.poster]
rows = 1
columns = 1
overlap_mm = 0.0
alignment_lines = true

[output.style]
dot_leaders = true

[output.style.fonts]
names = "Georgia 14"
dates = "Arial 10"
title = "Georgia 24"

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
```

## License

MIT — see `LICENSE`.

## Author's Note
This is my first attempt at a non-trivial project that is almost completely vibe-coded. I am not proficient in Rust, which I can read but can't (yet) write fluently. I am aware that the LLM-generated code is baroque and often more complicated than necessary. However, using AI allowed me to get the job done, the alternative being no project at all, as I don't have the time to hand-code it myself.

For this project, I used mostly *Claude Sonnet 4.6*, with some tasks delegated to *Haiku 4.5* for more efficient use of the token budget. I also used *qwen3.6:27b-coding-nvfp4* running on Ollama locally on my MacBook Pro M3 (40-core GPU with 400GB/s memory bandwidth) with 36GB of RAM. The local model ran surprisingly well, but it is orders of magnitude slower than Claude and sometimes gets stuck because of context rot. Qwen also had problems with the Edit tool, so I had to vibe-code alternative file-editing tools. In either case, I ran the models from *Claude Code* launched in a terminal inside Visual Studio Code.

During the course of the project I improved my prompting. My initial prompts were more terse and attempted to tackle larger problems. Over time I learned to make smaller incremental changes and to provide detailed descriptions, often supplemented with bug duplication instructions. It is an ongoing journey of learning to interact with non-human intelligence.

*A.B.*
