# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Versions up to and including 0.7.0 were developed while the repository was private;
this file begins at the first public release. The full pre-0.7.0 history is preserved
in the git tags `v0.1.0` … `v0.7.0`.

## [Unreleased]

## [0.7.0] — 2026-06-18

First public release. Capabilities at this point:

- **Layouts:** `simple` (descendants / ancestors / forest), `boxed_couples`, `fan`
  (half-circle pedigree), `fancy` (cascading), and `boxes` (one box per individual,
  with optional photos).
- **Output formats:** plain text, SVG, and PDF (including multi-page poster tiling).
- **Preferences:** multi-level TOML system with auto-loaded per-file preferences,
  `--pref` inline overrides, `--preff` files, and `--trace` / `--prpref` diagnostics.
- **Data features:** multi-GEDCOM merge with alias files, highlights files,
  configurable non-standard GEDCOM tags, photos (embedded or linked), HTML notes,
  and flexible date formatting with qualifier modes.
- **Styling:** SVG CSS classes on every element, custom paper sizes, and an optional
  organic `realistic_tree` background for `boxed_couples`.
- **Experimental:** a Lua parse-time plugin system behind the optional `lua` Cargo
  feature (default-off).

[Unreleased]: https://github.com/AleBeda/genechart/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/AleBeda/genechart/releases/tag/v0.7.0
