# Contributing to genechart

Thanks for your interest! genechart is a personal project that is largely
AI-assisted (see the *Author's Note* in the README). Contributions, bug reports,
and sample GEDCOM files that expose layout problems are all welcome.

## Reporting bugs

Open an issue with:
- the genechart version (`genechart --version`),
- the command line you ran,
- a minimal GEDCOM file that reproduces the problem (anonymised if needed), and
- what you expected vs. what you got (attach the SVG/PDF/text output if you can).

For placement/overlap bugs in the `boxed_couples` layout, the built-in debug log is
the fastest way to pinpoint the cause — see
[`docs/debugging-boxed-couples.md`](docs/debugging-boxed-couples.md).

## Development

```sh
cargo build                  # debug build
cargo test                   # run the test suite
cargo test --features lua    # include the optional Lua plugin tests
cargo fmt                    # format (CI enforces `cargo fmt --check`)
cargo clippy --all-targets   # lint (CI enforces `-D warnings`)
```

Before opening a pull request, please make sure `cargo fmt --check`, `cargo clippy`,
and `cargo test` all pass — CI runs the same checks (with and without `--features lua`).

## Coding conventions

- Rust 2024 edition; 4-space indentation, no tabs.
- Keep changes surgical: touch only what the change requires, and match the surrounding
  style rather than reformatting adjacent code.
- The deterministic-output rule matters: when iterating a `HashMap`/`HashSet` to emit
  ordered output (scene primitives, SVG elements), sort by a stable key (the GEDCOM id)
  first so output stays byte-stable across runs. See the architecture notes in
  `CLAUDE.md` for the full list of invariants.

## License

By contributing, you agree that your contributions are licensed under the MIT License
that covers this project.
