# Placement Debug Logging (`bc_debug`)

## Why this matters

The `boxed_couples` layout placement algorithm is one of the most complex parts of the
codebase, and placement bugs are among the hardest to diagnose. The algorithm operates
in three recursive passes over a tree that can easily span hundreds of nodes and a dozen
generations:

1. **`place_descendants`** — a single left-to-right, depth-first traversal that assigns
   every individual's initial position using a right-envelope constraint system.
2. **`compact_pass`** — a top-down sweep that closes excess gaps between siblings by
   shifting left-packed subtrees rightward.
3. **`recenter_pass`** — a bottom-up sweep that re-centres parents over their children
   after compaction has moved those children.
4. **`fix_overlaps_pass`** — a final per-generation scan that corrects any cross-family
   overlaps introduced by the interaction between the three passes above.

The root difficulty is that each pass reads positions left by the previous one. A
subtle error in pass 1 (say, a miscalculated right-envelope depth) is silently amplified
by passes 2 and 3, so a node that was initially placed by 65 units can end up 65 units
too far right after recenter — while its sibling-in-a-different-subtree was placed
against the *pre-recenter* position, causing the two to overlap.

The situation is further complicated by **multi-spouse boxes**. An individual with one
in-scope spouse gets a normal-width box; two spouses get a wider `box_w2` box with a
different connector offset; three spouses get an even wider `box_w3` box where the parent
is centred over the *median* of the second spouse's children rather than aligned to the
first child's connector. Each box type feeds a different centering formula in every pass,
and the formulas interact across generation levels through the `global_right` tracking
array and the `fill_env_from_global` mechanism. A GEDCOM file with only single-spouse
couples can mask a formula error that only surfaces when a three-spouse box appears
somewhere in the subtree — and even then, only when the subtree is wide enough for
the compact and recenter passes to produce a cascading shift.

Because these interactions are determined entirely by the runtime data in the GEDCOM
file, **it is essentially impossible to reproduce a placement bug by inspecting the code
alone**. The only reliable way to diagnose it is to observe the actual sequence of
placement decisions — which node was placed where, by which pass, and what prior
position it had before it was moved.

## Enabling the log

Compile with the `bc_debug` Cargo feature. This is a compile-time flag; **no logging
code is present in a normal build**, so there is zero runtime cost unless the feature
is explicitly enabled.

```sh
# Run with debug logging enabled; log goes to /tmp/bc_debug.log by default
BC_DEBUG_LOG=/path/to/my.log cargo run --features bc_debug -- family.ged \
  --preff family.toml --type boxed_couples -o chart.svg
```

The `BC_DEBUG_LOG` environment variable controls the output file path. If unset, the
log is written to `/tmp/bc_debug.log`. A confirmation line is printed to stderr when the
feature is active:

```
bc_debug: logging to /tmp/bc_debug.log
```

## Log format

The log is a CSV file with one header row followed by one row per placement event:

```
op,id,x_before,x_after,dx,generation,source
PLACE,I1511,,7585.0,7585.0,2,src/layout/boxed_couples.rs:1069
SHIFT,I1580,7290.0,7420.0,130.0,4,src/layout/boxed_couples.rs:407/compact
RECENTER,I1513,7585.0,7650.0,65.0,3,src/layout/boxed_couples.rs:612
SHIFT,I1501,7805.0,7870.0,65.0,2,src/layout/boxed_couples.rs:407/fix_overlap
```

| Column | Meaning |
|--------|---------|
| `op` | Operation type: `PLACE`, `SHIFT`, or `RECENTER` |
| `id` | Individual ID (e.g. `I1511`) |
| `x_before` | x coordinate before the operation (empty for `PLACE`) |
| `x_after` | x coordinate after the operation |
| `dx` | Change in x (empty for `PLACE`) |
| `generation` | Generation depth from the root (0 = root) |
| `source` | Source file and line, plus caller context for `SHIFT` |

The `source` field for `SHIFT` events appends a slash-separated context label that
identifies which caller triggered the shift:

| Context label | Cause |
|---|---|
| `place/align` | `place_descendants` shifted a child subtree so the parent can sit at `x_default` |
| `compact` | `compact_siblings` closed an excess gap between siblings |
| `fix_overlap` | `fix_overlaps_pass` resolved a cross-family overlap |

## Diagnostic workflow

A typical session for diagnosing a placement overlap:

```sh
# 1. Generate the chart with logging
BC_DEBUG_LOG=/tmp/bug.log cargo run --features bc_debug -- family.ged \
  --preff family.toml -o /tmp/chart.svg

# 2. Identify overlapping individuals
#    (compare each generation row's box x-ranges in the SVG; any pair whose
#     [x, x+width] intervals intersect on the same row is an overlap)

# 3. Trace the placement history of both individuals
grep "I1511\|I1501" /tmp/bug.log

# 4. Follow the sequence of operations to find the root cause
#    Look for a RECENTER that moves a node rightward after its right
#    sibling was already PLACED at a position that assumed the old x.
```

The combination of the debug log (which shows *when* and *how much* each node moved)
and an inspection of the final SVG box positions (which reveals any overlaps) lets
you pinpoint the exact pass and the exact node that caused the problem without
re-running any code.
