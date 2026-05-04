# Fix Missing Horizontal Connectors in boxed_couples Layout

When children are pushed far away from their parents due to placement constraints, the horizontal segment of the connector that should link the parent's vertical drop to the children's crossbar is sometimes missing. This happens because the crossbar's horizontal extent is currently calculated using only the children's coordinates, neglecting the parent's exit point.

## 1. Analyze `src/backend/svg.rs`
Locate the `render_boxed_couples` function and identify the section where family connectors are drawn.

## 2. Identify the Spouse-Specific Connection Groups
A family in the `boxed_couples` layout may have children associated with up to two spouses. These are rendered as two distinct connector groups:
- **Group 1**: Children connected to `FamilyGeo.conn_out1_x`.
- **Group 2**: Children connected to `FamilyGeo.conn_out2_x`.

## 3. Fix Horizontal Crossbar Bounds
For each group of children, the horizontal crossbar must span the entire width required to connect all children AND the parent's vertical exit line.

For each spouse's group of children:
1.  Determine the parent's exit point `parent_x`:
    - For Group 1: `fam_geo.conn_out1_x`.
    - For Group 2: `fam_geo.conn_out2_x`.
2.  Find the minimum and maximum X coordinates among the `conn_in_x` points of all children in that specific group.
3.  Calculate the corrected horizontal bounds for the crossbar:
    - `start_x = parent_x.min(min_child_x)`
    - `end_x = parent_x.max(max_child_x)`
4.  Draw the horizontal line at the established midpoint Y using these corrected `start_x` and `end_x` values.

This ensures that if the parent is located to the left or right of the entire children's block, the horizontal crossbar will correctly extend to meet the parent's vertical line.

## 4. Verification
- Run `cargo build` and `cargo test`.
- Test specifically with `tests/fixtures/local/bedarida.ged` (root `I131`). Ensure that horizontal connectors for individuals like I174 are clearly visible and connected.
- Manually inspect the generated SVG.

## Implementation Notes
- Use `f64::min` and `f64::max` to determine the bounds.
- If a group has only one child, the code must still draw a horizontal segment if the child's `conn_in_x` is not equal to the `parent_x`.
- The existing `svg_line` helper should be used for drawing.
- Midpoint Y calculation remains unchanged as it is already correct.

---
**Restriction:** Do not modify the placement logic in `src/layout/boxed_couples.rs`. This fix must be applied only to the rendering logic in `src/backend/svg.rs`.