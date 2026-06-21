# Realistic tree branches (experimental)

> ⚠️ **Experimental and a work in progress.** The output is not yet satisfactory and the
> algorithm is expected to change substantially. Treat this feature as a preview; the
> styles, defaults, and configuration keys may change without notice.

This feature applies only to the [`boxed_couples`](../README.md#boxed_couples) layout.
When `layout.root_pos = "bottom"` (the default), you can replace the straight connector
lines with organic-looking tree branches via `output.style.realistic_tree.enabled = true`.
Four rendering styles are available:

| Style | Description |
|---|---|
| `"tapered"` (default) | Filled closed Bézier paths; branch width decreases globally from root to tips |
| `"stroke"` | Layered stroked S-curve Bézier paths with opacity-based taper |
| `"filter"` | Thick rounded paths with a white highlight for a cylindrical 3D look |
| `"ink"` | Hand-drawn coherent tree, modelled on a hand-drawn reference: a flared trunk that shows below the root box, organic buttress roots, a continuous flat-topped open-ellipse leaf canopy, and each child connected by a single continuous tapered branch that runs along the main limb and turns up under its box. Trunk and limbs carry short white bark scratches with a lit side |

```sh
# Tapered style with medium leaf density
genechart family.ged -r I1 --type boxed_couples \
  --pref 'output.style.realistic_tree.enabled = true' \
  --pref 'output.style.realistic_tree.style = "tapered"' \
  -o chart.svg

# ink style — hand-drawn tree look
genechart family.ged -r I1 --type boxed_couples \
  --pref 'output.style.realistic_tree.enabled = true' \
  --pref 'output.style.realistic_tree.style = "ink"' \
  -o chart.svg
```

Configuration: `[output.style.realistic_tree]` — `enabled` (bool), `style` (`"tapered"` | `"stroke"` | `"filter"` | `"ink"`), `trunk_color` (hex), `leaf_color` (hex), `leaf_density` (`"none"` | `"low"` | `"medium"` | `"high"`).

The generated SVG carries CSS classes `realistic-tree` (the layer `<g>`), `tree-branch`
(branch paths/lines), and `tree-leaf` (leaf shapes) so the tree can be restyled without
changing TOML.
