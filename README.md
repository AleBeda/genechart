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
| `--root <ID>` | Root individual ID (default: first individual in file) |
| `-g <N>` / `--generations <N>` | Number of generations to show |
| `--preff <FILE>` | Load an explicit TOML preferences file (see priority below) |
| `--pref '<key=val, ...>'` | Override any preference inline (TOML syntax, repeatable) |
| `-h` / `--help` | Show help |
| `--version` | Show version |

### Examples

```sh
# Generate a 4-generation descendant chart as SVG
genechart family.ged --root I1 -g 4

# Generate a pedigree fan chart as PDF
genechart family.ged --root I1 --pref 'layout.type = "fan", output.type = "pdf"'

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
