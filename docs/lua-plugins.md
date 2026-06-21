# Lua Plugins (experimental)

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

## Configuration

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

## Writing a script

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

## Example — append a nickname to the given name

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

## Example — normalise US places

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
> redistributing a binary built with `--features lua` (see `THIRD_PARTY_NOTICES.md`).
