# genechart

A command-line tool that reads a [GEDCOM](https://gedcom.io/) genealogical file and generates a family-tree chart as text, SVG, or PDF.

## Status

Under development. See `TODO.md` for current progress.

## Usage

```
genechart [OPTIONS] [GEDCOM_FILE]
```

### Options

| Flag | Description |
|---|---|
| `-r` / `--root <ID>` | Root individual ID (default: first individual in file) |
| `-g <N>` / `--generations <N>` / `--gen <N>` | Number of generations to show |
| `--dir <DIRECTION>` | Chart direction: `descendants`, `ancestors`, `pedigree`, `forest` |
| `--type <TYPE>` | Layout algorithm: `simple`, `fan`, `boxed_couples` |
| `--text` | Output as plain text |
| `--svg` | Output as SVG |
| `--pdf` | Output as PDF |
| `-o` / `--output <FILE>` | Output file (extension infers type if no `--text`/`--svg`/`--pdf` flag) |
| `--pref '<key=val, ...>'` | Override any preference inline (TOML syntax, repeatable) |
| `--pref` | Bare `--pref` (no value): dump merged preferences to stdout and exit |
| `--preff <FILE>` | Load an explicit TOML preferences file (see priority below) |
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

# Dump merged preferences (useful for debugging)
genechart family.ged --pref

# Use a shared preferences file for a project-specific style
genechart family.ged --preff ~/projects/genealogy/style.toml
```

## Configuration

Preferences are read from (lowest to highest priority):

1. Installation-directory defaults (`defaults.toml`)
2. User home (`~/.genechart.toml`)
3. Directory TOML (`genechart.toml` in the same directory as the GEDCOM file)
4. File TOML (same basename as the GEDCOM file, e.g. `family.toml` for `family.ged`)
5. `--preff <FILE>` — an explicit preferences file (errors if the path does not exist)
6. `--pref` command-line overrides

## Building

```sh
cargo build            # debug
cargo build --release  # release
cargo test             # run all tests
```

## License

MIT — see `LICENSE`.
