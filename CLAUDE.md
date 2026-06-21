# CLAUDE.md — genechart

## Project overview
`genechart` is a Rust CLI that reads a GEDCOM 5.5.1 file and produces genealogical charts as text, SVG, or PDF.

## Architecture (bottom-up)
```
src/
  main.rs           # Pipeline orchestration (~50 lines)
  cli.rs            # clap-based argument parser
  preferences.rs    # Multi-level TOML preference system
  scene.rs          # Scene/Primitive IR (layout→backend contract)
  format.rs         # format_name / format_event; GEDCOM date parsing + strftime-like formatting; qualifier modes (none/gedcom/compact)
  text_metrics.rs   # Shared font constants (LINE_HEIGHT, CHAR_WIDTH_RATIO) and parsed_font()
  photos.rs         # Photo loading/processing for boxes layout: build_photo_map (parallel via rayon)
  plugins/
    mod.rs          # Experimental Lua plugin engine (parse-time hooks); gated behind the optional `lua` feature
  parser/
    mod.rs          # GEDCOM 5.5.1 line parser (UTF-8 only); also parse_and_merge()
    genrep.rs       # Internal representation: Individual<G>, Family<G>, Genrep<G>
    merge.rs        # Multi-GEDCOM merge: alias-file parsing, ID remapping, gap-filling merge
  layout/
    mod.rs          # Layout trait + dispatcher; LayoutOutput with into_scene() / is_fan()
    simple.rs       # Text-like layout (desc, anc, forest); emit_scene() → Scene
    boxed_couples.rs# Recursive box-placement algorithm; emit_scene() → Scene
    fan.rs          # Half-circle pedigree fan (180°); emit_scene() → Scene
    fancy.rs        # Cascading chart with spouses; descendants and ancestors directions; emit_scene() → Scene
  backend/
    mod.rs          # Renderer trait
    text.rs         # Plain-text output; consumes Scene via render_scene_text()
    svg.rs          # SVG output; render_scene() consumes Scene for all layouts
    pdf.rs          # PDF via SVG conversion crate
```

## Key decisions
- **GEDCOM**: 5.5.1, UTF-8 only
- **Dependencies**: `clap` (CLI), `toml` (preferences), `strfmt` (format strings), `svg2pdf` (SVG→PDF), `pdf-writer` (multi-page tiling), `fontdb` + `ttf-parser` (glyph metrics), `image` (JPEG/PNG loading + resize/crop), `base64` (data URI encoding), `rayon` (parallel photo loading)
- **Error handling**: warn to stderr + continue with defaults; no panic on recoverable errors
- **geo field**: `Individual<G>` is generic over a layout-specific geo type (`G = ()` before layout runs)
- **Scope**: `in_scope: bool` flag on each `Individual` / `Family`; set by a post-parse walk
- **Highlights**: a file of `ID [name...] [# comment]` lines; IDs are visually highlighted in the chart
- **Poster tiling**: configurable `overlap_mm` under `[output.poster]`
- **Custom paper size**: `output.paper.size = "custom"` uses `output.paper.custom.width` and `output.paper.custom.height` (both in mm, both must be > 0). `output.paper.orientation` is **ignored** for custom sizes — the user already specifies both dimensions explicitly. Default `custom.width` / `custom.height` are `0.0` ("unset"); setting only `size = "custom"` without dimensions returns `None` from `paper_size_mm` (no paper constraint). Implemented via early `return Some((cw, ch))` in the "CUSTOM" arm of `paper_size_mm` in `src/backend/svg.rs`.
- **PDF text**: always use `embed_text: true` in `svg2pdf::ConversionOptions`; `false` causes viewer font substitution
- **PDF font corruption — split at U+2000**: `svg2pdf` 0.13 cannot do per-character font fallback within a single `<text>` element. Any text string that may contain Unicode symbols (codepoint ≥ U+2000, such as ♂ ♀ ⚭ ⊗) must be split at the U+2000 boundary and rendered as separate `<text>` elements — one per (Latin, symbol) run — each with a single unambiguous `font-family`. For absolute-coordinate text use `render_mixed_text()`; for text centered at x=0 inside a rotated `<g>` (fan wedges) use `render_mixed_text_rotated()`. **Never write a raw `<text>` element with a CSS fallback font-family list that may contain mixed Latin+symbol content.** This rule applies to every layout, including any new layout introduced in the future.
- **Scene IR**: `src/scene.rs` defines a `Primitive` enum with variants: `Box`, `Text`, `Connector` (used by `simple` and `boxed_couples`); `Wedge` (fan); `FancyText`, `FancyConn` (fancy); `Group` (boxed_couples and fancy ancestors); `Image(ImagePrimitive)` (boxes layout photos — `bbox: Rect`, `href: String`). All five layouts call `emit_scene()` and produce a `Scene`; backends consume it without genealogical knowledge. `boxed_couples` wraps its primitives in `Primitive::Group` nodes (double-wrapped at box and connector level); the fancy ancestors layout also uses double-wrapped `Group` nodes for text (`<g id="anc-text-{id}">` → inner `<g>` → `FancyTextItem`) and `<g id="anc-conn-{id}">` for connectors. The text backend flattens groups via `flatten_primitives` before processing. `LayoutOutput` has `into_scene()` and `is_fan()` helpers; SVG calls `render_scene()` for all layouts; text backend guards against fan layout via `is_fan()`. `format_name`/`format_event` live in `src/format.rs`; font metric constants live in `src/text_metrics.rs`. `FancyConnector` has a `stroke_dasharray: String` field; empty string = solid line; SVG backend emits `stroke-dasharray="..."` when non-empty (used for dashed links between consanguineous duplicate instances). `TextAttr` variants: `IndividualName`, `SpouseName`, `BirthData`, `DeathData`, `MarriageData`, `IndividualId`, `GenerationNum`, `Highlighted`, `NoteText` — SVG backend resolves font/color via `semantic_attr()` with `_` wildcard fallbacks; adding new variants only requires updating `font_for_attr` in `svg.rs` if custom styling is needed. `BoxPrimitive` has a `two_spouses: bool` field (set to `true` by `boxed_couples` when `geo.width > box_width + 1.0`); the SVG backend uses it to emit `class="box double"` vs `class="box"`. `ConnectorPrimitive` has a `bar_y_fraction: f64` field (default 0.5 = midpoint); the SVG backend computes `bar_y = parent_y + (child_y - parent_y) * bar_y_fraction`. For 3-spouse `boxed_couples` boxes where all 3 spouses have ≥2 children, sp1/sp3 families use 1/3 (lower channel) and sp2 uses 2/3 (upper channel) to avoid connector crossings.
- **Fancy ancestors consanguinity**: When the same individual appears via multiple paths in a pedigree, `place_anc_subtree` and `emit_anc_subtree` thread a `visit_count: &mut HashMap<String, usize>` and use `anc_instance_key(id, count)` to assign unique keys (`"I5"` for the first visit, `"I5##1"` for the second, etc.). Both functions traverse in the same DFS order (father first, then mother), so they assign consistent instance numbers. The `show.duplicated_individual` preference (default `false`) controls whether `emit_anc_dup_links` draws a dashed polyline connecting all instances of the same individual; both instances are always emitted regardless of the preference.
- **`layout.fancy.compact`** (default `true`, descendants mode only): when `true`, children are stacked below the spouse column instead of being placed `gen_width` to the right; `gen_width` is ignored. The `SpouseToChildren` connector becomes a vertical segment below the first letter of the spouse's name (x = `compact_xv_spouse(ind_x, prefs)`), branching rightward to each child with a quarter-circle arc. Helpers `compact_xv_spouse` and `compact_child_text_x` in `src/layout/fancy.rs` compute these positions. The `IndivToSpouse` connector is unchanged. Ancestors mode is unaffected.
- **Date formatting**: `format_event` pre-processes `{date:FORMAT}` templates before passing to `strfmt` — `extract_date_format()` strips the strftime spec and `format_ged_date()` renders the result. Do NOT pass `{date:...}` to `strfmt` directly; it does not understand the `:FORMAT` part. `gedcom` mode with no pattern is a fast path that returns the raw GEDCOM string unchanged (backward-compatible). All date logic lives in `src/format.rs`.
- **Multi-GEDCOM merge**: `parser::parse_and_merge()` parses the main GEDCOM then, for each further GEDCOM, reads its alias file, remaps all IDs (alias map wins; non-aliased IDs get a prefix letter B–Z inserted after char 0), and merges into the main `Genrep`. Merge logic: main wins for non-`None` fields; fams/famc/children_ids/notes are unioned (deduplicated). Alias file maps `further_id → main_id`. CLI `--merge <GEDCOM> <ALIAS>` takes precedence over `files.merge`/`files.merge_aliases`. Paths are resolved relative to the main GEDCOM file's directory.
- **`format.no_name`**: when an `Individual` has no name in the GEDCOM file, `format_name()` substitutes this string. Empty string (default) means the name line is omitted entirely. Lives in `src/format.rs`; `format_event` already handles the substitution before templating.
- **`custom.gedcom.tags`**: configurable non-standard GEDCOM tag names. Defaults in `defaults.toml` preserve backward compatibility. Tag names are loaded via `parser::set_parser_tags(prefs.custom.gedcom.tags.clone())` (called in `main.rs` after preferences load, before parsing). The parser uses `OnceLock<CustomGedcomTagsPrefs>` with a fallback to `Prefs::default().custom.gedcom.tags` so tests work without calling `set_parser_tags`. **No GEDCOM tag strings are hardcoded in `.rs` files** — they all live in `defaults.toml`. Configurable tags: `alt_name` (default `NAM2`) → `Individual.alt_name`; `relig_name` (default `NAMH`) → `Individual.relig_name`; `living` (default `_LIVING`) → `Individual.living` (Y/N bool); `relig_marr` (default `JMAR`) → `Family.relig_marr`.
- **Format template variables for extra fields**: `format_name()` exposes `{alt_name}`, `{relig_name}`, `{living}` as opt-in variables in `format.individual`. `{living}` expands to `format.living` (default `""`) when `living = Some(true)`, empty otherwise. For marriage, `format_event_extra()` in `src/format.rs` extends `format_event` with additional key-value pairs; marriage call sites pass `("relig_marr", fam.relig_marr.as_deref().unwrap_or(""))` so `{relig_marr}` is available in `format.marriage`. Default templates do not include these variables — output is unchanged until the user opts in.
- **SVG CSS classes**: every SVG element carries a `class=` attribute so charts can be restyled without touching TOML. The SVG backend helper `class_for_attrs(attrs: TextAttr) → &str` maps `TextAttr` variants to class names (`indi_name`, `spouse_name`, `indi_birth`, `indi_death`, `indi_marriage`, `indi_id`, `gen_num`, `note_text`; highlighted text also adds `highlighted`). Non-text elements use fixed class names: boxes → `box` (or `box double` for two-spouse boxes), connectors → `connector`, wedges → `wedge`, row rules → `row_rule`, note bars → `note_bar`, note links → `note_link`, highlight backgrounds → `highlight_rect`, photos → `photo`. Functions `svg_text_full`, `svg_line`, `svg_rect`, `render_mixed_text`, and `render_mixed_text_rotated` all accept a `class: &str` parameter.
- **`output.style.spacing.title/copyright`**: canvas-unit gap between the title/copyright text and the chart body. Both default to `12.0`. Stored in `StyleSpacingPrefs` in `src/preferences.rs`; consumed by the SVG backend when positioning title/copyright text.
- **`output.style.realistic_tree`**: optional organic tree-branch background for the `boxed_couples` layout (root_pos = bottom only). When `enabled = true`, `src/backend/svg.rs` suppresses default connector rendering (`skip_connectors: true` on `BcSvgCtx`) and instead calls `src/backend/realistic_tree.rs::render_tree_layer()` before the main primitive loop — so the tree layer is drawn first and boxes appear on top. The SVG canvas is expanded downward by `root_extra_height()` to accommodate roots below the root box. Four styles: `"tapered"` (filled Bézier paths with globally Y-position-driven width), `"stroke"` (layered stroked S-curves), `"filter"` (two-layer filled paths with a white cylindrical highlight), `"ink"` (hand-drawn coherent tree modelled on the reference in `tests/fixtures/local/realistic_tree_samples`: flared trunk visible below the root box, organic buttress roots, continuous flat-topped leaf canopy via `ink_canopy`/value noise, and one continuous tapered stroke per child that runs along the main limb then turns up under its box — the visible limb is the overlap of these strokes; self-contained `ink_*` helpers). Earlier `"ink"`/`"ink2"`/`"ink3"` iterations were removed in favour of this implementation, which took the `"ink"` name. Leaf density: `"none"` | `"low"` | `"medium"` (default) | `"high"`. The `realistic_tree.rs` module is self-contained; `boxed_couples.rs` is not modified by this feature.
- **Photos (boxes layout only)**: `show.photo = true` triggers `src/photos.rs::build_photo_map`, which resolves individual IDs to image hrefs. Discovery order: index file first (if `photos.index` is set), then `<photos_dir>/<id>.<ext>`. Processing (open → scale → encode/link) runs in parallel via `rayon::par_iter`. Embedded images (`embedded = true` or PDF output) become `data:image/jpeg;base64,...` / `data:image/png;base64,...` URIs; linked images use a path relative to the SVG output file. The `boxes` layout places the photo centered horizontally in the top portion of each box, offset by `margin`, above the name text. `Primitive::Image` must be handled in every exhaustive `match` over `Primitive` — SVG renders it as `<image href="..." preserveAspectRatio="none"/>`, text backend is a no-op. Non-`boxes` layouts emit a warning when `show.photo = true` but never produce `Primitive::Image`.

## Clone on generic structs

`Individual<G>` and `Family<G>` have `#[derive(Clone)]`, but this does **not**
eliminate the manual field-copying in `layout/simple.rs` and `layout/fan.rs`.
Rust cannot convert `Individual<()>` to `Individual<FanGeo>` via `clone()` +
`geo = ...` because the generic parameter `G` is part of the concrete type.
A `map_geo()` method would require the same number of field assignments,
offering no savings. Leave the manual copying as-is. `Clone` is retained
for future use (e.g. layout algorithms that copy within the same `G` type).

## boxed_couples placement algorithm
The placement algorithm in `src/layout/boxed_couples.rs` is complex; understand these invariants before touching it:
- `env_left[j]` = right-edge constraint for individuals at depth `current_generation + j`
- `global_right[g]` = rightmost right-edge placed at absolute generation `g`; carried as `&mut Vec<f64>` through all recursive calls and updated after each individual is placed
- When a sibling's right-envelope is shorter than needed, missing slots are filled from `global_right` at the matching absolute generation (`fill_env_from_global`) — NOT from the last value (which over-constrains)
- **Parent-centring rule**: parent x is always the centre of its children; never clamp the parent — instead shift the entire child subtree rightward (`shift_subtree`) so the parent's natural centre equals `x_default`
- `shift_subtree` must also update `global_right` as it shifts, to avoid stale boundaries for later siblings
- **`recenter_pass` clamping**: `recenter_pass` re-centers each parent over its children after initial placement. Its rightward movement must be bounded by `max_center_x` — derived from the right sibling's pre-recenter position minus `gap_w / 2` — so it never pushes a node past its right sibling. Pass `max_center_x: Option<f64>` recursively; clamp `new_x` to `min(new_x, max_center_x)` before updating `out`.
- **3-spouse box**: `FamilyGeo` has `conn_out3_x`, `conn_out3_y`, `has_spouse3` fields for the third-spouse connector (far-right column). The box layout is [sp1 top | sp2 top + ind bottom | sp3 top] with total width `box_width_3_spouses`. Children are placed sequentially left-to-right (c1, c2, c3); parent x is derived from children2's median. `prune_spouses` limits to 3 (was 2). `has_spouse2` is true for both 2- and 3-spouse boxes; `has_spouse3` distinguishes the triple-wide case. In the connector loop, `fam_index` in the sorted list is replaced by the pruned-list index to correctly route to `conn_out1/2/3_x`. 2-channel routing (`bar_y_fraction` 1/3 for sp1/sp3, 2/3 for sp2) only fires when the box is triple-wide **and** all 3 spouses have ≥2 children.

## boxed_couples text backend invariants
Two subtle invariants in `src/backend/text.rs` that are easy to break:

- **Blank spouse rows**: The text backend assigns rows by grouping primitives by Y coordinate in order. When a box has no spouse, `emit_blank_spouse_section()` **must** be called so the spouse-section Y coordinates appear in the group map. Without it, the individual's name gets row 1 (just below the top border) instead of its correct position deeper in the box, breaking vertical alignment across generations.
- **Bottom-border row off-by-one**: `display_to_row(bbox.y + bbox.h)` maps to `row1`, but `draw_box` places the bottom border at `row1 − 1`. In `draw_connector_on_grid()`, the row for a "bottom border" endpoint must be `raw.saturating_sub(1)`: for downward trees this is the parent row; for upward trees this is the child row.

## Simple layout: forest direction
Forest mode sets all individuals in scope, finds roots (individuals with no in-scope `famc`), and renders each root as an independent descendants sub-chart stacked vertically. Key invariants:

- `global_shown: HashSet<String>` — canonical IDs of non-spouse individuals placed in all previous trees. When every child of a family is already in `global_shown`, the subtree is replaced by a synthetic `"...N"` ellipsis entry (`is_ellipsis: true` in `SimpleGeo`) rather than repeating the whole sub-tree.
- `global_shown_spouse: HashSet<String>` — IDs of individuals who appeared as a spouse in a prior tree. When `show.last_gen_spouses = true` and a root is in `global_shown_spouse` and all its families' children are already in `global_shown` (or empty), the root's standalone tree is skipped entirely.
- Instance keys `"ID##N"` — when the same individual appears in more than one tree, subsequent occurrences get a `##N` suffix so their geo entries can coexist in `geo_map`.
- Couple deduplication — before the loop, if two roots are married to each other, the one with the smaller tree (or alphabetically later ID on a tie) is removed from the `roots` list; it will appear inside the other root's tree as a spouse.
- Ellipsis x alignment in `emit_scene`: `x = id_col_px + indent * indent_px + gen_prefix_px(generation)` — includes `gen_prefix_px` so the `...` aligns exactly at the child name column, accounting for the text backend's `fallback_shift`.

## Simple layout: show.notes
`show.notes = true` renders GEDCOM `NOTE` text as additional rows below each individual.

- `Individual.notes: Vec<String>` and `Family.notes: Vec<String>` are populated by the parser from `1 NOTE` tags (inline, non-pointer) and `2 CONT`/`2 CONC` continuations.
- `count_note_lines(notes)` computes how many extra lines to allocate: each non-empty note contributes `note.lines().count().max(1)` lines.
- `visit()` and `layout_ancestors()` both call `count_note_lines` and advance the running line counter by `1 + note_lines + spacing` instead of `1 + spacing`.
- `emit_scene()` renders note sub-lines at `(geo.line + offset) * line_height_px`, using `TextAttr::NoteText` (which SVG renders with the names font via the `IndividualName` arm in `font_for_attr`). Note x is `id_col_px + indent * indent_px` (no gen-prefix — notes don't carry a generation number).
- `max_line` in `emit_scene` is computed including note lines so the canvas height is correct.
- **`show.notes_html`**: when `true`, `<a href="…">text</a>` tags inside note text are rendered as clickable SVG hyperlinks (`<a href="…" class="note_link">`) with blue underline styling; PDF output includes link annotations. Other HTML tags have their content extracted as plain text. Enabled only in SVG/PDF output (text backend ignores it). Multiple words in the same `<a>` tag are merged into a single link; bare `\r` is treated as a line separator.

## Multi-GEDCOM merge

`src/parser/merge.rs` contains three public functions; `src/parser/mod.rs` exposes `parse_and_merge`:

- **`parse_alias_content(content: &str) → HashMap<further_id, main_id>`** — parses the in-memory text of an alias file. Two whitespace-separated fields per line; `#` and blank lines ignored; anything beyond field 2 ignored. `read_alias_file(path)` wraps this.
- **`remap_genrep(genrep, alias, prefix) → Genrep`** — rewrites every ID in a further `Genrep`: `alias.get(id)` → main ID; otherwise `prefix_id(id, prefix)` inserts `prefix` after char 0 (e.g. `I45` + `'B'` → `IB45`). Remaps own ID, `fams`, `famc`, `husband_id`, `wife_id`, `children_ids`.
- **`merge_into(main, further)`** — for each entity in `further`: if the ID already exists in `main` (aliased or unexpected collision), fill `None` fields from `further`, union `fams`/`famc`/`children_ids`/`notes`, warn on name mismatch; otherwise insert directly.
- **`parse_and_merge(main_path, further: &[(PathBuf, PathBuf)])`** — prefix letters are `b"BCDEFGHIJKLMNOPQRSTUVWXYZ"[i]`; errors if `i >= 25`. The main GEDCOM's `first_individual_id` is preserved; further GEDCOMs' values are ignored.

ID collision invariant: a non-aliased further-GEDCOM ID that happens to equal a main-GEDCOM ID (e.g. main has `IB5` and the 2nd further GEDCOM maps `I5` → `IB5`) is silently treated as an alias (main record wins). This is an edge case the user must avoid by choice of GEDCOM IDs.

## Deterministic emit order
SVG output must be **byte-stable across runs for identical input**, so `diff` reveals only the effect of a real change. `Genrep.individuals` and `Genrep.families` are `HashMap`s, whose iteration order is randomised per process. Layout *placement* (coordinates) is unaffected — it traverses from the root via ordered `children_ids`/spouse lists — but any code that **emits primitives by iterating those HashMaps** would otherwise produce a randomly-ordered element list.

Rule: whenever you iterate `individuals`/`families` (or any `HashMap`/`HashSet`) to **produce ordered output** (scene primitives, SVG elements), collect into a `Vec` and sort by a stable key (the GEDCOM **id**) first. Current emit-order fixes (all tie-break by id):
- `layout/boxed_couples.rs::emit_scene` — `placed` is `sort_by` id; families iterated via a `sorted_families` Vec sorted by family id.
- `layout/boxes.rs::emit_scene` — `placed` is `sort_by` id.
- `layout/simple.rs::emit_scene` — `entries.sort_by(line, then id)` (line-major, id tie-break).
- `backend/svg.rs` row-rule underlines — `row_info` collected into a Vec and sorted by row before emit.

Text output was already stable (the text backend regroups primitives by Y), and these changes do not alter it. Note this is **emit order** only; the realistic-tree `"ink"` style additionally seeds its per-branch randomness from the GEDCOM family id (not the list index) so its geometry is stable independent of emit order (see the `realistic_tree` decision above).

## Plugin system
Experimental Lua plugin layer in `src/plugins/mod.rs`, **gated behind the optional `lua` Cargo feature (default-off)** so normal builds need no C compiler. With `--features lua` it pulls in `mlua` (vendored Lua 5.4); both are MIT (genechart stays MIT — retain Lua's notice for binary distribution). First increment: **parse-time hooks** via `plugins.parse.{indi,fam,all}` (each a Lua script path; `all` runs before the type-specific one; missing function = no-op).

Key implementation points:
- **`PluginEngine` is feature-uniform**: a real engine with `--features lua`, a no-op stub otherwise (so the parser is `cfg`-free). The stub's `from_prefs` **bails** if any plugin is configured ("rebuild with `--features lua`"); the real one loads/compiles scripts (compile error = fatal). Built once in `main::run` and **passed by reference** into `parser::parse`/`parse_and_merge`/`parse_str_with` — it cannot live in a `static OnceLock` like `set_parser_tags` because `mlua::Lua` is `!Sync`.
- **Hook point**: `parser::commit_record` runs `engine.run_individual/run_family` on the finished record just before inserting it. Records are marshalled to Lua tables; the callback returns a table of changes that is applied with **plausibility checks** (whitelisted text/event fields only; structural ids/links are read-only and ignored with a warning; type mismatches warned+ignored; never panics). `all` then specific, each rebuilding the table so the specific script sees prior edits.
- **Unparsed tags**: tags the parser doesn't map (e.g. `2 NICK`) are needed by scripts but **not stored on `Individual`/`Family`**. The parse loop keeps a transient per-record `Vec<UnparsedTag>` (captured in the three `_ =>` arms only when `engine.active()`), passed to `commit_record` and cleared after — so the structs/copy-helpers/layouts are untouched.
- **Determinism**: hooks run per record in GEDCOM file order. `print` goes to stdout (may interleave with text-chart output → prefer `-o`); runtime errors and gated warnings go to stderr.
- **Backward-compat**: `parse_str(content)` is a `#[cfg(test)]` wrapper over `parse_str_with(content, &PluginEngine::disabled())`, so the ~50 test call sites are unchanged.
- **Tests**: Lua functional tests are `#[cfg(feature = "lua")]` (skipped by a plain `cargo test`); run them with `cargo test --features lua`. A `#[cfg(not(feature = "lua"))]` test asserts the configured-without-feature error. Example scripts + GEDCOM live in `tests/fixtures/plugins/` and `tests/fixtures/plugins_sample.ged`.
- **`--plugin-parse <FILE>` CLI flag**: shorthand for `plugins.parse.all`. Applied in the `main::run` CLI-shortcuts block (sets `prefs.plugins.parse.all`, CLI wins over the pref) **before** the `--prpref` dump and the engine build — so it also shows under `--prpref` and triggers the no-`lua` bail. The flag is always present (not `cfg`-gated).
- **Extensibility**: designed so later hooks (layout placement, SVG post-processing) can reuse `PluginEngine`.

## Versioning
Semantic versioning starting at v0.1.0 after all modules are complete. Version bumps above the patch level require the user's approval.

## Rust Conventions
- Target Rust 2024 edition: avoid the reserved keyword `gen` as an identifier, and prefer pattern destructuring over `ref mut`.

## Code conventions
Code indentation is 4 spaces. Never use tabs.

## Operating system
Development is done in a macOS environment. Do **not** use Linux-specific commands unless you have checked (with the `which` command) that they are also available on this machine.

## Common commands
```bash
# Showing non-printing characters in a file
cat -etv # Equivalent to `cat -A` on Linux, which is not available on MacOS
```

## Maintainer workflow (private)
This repository is developed with an AI-assisted workflow whose tooling is intentionally
not published. If a file `.claude/CLAUDE.private.md` exists in your checkout, read it for
those maintainer-specific instructions (planning queue, helper scripts, agent-session
checklist, file-editing conventions). If it is absent, ignore this section and follow your
own workflow conventions — nothing below depends on it.
