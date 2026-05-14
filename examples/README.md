# genechart examples

Each subdirectory contains a real GEDCOM file and a same-basename TOML preferences file
that genechart loads automatically. Generated output files are committed so you can browse
them without installing anything.

## Prerequisites

Build and install genechart from the project root:

```sh
cargo install --path .
```

## Regenerating outputs

```sh
./examples/generate.sh
```

## Examples

| Directory | Source | What it demonstrates |
|-----------|--------|----------------------|
| `kennedy/` | Kennedy family, 70 individuals, 5 generations | All four layout types; highlights; TOML auto-load |
