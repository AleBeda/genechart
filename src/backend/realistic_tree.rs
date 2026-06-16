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
//!   "ink"     — hand-drawn coherent tree (modelled on the reference in
//!               tests/fixtures/local/realistic_tree_samples): a flared trunk visible below the
//!               root box, organic buttress roots, one continuous tapered stroke per child that
//!               runs along the main limb and turns up under its box, short white bark-grain
//!               scratches with a lit side, and a continuous flat-topped open-ellipse leaf canopy

use crate::preferences::Prefs;
use crate::scene::{ConnectorPrimitive, Primitive};

// ── Public API ────────────────────────────────────────────────────────────────

/// One connector paired with a stable seed key (the id of its nearest enclosing
/// non-empty group, e.g. `"F12-connectors"` in the boxed_couples layout). The key
/// is derived from the GEDCOM family/individual ids, so any per-branch randomness
/// keyed off it is reproducible regardless of HashMap emit order — and stable
/// across preference changes (box size, gaps) that only move coordinates.
pub type SeededConnector<'a> = (String, &'a ConnectorPrimitive);

/// Recursively collect all `ConnectorPrimitive` references from a primitive tree,
/// pairing each with the id of its nearest enclosing non-empty group.
pub fn collect_connectors<'a>(
    primitives: &'a [Primitive],
    current_id: &str,
    out: &mut Vec<SeededConnector<'a>>,
) {
    for prim in primitives {
        match prim {
            Primitive::Connector(c) => out.push((current_id.to_string(), c)),
            Primitive::Group(g) => {
                let id = if g.id.is_empty() { current_id } else { &g.id };
                collect_connectors(&g.children, id, out);
            }
            _ => {}
        }
    }
}

/// FNV-1a 32-bit hash of a string — a stable, deterministic seed from a GEDCOM id.
fn seed_from_key(key: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in key.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Extra canvas height (display-space units) to add below the SVG for tree roots.
///
/// Only non-zero for root_pos = bottom charts (the default). Returns 0.0 for empty
/// connector lists or when parent points are above child points (root_pos = top).
pub fn root_extra_height(connectors: &[SeededConnector]) -> f64 {
    if connectors.is_empty() {
        return 0.0;
    }
    let sample_parent_y = connectors[0]
        .1
        .parent_points
        .first()
        .map(|p| p.y)
        .unwrap_or(0.0);
    let sample_child_y = connectors[0]
        .1
        .child_points
        .first()
        .map(|p| p.y)
        .unwrap_or(0.0);
    if sample_parent_y <= sample_child_y {
        return 0.0;
    }
    let y_root: f64 = connectors
        .iter()
        .flat_map(|(_, c)| c.parent_points.iter().map(|p| p.y))
        .fold(f64::NEG_INFINITY, f64::max);
    let y_top: f64 = connectors
        .iter()
        .flat_map(|(_, c)| c.child_points.iter().map(|p| p.y))
        .fold(f64::INFINITY, f64::min);
    ((y_root - y_top) * 0.45).max(40.0)
}

/// Render the full tree-branch SVG layer.
///
/// Returns an SVG fragment (no outer `<svg>` tag) wrapped in
/// `<g id="realistic-tree" class="realistic-tree">…</g>`.
pub fn render_tree_layer(
    connectors: &[SeededConnector],
    to_svg_x: &dyn Fn(f64) -> f64,
    to_svg_y: &dyn Fn(f64) -> f64,
    prefs: &Prefs,
) -> String {
    if connectors.is_empty() {
        return String::new();
    }

    let branches: Vec<Branch> = connectors
        .iter()
        .map(|(seed_key, c)| Branch {
            seed: seed_from_key(seed_key),
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
    /// Stable per-branch seed derived from the GEDCOM family/individual id, so any
    /// randomness keyed off it is reproducible regardless of emit order.
    seed: u32,
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

// ── Shared helpers for the "ink" style ───────────────────────────────────────

// Deterministic hash PRNG (stable SVG diffs across runs).
fn ink_rand(px: f64, py: f64, seed: u32) -> f64 {
    let a = (px * 7.0).round();
    let b = (py * 7.0).round();
    let v = (a * 12.9898 + b * 78.233 + seed as f64 * 37.719).sin() * 43758.5453;
    v - v.floor()
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

fn ink_color_hex(c: u32) -> String {
    format!("#{:06X}", c & 0xFFFFFF)
}

// bark fill: darkens near base (s≈0), lightens toward crown (s≈1)
fn bark_fill_color(trunk_color: u32, s: f64) -> String {
    let c_base = darken_color(trunk_color, 0.12);
    let c_tip = lighten_color(trunk_color, 0.10);
    ink_color_hex(mix_color(c_base, c_tip, s.clamp(0.0, 1.0)))
}

// §5 — height fraction s: 0 at soil (y_root), 1 at crown (y_top)
fn height_frac_s(y: f64, y_root: f64, y_range: f64) -> f64 {
    ((y_root - y) / y_range).clamp(0.0, 1.0)
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

// §2 — dark soil mound seating the trunk at the ground line
fn ink_soil_mound(x_trunk: f64, y_root: f64, bigb: f64, trunk_color: u32) -> String {
    let rx = bigb * 0.050 * 1.85 * 0.5 * 0.9;
    let ry = rx * 0.55;
    let fill = ink_color_hex(darken_color(trunk_color, 0.10));
    format!(
        "  <ellipse cx=\"{x_trunk:.2}\" cy=\"{y_root:.2}\" rx=\"{rx:.2}\" ry=\"{ry:.2}\" \
         fill=\"{fill}\" opacity=\"0.75\" class=\"tree-root\"/>\n"
    )
}

// ── Style: ink ────────────────────────────────────────────────────────────────
//
// Hand-drawn coherent tree, modelled on the artist's reference
// (tests/fixtures/local/realistic_tree_samples). Two ideas drive the look:
//
//  1. WOOD as a real tree, not a wiring diagram. A flared trunk (visible below the
//     root box) with organic buttress roots; each child is one continuous tapered
//     stroke that runs along the main limb then turns up sharply under its box.
//     The visible main limb is the overlap of those strokes shedding branches.
//
//  2. FOLIAGE as one continuous canopy, not isolated puff-balls. The whole crown
//     is filled with a single flat-topped, billowy, clump-textured mass of
//     open-ellipse leaves (value-noise density), drawn behind the boxes.

// §5 — depth-based full width: thick at base (s=0), thin at crown (s=1), power-law.
fn ink_width(s: f64, bigb: f64) -> f64 {
    let w_max = bigb * 0.050;
    let w_min = bigb * 0.005;
    w_min + (w_max - w_min) * (1.0 - s.clamp(0.0, 1.0)).powf(1.8)
}

// §2 — grass tufts at the soil line, growing upward; gated by caller on density.
fn ink_grass(x_trunk: f64, y_root: f64, bigb: f64, trunk_color: u32) -> String {
    let col = ink_color_hex(darken_color(trunk_color, 0.40));
    let spread = bigb * 0.13; // ≈ 1.4 × full flare half-width
    let mut out = String::new();
    for i in 0..24u32 {
        let r1 = ink_rand(x_trunk + i as f64 * 3.1, y_root, 700 + i);
        let r2 = ink_rand(x_trunk - i as f64 * 2.3, y_root, 740 + i);
        let r3 = ink_rand(x_trunk + i as f64 * 1.7, y_root, 780 + i);
        let bx = x_trunk + (r1 * 2.0 - 1.0) * spread;
        let len = bigb * (0.02 + r2 * 0.03);
        let lean = (r3 * 2.0 - 1.0) * len * 0.5;
        let tip_x = bx + lean;
        let tip_y = y_root - len;
        let mid_x = bx + lean * 0.4;
        let mid_y = y_root - len * 0.55;
        let sw = 0.5 + r2 * 0.7;
        out.push_str(&format!(
            "  <path d=\"M {bx:.2},{y_root:.2} Q {mid_x:.2},{mid_y:.2} {tip_x:.2},{tip_y:.2}\" \
             fill=\"none\" stroke=\"{col}\" stroke-width=\"{sw:.2}\" stroke-linecap=\"round\" \
             class=\"tree-grass\"/>\n"
        ));
    }
    out
}

// Smooth low-frequency wander in ~[-1, 1] for organic branch centrelines/edges.
fn ink_wave(t: f64, seed: u32) -> f64 {
    let s = seed as f64;
    let p1 = (s * 0.013).sin() * std::f64::consts::TAU;
    let p2 = (s * 0.027).sin() * std::f64::consts::TAU;
    let k1 = 2.0 + (seed % 3) as f64;
    let k2 = 5.0 + (seed % 4) as f64;
    let pi = std::f64::consts::PI;
    0.62 * (t * pi * k1 + p1).sin() + 0.38 * (t * pi * k2 + p2).sin()
}

// Local unit tangent of a sampled centreline at index `i` (finite difference).
fn ink_cl_tangent(cl: &[(f64, f64)], i: usize) -> (f64, f64) {
    let n = cl.len() - 1;
    let (a, b) = if i == 0 {
        (cl[0], cl[1])
    } else if i == n {
        (cl[n - 1], cl[n])
    } else {
        (cl[i - 1], cl[i + 1])
    };
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = dx.hypot(dy).max(1e-4);
    (dx / len, dy / len)
}

// Smooth 1-D value noise in [-1, 1] with `freq` random humps over t∈[0,1].
// More organic than a sum of sines (which reads as a regular wave).
fn ink_wander1(t: f64, freq: f64, seed: u32) -> f64 {
    let g = t * freq;
    let i0 = g.floor();
    let f = g - i0;
    let s = f * f * (3.0 - 2.0 * f);
    let a = ink_rand(i0, 7.0, seed) * 2.0 - 1.0;
    let b = ink_rand(i0 + 1.0, 7.0, seed) * 2.0 - 1.0;
    a + (b - a) * s
}

// Organic filled branch along a cubic: the centreline gently WANDERS (irregular
// value-noise, not a regular sine), the two edges carry independent small BUMPS,
// width is near-constant then tapers toward the rounded tip. `w0`/`w1` are full
// widths. When `sag_only` is set (horizontal limbs), upward excursions are
// damped so the limb never humps up into the boxes above it.
fn ink_fill_cubic(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    w0: f64,
    w1: f64,
    fill: &str,
    bigb: f64,
    seed: u32,
    sag_only: bool,
) -> String {
    const N: usize = 28;
    let pi = std::f64::consts::PI;
    let seg_len = (p3.0 - p0.0).hypot(p3.1 - p0.1).max(1.0);
    let wander = (seg_len * 0.04).min(bigb * 0.022);

    // wandering centreline (displacement pinned to 0 at both ends)
    let mut cl: Vec<(f64, f64)> = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let t = i as f64 / N as f64;
        let (bx, by) = cubic_pt2(p0, p1, p2, p3, t);
        let (tx, ty) = cubic_tang2(p0, p1, p2, p3, t);
        let env = (t * pi).sin();
        let off = ink_wander1(t, 3.3, seed) * wander * env;
        let cx = bx - ty * off;
        let mut cy = by + tx * off;
        if sag_only && cy < by {
            // damp upward (toward the boxes); keep full downward sag
            cy = by + (cy - by) * 0.2;
        }
        cl.push((cx, cy));
    }

    // width profile: hold ~90 % of full width through most of the length, then
    // taper to the tip over the last ~30 % (reference branches keep an even
    // width then narrow near the end / fork).
    let half_w = |t: f64| -> f64 {
        let w = w0 + (w1 - w0) * t;
        let taper = if t < 0.70 {
            1.0
        } else {
            1.0 - 0.55 * ((t - 0.70) / 0.30).powf(1.4)
        };
        0.5 * w * taper
    };

    let mut right: Vec<(f64, f64)> = Vec::with_capacity(N + 1);
    let mut left: Vec<(f64, f64)> = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let t = i as f64 / N as f64;
        let (tx, ty) = ink_cl_tangent(&cl, i);
        let nx = -ty;
        let ny = tx;
        let base_hw = half_w(t);
        let er = (1.0 + ink_wave(t * 6.5, seed.wrapping_add(101)) * 0.09).max(0.25);
        let el = (1.0 + ink_wave(t * 6.5, seed.wrapping_add(202)) * 0.09).max(0.25);
        let (x, y) = cl[i];
        right.push((x + nx * base_hw * er, y + ny * base_hw * er));
        left.push((x - nx * base_hw * el, y - ny * base_hw * el));
    }

    // rounded tip at p3
    let (tip_x, tip_y) = cl[N];
    let (tx, ty) = ink_cl_tangent(&cl, N);
    let nx = -ty;
    let ny = tx;
    let r = half_w(1.0).max(0.5);
    const KC: f64 = 0.5523;

    let mut d = format!("M {:.2},{:.2}", right[0].0, right[0].1);
    for pt in right.iter().skip(1) {
        d.push_str(&format!(" L {:.2},{:.2}", pt.0, pt.1));
    }
    d.push_str(&format!(
        " C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2}",
        tip_x + nx * r + tx * r * KC,
        tip_y + ny * r + ty * r * KC,
        tip_x + tx * r + nx * r * KC,
        tip_y + ty * r + ny * r * KC,
        tip_x + tx * r,
        tip_y + ty * r,
        tip_x + tx * r - nx * r * KC,
        tip_y + ty * r - ny * r * KC,
        tip_x - nx * r + tx * r * KC,
        tip_y - ny * r + ty * r * KC,
        tip_x - nx * r,
        tip_y - ny * r,
    ));
    for pt in left.iter().rev().skip(1) {
        d.push_str(&format!(" L {:.2},{:.2}", pt.0, pt.1));
    }
    d.push_str(" Z");
    format!("  <path d=\"{d}\" fill=\"{fill}\" class=\"tree-branch\"/>\n")
}

// Dense white bark grain along the wood axis. Strokes run longitudinally, are
// brighter/denser on the lit (left) side and fade to a darker shadow side —
// reproducing the cylindrical highlight and stippled bark of the reference.
fn ink_grain(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    w0: f64,
    w1: f64,
    bigb: f64,
    seed: u32,
) -> String {
    let avg_hw = (w0 + w1) * 0.25;
    if avg_hw < bigb * 0.006 {
        return String::new(); // only thick wood is textured
    }
    let seg_len = (p3.0 - p0.0).hypot(p3.1 - p0.1).max(1.0);
    let n = ((avg_hw / (bigb * 0.0014)) as usize).clamp(14, 150);
    let mut out = String::new();
    for i in 0..n {
        let fi = i as f64;
        let r1 = ink_rand(
            p0.0 + fi * 1.3,
            p0.1 + fi * 1.7,
            seed.wrapping_add(i as u32 * 3),
        );
        let r2 = ink_rand(
            p0.0 + fi * 2.1,
            p0.1 + fi * 0.9,
            seed.wrapping_add(i as u32 * 5 + 1),
        );
        let r3 = ink_rand(
            p3.0 + fi * 1.1,
            p3.1 + fi * 2.3,
            seed.wrapping_add(i as u32 * 7 + 2),
        );
        let t = 0.05 + r1 * 0.9;
        let (bx, by) = cubic_pt2(p0, p1, p2, p3, t);
        let (tx, ty) = cubic_tang2(p0, p1, p2, p3, t);
        let nx = -ty;
        let ny = tx;
        let hw = 0.5 * (w0 + (w1 - w0) * t);
        let lat_frac = r2 * 2.0 - 1.0; // −1 = lit side (left), +1 = shadow (right)
        let lat = lat_frac * hw * 0.78;
        let half_l = (seg_len * (0.025 + r3 * 0.045)).min(bigb * 0.018);
        let sx = bx + nx * lat - tx * half_l;
        let sy = by + ny * lat - ty * half_l;
        let ex = bx + nx * lat + tx * half_l;
        let ey = by + ny * lat + ty * half_l;
        let bow = (r2 - 0.5) * half_l * 0.25;
        let mx = (sx + ex) * 0.5 + nx * bow;
        let my = (sy + ey) * 0.5 + ny * bow;
        let lit = (1.0 - lat_frac) * 0.5; // 1 on the lit side
        let opacity = (0.38 + 0.48 * lit).clamp(0.28, 0.9);
        let sw = 0.45 + r1 * 0.9;
        out.push_str(&format!(
            "  <path d=\"M {sx:.2},{sy:.2} Q {mx:.2},{my:.2} {ex:.2},{ey:.2}\" fill=\"none\" \
             stroke=\"#FFFFFF\" stroke-width=\"{sw:.2}\" opacity=\"{opacity:.2}\" \
             stroke-linecap=\"round\" class=\"tree-bark\"/>\n"
        ));
    }
    out
}

// Irregular flared trunk from the first fork (`top_y`) down to `base_y` — which
// sits BELOW the root box so a visible length of trunk shows under the oldest
// ancestor. Strong base flare, wandering outline, dense vertical grain, lit-side
// highlight lens.
fn ink_trunk(
    x_trunk: f64,
    y_root: f64,
    top_y: f64,
    base_y: f64,
    bigb: f64,
    trunk_color: u32,
) -> String {
    let w_trunk = bigb * 0.055;
    let w_flare = w_trunk * 1.95;
    let h = (base_y - top_y).max(1.0);
    let h_flare = (h * 0.45).min(bigb * 0.10);

    const N: usize = 16;
    let mut right: Vec<(f64, f64)> = Vec::with_capacity(N + 1);
    let mut left: Vec<(f64, f64)> = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let t = i as f64 / N as f64; // 0 at top (fork), 1 at base (soil)
        let y = top_y + t * h;
        let cx = x_trunk + ink_wave(t * 1.7, 731) * bigb * 0.012;
        let near_base = (y - (base_y - h_flare)) / h_flare; // <0 above flare
        let flare = if near_base > 0.0 {
            near_base.clamp(0.0, 1.0).powf(2.0)
        } else {
            0.0
        };
        let hw = 0.5 * (w_trunk + (w_flare - w_trunk) * flare);
        let er = (1.0 + ink_wave(t * 5.0, 742) * 0.10).max(0.3);
        let el = (1.0 + ink_wave(t * 5.0, 753) * 0.10).max(0.3);
        right.push((cx + hw * er, y));
        left.push((cx - hw * el, y));
    }

    let fill = bark_fill_color(trunk_color, height_frac_s(top_y, y_root, bigb));
    let mut d = format!("M {:.2},{:.2}", right[0].0, right[0].1);
    for pt in right.iter().skip(1) {
        d.push_str(&format!(" L {:.2},{:.2}", pt.0, pt.1));
    }
    for pt in left.iter().rev() {
        d.push_str(&format!(" L {:.2},{:.2}", pt.0, pt.1));
    }
    d.push_str(" Z");
    let mut out = format!("  <path d=\"{d}\" fill=\"{fill}\" class=\"tree-trunk\"/>\n");

    // vertical grain over the full trunk height
    let mid_y = (base_y + top_y) * 0.5;
    out.push_str(&ink_grain(
        (x_trunk, base_y),
        (x_trunk, mid_y),
        (x_trunk, mid_y),
        (x_trunk, top_y),
        w_flare,
        w_trunk,
        bigb,
        760,
    ));

    // lit-side highlight lens
    let cx = x_trunk - w_trunk * 0.25;
    out.push_str(&format!(
        "  <ellipse cx=\"{cx:.2}\" cy=\"{mid_y:.2}\" rx=\"{:.2}\" ry=\"{:.2}\" \
         fill=\"#FFFFFF\" opacity=\"0.10\" class=\"tree-trunk\"/>\n",
        w_trunk * 0.30,
        h * 0.42
    ));
    out
}

// Exposed roots flaring from the trunk base (`soil_y`) into the soil over
// `depth`. Chunky and organic near the trunk (buttress-like), tapering to fine
// points, with grain on the thick part.
fn ink_roots(x_trunk: f64, soil_y: f64, depth: f64, bigb: f64, trunk_color: u32) -> String {
    let fill = bark_fill_color(trunk_color, 0.0); // darkest, at the base
    let w_flare = bigb * 0.055 * 1.95; // trunk base full width
    const N: usize = 7;
    let mut out = String::new();
    for i in 0..N {
        let fi = i as f64;
        let t = fi / (N - 1) as f64;
        let dir = t * 2.0 - 1.0; // −1 .. +1
        let central = i == N / 2;
        let r1 = ink_rand(x_trunk + fi * 7.3, soil_y + fi * 3.1, 820 + i as u32);
        let r2 = ink_rand(x_trunk - fi * 5.7, soil_y - fi * 4.3, 840 + i as u32);

        let (tip_x, tip_y) = if central {
            (
                x_trunk + dir * bigb * 0.03,
                soil_y + depth * (0.85 + r2 * 0.15),
            )
        } else {
            let reach = bigb * (0.08 + r1 * 0.16); // 0.08 .. 0.24 B (short)
            let drop = depth * (0.45 + r2 * 0.45);
            (x_trunk + dir * reach, soil_y + drop)
        };

        let start_x = x_trunk + dir * w_flare * 0.30;
        let start_y = soil_y - bigb * 0.01;
        let dx = tip_x - start_x;
        let dy = tip_y - start_y;
        // steep departure from the flare, then bend toward shallow soil contact
        let p0 = (start_x, start_y);
        let p1 = (start_x + dx * 0.20, start_y + dy * 0.55);
        let p2 = (start_x + dx * 0.70, tip_y - dy * 0.12);
        let p3 = (tip_x, tip_y);
        let w_top = (w_flare * (0.34 - dir.abs() * 0.10)).max(w_flare * 0.14);
        let w_tip = w_flare * 0.02;
        out.push_str(&ink_fill_cubic(
            p0,
            p1,
            p2,
            p3,
            w_top,
            w_tip,
            &fill,
            bigb,
            820 + i as u32,
            false,
        ));
        out.push_str(&ink_grain(
            p0,
            p1,
            p2,
            p3,
            w_top,
            w_tip,
            bigb,
            860 + i as u32,
        ));
    }
    out
}

// Smooth (bilinear, Hermite-faded) value noise on a grid of `scale` units.
// Returns ~[0,1]; continuous so foliage forms coherent clumps and gaps.
fn ink_noise(x: f64, y: f64, scale: f64, seed: u32) -> f64 {
    let gx = x / scale;
    let gy = y / scale;
    let x0 = gx.floor();
    let y0 = gy.floor();
    let fx = gx - x0;
    let fy = gy - y0;
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let corner = |i: f64, j: f64| ink_rand((x0 + i) * scale, (y0 + j) * scale, seed);
    let a = corner(0.0, 0.0);
    let b = corner(1.0, 0.0);
    let c = corner(0.0, 1.0);
    let d = corner(1.0, 1.0);
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sy
}

// One continuous canopy filling the whole crown: a flat-topped, billowy,
// clump-textured mass of open-ellipse leaves drawn behind the boxes.
// Returns (back 82 %, front 18 %).
fn ink_canopy(
    branches: &[Branch],
    y_root: f64,
    y_top: f64,
    bigb: f64,
    leaf_color: u32,
    total: usize,
) -> (String, String) {
    if total == 0 {
        return (String::new(), String::new());
    }
    // crown horizontal extent = spread of all child tips, with a margin
    let min_cx = branches
        .iter()
        .flat_map(|b| b.child_pts.iter().map(|p| p.0))
        .fold(f64::INFINITY, f64::min);
    let max_cx = branches
        .iter()
        .flat_map(|b| b.child_pts.iter().map(|p| p.0))
        .fold(f64::NEG_INFINITY, f64::max);
    if !min_cx.is_finite() || !max_cx.is_finite() {
        return (String::new(), String::new());
    }
    let margin = bigb * 0.10;
    let crown_l = min_cx - margin;
    let crown_r = max_cx + margin;
    let center_x = (crown_l + crown_r) * 0.5;
    let half_w = ((crown_r - crown_l) * 0.5).max(1.0);
    let crown_top = y_top - bigb * 0.04;
    let crown_bot = y_root - bigb * 0.12; // keep the trunk/lowest limbs bare
    let band = (crown_bot - crown_top).max(1.0);

    // Dome silhouette: flat across the middle, dropping toward the edges, with a
    // noisy billowing top edge — the foliage "fills below" this line.
    let top_y = |x: f64| -> f64 {
        let e = ((x - center_x).abs() / half_w).clamp(0.0, 1.0);
        let shoulder = e.powi(4) * band * 0.62; // round only near the edges
        let billow = (ink_noise(x, 0.0, bigb * 0.10, 4_001) - 0.5) * bigb * 0.10;
        crown_top + shoulder + billow
    };

    let stroke_col = ink_color_hex(darken_color(leaf_color, 0.25));
    let leaf_len = bigb * 0.011;
    let mut back = String::new();
    let mut front = String::new();
    let mut seed: u64 = 0x1234_5678_9abc_def1;
    let mut rng = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as f64 / (1u64 << 31) as f64
    };

    let mut placed = 0usize;
    let mut tries = 0usize;
    let max_tries = total * 4;
    while placed < total && tries < max_tries {
        tries += 1;
        let x = crown_l + rng() * (crown_r - crown_l);
        let ty = top_y(x);
        if ty >= crown_bot {
            continue;
        }
        let y = ty + rng() * (crown_bot - ty);
        // clump texture: denser where the noise field is high
        let dens = ink_noise(x, y, bigb * 0.06, 4_002);
        if rng() > 0.28 + 0.72 * dens {
            continue;
        }
        // fade the very top edge so it doesn't look like a hard ceiling
        let edge_fade = ((y - ty) / (band * 0.12)).clamp(0.0, 1.0);
        if rng() > edge_fade.max(0.35) {
            continue;
        }
        placed += 1;

        let sz = leaf_len * 0.5 * (0.72 + rng() * 0.56);
        let rot = rng() * 180.0;
        let sw = 0.55 + rng() * 0.45;
        let elem = format!(
            "  <ellipse cx=\"{x:.2}\" cy=\"{y:.2}\" rx=\"{sz:.2}\" ry=\"{:.2}\" \
             transform=\"rotate({rot:.1},{x:.2},{y:.2})\" fill=\"none\" \
             stroke=\"{stroke_col}\" stroke-width=\"{sw:.2}\" class=\"tree-leaf\"/>\n",
            sz * 0.62
        );
        if rng() < 0.82 {
            back.push_str(&elem);
        } else {
            front.push_str(&elem);
        }
    }
    (back, front)
}

fn render_ink_style(branches: &[Branch], prefs: &Prefs) -> String {
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

    let total_leaves: usize = match prefs.output.style.realistic_tree.leaf_density.as_str() {
        "none" => 0,
        "low" => 3500,
        "high" => 18000,
        _ => 9000,
    };

    let mut soil_svg = String::new();
    let mut roots_svg = String::new();
    let mut trunk_svg = String::new();
    let mut wood: Vec<(f64, String)> = Vec::new();
    let mut bark_svg = String::new();
    let mut grass_svg = String::new();

    // Trunk rises from the soil to the first fork (the root branch's hub), so it
    // meets the main limbs cleanly instead of poking past them.
    let trunk_top_y = branches
        .iter()
        .filter(|b| !b.parent_pts.is_empty() && !b.child_pts.is_empty())
        .map(|b| {
            let py = b.parent_pts.iter().map(|p| p.1).sum::<f64>() / b.parent_pts.len() as f64;
            let mcy = b.child_pts.iter().map(|p| p.1).sum::<f64>() / b.child_pts.len() as f64;
            (py, py - 0.35 * (py - mcy))
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, hub_y)| hub_y)
        .unwrap_or(y_root - bigb * 0.10)
        .min(y_root - bigb * 0.05);
    // Extend the trunk a visible distance below the root box before the roots
    // flare, so the oldest ancestor sits on a real trunk rather than directly on
    // the root crown.
    let below_h = root_extra * 0.40;
    let soil_y = y_root + below_h;
    let root_depth = (root_extra - below_h).max(bigb * 0.05);
    trunk_svg.push_str(&ink_trunk(
        x_trunk,
        y_root,
        trunk_top_y,
        soil_y,
        bigb,
        trunk_color,
    ));
    roots_svg.push_str(&ink_roots(x_trunk, soil_y, root_depth, bigb, trunk_color));
    soil_svg.push_str(&ink_soil_mound(x_trunk, soil_y, bigb, trunk_color));
    if total_leaves > 0 {
        grass_svg.push_str(&ink_grass(x_trunk, soil_y, bigb, trunk_color));
    }

    // Continuous canopy behind everything (and a thin front layer for depth).
    let (leaves_back, leaves_front) =
        ink_canopy(branches, y_root, y_top, bigb, leaf_color, total_leaves);

    for branch in branches.iter() {
        if branch.parent_pts.is_empty() || branch.child_pts.is_empty() {
            continue;
        }
        let px =
            branch.parent_pts.iter().map(|p| p.0).sum::<f64>() / branch.parent_pts.len() as f64;
        let py =
            branch.parent_pts.iter().map(|p| p.1).sum::<f64>() / branch.parent_pts.len() as f64;
        let s_p = height_frac_s(py, y_root, y_range);
        let w_p = ink_width(s_p, bigb);
        // Stable seed from the GEDCOM id (not the emit-order index), so the
        // bark/branch wander is reproducible across runs and preference changes.
        let seed_base = branch.seed;

        let n_ch = branch.child_pts.len();
        let mean_cy = branch.child_pts.iter().map(|p| p.1).sum::<f64>() / n_ch as f64;

        // children sorted by x
        let mut sorted_ch = branch.child_pts.clone();
        sorted_ch.sort_by(|a, bv| a.0.partial_cmp(&bv.0).unwrap_or(std::cmp::Ordering::Equal));
        // Hub: low in the tree (35 % up from parent), centred under the trunk.
        let y_hub = py - 0.35 * (py - mean_cy);
        let x_hub = px;
        let s_hub = height_frac_s(y_hub, y_root, y_range);
        let w_hub = ink_width(s_hub, bigb).max(w_p);
        // One fill per fork keeps the overlapping strokes seamless.
        let limb_fill = bark_fill_color(trunk_color, s_hub);

        // 1. Stem: parent → hub (organic, wandering, grained).
        let (s0, s1, s2, s3) = branch_cubic2(px, py, x_hub, y_hub);
        wood.push((
            1e5 + w_p,
            ink_fill_cubic(
                s0, s1, s2, s3, w_p, w_hub, &limb_fill, bigb, seed_base, false,
            ),
        ));
        bark_svg.push_str(&ink_grain(
            s0,
            s1,
            s2,
            s3,
            w_p,
            w_hub,
            bigb,
            seed_base.wrapping_add(500),
        ));

        // 2. One continuous tapered stroke per child. From the hub it runs
        //    horizontally to the child's column, then turns up (quickly, near the
        //    end) and inserts into the box — a SINGLE path with ONE consistent
        //    taper (w_hub → w_c), so there is no limb/stub overlap and the change
        //    of direction is smooth and continuous. The overlapping horizontal
        //    runs of all children form the thick main limb that sheds branches.
        for (ci, &(chx, chy)) in sorted_ch.iter().enumerate() {
            let cseed = seed_base.wrapping_add(10).wrapping_add(ci as u32);
            let w_c = ink_width(height_frac_s(chy, y_root, y_range), bigb);
            // Both interior control points sit at the child's column on the limb
            // line, so the stroke stays horizontal until it is essentially under
            // the box, then turns up sharply (≈90 % of the bend happens under the
            // box) and inserts vertically.
            let c0 = (x_hub, y_hub); // at the fork
            let c1 = (chx, y_hub); // horizontal run to the child's column
            let c2 = (chx, y_hub); // tight corner directly under the box
            let c3 = (chx, chy); // insert vertically into the box
            // Outer (longer) strokes sit under inner/shorter ones.
            let key = 1.0e4 - (chx - x_hub).abs();
            wood.push((
                key,
                ink_fill_cubic(c0, c1, c2, c3, w_hub, w_c, &limb_fill, bigb, cseed, true),
            ));
            bark_svg.push_str(&ink_grain(
                c0,
                c1,
                c2,
                c3,
                w_hub,
                w_c,
                bigb,
                cseed.wrapping_add(500),
            ));
        }
    }

    // Sort wood: large keys (thick stems/limbs) first → bottom; junction blobs
    // (negative keys) last → on top, hiding the seams.
    wood.sort_by(|a, bv| bv.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Render order: canopy-back → soil → roots → trunk → wood → bark → canopy-front → grass.
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
    out.push_str(&grass_svg);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{ConnectorPrimitive, GroupPrimitive, Point, Primitive};

    #[test]
    fn seed_from_key_is_stable_and_key_sensitive() {
        // Deterministic across calls, and distinct keys give distinct seeds.
        assert_eq!(
            seed_from_key("F12-connectors"),
            seed_from_key("F12-connectors")
        );
        assert_ne!(
            seed_from_key("F12-connectors"),
            seed_from_key("F13-connectors")
        );
    }

    #[test]
    fn collect_connectors_uses_enclosing_group_id_as_seed_key() {
        // Mirror the boxed_couples connector nesting: outer empty group →
        // "{fam}-connectors" group → Connector.
        let conn = ConnectorPrimitive {
            parent_points: vec![Point { x: 0.0, y: 10.0 }],
            child_points: vec![Point { x: 0.0, y: 0.0 }],
            bar_y_fraction: 0.5,
        };
        let prims = vec![Primitive::Group(GroupPrimitive {
            id: String::new(),
            children: vec![Primitive::Group(GroupPrimitive {
                id: "F7-connectors".to_string(),
                children: vec![Primitive::Connector(conn)],
            })],
        })];
        let mut out: Vec<SeededConnector> = Vec::new();
        collect_connectors(&prims, "", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "F7-connectors");
        // The seed key is the GEDCOM-derived family id group, so the resulting
        // seed is independent of emit order.
        assert_eq!(seed_from_key(&out[0].0), seed_from_key("F7-connectors"));
    }
}
