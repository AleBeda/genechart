# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Versions up to and including 0.7.0 were developed while the repository was private;
this file begins at the first public release. The full pre-0.7.0 history is preserved
in the git tags `v0.1.0` … `v0.7.0`.

## [Unreleased]

## [0.8.0] — 2026-06-25

### Fixed
- Single-spouse parents in the `boxes` and `boxed_couples` layouts are now centered on the
  geometric midpoint of their first and last child, instead of over the median child. This
  removes the lopsided parent position when sibling subtrees differ greatly in width. (Output is
  unchanged for parents with one or two children, or evenly-spaced children.)
- `output.style.text.names` and `output.style.text.dates` were defined but ignored; name and
  date text now use these colors (in all hex forms, including alpha).
- Box layouts (`boxed_couples`, `boxes`) now draw boxes on top of connectors, so thick
  connectors no longer visibly overshoot and overlap the box edges.
- The `fancy` layout's connector color now supports the full hex range (4-/6-/8-digit and
  alpha); it previously used a 3-digit-only converter, unlike every other layout.

### Added
- `[show] exclude` prunes branches from the chart: list individual IDs (the dual of `--merge`,
  useful for trimming uninteresting/unreliable lines or partitioning a large tree). Traversal stops
  at each excluded individual — its spouse(s)/descendants (descendants mode) or ancestors
  (ancestors mode) are dropped, unless also reachable via a non-excluded path (reconvergence). Each
  entry takes an optional `msg` shown in place of the name (with no birth/death/marriage data),
  styled via `output.style.text.exclude_msg` and `output.style.fonts.exclude_msg` in every layout;
  an empty `msg` omits the individual entirely. Example:
  `exclude = [{ id = "I123", msg = "see separate tree" }]`.
- Nickname support: the GEDCOM `NICK` field (`2 NICK` under `NAME`, or a flat `1 NICK`) is now
  parsed. With `show.nickname = true`, individuals who have a nickname are formatted with
  `format.individual_nickname` (which adds a `{nickname}` placeholder) instead of `format.individual`.
- `output.style.spacing.names_autocompress` (default `0.85`): in the `boxes` and
  `boxed_couples` layouts, names too wide for their box are compressed horizontally to fit
  (down to this fraction; `>= 1.0` disables it), with optional `info`/`warning` diagnostics.
- `output.style.text.gen_numbers` and `output.style.text.notes` color preferences (default
  opaque black `0x000`) for the generation-number prefix and GEDCOM note text.
- `output.style.text.title`, `…copyright`, `…row_rule`, `…note_bar`, and `…note_link` color
  preferences (defaults match the previous hardcoded colors), so the chart title, copyright,
  row-rule underline, note bar, and note hyperlinks are all configurable.
- `diagnostics.errors` is now honored: setting it to `false` suppresses error messages from the
  output pipeline (errors raised before preferences load are still reported). `diagnostics.info`
  and `diagnostics.debug` remain reserved for future use.
- `[output.style.wedges]` preferences (`width`, `border`, `background`) to style the `fan`
  layout's wedges independently, with the same meaning as `[output.style.boxes]`.
- Color transparency: color preferences now accept 4-digit (`0xRGBA`) and 8-digit
  (`0xRRGGBBAA`) hex with an **alpha-last** channel, in addition to the existing 3-digit and
  6-digit opaque forms. Applies to every color preference and works in both SVG and PDF output.

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

[Unreleased]: https://github.com/AleBeda/genechart/compare/v0.8.0...HEAD
[0.8.0]: https://github.com/AleBeda/genechart/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/AleBeda/genechart/releases/tag/v0.7.0
