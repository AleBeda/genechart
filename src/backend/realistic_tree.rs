//! Realistic tree branch rendering for the boxed_couples layout.
//!
//! Generates an SVG background layer of organic tree branches/trunk that
//! replaces the default straight connectors. Boxes are rendered on top by the caller.
//!
//! Four style variants (selectable via `output.style.realistic_tree.style`):
//!   "tapered" — filled closed Bézier paths, width globally decreasing from root to tips (default)
//!   "stroke"  — layered stroked Bézier paths with global width taper
//!   "filter"  — two-layer filled paths with a white cylindrical highlight for a 3D rounded look
//!   "ink"     — near-solid black filled branches with white longitudinal bark-scratch strokes;
//!               hollow ellipse outlines for leaves (ink-drawing aesthetic)

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
        return 0.0;
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

    let inner = match prefs.output.style.realistic_tree.style.as_str() {
        "stroke" => render_stroke_style(&branches, prefs),
        "filter" => render_filter_style(&branches, prefs),
        "ink" => render_ink_style(&branches, prefs),
        _ => render_tapered_style(&branches, prefs),
    };
    format!("<g id=\"realistic-tree\" class=\"realistic-tree\">\n{inner}</g>\n")
}

// ── Internal types ────────────────────────────────────────────────────────────

struct Branch {
    parent_pts: Vec<(f64, f64)>,
    child_pts: Vec<(f64, f64)>,
}

// ── Shared geometry helpers ───────────────────────────────────────────────────

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

/// X of the root-level branch parent (the branch with the largest parent Y).
fn root_center_x(branches: &[Branch], _y_root: f64) -> f64 {
    branches
        .iter()
        .filter(|b| !b.parent_pts.is_empty())
        .max_by(|a, b| {
            let ya = a
                .parent_pts
                .iter()
                .map(|p| p.1)
                .fold(f64::NEG_INFINITY, f64::max);
            let yb = b
                .parent_pts
                .iter()
                .map(|p| p.1)
                .fold(f64::NEG_INFINITY, f64::max);
            ya.partial_cmp(&yb).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|b| b.parent_pts.iter().map(|p| p.0).sum::<f64>() / b.parent_pts.len() as f64)
        .unwrap_or_else(|| branches[0].parent_pts[0].0)
}

/// SVG `d` attribute for a tapered filled Bézier outline.
/// Width is measured perpendicular to the travel direction so diagonal/horizontal
/// branches look as wide as vertical ones at the same `w` value.
fn build_tapered_d(x1: f64, y1: f64, x2: f64, y2: f64, w1: f64, w2: f64) -> String {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = dx.hypot(dy).max(0.01);
    let nx = -dy / len;
    let ny = dx / len;
    let (ax, ay) = (x1 + nx * w1, y1 + ny * w1);
    let (bx, by) = (x1 - nx * w1, y1 - ny * w1);
    let (cpx, cpy) = (x2 + nx * w2, y2 + ny * w2);
    let (ex, ey) = (x2 - nx * w2, y2 - ny * w2);
    let (cdx, cdy) = (dx * 0.4, dy * 0.4);
    format!(
        "M {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2} \
         L {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2} Z",
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
        by
    )
}

fn tapered_branch_path(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    w1: f64,
    w2: f64,
    color: &str,
) -> String {
    let d = build_tapered_d(x1, y1, x2, y2, w1, w2);
    format!("  <path d=\"{d}\" fill=\"{color}\" class=\"tree-branch\"/>\n")
}

/// White cylindrical highlight stripe (25 % width, 20 % opacity) for the filter style.
fn tapered_branch_highlight(x1: f64, y1: f64, x2: f64, y2: f64, w1: f64, w2: f64) -> String {
    let d = build_tapered_d(x1, y1, x2, y2, w1 * 0.25, w2 * 0.25);
    format!("  <path d=\"{d}\" fill=\"white\" opacity=\"0.20\" class=\"tree-branch\"/>\n")
}

/// Leaf ellipses scattered in an oval around (cx, cy).
/// `seed_off` lets callers produce distinct patterns at the same coordinate.
fn leaf_cluster(cx: f64, cy: f64, count: usize, color: &str, seed_off: u64) -> String {
    let mut seed = ((cx * 1000.0) as u64 ^ (cy * 1000.0) as u64).wrapping_add(seed_off);
    let mut out = String::new();
    for _ in 0..count {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let angle = (seed & 0xFFFF) as f64 / 65535.0 * std::f64::consts::TAU;
        let radius = ((seed >> 16) & 0xFFFF) as f64 / 65535.0 * 28.0 + 6.0;
        let lx = cx + angle.cos() * radius;
        let ly = cy + angle.sin() * radius * 0.55;
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ls = (seed & 0xFF) as f64 / 255.0 * 3.5 + 3.5;
        out.push_str(&format!(
            "  <ellipse cx=\"{lx:.2}\" cy=\"{ly:.2}\" rx=\"{ls:.2}\" ry=\"{:.2}\" \
             fill=\"{color}\" opacity=\"0.72\" class=\"tree-leaf\"/>\n",
            ls * 0.65
        ));
    }
    out
}

/// Three stacked leaf clouds centred at (cx, cy) and above it, creating a canopy.
/// The uppermost cloud extends well above the child box so leaves appear above it.
fn canopy_leaves(cx: f64, cy: f64, count: usize, color: &str) -> String {
    let mut out = String::new();
    out.push_str(&leaf_cluster(cx, cy, count, color, 0));
    out.push_str(&leaf_cluster(cx, cy - 28.0, count * 2 / 3 + 1, color, 1));
    out.push_str(&leaf_cluster(cx, cy - 56.0, count / 3 + 1, color, 2));
    out
}

// ── Horizontal-bar helpers ────────────────────────────────────────────────────

/// Deterministic wave value in –1..+1 derived from a segment's position.
fn segment_wave(x1: f64, y: f64, x2: f64) -> f64 {
    let h = ((x1 + x2) * 63.5 + y * 311.7) as i64 as u64;
    let h = h
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (h & 0xFFFF) as f64 / 65535.0 * 2.0 - 1.0
}

/// SVG `d` for a horizontal bar whose midline bows by ≈ 60 % of `w` (both edges move
/// together so the bar width stays constant; only the endpoints are anchored at `y ± w`).
fn build_bar_d(x1: f64, y: f64, x2: f64, w: f64) -> String {
    let wave_h = segment_wave(x1, y, x2) * w * 0.6;
    let cdx = (x2 - x1) * 0.4;
    format!(
        "M {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2} \
         L {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2} Z",
        x1,
        y + w,
        x1 + cdx,
        y + w + wave_h,
        x2 - cdx,
        y + w + wave_h,
        x2,
        y + w,
        x2,
        y - w,
        x2 - cdx,
        y - w + wave_h,
        x1 + cdx,
        y - w + wave_h,
        x1,
        y - w
    )
}

fn tapered_bar_path(x1: f64, y: f64, x2: f64, w: f64, color: &str) -> String {
    let d = build_bar_d(x1, y, x2, w);
    format!("  <path d=\"{d}\" fill=\"{color}\" class=\"tree-branch\"/>\n")
}

fn tapered_bar_highlight(x1: f64, y: f64, x2: f64, w: f64) -> String {
    let d = build_bar_d(x1, y, x2, w * 0.25);
    format!("  <path d=\"{d}\" fill=\"white\" opacity=\"0.20\" class=\"tree-branch\"/>\n")
}

/// Filled circle that rounds off a T-junction (trunk↔bar or drop↔bar).
fn junction_circle(x: f64, y: f64, r: f64, color: &str) -> String {
    format!(
        "  <circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"{r:.2}\" \
         fill=\"{color}\" class=\"tree-branch\"/>\n"
    )
}

// ── Common per-branch geometry ────────────────────────────────────────────────

/// Compute the horizontal bar position for one branch.
///
/// Layout mirrors the standard SVG connectors:
///   • short vertical trunk from parent attachment down to `bar_y`
///   • horizontal bar at `bar_y` spanning all children and the parent x
///   • vertical drops from the bar down to each child attachment
///
/// `bar_y` is placed 25 % of the way from parent toward children — close
/// enough to the parent box to look like a main branch forking early.
fn branch_bar(py: f64, mean_cy: f64, px: f64, child_pts: &[(f64, f64)]) -> (f64, f64, f64) {
    let bar_y = py - (py - mean_cy) * 0.25;
    let min_cx = child_pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let max_cx = child_pts
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let bar_min_x = min_cx.min(px);
    let bar_max_x = max_cx.max(px);
    (bar_y, bar_min_x, bar_max_x)
}

// ── Cubic Bézier helpers (used by ink style) ─────────────────────────────────

/// Evaluate a cubic Bézier at parameter t ∈ [0, 1].
fn bezier3(
    p0x: f64,
    p0y: f64,
    p1x: f64,
    p1y: f64,
    p2x: f64,
    p2y: f64,
    p3x: f64,
    p3y: f64,
    t: f64,
) -> (f64, f64) {
    let m = 1.0 - t;
    (
        m * m * m * p0x + 3.0 * m * m * t * p1x + 3.0 * m * t * t * p2x + t * t * t * p3x,
        m * m * m * p0y + 3.0 * m * m * t * p1y + 3.0 * m * t * t * p2y + t * t * t * p3y,
    )
}

/// Normalised tangent of a cubic Bézier at t.
fn bezier3_tangent(
    p0x: f64,
    p0y: f64,
    p1x: f64,
    p1y: f64,
    p2x: f64,
    p2y: f64,
    p3x: f64,
    p3y: f64,
    t: f64,
) -> (f64, f64) {
    let m = 1.0 - t;
    let dx = 3.0 * (m * m * (p1x - p0x) + 2.0 * m * t * (p2x - p1x) + t * t * (p3x - p2x));
    let dy = 3.0 * (m * m * (p1y - p0y) + 2.0 * m * t * (p2y - p1y) + t * t * (p3y - p2y));
    let len = dx.hypot(dy).max(0.001);
    (dx / len, dy / len)
}

// ── Style: tapered ────────────────────────────────────────────────────────────

fn render_tapered_style(branches: &[Branch], prefs: &Prefs) -> String {
    let trunk_color = format!("#{:06X}", prefs.output.style.realistic_tree.trunk_color);
    let leaf_color = format!("#{:06X}", prefs.output.style.realistic_tree.leaf_color);
    let leaf_count: usize = match prefs.output.style.realistic_tree.leaf_density.as_str() {
        "none" => 0,
        "low" => 15,
        "high" => 65,
        _ => 35, // "medium"
    };

    let (y_root, y_top) = y_bounds(branches);
    let y_range = (y_root - y_top).max(1.0);
    const MAX_HW: f64 = 9.0;
    const MIN_HW: f64 = 1.0;

    let mut out = String::new();

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

        let (bar_y, bar_min_x, bar_max_x) = branch_bar(py, mean_cy, px, &branch.child_pts);
        let w_py = width_at(py, y_top, y_range, MAX_HW, MIN_HW);
        let w_bar = width_at(bar_y, y_top, y_range, MAX_HW, MIN_HW);

        // Short vertical trunk from parent down to horizontal bar
        out.push_str(&tapered_branch_path(
            px,
            py,
            px,
            bar_y,
            w_py,
            w_bar,
            &trunk_color,
        ));

        // Horizontal bar with organic wave
        if bar_max_x - bar_min_x > 1.0 {
            out.push_str(&tapered_bar_path(
                bar_min_x,
                bar_y,
                bar_max_x,
                w_bar,
                &trunk_color,
            ));
        }
        // Round junction blobs at trunk↔bar and drop↔bar joins
        out.push_str(&junction_circle(px, bar_y, w_bar, &trunk_color));
        for &(cx, _) in &branch.child_pts {
            out.push_str(&junction_circle(cx, bar_y, w_bar, &trunk_color));
        }

        // Vertical drops from bar to each child + canopy leaves
        for &(cx, cy) in &branch.child_pts {
            let w_cy = width_at(cy, y_top, y_range, MAX_HW, MIN_HW);
            out.push_str(&tapered_branch_path(
                cx,
                bar_y,
                cx,
                cy,
                w_bar,
                w_cy,
                &trunk_color,
            ));
            if leaf_count > 0 {
                out.push_str(&canopy_leaves(cx, cy, leaf_count, &leaf_color));
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

    let mut out = tapered_branch_path(
        root_x,
        y_root,
        root_x,
        junction_y,
        junction_hw,
        junction_hw * 0.88,
        color,
    );

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

// ── Style: stroke ─────────────────────────────────────────────────────────────

fn render_stroke_style(branches: &[Branch], prefs: &Prefs) -> String {
    let trunk_color = format!("#{:06X}", prefs.output.style.realistic_tree.trunk_color);
    let leaf_color = format!("#{:06X}", prefs.output.style.realistic_tree.leaf_color);
    let leaf_count: usize = match prefs.output.style.realistic_tree.leaf_density.as_str() {
        "none" => 0,
        "low" => 15,
        "high" => 65,
        _ => 35, // "medium"
    };

    let (y_root, y_top) = y_bounds(branches);
    let y_range = (y_root - y_top).max(1.0);
    const MAX_SW: f64 = 14.0;
    const MIN_SW: f64 = 2.0;

    let mut out = String::new();

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

        let (bar_y, bar_min_x, bar_max_x) = branch_bar(py, mean_cy, px, &branch.child_pts);
        let sw_py = width_at(py, y_top, y_range, MAX_SW, MIN_SW);
        let sw_bar = width_at(bar_y, y_top, y_range, MAX_SW, MIN_SW);

        // Short vertical trunk
        out.push_str(&stroke_bezier_layers(
            px,
            py,
            px,
            bar_y,
            sw_py,
            0.0,
            &trunk_color,
        ));

        // Horizontal bar with organic wave
        if bar_max_x - bar_min_x > 1.0 {
            out.push_str(&stroke_bar_layers(
                bar_min_x,
                bar_y,
                bar_max_x,
                sw_bar,
                &trunk_color,
            ));
        }
        // Round junction blobs at T-joins
        out.push_str(&junction_circle(px, bar_y, sw_bar / 2.0, &trunk_color));
        for &(cx, _) in &branch.child_pts {
            out.push_str(&junction_circle(cx, bar_y, sw_bar / 2.0, &trunk_color));
        }

        // Vertical drops + canopy leaves
        for &(cx, cy) in &branch.child_pts {
            out.push_str(&stroke_bezier_layers(
                cx,
                bar_y,
                cx,
                cy,
                sw_bar,
                0.0,
                &trunk_color,
            ));
            if leaf_count > 0 {
                out.push_str(&canopy_leaves(cx, cy, leaf_count, &leaf_color));
            }
        }
    }

    out
}

/// Four root branches spreading downward from the root box, stroke style.
fn stroke_roots(root_x: f64, y_root: f64, root_depth: f64, max_sw: f64, color: &str) -> String {
    let junction_y = y_root + root_depth * 0.55;

    let mut out = stroke_bezier_layers(root_x, y_root, root_x, junction_y, max_sw, 0.0, color);

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

/// Horizontal bar for stroke style: three overlapping stroked paths with organic wave.
fn stroke_bar_layers(x1: f64, y: f64, x2: f64, sw: f64, color: &str) -> String {
    let wave_h = segment_wave(x1, y, x2) * sw * 0.4;
    let cdx = (x2 - x1) * 0.4;
    let d = format!(
        "M {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2}",
        x1,
        y,
        x1 + cdx,
        y + wave_h,
        x2 - cdx,
        y + wave_h,
        x2,
        y
    );
    format!(
        "  <path d=\"{d}\" stroke=\"{c}\" stroke-width=\"{:.2}\" stroke-linecap=\"round\" \
         opacity=\"0.35\" fill=\"none\" class=\"tree-branch\"/>\n\
           <path d=\"{d}\" stroke=\"{c}\" stroke-width=\"{:.2}\" stroke-linecap=\"round\" \
         opacity=\"0.55\" fill=\"none\" class=\"tree-branch\"/>\n\
           <path d=\"{d}\" stroke=\"{c}\" stroke-width=\"{:.2}\" stroke-linecap=\"round\" \
         opacity=\"0.85\" fill=\"none\" class=\"tree-branch\"/>\n",
        sw,
        sw * 0.6,
        sw * 0.3,
        c = color
    )
}

// ── Style: filter ─────────────────────────────────────────────────────────────
//
// Uses two-layer filled tapered paths (main shape + white highlight) to achieve a
// cylindrical/3D look without relying on SVG filter primitives, which are not
// universally supported by all SVG viewers.

fn render_filter_style(branches: &[Branch], prefs: &Prefs) -> String {
    if branches.is_empty() {
        return String::new();
    }

    let trunk_color = format!("#{:06X}", prefs.output.style.realistic_tree.trunk_color);
    let leaf_color = format!("#{:06X}", prefs.output.style.realistic_tree.leaf_color);
    let leaf_count: usize = match prefs.output.style.realistic_tree.leaf_density.as_str() {
        "none" => 0,
        "low" => 15,
        "high" => 65,
        _ => 35, // "medium"
    };

    let (y_root, y_top) = y_bounds(branches);
    let y_range = (y_root - y_top).max(1.0);
    const MAX_HW: f64 = 11.0;
    const MIN_HW: f64 = 1.5;

    let mut out = String::new();

    if y_root > y_top {
        let rx = root_center_x(branches, y_root);
        let root_depth = y_range * 0.22;
        // Reuse tapered_roots; add highlight over the whole roots section
        let junction_y = y_root + root_depth * 0.55;
        let junction_hw = MAX_HW;

        out.push_str(&tapered_branch_path(
            rx,
            y_root,
            rx,
            junction_y,
            junction_hw,
            junction_hw * 0.88,
            &trunk_color,
        ));
        out.push_str(&tapered_branch_highlight(
            rx,
            y_root,
            rx,
            junction_y,
            junction_hw,
            junction_hw * 0.88,
        ));

        let tips: [(f64, f64, f64); 4] = [
            (rx - root_depth * 0.48, y_root + root_depth * 0.85, 0.28),
            (rx - root_depth * 0.20, y_root + root_depth * 0.95, 0.38),
            (rx + root_depth * 0.20, y_root + root_depth * 0.95, 0.38),
            (rx + root_depth * 0.48, y_root + root_depth * 0.85, 0.28),
        ];
        for (ex, ey, end_scale) in tips {
            let end_hw = MIN_HW + (junction_hw * 0.88 - MIN_HW) * end_scale;
            out.push_str(&tapered_branch_path(
                rx,
                junction_y,
                ex,
                ey,
                junction_hw * 0.88,
                end_hw,
                &trunk_color,
            ));
            out.push_str(&tapered_branch_highlight(
                rx,
                junction_y,
                ex,
                ey,
                junction_hw * 0.88,
                end_hw,
            ));
        }
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

        let (bar_y, bar_min_x, bar_max_x) = branch_bar(py, mean_cy, px, &branch.child_pts);
        let w_py = width_at(py, y_top, y_range, MAX_HW, MIN_HW);
        let w_bar = width_at(bar_y, y_top, y_range, MAX_HW, MIN_HW);

        // Short vertical trunk — main shape + highlight
        out.push_str(&tapered_branch_path(
            px,
            py,
            px,
            bar_y,
            w_py,
            w_bar,
            &trunk_color,
        ));
        out.push_str(&tapered_branch_highlight(px, py, px, bar_y, w_py, w_bar));

        // Horizontal bar with organic wave
        if bar_max_x - bar_min_x > 1.0 {
            out.push_str(&tapered_bar_path(
                bar_min_x,
                bar_y,
                bar_max_x,
                w_bar,
                &trunk_color,
            ));
            out.push_str(&tapered_bar_highlight(bar_min_x, bar_y, bar_max_x, w_bar));
        }
        // Round junction blobs at T-joins (main + highlight)
        out.push_str(&junction_circle(px, bar_y, w_bar, &trunk_color));
        out.push_str(&junction_circle(px, bar_y, w_bar * 0.25, "white"));
        for &(cx, _) in &branch.child_pts {
            out.push_str(&junction_circle(cx, bar_y, w_bar, &trunk_color));
            out.push_str(&junction_circle(cx, bar_y, w_bar * 0.25, "white"));
        }

        // Vertical drops + canopy leaves
        for &(cx, cy) in &branch.child_pts {
            let w_cy = width_at(cy, y_top, y_range, MAX_HW, MIN_HW);
            out.push_str(&tapered_branch_path(
                cx,
                bar_y,
                cx,
                cy,
                w_bar,
                w_cy,
                &trunk_color,
            ));
            out.push_str(&tapered_branch_highlight(cx, bar_y, cx, cy, w_bar, w_cy));
            if leaf_count > 0 {
                out.push_str(&canopy_leaves(cx, cy, leaf_count, &leaf_color));
            }
        }
    }

    out
}

// ── Style: ink ────────────────────────────────────────────────────────────────
//
// Each branch segment is drawn in two passes:
//   1. A stroked closed Bézier perimeter outline (build_tapered_d / build_bar_d)
//      gives the branch a clear delineated edge.
//   2. Many short ink strokes (12–30 SVG units, ±14° angular noise) scattered
//      within the branch volume texture the interior.
// Triangle-distributed perpendicular position weights strokes toward the centre.
// Leaves are hollow ellipse outlines (fill="none").

/// Many short ink strokes scattered within a tapered branch segment.
fn ink_branch_strokes(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    hw1: f64,
    hw2: f64,
    seed_off: u64,
) -> String {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = dx.hypot(dy).max(0.01);
    let ux = dx / len;
    let uy = dy / len;
    let nx = -uy; // perpendicular
    let ny = ux;

    let avg_hw = (hw1 + hw2) * 0.5;
    // ~0.15 strokes per sq-unit of branch cross-section × length
    let n = ((len * avg_hw * 0.15) as usize + 1).max(8).min(120);

    let mut seed = ((x1 * 1000.0) as u64)
        .wrapping_add((y1 * 997.0) as u64)
        .wrapping_add((x2 * 991.0) as u64)
        .wrapping_add((y2 * 983.0) as u64)
        .wrapping_add(seed_off);
    let mut out = String::new();

    // Pass 1: stroked perimeter outline gives a clear branch edge
    let outline_d = build_tapered_d(x1, y1, x2, y2, hw1, hw2);
    out.push_str(&format!(
        "  <path d=\"{outline_d}\" fill=\"none\" stroke=\"#111\" stroke-width=\"1.20\" \
         stroke-linejoin=\"round\" class=\"tree-branch\"/>\n"
    ));

    // Pass 2: interior ink strokes for texture
    for _ in 0..n {
        // Random position along branch
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let t = (seed & 0xFFFF) as f64 / 65535.0;
        let along = t * len;
        let hw_t = hw1 + (hw2 - hw1) * t;

        // Perpendicular offset: triangle distribution → centre-heavy
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let u1 = (seed & 0xFFFF) as f64 / 65535.0;
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let u2 = (seed & 0xFFFF) as f64 / 65535.0;
        let perp_frac = u1 + u2 - 1.0; // range [−1, 1]
        let perp = perp_frac * hw_t;

        // Centre of this short stroke on the branch surface
        let mx = x1 + ux * along + nx * perp;
        let my = y1 + uy * along + ny * perp;

        // Stroke length: 12–30 SVG units
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let slen = 12.0 + (seed & 0xFF) as f64 / 255.0 * 18.0;

        // Angular deviation from branch axis: ±0.25 rad (≈ ±14°)
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let dev = ((seed & 0xFFFF) as f64 / 65535.0 - 0.5) * 0.50;
        let cos_d = dev.cos();
        let sin_d = dev.sin();
        let sdx = ux * cos_d - uy * sin_d;
        let sdy = ux * sin_d + uy * cos_d;

        let sx = mx - sdx * slen * 0.5;
        let sy = my - sdy * slen * 0.5;
        let ex = mx + sdx * slen * 0.5;
        let ey = my + sdy * slen * 0.5;

        // Small transverse bow for organic feel
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let bow = ((seed & 0xFFFF) as f64 / 65535.0 - 0.5) * slen * 0.30;
        let bx = -sdy;
        let by = sdx;
        let c1x = sx + (ex - sx) * 0.33 + bx * bow;
        let c1y = sy + (ey - sy) * 0.33 + by * bow;
        let c2x = sx + (ex - sx) * 0.67 + bx * bow;
        let c2y = sy + (ey - sy) * 0.67 + by * bow;

        // Opacity: denser toward centre, lighter at edges
        let edge = perp_frac.abs();
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let rand_o = (seed & 0xFF) as f64 / 255.0;
        let opacity = (0.78 - edge * 0.38 + rand_o * 0.12).clamp(0.25, 0.92);

        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let sw = 0.5 + (seed & 0xFF) as f64 / 255.0 * 1.0; // 0.5–1.5

        out.push_str(&format!(
            "  <path d=\"M {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2}\" \
             stroke=\"#111\" stroke-width=\"{:.2}\" opacity=\"{:.2}\" \
             fill=\"none\" stroke-linecap=\"round\" class=\"tree-bark\"/>\n",
            sx, sy, c1x, c1y, c2x, c2y, ex, ey, sw, opacity
        ));
    }
    out
}

/// Hollow ellipse ink-style leaves scattered around (cx, cy).
fn ink_leaf_cluster(cx: f64, cy: f64, count: usize, seed_off: u64) -> String {
    let mut seed = ((cx * 1000.0) as u64 ^ (cy * 1000.0) as u64).wrapping_add(seed_off);
    let mut out = String::new();
    for _ in 0..count {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let angle = (seed & 0xFFFF) as f64 / 65535.0 * std::f64::consts::TAU;
        let radius = ((seed >> 16) & 0xFFFF) as f64 / 65535.0 * 55.0 + 8.0;
        let lx = cx + angle.cos() * radius;
        let ly = cy + angle.sin() * radius * 0.55;
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let rx = (seed & 0xFF) as f64 / 255.0 * 7.0 + 5.0;
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ry = rx * (0.38 + (seed & 0xFF) as f64 / 255.0 * 0.28);
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let rot = (seed & 0xFFFF) as f64 / 65535.0 * 180.0;
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let sw = 0.6 + (seed & 0xFF) as f64 / 255.0 * 0.5;
        out.push_str(&format!(
            "  <ellipse cx=\"{0:.2}\" cy=\"{1:.2}\" rx=\"{2:.2}\" ry=\"{3:.2}\" \
             transform=\"rotate({4:.1},{0:.2},{1:.2})\" \
             fill=\"none\" stroke=\"#111\" stroke-width=\"{5:.2}\" class=\"tree-leaf\"/>\n",
            lx, ly, rx, ry, rot, sw
        ));
    }
    out
}

/// Three stacked ink leaf clouds creating a canopy above (cx, cy).
fn ink_leaf_canopy(cx: f64, cy: f64, count: usize) -> String {
    let mut out = String::new();
    out.push_str(&ink_leaf_cluster(cx, cy, count, 0));
    out.push_str(&ink_leaf_cluster(cx, cy - 36.0, count * 2 / 3 + 1, 1));
    out.push_str(&ink_leaf_cluster(cx, cy - 72.0, count / 2 + 1, 2));
    out
}

/// Root spread for ink style: scattered short strokes fanning below the root box.
fn ink_roots(root_x: f64, y_root: f64, root_depth: f64, max_hw: f64, min_hw: f64) -> String {
    let junction_y = y_root + root_depth * 0.55;
    let junction_hw = max_hw;

    let mut out = ink_branch_strokes(
        root_x,
        y_root,
        root_x,
        junction_y,
        junction_hw,
        junction_hw * 0.88,
        1001,
    );

    let tips: [(f64, f64, f64); 4] = [
        (root_x - root_depth * 0.48, y_root + root_depth * 0.85, 0.28),
        (root_x - root_depth * 0.20, y_root + root_depth * 0.95, 0.38),
        (root_x + root_depth * 0.20, y_root + root_depth * 0.95, 0.38),
        (root_x + root_depth * 0.48, y_root + root_depth * 0.85, 0.28),
    ];
    for (i, (ex, ey, end_scale)) in tips.iter().enumerate() {
        let end_hw = min_hw + (junction_hw * 0.88 - min_hw) * end_scale;
        out.push_str(&ink_branch_strokes(
            root_x,
            junction_y,
            *ex,
            *ey,
            junction_hw * 0.88,
            end_hw,
            1002 + i as u64,
        ));
    }
    out
}

/// One continuous bark-striation line following the Bézier path at a perpendicular
/// offset of `frac × local_hw` from the centre. `frac` ∈ (−1, 1).
///
/// At each of the N sample points along the curve, a small per-point noise is added
/// perpendicular to the line so the striation wiggles like a real wood-grain fiber.
/// The result is a single connected path rather than scattered marks, so adjacent
/// striations (called at different `frac` values) are genuinely parallel.
fn ink_grain_line(
    p0x: f64,
    p0y: f64,
    p1x: f64,
    p1y: f64,
    p2x: f64,
    p2y: f64,
    p3x: f64,
    p3y: f64,
    hw_p: f64,
    hw_c: f64,
    frac: f64,
    opacity: f64,
    seed_off: u64,
) -> String {
    const N: usize = 32;
    let mut seed = seed_off;
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(N);

    for i in 0..N {
        let t = i as f64 / (N - 1) as f64;
        let (bx, by) = bezier3(p0x, p0y, p1x, p1y, p2x, p2y, p3x, p3y, t);
        let (tang_x, tang_y) = bezier3_tangent(p0x, p0y, p1x, p1y, p2x, p2y, p3x, p3y, t);
        let norm_x = -tang_y;
        let norm_y = tang_x;
        let hw = hw_p + (hw_c - hw_p) * t;

        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let noise = ((seed & 0xFFFF) as f64 / 65535.0 - 0.5) * hw * 0.20;

        pts.push((
            bx + norm_x * (frac * hw + noise),
            by + norm_y * (frac * hw + noise),
        ));
    }

    let mut d = format!("M {:.2},{:.2}", pts[0].0, pts[0].1);
    for (x, y) in pts.iter().skip(1) {
        d.push_str(&format!(" L {:.2},{:.2}", x, y));
    }

    format!(
        "  <path d=\"{d}\" fill=\"none\" stroke=\"#111\" stroke-width=\"0.70\" \
         opacity=\"{:.2}\" stroke-linecap=\"round\" class=\"tree-bark\"/>\n",
        opacity
    )
}

/// Smooth S-curve branch from (px,py) to (cx,cy) with ink texture.
///
/// Uses a cubic Bézier with control points at (px, mid_y) and (cx, mid_y),
/// which produces a tangent that is perfectly vertical at both endpoints and
/// horizontal at the midpoint — no right-angle elbows.  The offset-curve
/// polyline gives a visible perimeter; interior ink strokes follow the local
/// tangent at each sample point.
fn ink_smooth_branch(
    px: f64,
    py: f64,
    cx: f64,
    cy: f64,
    hw_p: f64,
    hw_c: f64,
    seed_off: u64,
) -> String {
    // Fork at 25 % from the parent so the branch bends quickly near the trunk
    // and extends in a more direct line toward the child — matches real branch geometry.
    let fork_y = py + (cy - py) * 0.25;
    let p1x = px;
    let p1y = fork_y;
    let p2x = cx;
    let p2y = cy - (cy - py) * 0.25; // symmetric arrival, 25 % from child

    const N: usize = 24;

    // Build noisy polyline offset curves for the perimeter outline.
    // A small per-point perturbation of the local hw gives an organic silhouette
    // rather than a geometrically perfect smooth curve.
    let mut outline_seed = ((px * 1000.0) as u64)
        .wrapping_add((py * 997.0) as u64)
        .wrapping_add((cx * 991.0) as u64)
        .wrapping_add((cy * 983.0) as u64)
        .wrapping_add(seed_off.wrapping_mul(1_000_003));
    let mut outer: Vec<(f64, f64)> = Vec::with_capacity(N + 1);
    let mut inner: Vec<(f64, f64)> = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let t = i as f64 / N as f64;
        let (bx, by) = bezier3(px, py, p1x, p1y, p2x, p2y, cx, cy, t);
        let (tx, ty) = bezier3_tangent(px, py, p1x, p1y, p2x, p2y, cx, cy, t);
        let hw = hw_p + (hw_c - hw_p) * t;
        outline_seed = outline_seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let noise_o = ((outline_seed & 0xFFFF) as f64 / 65535.0 - 0.5) * hw * 0.18;
        outline_seed = outline_seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let noise_i = ((outline_seed & 0xFFFF) as f64 / 65535.0 - 0.5) * hw * 0.18;
        // Right-normal = CCW 90° of tangent: (-ty, tx)
        outer.push((bx + (-ty) * (hw + noise_o), by + tx * (hw + noise_o)));
        inner.push((bx - (-ty) * (hw + noise_i), by - tx * (hw + noise_i)));
    }

    // Closed outline: outer forward → end cap → inner backward → start cap
    let mut d = format!("M {:.2},{:.2}", outer[0].0, outer[0].1);
    for (x, y) in outer.iter().skip(1) {
        d.push_str(&format!(" L {:.2},{:.2}", x, y));
    }
    d.push_str(&format!(" L {:.2},{:.2}", inner[N].0, inner[N].1));
    for (x, y) in inner.iter().rev().skip(1) {
        d.push_str(&format!(" L {:.2},{:.2}", x, y));
    }
    d.push_str(" Z");

    let mut out = String::new();
    out.push_str(&format!(
        "  <path d=\"{d}\" fill=\"none\" stroke=\"#111\" stroke-width=\"1.20\" \
         stroke-linejoin=\"round\" stroke-linecap=\"round\" class=\"tree-branch\"/>\n"
    ));

    // Bark striations: N continuous grain lines, each following the full Bézier arc
    // at a fixed perpendicular offset from the centreline.  Because every line traces
    // the same curve shape (just shifted across the branch width), adjacent lines are
    // inherently parallel — they read as wood-grain fibres, not scattered marks.
    // Per-point perpendicular noise on each line gives organic waviness.
    let avg_hw = (hw_p + hw_c) * 0.5;
    let n_grains = ((avg_hw * 2.0 / 4.5) as usize + 2).max(2).min(16);
    let branch_seed = ((px * 1000.0) as u64)
        .wrapping_add((py * 997.0) as u64)
        .wrapping_add((cx * 991.0) as u64)
        .wrapping_add((cy * 983.0) as u64)
        .wrapping_add(seed_off.wrapping_mul(2_654_435_761));

    for i in 0..n_grains {
        // Evenly spaced from −0.82 to +0.82 of the local branch half-width
        let frac = -0.82 + (i as f64 + 0.5) * 1.64 / n_grains as f64;
        // Opacity: dense at centre (dark ink core), transparent at edges
        let edge_dist = frac.abs();
        let opacity = (0.80 - edge_dist * 0.45).clamp(0.18, 0.80);
        out.push_str(&ink_grain_line(
            px,
            py,
            p1x,
            p1y,
            p2x,
            p2y,
            cx,
            cy,
            hw_p,
            hw_c,
            frac,
            opacity,
            branch_seed.wrapping_add(i as u64 * 7_919),
        ));
    }

    out
}

fn render_ink_style(branches: &[Branch], prefs: &Prefs) -> String {
    let leaf_count: usize = match prefs.output.style.realistic_tree.leaf_density.as_str() {
        "none" => 0,
        "low" => 20,
        "high" => 80,
        _ => 50,
    };

    let (y_root, y_top) = y_bounds(branches);
    let y_range = (y_root - y_top).max(1.0);
    const MAX_HW: f64 = 12.0;
    const MIN_HW: f64 = 1.2;

    let mut out = String::new();

    if y_root > y_top {
        let rx = root_center_x(branches, y_root);
        let root_depth = y_range * 0.22;
        out.push_str(&ink_roots(rx, y_root, root_depth, MAX_HW, MIN_HW));
    }

    for branch in branches {
        if branch.parent_pts.is_empty() || branch.child_pts.is_empty() {
            continue;
        }

        let px =
            branch.parent_pts.iter().map(|p| p.0).sum::<f64>() / branch.parent_pts.len() as f64;
        let py =
            branch.parent_pts.iter().map(|p| p.1).sum::<f64>() / branch.parent_pts.len() as f64;
        let w_py = width_at(py, y_top, y_range, MAX_HW, MIN_HW);

        // One smooth S-curve per child: vertical at parent, horizontal in middle,
        // vertical at child — no right-angle elbows or junction circles.
        for (i, &(cx, cy)) in branch.child_pts.iter().enumerate() {
            let w_cy = width_at(cy, y_top, y_range, MAX_HW, MIN_HW);
            out.push_str(&ink_smooth_branch(px, py, cx, cy, w_py, w_cy, i as u64));
            if leaf_count > 0 {
                out.push_str(&ink_leaf_canopy(cx, cy, leaf_count));
            }
        }
    }

    out
}
