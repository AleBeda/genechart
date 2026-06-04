//! Realistic tree branch rendering for the boxed_couples layout.
//!
//! This module generates an SVG background layer of organic tree branches/trunk that
//! replaces the default straight connectors. Boxes are rendered on top by the caller.
//!
//! Three style variants are available, selectable via `output.style.realistic_tree.style`:
//!   "tapered" — filled closed Bézier paths, width globally decreasing from root to tips (default)
//!   "stroke"  — layered stroked Bézier S-curves with global width taper
//!   "filter"  — thick rounded paths with an SVG feTurbulence/feDisplacementMap filter for texture

use crate::preferences::Prefs;
use crate::scene::{ConnectorPrimitive, Primitive};

// ── Public API ────────────────────────────────────────────────────────────────

/// Recursively collect all `ConnectorPrimitive` references from a primitive tree.
pub fn collect_connectors<'a>(primitives: &'a [Primitive], out: &mut Vec<&'a ConnectorPrimitive>) {
    for prim in primitives {
        match prim {
            Primitive::Connector(c) => out.push(c),
            Primitive::Group(g) => collect_connectors(&g.children, out),
            _ => {}
        }
    }
}

/// Extra canvas height (display-space units) to add below the SVG for tree roots.
///
/// Only non-zero for root_pos = bottom charts (the default). Returns 0.0 for empty
/// connector lists or when parent points are above child points (root_pos = top).
pub fn root_extra_height(connectors: &[&ConnectorPrimitive]) -> f64 {
    if connectors.is_empty() {
        return 0.0;
    }
    // Detect root_pos_bottom: parent attachment points sit below (larger SVG Y) child points.
    let sample_parent_y = connectors[0]
        .parent_points
        .first()
        .map(|p| p.y)
        .unwrap_or(0.0);
    let sample_child_y = connectors[0]
        .child_points
        .first()
        .map(|p| p.y)
        .unwrap_or(0.0);
    if sample_parent_y <= sample_child_y {
        return 0.0; // root_pos = top — roots would extend above chart, skip
    }
    let y_root: f64 = connectors
        .iter()
        .flat_map(|c| c.parent_points.iter().map(|p| p.y))
        .fold(f64::NEG_INFINITY, f64::max);
    let y_top: f64 = connectors
        .iter()
        .flat_map(|c| c.child_points.iter().map(|p| p.y))
        .fold(f64::INFINITY, f64::min);
    ((y_root - y_top) * 0.22).max(40.0)
}

/// Render the full tree-branch SVG layer.
///
/// Returns an SVG fragment (no outer `<svg>` tag) wrapped in
/// `<g id="realistic-tree" class="realistic-tree">…</g>`.
///
/// `to_svg_x`/`to_svg_y` are the same display→SVG coordinate transforms used in `svg.rs`.
pub fn render_tree_layer(
    connectors: &[&ConnectorPrimitive],
    to_svg_x: &dyn Fn(f64) -> f64,
    to_svg_y: &dyn Fn(f64) -> f64,
    prefs: &Prefs,
) -> String {
    if connectors.is_empty() {
        return String::new();
    }

    let branches: Vec<Branch> = connectors
        .iter()
        .map(|c| Branch {
            parent_pts: c
                .parent_points
                .iter()
                .map(|p| (to_svg_x(p.x), to_svg_y(p.y)))
                .collect(),
            child_pts: c
                .child_points
                .iter()
                .map(|p| (to_svg_x(p.x), to_svg_y(p.y)))
                .collect(),
        })
        .collect();

    match prefs.output.style.realistic_tree.style.as_str() {
        "stroke" => {
            let inner = render_stroke_style(&branches, prefs);
            format!("<g id=\"realistic-tree\" class=\"realistic-tree\">\n{inner}</g>\n")
        }
        "filter" => {
            // Emit <defs> before the group so the filter ID is always resolvable.
            let (defs, inner) = render_filter_style(&branches, prefs);
            format!("{defs}<g id=\"realistic-tree\" class=\"realistic-tree\">\n{inner}</g>\n")
        }
        _ => {
            let inner = render_tapered_style(&branches, prefs);
            format!("<g id=\"realistic-tree\" class=\"realistic-tree\">\n{inner}</g>\n")
        }
    }
}

// ── Internal types ────────────────────────────────────────────────────────────

struct Branch {
    parent_pts: Vec<(f64, f64)>,
    child_pts: Vec<(f64, f64)>,
}

// ── Shared geometry helpers ────────────────────────────────────────────────────

/// Width interpolated at SVG Y coordinate.
/// Returns `max_w` at `y_root` (large Y, near root) and `min_w` at `y_top` (small Y, near tips).
fn width_at(y: f64, y_top: f64, y_range: f64, max_w: f64, min_w: f64) -> f64 {
    let t = ((y - y_top) / y_range).clamp(0.0, 1.0);
    min_w + (max_w - min_w) * t
}

/// Maximum parent Y and minimum child Y across all branches (SVG space).
fn y_bounds(branches: &[Branch]) -> (f64, f64) {
    let y_root = branches
        .iter()
        .flat_map(|b| b.parent_pts.iter().map(|p| p.1))
        .fold(f64::NEG_INFINITY, f64::max);
    let y_top = branches
        .iter()
        .flat_map(|b| b.child_pts.iter().map(|p| p.1))
        .fold(f64::INFINITY, f64::min);
    (y_root, y_top)
}

/// X coordinate of the root-level branch parent attachment (the branch with the largest parent Y).
fn root_center_x(branches: &[Branch], _y_root: f64) -> f64 {
    branches
        .iter()
        .filter(|b| !b.parent_pts.is_empty())
        .max_by(|a, bb| {
            let ya = a
                .parent_pts
                .iter()
                .map(|p| p.1)
                .fold(f64::NEG_INFINITY, f64::max);
            let yb = bb
                .parent_pts
                .iter()
                .map(|p| p.1)
                .fold(f64::NEG_INFINITY, f64::max);
            ya.partial_cmp(&yb).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|b| b.parent_pts.iter().map(|p| p.0).sum::<f64>() / b.parent_pts.len() as f64)
        .unwrap_or_else(|| branches[0].parent_pts[0].0)
}

// ── Style: tapered ────────────────────────────────────────────────────────────

fn render_tapered_style(branches: &[Branch], prefs: &Prefs) -> String {
    let trunk_color = format!("#{:06X}", prefs.output.style.realistic_tree.trunk_color);
    let leaf_color = format!("#{:06X}", prefs.output.style.realistic_tree.leaf_color);
    let leaf_count: usize = match prefs.output.style.realistic_tree.leaf_density.as_str() {
        "none" => 0,
        "low" => 5,
        "high" => 25,
        _ => 12, // "medium"
    };

    let (y_root, y_top) = y_bounds(branches);
    let y_range = (y_root - y_top).max(1.0);
    const MAX_HW: f64 = 9.0;
    const MIN_HW: f64 = 1.0;

    let mut out = String::new();

    // Tree roots below the root box (root_pos_bottom only)
    if y_root > y_top {
        let rx = root_center_x(branches, y_root);
        let root_depth = y_range * 0.22;
        out.push_str(&tapered_roots(
            rx,
            y_root,
            root_depth,
            MAX_HW,
            MIN_HW,
            &trunk_color,
        ));
    }

    for branch in branches {
        if branch.parent_pts.is_empty() || branch.child_pts.is_empty() {
            continue;
        }

        let px =
            branch.parent_pts.iter().map(|p| p.0).sum::<f64>() / branch.parent_pts.len() as f64;
        let py =
            branch.parent_pts.iter().map(|p| p.1).sum::<f64>() / branch.parent_pts.len() as f64;
        let mean_cy =
            branch.child_pts.iter().map(|p| p.1).sum::<f64>() / branch.child_pts.len() as f64;
        let bar_y = (py + mean_cy) / 2.0;

        // Trunk from parent attachment to bar, width driven globally by Y position
        let w_py = width_at(py, y_top, y_range, MAX_HW, MIN_HW);
        let w_bar = width_at(bar_y, y_top, y_range, MAX_HW, MIN_HW);
        out.push_str(&tapered_branch_path(
            px,
            py,
            px,
            bar_y,
            w_py,
            w_bar,
            &trunk_color,
        ));

        // Sub-branches from bar to each child
        for &(cx, cy) in &branch.child_pts {
            let w_cy = width_at(cy, y_top, y_range, MAX_HW, MIN_HW);
            out.push_str(&tapered_branch_path(
                px,
                bar_y,
                cx,
                cy,
                w_bar,
                w_cy,
                &trunk_color,
            ));
        }

        // Leaf clusters at child tips
        if leaf_count > 0 {
            for &(cx, cy) in &branch.child_pts {
                out.push_str(&tapered_leaf_cluster(cx, cy, leaf_count, &leaf_color));
            }
        }
    }

    out
}

/// Four root branches spreading downward from the root box attachment.
fn tapered_roots(
    root_x: f64,
    y_root: f64,
    root_depth: f64,
    max_hw: f64,
    min_hw: f64,
    color: &str,
) -> String {
    let junction_y = y_root + root_depth * 0.55;
    let junction_hw = max_hw;

    // Short vertical trunk from root box down to junction
    let mut out = tapered_branch_path(
        root_x,
        y_root,
        root_x,
        junction_y,
        junction_hw,
        junction_hw * 0.88,
        color,
    );

    // Four root branches fanning outward and downward
    let tips: [(f64, f64, f64); 4] = [
        (root_x - root_depth * 0.48, y_root + root_depth * 0.85, 0.28),
        (root_x - root_depth * 0.20, y_root + root_depth * 0.95, 0.38),
        (root_x + root_depth * 0.20, y_root + root_depth * 0.95, 0.38),
        (root_x + root_depth * 0.48, y_root + root_depth * 0.85, 0.28),
    ];
    for (ex, ey, end_scale) in tips {
        let end_hw = min_hw + (junction_hw * 0.88 - min_hw) * end_scale;
        out.push_str(&tapered_branch_path(
            root_x,
            junction_y,
            ex,
            ey,
            junction_hw * 0.88,
            end_hw,
            color,
        ));
    }
    out
}

/// Filled closed Bézier path for one branch segment.
/// Width is measured perpendicular to the branch direction, so diagonal and horizontal
/// branches appear as wide as vertical ones at the same `w` value.
fn tapered_branch_path(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    w1: f64,
    w2: f64,
    color: &str,
) -> String {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = dx.hypot(dy).max(0.01);
    // Perpendicular unit normal (90° CCW from travel direction)
    let nx = -dy / len;
    let ny = dx / len;
    // Four outline corners
    let (ax, ay) = (x1 + nx * w1, y1 + ny * w1);
    let (bx, by) = (x1 - nx * w1, y1 - ny * w1);
    let (cpx, cpy) = (x2 + nx * w2, y2 + ny * w2);
    let (ex, ey) = (x2 - nx * w2, y2 - ny * w2);
    // Bézier control offset: 40% along travel direction for smooth taper
    let (cdx, cdy) = (dx * 0.4, dy * 0.4);
    format!(
        "  <path d=\"M {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2} \
         L {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2} Z\" \
         fill=\"{}\" class=\"tree-branch\"/>\n",
        ax,
        ay,
        ax + cdx,
        ay + cdy,
        cpx - cdx,
        cpy - cdy,
        cpx,
        cpy,
        ex,
        ey,
        ex - cdx,
        ey - cdy,
        bx + cdx,
        by + cdy,
        bx,
        by,
        color
    )
}

fn tapered_leaf_cluster(cx: f64, cy: f64, count: usize, color: &str) -> String {
    let mut seed = (cx * 1000.0) as u64 ^ (cy * 1000.0) as u64;
    let mut out = String::new();
    for _ in 0..count {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let angle = (seed & 0xFFFF) as f64 / 65535.0 * std::f64::consts::TAU;
        let radius = ((seed >> 16) & 0xFFFF) as f64 / 65535.0 * 20.0 + 5.0;
        let ex = cx + angle.cos() * radius;
        let ey = cy + angle.sin() * radius * 0.6;
        out.push_str(&format!(
            "  <ellipse cx=\"{:.2}\" cy=\"{:.2}\" rx=\"4\" ry=\"2.5\" \
             fill=\"{}\" opacity=\"0.7\" class=\"tree-leaf\"/>\n",
            ex, ey, color
        ));
    }
    out
}

// ── Style: stroke ─────────────────────────────────────────────────────────────

fn render_stroke_style(branches: &[Branch], prefs: &Prefs) -> String {
    let trunk_color = format!("#{:06X}", prefs.output.style.realistic_tree.trunk_color);
    let leaf_color = format!("#{:06X}", prefs.output.style.realistic_tree.leaf_color);
    let leaf_count: usize = match prefs.output.style.realistic_tree.leaf_density.as_str() {
        "none" => 0,
        "low" => 4,
        "high" => 22,
        _ => 10, // "medium"
    };

    let (y_root, y_top) = y_bounds(branches);
    let y_range = (y_root - y_top).max(1.0);
    const MAX_SW: f64 = 14.0;
    const MIN_SW: f64 = 2.0;

    let mut out = String::new();

    // Tree roots below the root box (root_pos_bottom only)
    if y_root > y_top {
        let rx = root_center_x(branches, y_root);
        let root_depth = y_range * 0.22;
        out.push_str(&stroke_roots(rx, y_root, root_depth, MAX_SW, &trunk_color));
    }

    for branch in branches {
        if branch.parent_pts.is_empty() || branch.child_pts.is_empty() {
            continue;
        }

        let px =
            branch.parent_pts.iter().map(|p| p.0).sum::<f64>() / branch.parent_pts.len() as f64;
        let py =
            branch.parent_pts.iter().map(|p| p.1).sum::<f64>() / branch.parent_pts.len() as f64;
        let mean_cy =
            branch.child_pts.iter().map(|p| p.1).sum::<f64>() / branch.child_pts.len() as f64;
        let bar_y = (py + mean_cy) / 2.0;

        // Trunk from parent to bar_y
        let sw_py = width_at(py, y_top, y_range, MAX_SW, MIN_SW);
        out.push_str(&stroke_bezier_layers(
            px,
            py,
            px,
            bar_y,
            sw_py,
            0.0,
            &trunk_color,
        ));

        // Sub-branches from bar_y to each child
        let sw_bar = width_at(bar_y, y_top, y_range, MAX_SW, MIN_SW);
        for &(cx, cy) in &branch.child_pts {
            let lateral = (cx - px) * 0.15;
            out.push_str(&stroke_bezier_layers(
                px,
                bar_y,
                cx,
                cy,
                sw_bar,
                lateral,
                &trunk_color,
            ));
        }

        // Leaf clusters at child tips
        if leaf_count > 0 {
            for &(cx, cy) in &branch.child_pts {
                out.push_str(&stroke_leaf_cluster(cx, cy, leaf_count, &leaf_color));
            }
        }
    }

    out
}

/// Four root branches spreading downward from the root box, stroke style.
fn stroke_roots(root_x: f64, y_root: f64, root_depth: f64, max_sw: f64, color: &str) -> String {
    let junction_y = y_root + root_depth * 0.55;

    // Short vertical trunk from root box down to junction
    let mut out = stroke_bezier_layers(root_x, y_root, root_x, junction_y, max_sw, 0.0, color);

    // Four root branches fanning outward and downward
    let tips: [(f64, f64, f64); 4] = [
        (
            root_x - root_depth * 0.48,
            y_root + root_depth * 0.85,
            max_sw * 0.50,
        ),
        (
            root_x - root_depth * 0.20,
            y_root + root_depth * 0.95,
            max_sw * 0.65,
        ),
        (
            root_x + root_depth * 0.20,
            y_root + root_depth * 0.95,
            max_sw * 0.65,
        ),
        (
            root_x + root_depth * 0.48,
            y_root + root_depth * 0.85,
            max_sw * 0.50,
        ),
    ];
    for (ex, ey, sw) in tips {
        let lateral = (ex - root_x) * 0.12;
        out.push_str(&stroke_bezier_layers(
            root_x, junction_y, ex, ey, sw, lateral, color,
        ));
    }
    out
}

/// Three overlapping stroked cubic Bézier `<path>` elements (thick→thin, low→high opacity)
/// to simulate a tapered organic branch. `lateral_offset = 0.0` uses `dy * 0.08` S-bow.
fn stroke_bezier_layers(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    base_sw: f64,
    lateral_offset: f64,
    color: &str,
) -> String {
    let dy = y1 - y2;
    let lat = if lateral_offset == 0.0 {
        dy * 0.08
    } else {
        lateral_offset
    };
    let cx1 = x1 + lat;
    let cy1 = y1 - dy * 0.35;
    let cx2 = x2 - lat;
    let cy2 = y2 + dy * 0.35;
    let d = format!(
        "M {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2}",
        x1, y1, cx1, cy1, cx2, cy2, x2, y2
    );
    format!(
        "  <path d=\"{d}\" stroke=\"{c}\" stroke-width=\"{:.2}\" opacity=\"0.35\" fill=\"none\" class=\"tree-branch\"/>\n\
           <path d=\"{d}\" stroke=\"{c}\" stroke-width=\"{:.2}\" opacity=\"0.55\" fill=\"none\" class=\"tree-branch\"/>\n\
           <path d=\"{d}\" stroke=\"{c}\" stroke-width=\"{:.2}\" opacity=\"0.85\" fill=\"none\" class=\"tree-branch\"/>\n",
        base_sw,
        base_sw * 0.6,
        base_sw * 0.3,
        c = color
    )
}

fn stroke_leaf_cluster(cx: f64, cy: f64, count: usize, color: &str) -> String {
    let mut seed = (cx * 1000.0) as u64 ^ (cy * 1000.0) as u64;
    let mut out = String::new();
    for _ in 0..count {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let angle = (seed & 0xFFFF) as f64 / 65535.0 * std::f64::consts::TAU;
        let radius = ((seed >> 16) & 0xFFFF) as f64 / 65535.0 * 18.0 + 4.0;
        let lx = cx + angle.cos() * radius;
        let ly = cy + angle.sin() * radius;
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let rx = ((seed & 0xFF) as f64 / 255.0) * 3.0 + 2.0;
        let ry = rx * 1.8;
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let rot_deg = (seed & 0xFFFF) as f64 / 65535.0 * 360.0;
        out.push_str(&format!(
            "  <path d=\"M {:.2},{:.2} Q {:.2},{:.2} {:.2},{:.2} Q {:.2},{:.2} {:.2},{:.2}\" \
             fill=\"{}\" opacity=\"0.75\" transform=\"rotate({:.1},{:.2},{:.2})\" \
             class=\"tree-leaf\"/>\n",
            lx,
            ly,
            lx + rx,
            ly - ry,
            lx,
            ly - ry * 2.0,
            lx - rx,
            ly - ry,
            lx,
            ly,
            color,
            rot_deg,
            lx,
            ly
        ));
    }
    out
}

// ── Style: filter ─────────────────────────────────────────────────────────────

fn render_filter_style(branches: &[Branch], prefs: &Prefs) -> (String, String) {
    if branches.is_empty() {
        return (String::new(), String::new());
    }

    let trunk_color = format!("#{:06X}", prefs.output.style.realistic_tree.trunk_color);
    let leaf_color = format!("#{:06X}", prefs.output.style.realistic_tree.leaf_color);
    let leaf_count: usize = match prefs.output.style.realistic_tree.leaf_density.as_str() {
        "none" => 0,
        "low" => 6,
        "high" => 28,
        _ => 14, // "medium"
    };

    let (y_root, y_top) = y_bounds(branches);
    let y_range = (y_root - y_top).max(1.0);
    const MAX_SW: f64 = 16.0;
    const MIN_SW: f64 = 3.0;

    let filter_def = concat!(
        "<defs>\n",
        "  <filter id=\"bark-texture\" x=\"-10%\" y=\"-10%\" width=\"120%\" height=\"120%\">\n",
        "    <feTurbulence type=\"fractalNoise\" baseFrequency=\"0.035 0.018\" \
         numOctaves=\"4\" seed=\"42\" result=\"noise\"/>\n",
        "    <feDisplacementMap in=\"SourceGraphic\" in2=\"noise\" scale=\"5\" \
         xChannelSelector=\"R\" yChannelSelector=\"G\"/>\n",
        "  </filter>\n",
        "</defs>\n",
    );

    let mut filter_body = String::new();
    let mut leaves = String::new();

    // Tree roots inside the filter group (root_pos_bottom only)
    if y_root > y_top {
        let rx = root_center_x(branches, y_root);
        let root_depth = y_range * 0.22;
        filter_body.push_str(&filter_roots(rx, y_root, root_depth, MAX_SW, &trunk_color));
    }

    for branch in branches {
        if branch.parent_pts.is_empty() || branch.child_pts.is_empty() {
            continue;
        }

        let px =
            branch.parent_pts.iter().map(|p| p.0).sum::<f64>() / branch.parent_pts.len() as f64;
        let py =
            branch.parent_pts.iter().map(|p| p.1).sum::<f64>() / branch.parent_pts.len() as f64;
        let mean_cy =
            branch.child_pts.iter().map(|p| p.1).sum::<f64>() / branch.child_pts.len() as f64;
        let bar_y = (py + mean_cy) / 2.0;

        // Trunk
        let trunk_sw = width_at(py, y_top, y_range, MAX_SW, MIN_SW);
        let dy_trunk = py - bar_y;
        filter_body.push_str(&filter_trunk_path(
            px,
            py,
            bar_y,
            dy_trunk,
            trunk_sw,
            &trunk_color,
        ));
        filter_body.push_str(&filter_highlight_path(px, py, bar_y, dy_trunk, trunk_sw));

        // Sub-branches
        let sub_sw = width_at(bar_y, y_top, y_range, MAX_SW, MIN_SW);
        for &(cx, cy) in &branch.child_pts {
            filter_body.push_str(&filter_sub_path(px, bar_y, cx, cy, sub_sw, &trunk_color));
        }

        // Leaf clusters (outside filter group so leaves aren't displaced)
        if leaf_count > 0 {
            for &(cx, cy) in &branch.child_pts {
                leaves.push_str(&filter_leaf_cluster(cx, cy, leaf_count, &leaf_color));
            }
        }
    }

    let body = format!("<g filter=\"url(#bark-texture)\">\n{filter_body}</g>\n{leaves}");
    (filter_def.to_string(), body)
}

/// Four root branches inside the filter group, spreading downward.
fn filter_roots(root_x: f64, y_root: f64, root_depth: f64, max_sw: f64, color: &str) -> String {
    let junction_y = y_root + root_depth * 0.55;
    let dy_trunk = y_root - junction_y; // negative (going down)

    // Short vertical trunk from root box down to junction
    let mut out = filter_trunk_path(root_x, y_root, junction_y, dy_trunk, max_sw, color);
    out.push_str(&filter_highlight_path(
        root_x, y_root, junction_y, dy_trunk, max_sw,
    ));

    // Four root branches fanning outward and downward
    let tips: [(f64, f64, f64); 4] = [
        (
            root_x - root_depth * 0.48,
            y_root + root_depth * 0.85,
            max_sw * 0.55,
        ),
        (
            root_x - root_depth * 0.20,
            y_root + root_depth * 0.95,
            max_sw * 0.70,
        ),
        (
            root_x + root_depth * 0.20,
            y_root + root_depth * 0.95,
            max_sw * 0.70,
        ),
        (
            root_x + root_depth * 0.48,
            y_root + root_depth * 0.85,
            max_sw * 0.55,
        ),
    ];
    for (ex, ey, sw) in tips {
        out.push_str(&filter_sub_path(root_x, junction_y, ex, ey, sw, color));
    }
    out
}

fn filter_trunk_path(px: f64, py: f64, bar_y: f64, dy: f64, sw: f64, color: &str) -> String {
    let qcx = px + dy * 0.05;
    let qcy = bar_y + (py - bar_y) * 0.5;
    format!(
        "  <path d=\"M {:.2},{:.2} Q {:.2},{:.2} {:.2},{:.2}\" \
         stroke=\"{}\" stroke-width=\"{:.2}\" stroke-linecap=\"round\" \
         stroke-linejoin=\"round\" fill=\"none\" class=\"tree-branch\"/>\n",
        px, py, qcx, qcy, px, bar_y, color, sw
    )
}

fn filter_highlight_path(px: f64, py: f64, bar_y: f64, dy: f64, trunk_sw: f64) -> String {
    let qcx = px + dy * 0.05;
    let qcy = bar_y + (py - bar_y) * 0.5;
    format!(
        "  <path d=\"M {:.2},{:.2} Q {:.2},{:.2} {:.2},{:.2}\" \
         stroke=\"white\" stroke-width=\"{:.2}\" stroke-linecap=\"round\" \
         stroke-linejoin=\"round\" fill=\"none\" opacity=\"0.15\"/>\n",
        px,
        py,
        qcx,
        qcy,
        px,
        bar_y,
        trunk_sw * 0.25
    )
}

fn filter_sub_path(px: f64, bar_y: f64, cx: f64, cy: f64, sw: f64, color: &str) -> String {
    let qcx = cx * 0.4 + px * 0.6;
    let qcy = bar_y + (cy - bar_y) * 0.4;
    format!(
        "  <path d=\"M {:.2},{:.2} Q {:.2},{:.2} {:.2},{:.2}\" \
         stroke=\"{}\" stroke-width=\"{:.2}\" stroke-linecap=\"round\" \
         stroke-linejoin=\"round\" fill=\"none\" class=\"tree-branch\"/>\n",
        px, bar_y, qcx, qcy, cx, cy, color, sw
    )
}

fn filter_leaf_cluster(cx: f64, cy: f64, count: usize, color: &str) -> String {
    let mut seed = (cx * 1000.0) as u64 ^ (cy * 1000.0) as u64;
    let mut out = String::new();
    for _ in 0..count {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let angle = (seed & 0xFFFF) as f64 / 65535.0 * std::f64::consts::TAU;
        let dist = ((seed >> 16) & 0xFF) as f64 / 255.0 * 22.0;
        let lx = cx + angle.cos() * dist;
        let ly = cy + angle.sin() * dist;
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let radius = ((seed >> 24) & 0xF) as f64 / 15.0 * 3.5 + 1.5;
        out.push_str(&format!(
            "  <circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"{:.2}\" \
             fill=\"{}\" opacity=\"0.65\" class=\"tree-leaf\"/>\n",
            lx, ly, radius, color
        ));
    }
    out
}
