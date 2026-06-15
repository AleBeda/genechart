//! Realistic tree branch rendering for the boxed_couples layout.
//! (experimental, may be removed or redesigned in the future)
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
    ((y_root - y_top) * 0.45).max(40.0)
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
        "ink2" => render_ink2_style(&branches, prefs),
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
    let trunk_hw = max_hw;
    let trunk_end_y = y_root + root_depth * 0.60;
    let trunk_end_hw = trunk_hw * 0.82;

    // 1. Visible trunk chunk directly below the root ancestor box.
    let mut out = ink_smooth_branch(
        root_x,
        y_root,
        root_x,
        trunk_end_y,
        trunk_hw,
        trunk_end_hw,
        9_001,
    );

    // 2. Lateral roots: nearly horizontal, long, tapering to thin points.
    //    (tip_x, tip_y, tip_hw_fraction, seed)
    let span = root_depth * 1.8;
    let roots: [(f64, f64, f64, u64); 6] = [
        (root_x - span, y_root + root_depth * 0.82, 0.03, 9_011),
        (
            root_x - span * 0.60,
            y_root + root_depth * 0.74,
            0.06,
            9_012,
        ),
        (
            root_x - span * 0.24,
            y_root + root_depth * 0.70,
            0.10,
            9_013,
        ),
        (
            root_x + span * 0.24,
            y_root + root_depth * 0.70,
            0.10,
            9_014,
        ),
        (
            root_x + span * 0.60,
            y_root + root_depth * 0.74,
            0.06,
            9_015,
        ),
        (root_x + span, y_root + root_depth * 0.82, 0.03, 9_016),
    ];

    for (tip_x, tip_y, tip_frac, seed) in roots.iter() {
        let end_hw = min_hw * 0.3 + (trunk_end_hw - min_hw) * tip_frac;
        let dx = tip_x - root_x;
        let dy = tip_y - trunk_end_y;
        // Control points give: ~20° departure angle from trunk base, nearly horizontal arrival.
        out.push_str(&ink_branch(
            root_x,
            trunk_end_y,
            root_x + dx * 0.15,
            trunk_end_y + dy * 0.50,
            root_x + dx * 0.75,
            *tip_y - dy * 0.10,
            *tip_x,
            *tip_y,
            trunk_end_hw,
            end_hw,
            *seed,
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
    t_start: f64,
    t_end: f64,
    opacity: f64,
    stroke_width: f64,
    color: &str,
    seed_off: u64,
) -> String {
    const N: usize = 20;
    let mut seed = seed_off;
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(N);

    // Sample only within [t_start, t_end]; width follows pressure envelope.
    for i in 0..N {
        let t = t_start + (t_end - t_start) * (i as f64 / (N - 1) as f64);
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

    // Build path with random gaps: brief interruptions along the line simulate
    // the discontinuous quality of real ink strokes on fibrous paper.
    let mut gap_seed = seed_off.wrapping_mul(1_234_567_891);
    let mut in_gap = true; // begin with a Move command
    let mut remaining_skip: usize = 0;
    let mut d = String::new();

    for (x, y) in &pts {
        if remaining_skip > 0 {
            remaining_skip -= 1;
            in_gap = true;
            continue;
        }
        // ~10 % chance of starting a brief gap at this sample point
        gap_seed = gap_seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        if gap_seed & 0xFF < 26 {
            gap_seed = gap_seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            remaining_skip = (gap_seed & 1) as usize + 1; // skip 1–2 samples
            in_gap = true;
            continue;
        }
        if in_gap {
            d.push_str(&format!("M {x:.2},{y:.2}"));
            in_gap = false;
        } else {
            d.push_str(&format!(" L {x:.2},{y:.2}"));
        }
    }

    if d.is_empty() {
        return String::new();
    }

    format!(
        "  <path d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{stroke_width:.2}\" \
         opacity=\"{opacity:.2}\" stroke-linecap=\"round\" class=\"tree-bark\"/>\n"
    )
}

/// Core ink branch renderer with explicit cubic Bézier control points.
///
/// P0=(p0x,p0y)…P3=(p3x,p3y) define the centreline; P1 and P2 are the interior
/// control points.  Setting P1=P2 at a corner gives a smooth quarter-turn elbow.
/// Setting them at the 50 % positions gives the standard S-curve shape.
fn ink_branch(
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
    seed_off: u64,
) -> String {
    const N: usize = 24;

    let mut outline_seed = ((p0x * 1000.0) as u64)
        .wrapping_add((p0y * 997.0) as u64)
        .wrapping_add((p3x * 991.0) as u64)
        .wrapping_add((p3y * 983.0) as u64)
        .wrapping_add(seed_off.wrapping_mul(1_000_003));
    let mut outer: Vec<(f64, f64)> = Vec::with_capacity(N + 1);
    let mut inner: Vec<(f64, f64)> = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let t = i as f64 / N as f64;
        let (bx, by) = bezier3(p0x, p0y, p1x, p1y, p2x, p2y, p3x, p3y, t);
        let (tx, ty) = bezier3_tangent(p0x, p0y, p1x, p1y, p2x, p2y, p3x, p3y, t);
        let hw = hw_p + (hw_c - hw_p) * t;
        outline_seed = outline_seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let noise_o = ((outline_seed & 0xFFFF) as f64 / 65535.0 - 0.5) * hw * 0.18;
        outline_seed = outline_seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let noise_i = ((outline_seed & 0xFFFF) as f64 / 65535.0 - 0.5) * hw * 0.18;
        outer.push((bx + (-ty) * (hw + noise_o), by + tx * (hw + noise_o)));
        inner.push((bx - (-ty) * (hw + noise_i), by - tx * (hw + noise_i)));
    }

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
        "  <path d=\"{d}\" fill=\"#111\" stroke=\"none\" \
         stroke-linejoin=\"round\" stroke-linecap=\"round\" class=\"tree-branch\"/>\n"
    ));

    // Bristle bundle.  Lit-side convention:
    //   vertical   → right side lit → lit_sign = +1
    //   horizontal → top side lit  → lit_sign = −1 (normal points down in SVG)
    let is_horizontal = (p3x - p0x).abs() > (p3y - p0y).abs();
    let lit_sign: f64 = if is_horizontal { -1.0 } else { 1.0 };

    let avg_hw = (hw_p + hw_c) * 0.5;
    let n_total = ((avg_hw * 2.0 / 3.0) as usize + 4).max(10).min(36);
    let n_shadow = (n_total / 4).max(2);
    let n_lit = n_total - n_shadow;
    let branch_seed = ((p0x * 1000.0) as u64)
        .wrapping_add((p0y * 997.0) as u64)
        .wrapping_add((p3x * 991.0) as u64)
        .wrapping_add((p3y * 983.0) as u64)
        .wrapping_add(seed_off.wrapping_mul(2_654_435_761));

    let emit_bristle = |i: usize, frac: f64, opacity: f64, stroke_width: f64| -> String {
        let mut rseed = branch_seed
            .wrapping_add(i as u64 * 13_331)
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let length_frac = 0.75 + (rseed & 0xFFFF) as f64 / 65535.0 * 0.25;
        rseed = rseed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let t_start = (rseed & 0xFFFF) as f64 / 65535.0 * (1.0 - length_frac);
        let t_end = (t_start + length_frac).min(1.0);
        ink_grain_line(
            p0x,
            p0y,
            p1x,
            p1y,
            p2x,
            p2y,
            p3x,
            p3y,
            hw_p,
            hw_c,
            frac,
            t_start,
            t_end,
            opacity,
            stroke_width,
            "#FFF",
            branch_seed.wrapping_add(i as u64 * 7_919),
        )
    };

    for i in 0..n_shadow {
        let frac = lit_sign * (-0.85 + (i as f64 + 0.5) * 0.80 / n_shadow as f64);
        let s = emit_bristle(i, frac, 0.22, 0.38);
        out.push_str(&s);
    }
    for i in 0..n_lit {
        let base_frac = 0.05 + (i as f64 + 0.5) * 0.80 / n_lit as f64;
        let inner_t = base_frac / 0.85;
        let opacity = (0.62 - inner_t * 0.28).clamp(0.28, 0.62);
        let s = emit_bristle(n_shadow + i, lit_sign * base_frac, opacity, 0.65);
        out.push_str(&s);
    }

    out
}

/// S-curve branch using control points at 50 % of the span.  Wrapper around `ink_branch`.
fn ink_smooth_branch(
    px: f64,
    py: f64,
    cx: f64,
    cy: f64,
    hw_p: f64,
    hw_c: f64,
    seed_off: u64,
) -> String {
    let mid_y = py + (cy - py) * 0.50;
    ink_branch(
        px,
        py,
        px,
        mid_y,
        cx,
        cy - (cy - py) * 0.50,
        cx,
        cy,
        hw_p,
        hw_c,
        seed_off,
    )
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
        let root_depth = y_range * 0.45;
        out.push_str(&ink_roots(rx, y_root, root_depth, MAX_HW, MIN_HW));
    }

    for (bi, branch) in branches.iter().enumerate() {
        if branch.parent_pts.is_empty() || branch.child_pts.is_empty() {
            continue;
        }

        let px =
            branch.parent_pts.iter().map(|p| p.0).sum::<f64>() / branch.parent_pts.len() as f64;
        let py =
            branch.parent_pts.iter().map(|p| p.1).sum::<f64>() / branch.parent_pts.len() as f64;
        let w_py = width_at(py, y_top, y_range, MAX_HW, MIN_HW);

        let mean_cy =
            branch.child_pts.iter().map(|p| p.1).sum::<f64>() / branch.child_pts.len() as f64;
        let (bar_y, bar_min_x, bar_max_x) = branch_bar(py, mean_cy, px, &branch.child_pts);
        let w_bar = width_at(bar_y, y_top, y_range, MAX_HW, MIN_HW);

        let seed_base = bi as u64 * 100;

        // 1. Vertical trunk from parent attachment up to the horizontal bar.
        if (py - bar_y).abs() > 1.0 {
            out.push_str(&ink_smooth_branch(
                px, py, px, bar_y, w_py, w_bar, seed_base,
            ));
        }

        // Sort children by x so we can identify the farthest (leftmost / rightmost).
        let mut sorted_children: Vec<(f64, f64)> = branch.child_pts.clone();
        sorted_children.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let (lx, ly) = *sorted_children.first().unwrap();
        let (rx, ry) = *sorted_children.last().unwrap();

        // Elbow radius: a few branch-widths, never more than 20 % of the bar span.
        let bar_span = (bar_max_x - bar_min_x).max(1.0);
        let elbow_r = (w_bar * 3.5).min(bar_span * 0.20).max(1.0);

        // Which ends of the bar terminate at a child (vs the trunk x)?
        let left_is_child = (bar_min_x - lx).abs() < 1.0;
        let right_is_child = (bar_max_x - rx).abs() < 1.0;
        // A child directly above the parent gets a T-junction (no elbow), so the bar
        // must NOT be shortened at that end.
        let left_child_above_parent = (lx - px).abs() < 1.0;
        let right_child_above_parent = (rx - px).abs() < 1.0;

        // Bar endpoints are pulled inward by elbow_r only where a curved elbow takes over.
        let bar_x0 = if left_is_child && !left_child_above_parent {
            bar_min_x + elbow_r
        } else {
            bar_min_x
        };
        let bar_x1 = if right_is_child && !right_child_above_parent {
            bar_max_x - elbow_r
        } else {
            bar_max_x
        };

        // 2. Horizontal bar with an S-wave (5× more Y variation than a flat line).
        if bar_x1 > bar_x0 + 1.0 {
            let bw = bar_x1 - bar_x0;
            // Wave sign is seeded per-branch so adjacent bars don't all curve the same way.
            let wave_seed = (bar_y as u64)
                .wrapping_mul(997)
                .wrapping_add(px as u64 * 1013);
            let wave_sign: f64 = if wave_seed & 1 == 0 { 1.0 } else { -1.0 };
            let wave = (w_bar * 2.5).min(bw * 0.12) * wave_sign;
            out.push_str(&ink_branch(
                bar_x0,
                bar_y,
                bar_x0 + bw / 3.0,
                bar_y - wave,
                bar_x1 - bw / 3.0,
                bar_y + wave,
                bar_x1,
                bar_y,
                w_bar,
                w_bar,
                seed_base + 1,
            ));
        }

        // 3. Stubs / elbows — farthest children get a smooth quarter-turn elbow;
        //    intermediate children get straight stubs from the bar.
        for (ci, &(cx, cy)) in sorted_children.iter().enumerate() {
            let w_cy = width_at(cy, y_top, y_range, MAX_HW, MIN_HW);
            // Exception: if the child is directly above the parent x, the bar has no
            // horizontal approach to bend from — use a straight stub instead of an elbow.
            let child_above_parent = (cx - px).abs() < 1.0;
            let is_left = left_is_child && (cx - lx).abs() < 1.0 && !child_above_parent;
            let is_right = right_is_child
                && (cx - rx).abs() < 1.0
                && !(is_left && sorted_children.len() == 1)
                && !child_above_parent;

            if is_left {
                // Elbow: arrives from the right along the bar, departs upward to the child.
                // P1 = P2 = corner → horizontal departure, vertical arrival.
                out.push_str(&ink_branch(
                    lx + elbow_r,
                    bar_y,
                    lx,
                    bar_y,
                    lx,
                    bar_y,
                    lx,
                    ly,
                    w_bar,
                    w_cy,
                    seed_base + 2 + ci as u64,
                ));
            } else if is_right {
                // Elbow: arrives from the left along the bar, departs upward to the child.
                out.push_str(&ink_branch(
                    rx - elbow_r,
                    bar_y,
                    rx,
                    bar_y,
                    rx,
                    bar_y,
                    rx,
                    ry,
                    w_bar,
                    w_cy,
                    seed_base + 2 + ci as u64,
                ));
            } else {
                // Intermediate child: straight vertical stub.
                out.push_str(&ink_smooth_branch(
                    cx,
                    bar_y,
                    cx,
                    cy,
                    w_bar,
                    w_cy,
                    seed_base + 2 + ci as u64,
                ));
            }

            if leaf_count > 0 {
                out.push_str(&ink_leaf_canopy(cx, cy, leaf_count));
            }
        }
    }

    out
}

// ── Style: ink2 ──────────────────────────────────────────────────────────────
//
// Implements the coherent-tree algorithm (realistic_tree_spec.md):
// trunk+flare, power-law depth width, organic kinked branches,
// da-Vinci width splitting at forks, white bark scratches, open-ellipse leaves.

// §0 — deterministic hash PRNG (stable SVG diffs across runs)
fn ink2_rand(px: f64, py: f64, seed: u32) -> f64 {
    let a = (px * 7.0).round();
    let b = (py * 7.0).round();
    let v = (a * 12.9898 + b * 78.233 + seed as f64 * 37.719).sin() * 43758.5453;
    v - v.floor()
}

fn ink2_jitter(px: f64, py: f64, seed: u32) -> f64 {
    ink2_rand(px, py, seed) * 2.0 - 1.0
}

// §7 — color helpers; clamp each channel to [0, 255]
fn darken_color(c: u32, amount: f64) -> u32 {
    let f = (1.0 - amount.clamp(0.0, 1.0)) * 255.0;
    let r = (((c >> 16) & 0xFF) as f64 / 255.0 * f) as u32;
    let g = (((c >> 8) & 0xFF) as f64 / 255.0 * f) as u32;
    let bv = ((c & 0xFF) as f64 / 255.0 * f) as u32;
    (r.min(255) << 16) | (g.min(255) << 8) | bv.min(255)
}

fn lighten_color(c: u32, amount: f64) -> u32 {
    let lerp_ch = |ch: u32| -> u32 {
        (ch as f64 + (255.0 - ch as f64) * amount)
            .round()
            .clamp(0.0, 255.0) as u32
    };
    (lerp_ch((c >> 16) & 0xFF) << 16) | (lerp_ch((c >> 8) & 0xFF) << 8) | lerp_ch(c & 0xFF)
}

fn mix_color(ac: u32, bc: u32, t: f64) -> u32 {
    let lerp_ch = |a: u32, b: u32| -> u32 {
        (a as f64 + (b as f64 - a as f64) * t)
            .round()
            .clamp(0.0, 255.0) as u32
    };
    (lerp_ch((ac >> 16) & 0xFF, (bc >> 16) & 0xFF) << 16)
        | (lerp_ch((ac >> 8) & 0xFF, (bc >> 8) & 0xFF) << 8)
        | lerp_ch(ac & 0xFF, bc & 0xFF)
}

fn ink2_color_hex(c: u32) -> String {
    format!("#{:06X}", c & 0xFFFFFF)
}

// bark fill: darkens near base (s≈0), lightens toward crown (s≈1)
fn bark_fill_color(trunk_color: u32, s: f64) -> String {
    let c_base = darken_color(trunk_color, 0.12);
    let c_tip = lighten_color(trunk_color, 0.10);
    ink2_color_hex(mix_color(c_base, c_tip, s.clamp(0.0, 1.0)))
}

// §5 — height fraction s: 0 at soil (y_root), 1 at crown (y_top)
fn height_frac_s(y: f64, y_root: f64, y_range: f64) -> f64 {
    ((y_root - y) / y_range).clamp(0.0, 1.0)
}

// §5 — depth-based full width; subtle taper (~2.5:1 ratio, γ=1.2)
// s=0 at base/soil (thick end), s=1 at crown (thin tips): use (1-s)^γ
fn ink2_width(s: f64, bigb: f64) -> f64 {
    let w_max = bigb * 0.022;
    let w_min = bigb * 0.009;
    w_min + (w_max - w_min) * (1.0 - s.clamp(0.0, 1.0)).powf(1.2)
}

// Cubic Bézier helpers
fn cubic_pt2(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let m = 1.0 - t;
    (
        m * m * m * p0.0 + 3.0 * m * m * t * p1.0 + 3.0 * m * t * t * p2.0 + t * t * t * p3.0,
        m * m * m * p0.1 + 3.0 * m * m * t * p1.1 + 3.0 * m * t * t * p2.1 + t * t * t * p3.1,
    )
}

fn cubic_tang2(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    t: f64,
) -> (f64, f64) {
    let m = 1.0 - t;
    let dx = 3.0 * (m * m * (p1.0 - p0.0) + 2.0 * m * t * (p2.0 - p1.0) + t * t * (p3.0 - p2.0));
    let dy = 3.0 * (m * m * (p1.1 - p0.1) + 2.0 * m * t * (p2.1 - p1.1) + t * t * (p3.1 - p2.1));
    let len = dx.hypot(dy).max(0.001);
    (dx / len, dy / len)
}

// §3 + §10 — cubic control points for a branch (parent below child in SVG y)
fn branch_cubic2(
    px: f64,
    py: f64,
    cx: f64,
    cy: f64,
) -> ((f64, f64), (f64, f64), (f64, f64), (f64, f64)) {
    let dx = cx - px;
    let dy = (py - cy).abs().max(0.001);
    const K: f64 = 0.42;
    let horiz = dx.abs() / dy;
    let (c1, c2) = if horiz > 2.0 {
        // nearly horizontal departure, vertical arrival (C2.x = cx)
        ((px + K * dx, py), (cx, cy + 0.6 * dy))
    } else if horiz > 1.0 {
        // blend vertical → horizontal departure; arrival stays vertical
        let tb = (horiz - 1.0).clamp(0.0, 1.0);
        let lerp2 = |v: f64, h: f64| v + (h - v) * tb;
        let c1 = (lerp2(px, px + K * dx), lerp2(py - K * dy, py));
        let c2 = (cx, lerp2(cy + K * dy, cy + 0.6 * dy));
        (c1, c2)
    } else {
        // nearly vertical: vertical arrival (C2.x = cx)
        ((px, py - K * dy), (cx, cy + K * dy))
    };
    ((px, py), c1, c2, (cx, cy))
}

// Build a filled tapered SVG path along a cubic with rounded tip cap
fn ink2_cubic_fill(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    hw0: f64,
    hw1: f64,
    fill: &str,
    class: &str,
    n: usize,
) -> String {
    let mut right: Vec<(f64, f64)> = Vec::with_capacity(n + 1);
    let mut left: Vec<(f64, f64)> = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let (x, y) = cubic_pt2(p0, p1, p2, p3, t);
        let (tx, ty) = cubic_tang2(p0, p1, p2, p3, t);
        let hw = hw0 + (hw1 - hw0) * t;
        right.push((x + -ty * hw, y + tx * hw));
        left.push((x - -ty * hw, y - tx * hw));
    }
    // rounded tip (cubic Bézier semicircle, κ≈0.5523)
    let (tip_x, tip_y) = p3;
    let (tang_x, tang_y) = cubic_tang2(p0, p1, p2, p3, 1.0);
    let nx = -tang_y;
    let ny = tang_x;
    let r = hw1.max(0.5);
    const KC: f64 = 0.5523;
    let mut d = format!("M {:.2},{:.2}", right[0].0, right[0].1);
    for pt in right.iter().skip(1) {
        d.push_str(&format!(" L {:.2},{:.2}", pt.0, pt.1));
    }
    d.push_str(&format!(
        " C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2}",
        tip_x + nx * r + tang_x * r * KC,
        tip_y + ny * r + tang_y * r * KC,
        tip_x + tang_x * r + nx * r * KC,
        tip_y + tang_y * r + ny * r * KC,
        tip_x + tang_x * r,
        tip_y + tang_y * r,
        tip_x + tang_x * r - nx * r * KC,
        tip_y + tang_y * r - ny * r * KC,
        tip_x - nx * r + tang_x * r * KC,
        tip_y - ny * r + tang_y * r * KC,
        tip_x - nx * r,
        tip_y - ny * r,
    ));
    for pt in left.iter().rev().skip(1) {
        d.push_str(&format!(" L {:.2},{:.2}", pt.0, pt.1));
    }
    d.push_str(" Z");
    format!("  <path d=\"{d}\" fill=\"{fill}\" class=\"{class}\"/>\n")
}

// §3 — filled branch outline with organic kinks
fn ink2_branch_path(
    px: f64,
    py: f64,
    cx: f64,
    cy: f64,
    w_p: f64,
    w_c: f64,
    fill: &str,
    bigb: f64,
    y_root: f64,
    y_range: f64,
    seed: u32,
) -> String {
    let (c0, c1, c2, c3) = branch_cubic2(px, py, cx, cy);
    let is_short = (py - cy).abs() < bigb * 0.04 && (cx - px).abs() < bigb * 0.04;
    let _ = (y_root, y_range); // width uses t-interpolation along branch

    const N: usize = 12;
    let mut cline: Vec<(f64, f64)> = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let t = i as f64 / N as f64;
        let (x, y) = cubic_pt2(c0, c1, c2, c3, t);
        // Kink only at t≈0.33 and t≈0.67 (2 interior joints, not every sample)
        let pt = if (i == N / 3 || i == 2 * N / 3) && !is_short {
            let (tang_x, tang_y) = cubic_tang2(c0, c1, c2, c3, t);
            let amp = bigb * 0.005 * ink2_jitter(x, y, seed.wrapping_add(i as u32 * 7));
            (x + -tang_y * amp, y + tang_x * amp)
        } else {
            (x, y)
        };
        cline.push(pt);
    }

    let mut right: Vec<(f64, f64)> = Vec::with_capacity(N + 1);
    let mut left: Vec<(f64, f64)> = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let (x, y) = cline[i];
        let (tx, ty) = if i == 0 {
            let (x1, y1) = cline[1];
            let dl = (x1 - x).hypot(y1 - y).max(0.001);
            ((x1 - x) / dl, (y1 - y) / dl)
        } else if i == N {
            let (xp, yp) = cline[N - 1];
            let dl = (x - xp).hypot(y - yp).max(0.001);
            ((x - xp) / dl, (y - yp) / dl)
        } else {
            let (xp, yp) = cline[i - 1];
            let (xn, yn) = cline[i + 1];
            let dl = (xn - xp).hypot(yn - yp).max(0.001);
            ((xn - xp) / dl, (yn - yp) / dl)
        };
        let hw = 0.5 * (w_p + (w_c - w_p) * (i as f64 / N as f64));
        right.push((x + -ty * hw, y + tx * hw));
        left.push((x - -ty * hw, y - tx * hw));
    }

    // tip semicircle
    let (tip_x, tip_y) = cline[N];
    let (xp, yp) = cline[N - 1];
    let dl = (tip_x - xp).hypot(tip_y - yp).max(0.001);
    let tx = (tip_x - xp) / dl;
    let ty_t = (tip_y - yp) / dl;
    let nx = -ty_t;
    let ny = tx;
    let r = (0.5 * w_c).max(0.5);
    const KC: f64 = 0.5523;

    let mut d = format!("M {:.2},{:.2}", right[0].0, right[0].1);
    for pt in right.iter().skip(1) {
        d.push_str(&format!(" L {:.2},{:.2}", pt.0, pt.1));
    }
    d.push_str(&format!(
        " C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2}",
        tip_x + nx * r + tx * r * KC,
        tip_y + ny * r + ty_t * r * KC,
        tip_x + tx * r + nx * r * KC,
        tip_y + ty_t * r + ny * r * KC,
        tip_x + tx * r,
        tip_y + ty_t * r,
        tip_x + tx * r - nx * r * KC,
        tip_y + ty_t * r - ny * r * KC,
        tip_x - nx * r + tx * r * KC,
        tip_y - ny * r + ty_t * r * KC,
        tip_x - nx * r,
        tip_y - ny * r,
    ));
    for pt in left.iter().rev().skip(1) {
        d.push_str(&format!(" L {:.2},{:.2}", pt.0, pt.1));
    }
    d.push_str(" Z");
    format!("  <path d=\"{d}\" fill=\"{fill}\" class=\"tree-branch\"/>\n")
}

// §6 — white longitudinal bark scratches on thick wood
fn ink2_bark_scratches(
    px: f64,
    py: f64,
    cx: f64,
    cy: f64,
    w_p: f64,
    w_c: f64,
    bigb: f64,
    seed: u32,
) -> String {
    let avg_hw = (w_p + w_c) * 0.25;
    if avg_hw < bigb * 0.009 {
        return String::new();
    }
    let n = ((avg_hw / (bigb * 0.004)) as usize + 1).clamp(3, 40);
    let (c0, c1, c2, c3) = branch_cubic2(px, py, cx, cy);
    let seg_len = (cx - px).hypot(cy - py).max(1.0);

    let mut out = String::new();
    for i in 0..n {
        let r1 = ink2_rand(
            px + i as f64 * 1.3,
            py + i as f64 * 1.7,
            seed.wrapping_add(i as u32 * 3),
        );
        let r2 = ink2_rand(
            px + i as f64 * 2.1,
            py + i as f64 * 0.9,
            seed.wrapping_add(i as u32 * 5 + 1),
        );
        let r3 = ink2_rand(
            cx + i as f64 * 1.1,
            cy + i as f64 * 2.3,
            seed.wrapping_add(i as u32 * 7 + 2),
        );

        let t_c = 0.1 + r1 * 0.8;
        let (bx, by) = cubic_pt2(c0, c1, c2, c3, t_c);
        let (tang_x, tang_y) = cubic_tang2(c0, c1, c2, c3, t_c);
        let nx = -tang_y;
        let ny = tang_x;

        let hw_t = 0.5 * (w_p + (w_c - w_p) * t_c);
        // lateral: within ±0.8·hw, biased toward lit side (−0.25·hw)
        let lat = (r2 - 0.5) * 2.0 * hw_t * 0.8 - hw_t * 0.25;

        let half_l = seg_len * (0.075 + r3 * 0.15);
        let sx = bx + nx * lat - tang_x * half_l;
        let sy = by + ny * lat - tang_y * half_l;
        let ex = bx + nx * lat + tang_x * half_l;
        let ey = by + ny * lat + tang_y * half_l;
        let bow = (r2 - 0.5) * half_l * 0.2;
        let mid_x = (sx + ex) * 0.5 + nx * bow;
        let mid_y = (sy + ey) * 0.5 + ny * bow;

        let sw = 0.5 + r1 * 1.1;
        let opacity = 0.55 + r3 * 0.35;
        out.push_str(&format!(
            "  <path d=\"M {sx:.2},{sy:.2} Q {mid_x:.2},{mid_y:.2} {ex:.2},{ey:.2}\" \
             fill=\"none\" stroke=\"#FFFFFF\" stroke-width=\"{sw:.2}\" \
             opacity=\"{opacity:.2}\" stroke-linecap=\"round\" class=\"tree-bark\"/>\n"
        ));
    }
    out
}

// §1 — trunk shaft with bell flare at soil line
fn ink2_trunk(x_trunk: f64, y_root: f64, bigb: f64, trunk_color: u32, y_top: f64) -> String {
    let h_flare = bigb * 0.10;
    let w_trunk = bigb * 0.050;
    let w_flare = w_trunk * 1.85;
    let lean = ink2_jitter(x_trunk, y_root, 101) * bigb * 0.015;
    let y_shaft_top = y_root - h_flare * 2.2;

    const N: usize = 10;
    let mut right: Vec<(f64, f64)> = Vec::new();
    let mut left: Vec<(f64, f64)> = Vec::new();
    for i in 0..=N {
        let t = i as f64 / N as f64; // 0 = y_root (soil), 1 = shaft top
        let y = y_root - t * (y_root - y_shaft_top);
        let x = x_trunk + lean * t;
        let hw = if y > y_root - h_flare {
            // flare zone: power-2.2 swell toward soil
            let s_f = ((y_root - y) / h_flare).clamp(0.0, 1.0);
            0.5 * (w_trunk + (w_flare - w_trunk) * (1.0 - s_f).powf(2.2))
        } else {
            0.5 * w_trunk
        };
        right.push((x + hw, y));
        left.push((x - hw, y));
    }

    let y_range = (y_root - y_top).max(1.0);
    let s_mean = height_frac_s((y_root + y_shaft_top) * 0.5, y_root, y_range);
    let fill = bark_fill_color(trunk_color, s_mean);

    let mut d = format!("M {:.2},{:.2}", right[0].0, right[0].1);
    for pt in right.iter().skip(1) {
        d.push_str(&format!(" L {:.2},{:.2}", pt.0, pt.1));
    }
    for pt in left.iter().rev() {
        d.push_str(&format!(" L {:.2},{:.2}", pt.0, pt.1));
    }
    d.push_str(" Z");
    format!("  <path d=\"{d}\" fill=\"{fill}\" class=\"tree-trunk\"/>\n")
}

// §2 — dark soil mound seating the trunk at the ground line
fn ink2_soil_mound(x_trunk: f64, y_root: f64, bigb: f64, trunk_color: u32) -> String {
    let rx = bigb * 0.050 * 1.85 * 0.5 * 0.9;
    let ry = rx * 0.55;
    let fill = ink2_color_hex(darken_color(trunk_color, 0.10));
    format!(
        "  <ellipse cx=\"{x_trunk:.2}\" cy=\"{y_root:.2}\" rx=\"{rx:.2}\" ry=\"{ry:.2}\" \
         fill=\"{fill}\" opacity=\"0.75\" class=\"tree-root\"/>\n"
    )
}

// §2 — exposed roots fanning below y_root into the extra canvas area
fn ink2_roots(x_trunk: f64, y_root: f64, root_extra: f64, bigb: f64, trunk_color: u32) -> String {
    const N_ROOT: usize = 6;
    let w_max = bigb * 0.050;
    let fill_base = bark_fill_color(trunk_color, 0.0);
    let mut out = String::new();
    for i in 0..N_ROOT {
        let t = i as f64 / (N_ROOT - 1) as f64;
        let dir = t * 2.0 - 1.0; // −1..+1

        let r1 = ink2_rand(
            x_trunk + i as f64 * 7.3,
            y_root + i as f64 * 3.1,
            200 + i as u32,
        );
        let r2 = ink2_rand(
            x_trunk + i as f64 * 5.7,
            y_root - i as f64 * 4.3,
            201 + i as u32,
        );
        let is_central = i == N_ROOT / 2;

        let tip_x = if is_central {
            x_trunk + dir * bigb * 0.02
        } else {
            x_trunk + dir * bigb * (0.30 + r1 * 0.35)
        };
        let tip_y = if is_central {
            y_root + root_extra
        } else {
            y_root + root_extra * (0.55 + r2 * 0.45)
        };

        let start_x = x_trunk + dir * w_max * 0.5 * 1.85 * 0.4;
        let start_y = y_root + bigb * 0.03;
        let dx = tip_x - start_x;
        let dy = tip_y - start_y;
        let p1 = (start_x + dx * 0.15, start_y + dy * 0.50);
        let p2 = (start_x + dx * 0.75, tip_y - dy * 0.10);

        out.push_str(&ink2_cubic_fill(
            (start_x, start_y),
            p1,
            p2,
            (tip_x, tip_y),
            w_max * 0.45 * 0.5,
            w_max * 0.04 * 0.5,
            &fill_base,
            "tree-root",
            8,
        ));
    }
    out
}

// §8 — leaf cluster; returns (back 80%, front 20%)
fn ink2_leaves(
    cx: f64,
    cy: f64,
    k: usize,
    leaf_color: u32,
    bigb: f64,
    _y_top: f64,
    seed_off: u64,
) -> (String, String) {
    let stroke_col = ink2_color_hex(darken_color(leaf_color, 0.25));
    let r_disc = bigb * 0.18;
    let leaf_len = bigb * 0.012;
    let mut back = String::new();
    let mut front = String::new();

    let mut seed = seed_off
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);

    for i in 0..k {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let angle = (seed & 0xFFFF) as f64 / 65535.0 * std::f64::consts::TAU;
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let r = r_disc * ((seed & 0xFFFF) as f64 / 65535.0).sqrt();
        let lx = cx + angle.cos() * r;
        let ly = cy + angle.sin() * r * 0.7;

        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let sz = leaf_len * 0.5 * (0.75 + (seed & 0xFF) as f64 / 255.0 * 0.50);
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let rot = (seed & 0xFFFF) as f64 / 65535.0 * 180.0;
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let sw = 0.6 + (seed & 0xFF) as f64 / 255.0 * 0.4;

        let elem = format!(
            "  <ellipse cx=\"{lx:.2}\" cy=\"{ly:.2}\" rx=\"{sz:.2}\" ry=\"{:.2}\" \
             transform=\"rotate({rot:.1},{lx:.2},{ly:.2})\" fill=\"none\" \
             stroke=\"{stroke_col}\" stroke-width=\"{sw:.2}\" class=\"tree-leaf\"/>\n",
            sz * 0.6
        );
        if i * 5 < k * 4 {
            back.push_str(&elem);
        } else {
            front.push_str(&elem);
        }
    }
    (back, front)
}

fn render_ink2_style(branches: &[Branch], prefs: &Prefs) -> String {
    if branches.is_empty() {
        return String::new();
    }
    let (y_root, y_top) = y_bounds(branches);
    if y_root <= y_top {
        return String::new();
    }
    let y_range = (y_root - y_top).max(1.0);
    let bigb = y_range;
    let trunk_color = prefs.output.style.realistic_tree.trunk_color as u32;
    let leaf_color = prefs.output.style.realistic_tree.leaf_color as u32;
    let x_trunk = root_center_x(branches, y_root);
    let root_extra = y_range * 0.45;

    let leaf_k: usize = match prefs.output.style.realistic_tree.leaf_density.as_str() {
        "none" => 0,
        "low" => 400,
        "high" => 2400,
        _ => 1100,
    };

    let mut leaves_back = String::new();
    let mut soil_svg = String::new();
    let mut roots_svg = String::new();
    let mut trunk_svg = String::new();
    let mut wood: Vec<(f64, String)> = Vec::new(); // (sort-key width, svg)
    let mut bark_svg = String::new();
    let mut leaves_front = String::new();

    trunk_svg.push_str(&ink2_trunk(x_trunk, y_root, bigb, trunk_color, y_top));
    roots_svg.push_str(&ink2_roots(x_trunk, y_root, root_extra, bigb, trunk_color));
    soil_svg.push_str(&ink2_soil_mound(x_trunk, y_root, bigb, trunk_color));

    for (bi, branch) in branches.iter().enumerate() {
        if branch.parent_pts.is_empty() || branch.child_pts.is_empty() {
            continue;
        }
        let px =
            branch.parent_pts.iter().map(|p| p.0).sum::<f64>() / branch.parent_pts.len() as f64;
        let py =
            branch.parent_pts.iter().map(|p| p.1).sum::<f64>() / branch.parent_pts.len() as f64;
        let s_p = height_frac_s(py, y_root, y_range);
        let w_p = ink2_width(s_p, bigb);

        let n_ch = branch.child_pts.len();
        let mean_cy = branch.child_pts.iter().map(|p| p.1).sum::<f64>() / n_ch as f64;

        // §4 hub at 35% from parent toward mean child y
        let y_hub = py - 0.35 * (py - mean_cy);
        // Stem rises straight above the parent; limbs fan out from there.
        let x_hub = px;
        let s_hub = height_frac_s(y_hub, y_root, y_range);
        let w_hub = ink2_width(s_hub, bigb).max(w_p);

        // stem: parent → hub
        let seed_base = bi as u32 * 1000;
        let s_stem = height_frac_s((py + y_hub) * 0.5, y_root, y_range);
        let stem_fill = bark_fill_color(trunk_color, s_stem);
        wood.push((
            w_p,
            ink2_branch_path(
                px, py, x_hub, y_hub, w_p, w_hub, &stem_fill, bigb, y_root, y_range, seed_base,
            ),
        ));
        bark_svg.push_str(&ink2_bark_scratches(
            px,
            py,
            x_hub,
            y_hub,
            w_p,
            w_hub,
            bigb,
            seed_base + 500,
        ));

        // children sorted by x
        let mut sorted_ch = branch.child_pts.clone();
        sorted_ch.sort_by(|a, bv| a.0.partial_cmp(&bv.0).unwrap_or(std::cmp::Ordering::Equal));

        // §4b da-Vinci width split: w_sub_i = w_hub * sqrt(w_c_i^2 / Σ w_c_j^2)
        let child_ws: Vec<f64> = sorted_ch
            .iter()
            .map(|(_, chy)| ink2_width(height_frac_s(*chy, y_root, y_range), bigb))
            .collect();
        let sum_sq: f64 = child_ws.iter().map(|w| w * w).sum::<f64>().max(1e-9);
        let sub_ws: Vec<f64> = child_ws
            .iter()
            .map(|w_c| (w_hub * (w_c * w_c / sum_sq).sqrt()).min(*w_c))
            .collect();

        // limb fan: hub → each child
        for (ci, ((chx, chy), (w_c, w_sub))) in sorted_ch
            .iter()
            .zip(child_ws.iter().zip(sub_ws.iter()))
            .enumerate()
        {
            let cseed = bi as u32 * 1000 + ci as u32 * 10 + 1;
            let s_c = height_frac_s(*chy, y_root, y_range);
            let s_mid = height_frac_s((*chy + y_hub) * 0.5, y_root, y_range);
            let cfill = bark_fill_color(trunk_color, s_mid);
            wood.push((
                *w_sub,
                ink2_branch_path(
                    x_hub, y_hub, *chx, *chy, *w_sub, *w_c, &cfill, bigb, y_root, y_range, cseed,
                ),
            ));
            bark_svg.push_str(&ink2_bark_scratches(
                x_hub,
                y_hub,
                *chx,
                *chy,
                *w_sub,
                *w_c,
                bigb,
                cseed + 500,
            ));

            // Tip cluster
            if leaf_k > 0 {
                let (back, frt) = ink2_leaves(
                    *chx, *chy, leaf_k, leaf_color, bigb, y_top,
                    (bi * 100 + ci) as u64,
                );
                leaves_back.push_str(&back);
                leaves_front.push_str(&frt);
            }

            // Along-branch scatter: 6 points along hub→child, leaf_k/5 each
            if leaf_k > 0 {
                let along_k = (leaf_k / 5).max(1);
                for ai in 0..6usize {
                    let at = (ai as f64 + 0.5) / 6.0;
                    let alx = x_hub + (*chx - x_hub) * at;
                    let aly = y_hub + (*chy - y_hub) * at;
                    let (back, frt) = ink2_leaves(
                        alx, aly, along_k, leaf_color, bigb, y_top,
                        (bi * 10000 + ci * 100 + ai) as u64 + 200000,
                    );
                    leaves_back.push_str(&back);
                    leaves_front.push_str(&frt);
                }
            }
            let _ = s_c;
        }

        // Along-stem scatter: 3 points along parent→hub, leaf_k/8 each
        if leaf_k > 0 {
            let stem_k = (leaf_k / 8).max(1);
            for ai in 0..3usize {
                let at = (ai as f64 + 0.5) / 3.0;
                let alx = px + (x_hub - px) * at;
                let aly = py + (y_hub - py) * at;
                let (back, frt) = ink2_leaves(
                    alx, aly, stem_k, leaf_color, bigb, y_top,
                    (bi * 10000) as u64 + 300000 + ai as u64,
                );
                leaves_back.push_str(&back);
                leaves_front.push_str(&frt);
            }
        }
    }

    // §9 sort wood thick → thin so thinner branches overlay thicker ones
    wood.sort_by(|a, bv| bv.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // §9 rendering order: leaves-back → soil → roots → trunk → wood → bark → leaves-front
    let mut out = String::new();
    out.push_str(&leaves_back);
    out.push_str(&soil_svg);
    out.push_str(&roots_svg);
    out.push_str(&trunk_svg);
    for (_, svg) in &wood {
        out.push_str(svg);
    }
    out.push_str(&bark_svg);
    out.push_str(&leaves_front);
    out
}
