# genechart

A command-line tool that reads a [GEDCOM 5.5.1](https://gedcom.io/) genealogical file and generates a family-tree chart as text, SVG, or PDF.

**Version**: v0.0.1 — under development. See `TODO.md` for current progress.

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
| `--type <TYPE>` | Layout algorithm: `simple`, `fan`, `boxed_couples` |
| `--text` | Output as plain text |
| `--svg` | Output as SVG |
| `--pdf` | Output as PDF |
| `-o` / `--output <FILE>` | Output file (extension infers type if no `--text`/`--svg`/`--pdf` flag) |
| `--pref '<key name="val">'` | Override any preference inline (TOML syntax, repeatable) |
| `--pref` | Bare `--pref` (no value): dump merged preferences to stdout and exit |
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

# Trace how preferences are resolved across all sources
genechart family.ged -r I1 -g 5 --trace prefs 2>&1 | head

# Use a shared preferences file for a project-specific style
genechart family.ged --preff ~/projects/genealogy/style.toml
```

## Layout Types

### simple

Text-like layout with indented generations. Suitable for terminal output or simple SVG/PDF charts. Supports all directions (descendants, ancestors, forest).

```sh
genechart family.ged -r I1 --type simple -o chart.svg
```

Configuration: `[layout.simple]` — `indent` (columns per generation), `vert_spacing` (lines between generations).

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

Configuration: `[layout.fan]` — `ring_height`, `ring_gap`.

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
notes = false
last_gen_spouses = false
id = false

[format]
individual = "{firstname} {lastname} {sex}"
birth = "* {date}, {location}"
death = "x {date}, {location}"
marriage = "+ {date}, {location}"

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

[output]
type = "text"
path = ""

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

Format strings use `{key}` placeholders: `{firstname}`, `{lastname}`, `{sex}`, `{date}`, `{location}`.

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

Highlighted individuals are visually distinguished in SVG/PDF output (rendered in a different text color, configurable via `output.style.text.id`).

## Building

```sh
cargo build             # debug
cargo build --release   # release
cargo test              # run all tests
```

## License

MIT — see `LICENSE`.
