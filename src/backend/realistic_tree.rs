//! Realistic tree branch rendering for the boxed_couples layout.
//!
//! This module generates an SVG background layer of organic tree branches/trunk that
//! replaces the default straight connectors. Boxes are rendered on top by the caller.
//!
//! Three style variants are available, selectable via `output.style.realistic_tree.style`:
//!   "tapered" — filled closed Bézier paths, width proportional to subtree depth (default)
//!   "stroke"  — layered stroked Bézier curves with simulated taper and S-curves
//!   "filter"  — thick paths plus an SVG feTurbulence/feDisplacementMap filter for texture

use crate::preferences::Prefs;
use crate::scene::{ConnectorPrimitive, Primitive};

// ── Public helpers ────────────────────────────────────────────────────────────

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

/// Render the full tree-branch SVG layer.
///
/// Returns an SVG fragment (no outer `<svg>` tag) wrapped in
/// `<g id="realistic-tree" class="realistic-tree">…</g>`.
///
/// `to_svg_x` / `to_svg_y` are the same display→SVG coordinate transforms used in
/// `svg.rs` (add MARGIN and chart_top_offset).
pub fn render_tree_layer(
    connectors: &[&ConnectorPrimitive],
    to_svg_x: &dyn Fn(f64) -> f64,
    to_svg_y: &dyn Fn(f64) -> f64,
    prefs: &Prefs,
) -> String {
    if connectors.is_empty() {
        return String::new();
    }

    // Convert all connector points to SVG space.
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
        _ => render_tapered_style(&branches, prefs),
    };

    format!("<g id=\"realistic-tree\" class=\"realistic-tree\">\n{inner}</g>\n")
}

// ── Internal types ────────────────────────────────────────────────────────────

/// One connector expressed in SVG coordinates.
#[allow(dead_code)]
struct Branch {
    /// SVG-space attachment points at the bottom of the parent box (1 or 2).
    parent_pts: Vec<(f64, f64)>,
    /// SVG-space attachment points at the top of each child box.
    child_pts: Vec<(f64, f64)>,
}

// ── Style: tapered ────────────────────────────────────────────────────────────

fn render_tapered_style(branches: &[Branch], prefs: &Prefs) -> String {
    let trunk_color = format!("#{:06X}", prefs.output.style.realistic_tree.trunk_color);
    let leaf_color = format!("#{:06X}", prefs.output.style.realistic_tree.leaf_color);
    let leaf_density = prefs.output.style.realistic_tree.leaf_density.as_str();
    let leaf_count: usize = match leaf_density {
        "none" => 0,
        "low" => 5,
        "high" => 25,
        _ => 12, // "medium"
    };

    let base_width: f64 = 8.0;
    let mut out = String::new();

    for branch in branches {
        if branch.parent_pts.is_empty() || branch.child_pts.is_empty() {
            continue;
        }

        let child_count = branch.child_pts.len();

        // Parent center x: average of parent attachment points
        let px: f64 =
            branch.parent_pts.iter().map(|p| p.0).sum::<f64>() / branch.parent_pts.len() as f64;
        let py: f64 =
            branch.parent_pts.iter().map(|p| p.1).sum::<f64>() / branch.parent_pts.len() as f64;

        // Mean child Y
        let mean_child_y: f64 =
            branch.child_pts.iter().map(|p| p.1).sum::<f64>() / child_count as f64;

        // Bar Y: midpoint between parent Y and mean child Y
        let bar_y = (py + mean_child_y) / 2.0;

        // Trunk width (from parent up to bar_y)
        let trunk_half_w = base_width * (child_count as f64).sqrt() / 2.0;
        // Sub-branch half-width from bar_y to each child
        let sub_half_w = (base_width / (child_count as f64).sqrt() / 2.0).max(1.0);

        // Draw trunk segment: from (px, py) up to (px, bar_y)
        out.push_str(&tapered_branch_path(
            px,
            py,
            px,
            bar_y,
            trunk_half_w,
            trunk_half_w,
            &trunk_color,
        ));

        // Draw sub-branches from (px, bar_y) down to each child
        for &(cx, cy) in &branch.child_pts {
            out.push_str(&tapered_branch_path(
                px,
                bar_y,
                cx,
                cy,
                trunk_half_w,
                sub_half_w,
                &trunk_color,
            ));
        }

        // Horizontal bar connecting all sub-branch tops at bar_y (only if multiple children)
        if child_count > 1 {
            let min_cx = branch
                .child_pts
                .iter()
                .map(|p| p.0)
                .fold(f64::INFINITY, f64::min);
            let max_cx = branch
                .child_pts
                .iter()
                .map(|p| p.0)
                .fold(f64::NEG_INFINITY, f64::max);
            let bar_half_h = 1.5_f64;
            out.push_str(&format!(
                "  <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"{}\" class=\"tree-branch\"/>\n",
                min_cx,
                bar_y - bar_half_h,
                max_cx - min_cx,
                bar_half_h * 2.0,
                trunk_color
            ));
        }

        // Leaf clusters at each child tip
        if leaf_count > 0 {
            for &(cx, cy) in &branch.child_pts {
                out.push_str(&tapered_leaf_cluster(cx, cy, leaf_count, &leaf_color));
            }
        }
    }

    out
}

/// Generate a filled closed Bézier path for one branch segment.
/// (x1,y1) is the wide end (bottom in SVG space), (x2,y2) is the narrow end (top).
/// w1 and w2 are the half-widths at each end.
fn tapered_branch_path(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    w1: f64,
    w2: f64,
    color: &str,
) -> String {
    let dy = y1 - y2; // positive: y1 is lower (larger SVG y) than y2
    let ctrl = dy * 0.4;
    format!(
        "  <path d=\"M {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2} L {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2} Z\" fill=\"{}\" class=\"tree-branch\"/>\n",
        x1 - w1,
        y1,
        x1 - w1,
        y1 - ctrl,
        x2 - w2,
        y2 + ctrl,
        x2 - w2,
        y2,
        x2 + w2,
        y2,
        x2 + w2,
        y2 + ctrl,
        x1 + w1,
        y1 - ctrl,
        x1 + w1,
        y1,
        color
    )
}

/// Generate leaf ellipses at a terminal child tip using a deterministic LCG.
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
            "  <ellipse cx=\"{:.2}\" cy=\"{:.2}\" rx=\"4\" ry=\"2.5\" fill=\"{}\" opacity=\"0.7\" class=\"tree-leaf\"/>\n",
            ex, ey, color
        ));
    }
    out
}

// ── Style: stroke ─────────────────────────────────────────────────────────────

fn render_stroke_style(branches: &[Branch], prefs: &Prefs) -> String {
    let trunk_color = format!("#{:06X}", prefs.output.style.realistic_tree.trunk_color);
    let leaf_color = format!("#{:06X}", prefs.output.style.realistic_tree.leaf_color);
    let leaf_density = prefs.output.style.realistic_tree.leaf_density.as_str();
    let leaf_count: usize = match leaf_density {
        "none" => 0,
        "low" => 4,
        "high" => 22,
        _ => 10, // "medium"
    };

    let base_sw: f64 = 6.0;
    let mut out = String::new();

    for branch in branches {
        if branch.parent_pts.is_empty() || branch.child_pts.is_empty() {
            continue;
        }

        let child_count = branch.child_pts.len() as f64;

        // Parent center
        let px: f64 =
            branch.parent_pts.iter().map(|p| p.0).sum::<f64>() / branch.parent_pts.len() as f64;
        let py: f64 =
            branch.parent_pts.iter().map(|p| p.1).sum::<f64>() / branch.parent_pts.len() as f64;

        // Mean child Y
        let mean_child_y: f64 = branch.child_pts.iter().map(|p| p.1).sum::<f64>() / child_count;

        // Bar Y: midpoint between parent Y and mean child Y
        let bar_y = (py + mean_child_y) / 2.0;

        // ── Trunk: from (px, py) up to (px, bar_y) ──────────────────────────
        let trunk_sw = base_sw * child_count.sqrt();
        out.push_str(&stroke_bezier_layers(
            px,
            py,
            px,
            bar_y,
            trunk_sw,
            0.0,
            &trunk_color,
        ));

        // ── Sub-branches: from (px, bar_y) to each child ────────────────────
        let sub_sw = base_sw / child_count.sqrt();
        for &(cx, cy) in &branch.child_pts {
            let lateral = (cx - px) * 0.15;
            out.push_str(&stroke_bezier_layers(
                px,
                bar_y,
                cx,
                cy,
                sub_sw,
                lateral,
                &trunk_color,
            ));
        }

        // ── Horizontal connector at bar_y (multiple children only) ──────────
        if branch.child_pts.len() > 1 {
            let min_cx = branch
                .child_pts
                .iter()
                .map(|p| p.0)
                .fold(f64::INFINITY, f64::min);
            let max_cx = branch
                .child_pts
                .iter()
                .map(|p| p.0)
                .fold(f64::NEG_INFINITY, f64::max);
            out.push_str(&format!(
                "  <line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{}\" stroke-width=\"1.5\" opacity=\"0.7\" fill=\"none\" class=\"tree-branch\"/>\n",
                min_cx, bar_y, max_cx, bar_y, trunk_color
            ));
        }

        // ── Leaf clusters at each child tip ──────────────────────────────────
        if leaf_count > 0 {
            for &(cx, cy) in &branch.child_pts {
                out.push_str(&stroke_leaf_cluster(cx, cy, leaf_count, &leaf_color));
            }
        }
    }

    out
}

/// Emit three overlapping stroked cubic Bézier `<path>` elements (thick→thin, low→high opacity)
/// to simulate a tapered organic branch.
///
/// `lateral_offset` controls the S-bow direction: positive = bow right, negative = bow left.
/// For trunk segments call with `lateral_offset = 0.0` (uses `dy * 0.08` rightward bow).
/// For sub-branches call with `lateral_offset = (cx - px) * 0.15` (curves toward child).
fn stroke_bezier_layers(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    base_sw: f64,
    lateral_offset: f64,
    color: &str,
) -> String {
    let dy = y1 - y2; // positive: y1 is lower in SVG space
    let lat = if lateral_offset == 0.0 {
        dy * 0.08
    } else {
        lateral_offset
    };

    // Control points for a gentle S-curve
    let cx1 = x1 + lat;
    let cy1 = y1 - dy * 0.35;
    let cx2 = x2 - lat;
    let cy2 = y2 + dy * 0.35;

    let d = format!(
        "M {:.2},{:.2} C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2}",
        x1, y1, cx1, cy1, cx2, cy2, x2, y2
    );

    // Layer 1: thick, low opacity
    let l1 = format!(
        "  <path d=\"{}\" stroke=\"{}\" stroke-width=\"{:.2}\" opacity=\"0.35\" fill=\"none\" class=\"tree-branch\"/>\n",
        d, color, base_sw
    );
    // Layer 2: medium
    let l2 = format!(
        "  <path d=\"{}\" stroke=\"{}\" stroke-width=\"{:.2}\" opacity=\"0.55\" fill=\"none\" class=\"tree-branch\"/>\n",
        d,
        color,
        base_sw * 0.6
    );
    // Layer 3: thin, high opacity
    let l3 = format!(
        "  <path d=\"{}\" stroke=\"{}\" stroke-width=\"{:.2}\" opacity=\"0.85\" fill=\"none\" class=\"tree-branch\"/>\n",
        d,
        color,
        base_sw * 0.3
    );

    format!("{l1}{l2}{l3}")
}

/// Generate leaf shapes (Bézier teardrop) at a terminal child tip using a deterministic LCG.
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

        // Size randomization
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let rx = ((seed & 0xFF) as f64 / 255.0) * 3.0 + 2.0;
        let ry = rx * 1.8;

        // Rotation angle for the leaf
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let rot_deg = (seed & 0xFFFF) as f64 / 65535.0 * 360.0;

        // Simple leaf teardrop: M lx,ly  Q (lx+rx),(ly-ry)  lx,(ly-ry*2)  Q (lx-rx),(ly-ry)  lx,ly
        out.push_str(&format!(
            "  <path d=\"M {:.2},{:.2} Q {:.2},{:.2} {:.2},{:.2} Q {:.2},{:.2} {:.2},{:.2}\" \
             fill=\"{}\" opacity=\"0.75\" \
             transform=\"rotate({:.1},{:.2},{:.2})\" \
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

fn render_filter_style(branches: &[Branch], prefs: &Prefs) -> String {
    if branches.is_empty() {
        return String::new();
    }

    let trunk_color = format!("#{:06X}", prefs.output.style.realistic_tree.trunk_color);
    let leaf_color = format!("#{:06X}", prefs.output.style.realistic_tree.leaf_color);
    let leaf_density = prefs.output.style.realistic_tree.leaf_density.as_str();
    let leaf_count: usize = match leaf_density {
        "none" => 0,
        "low" => 6,
        "high" => 28,
        _ => 14, // "medium"
    };

    let base_sw: f64 = 9.0;

    // Build set of all parent points (rounded) for terminal child detection.
    let parent_set: std::collections::HashSet<(i64, i64)> = branches
        .iter()
        .flat_map(|b| b.parent_pts.iter())
        .map(|&(x, y)| (x.round() as i64, y.round() as i64))
        .collect();

    // SVG filter definition.
    let filter_def = concat!(
        "<defs>\n",
        "  <filter id=\"bark-texture\" x=\"-10%\" y=\"-10%\" width=\"120%\" height=\"120%\">\n",
        "    <feTurbulence type=\"fractalNoise\" baseFrequency=\"0.035 0.018\" numOctaves=\"4\" seed=\"42\" result=\"noise\"/>\n",
        "    <feDisplacementMap in=\"SourceGraphic\" in2=\"noise\" scale=\"5\" xChannelSelector=\"R\" yChannelSelector=\"G\"/>\n",
        "  </filter>\n",
        "</defs>\n",
    );

    let mut filter_body = String::new();
    let mut leaves = String::new();

    for branch in branches {
        if branch.parent_pts.is_empty() || branch.child_pts.is_empty() {
            continue;
        }

        let child_count = branch.child_pts.len();

        // Parent center (average of parent attachment points).
        let px: f64 =
            branch.parent_pts.iter().map(|p| p.0).sum::<f64>() / branch.parent_pts.len() as f64;
        let py: f64 =
            branch.parent_pts.iter().map(|p| p.1).sum::<f64>() / branch.parent_pts.len() as f64;

        // Mean child Y.
        let mean_child_y: f64 =
            branch.child_pts.iter().map(|p| p.1).sum::<f64>() / child_count as f64;

        // Bar Y: midpoint between parent Y and mean child Y.
        let bar_y = (py + mean_child_y) / 2.0;

        // Stroke widths.
        let trunk_sw = base_sw * (child_count as f64).sqrt();
        let sub_sw = (base_sw / (child_count as f64).sqrt()).max(3.0);

        // Draw trunk: (px, py) → (px, bar_y) with gentle quadratic Bézier.
        let dy_trunk = py - bar_y;
        filter_body.push_str(&filter_trunk_path(
            px,
            py,
            bar_y,
            dy_trunk,
            trunk_sw,
            &trunk_color,
        ));

        // Light highlight on trunk.
        filter_body.push_str(&filter_highlight_path(px, py, bar_y, dy_trunk, trunk_sw));

        // Draw sub-branches: (px, bar_y) → (cx, cy) for each child.
        for &(cx, cy) in &branch.child_pts {
            filter_body.push_str(&filter_sub_path(px, bar_y, cx, cy, sub_sw, &trunk_color));
        }

        // Horizontal bar at bar_y when multiple children.
        if child_count > 1 {
            let min_cx = branch
                .child_pts
                .iter()
                .map(|p| p.0)
                .fold(f64::INFINITY, f64::min);
            let max_cx = branch
                .child_pts
                .iter()
                .map(|p| p.0)
                .fold(f64::NEG_INFINITY, f64::max);
            filter_body.push_str(&format!(
                "  <line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" \
                 stroke=\"{}\" stroke-width=\"2.5\" stroke-linecap=\"round\" \
                 fill=\"none\" class=\"tree-branch\"/>\n",
                min_cx, bar_y, max_cx, bar_y, trunk_color
            ));
        }

        // Leaf clusters at terminal child tips (outside filter group, accumulated separately).
        if leaf_count > 0 {
            for &(cx, cy) in &branch.child_pts {
                let key = (cx.round() as i64, cy.round() as i64);
                if !parent_set.contains(&key) {
                    leaves.push_str(&filter_leaf_cluster(cx, cy, leaf_count, &leaf_color));
                }
            }
        }
    }

    format!("{filter_def}<g filter=\"url(#bark-texture)\">\n{filter_body}</g>\n{leaves}")
}

/// Quadratic Bézier trunk path (px, py) → (px, bar_y) with a gentle lateral bow.
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

/// Light highlight overlay on the trunk segment (white, low opacity).
fn filter_highlight_path(px: f64, py: f64, bar_y: f64, dy: f64, trunk_sw: f64) -> String {
    let qcx = px + dy * 0.05;
    let qcy = bar_y + (py - bar_y) * 0.5;
    let highlight_sw = trunk_sw * 0.25;
    format!(
        "  <path d=\"M {:.2},{:.2} Q {:.2},{:.2} {:.2},{:.2}\" \
         stroke=\"white\" stroke-width=\"{:.2}\" stroke-linecap=\"round\" \
         stroke-linejoin=\"round\" fill=\"none\" opacity=\"0.15\"/>\n",
        px, py, qcx, qcy, px, bar_y, highlight_sw
    )
}

/// Quadratic Bézier sub-branch path (px, bar_y) → (cx, cy).
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

/// Leaf cluster at a terminal child tip using a deterministic LCG.
fn filter_leaf_cluster(cx: f64, cy: f64, count: usize, color: &str) -> String {
    let mut seed = (cx * 1000.0) as u64 ^ (cy * 1000.0) as u64;
    let mut out = String::new();
    for _ in 0..count {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let angle = (seed & 0xFFFF) as f64 / 65535.0 * std::f64::consts::TAU;
        let r_norm = ((seed >> 16) & 0xFF) as f64 / 255.0;
        let dist = r_norm * 22.0;
        let lx = cx + angle.cos() * dist;
        let ly = cy + angle.sin() * dist;
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let radius = ((seed >> 24) & 0xF) as f64 / 15.0 * 3.5 + 1.5;
        out.push_str(&format!(
            "  <circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"{:.2}\" fill=\"{}\" opacity=\"0.65\" class=\"tree-leaf\"/>\n",
            lx, ly, radius, color
        ));
    }
    out
}
