//! Recursive box-placement layout for couples with envelope-based spacing.
//!
//! ## Coordinate system
//! - x increases rightward; y = 0 is the root's row.
//! - Descendants are placed at y = −generation × (box_h + gap_h), so y
//!   decreases (becomes more negative) further from the root.
//! - All x/y values in [`IndividualGeo`] are **box centres**.
//! - Units are layout pixels (≈ SVG pixels at default preferences).
//!
//! ## High-level placement algorithm
//! See [`place_descendants`] for details.  In brief:
//!
//! 1. Place the first child using `env_left[1..]` (the left-boundary constraints
//!    passed in by the caller, shifted by one generation).
//! 2. Place each subsequent child using the right-envelope of the previous child,
//!    extended with a global right-boundary array so leaf siblings do not over-
//!    constrain deeper generations.
//! 3. Derive the parent's x from the children's positions (centre rule).  If
//!    the natural centre falls left of `x_default` constraint, shift the
//!    entire child subtree rightward rather than clamping the parent.
//! 4. After all descendants are placed, [`compact_pass`] closes sibling gaps in
//!    a top-down sweep so left-packed siblings move right without cascading overlaps.

use super::Layout;
use super::common::{
    children_with_spouse, copy_families, copy_individual, fill_env_from_global, merge_max,
    merge_min, resolve_root_id, sort_families_by_date, spouses_of,
};
use crate::parser::genrep::{Genrep, Individual};
use crate::preferences::Prefs;
use crate::util::matches_direction;
use anyhow::Result;
use std::collections::HashMap;

// ── placement audit logging (compiled only with --features bc_debug) ─────────
#[cfg(feature = "bc_debug")]
mod bc_debug {
    use std::cell::RefCell;
    use std::fs::File;
    use std::io::{BufWriter, Write};

    thread_local! {
        static LOG: RefCell<Option<BufWriter<File>>> = const { RefCell::new(None) };
        pub(super) static SHIFT_CTX: RefCell<&'static str> = const { RefCell::new("?") };
    }

    pub(super) fn init() {
        let path =
            std::env::var("BC_DEBUG_LOG").unwrap_or_else(|_| "/tmp/bc_debug.log".to_string());
        let file = File::create(&path).expect("bc_debug: cannot create log file");
        let mut w = BufWriter::new(file);
        writeln!(w, "op,id,x_before,x_after,dx,generation,source").unwrap();
        LOG.with(|l| *l.borrow_mut() = Some(w));
        eprintln!("bc_debug: logging to {path}");
    }

    pub(super) fn flush() {
        LOG.with(|l| {
            if let Some(w) = l.borrow_mut().as_mut() {
                w.flush().ok();
            }
        });
    }

    pub(super) fn log(
        op: &str,
        id: &str,
        x_before: Option<f64>,
        x_after: f64,
        generation: u32,
        source: &str,
    ) {
        LOG.with(|l| {
            if let Some(w) = l.borrow_mut().as_mut() {
                let before = x_before.map(|v| format!("{v:.1}")).unwrap_or_default();
                let dx = x_before
                    .map(|v| format!("{:.1}", x_after - v))
                    .unwrap_or_default();
                writeln!(
                    w,
                    "{op},{id},{before},{x_after:.1},{dx},{generation},{source}"
                )
                .ok();
            }
        });
    }

    pub(super) fn shift_ctx() -> &'static str {
        SHIFT_CTX.with(|c| *c.borrow())
    }
}

// Public entry points called from main.rs (feature-gated, zero cost otherwise).
#[cfg(feature = "bc_debug")]
pub fn bc_debug_init() {
    bc_debug::init();
}
#[cfg(feature = "bc_debug")]
pub fn bc_debug_flush() {
    bc_debug::flush();
}

macro_rules! bc_log_place {
    ($id:expr, $x:expr, $gen:expr) => {
        #[cfg(feature = "bc_debug")]
        bc_debug::log("PLACE", $id, None, $x, $gen, concat!(file!(), ":", line!()));
    };
}

macro_rules! bc_log_shift {
    ($id:expr, $x_before:expr, $x_after:expr, $gen:expr) => {
        #[cfg(feature = "bc_debug")]
        {
            let ctx = bc_debug::shift_ctx();
            let src = concat!(file!(), ":", line!());
            let full_src = format!("{src}/{ctx}");
            bc_debug::log("SHIFT", $id, Some($x_before), $x_after, $gen, &full_src);
        }
    };
}

macro_rules! bc_log_recenter {
    ($id:expr, $x_before:expr, $x_after:expr, $gen:expr) => {
        #[cfg(feature = "bc_debug")]
        bc_debug::log(
            "RECENTER",
            $id,
            Some($x_before),
            $x_after,
            $gen,
            concat!(file!(), ":", line!()),
        );
    };
}

macro_rules! bc_set_shift_ctx {
    ($ctx:expr) => {
        #[cfg(feature = "bc_debug")]
        bc_debug::SHIFT_CTX.with(|c| *c.borrow_mut() = $ctx);
    };
}

/// Layout geometry for a placed descendant box.
#[derive(Debug, Clone)]
pub struct IndividualGeo {
    /// Horizontal centre of the box.
    pub x: f64,
    /// Vertical centre of the box (root at 0; more negative = deeper generation).
    pub y: f64,
    /// Box width: `box_w` for 0 or 1 in-scope spouses, `box_w2` for 2.
    pub width: f64,
    /// Box height (equals the `box_height` preference).
    pub height: f64,
    /// x of the incoming-connector attachment point (horizontally centred on box).
    pub conn_in_x: f64,
    /// y of the incoming-connector attachment point (top edge of box).
    #[allow(dead_code)]
    pub conn_in_y: f64,
    /// Generation depth (root = 0, children = 1, …).
    pub generation: u32,
}

/// Layout geometry for the outgoing connectors of a placed family (parent → children).
#[derive(Debug, Clone)]
pub struct FamilyGeo {
    /// x of the outgoing connector for the first spouse's children.
    /// For a 1-spouse box this equals the box centre; for a 2-spouse box it
    /// is offset left to the centre of the first spouse's column.
    pub conn_out1_x: f64,
    /// y of both outgoing connectors (bottom edge of the parent box).
    #[allow(dead_code)]
    pub conn_out1_y: f64,
    /// x of the outgoing connector for the second spouse's children (right column).
    pub conn_out2_x: f64,
    #[allow(dead_code)]
    pub conn_out2_y: f64,
    /// `true` when the parent box uses the wide `box_w2` form (2 in-scope spouses).
    pub has_spouse2: bool,
    /// x of the outgoing connector for the third spouse's children (far-right column).
    pub conn_out3_x: f64,
    #[allow(dead_code)]
    pub conn_out3_y: f64,
    /// `true` when the parent box uses the triple-wide `box_w3` form (3 in-scope spouses).
    pub has_spouse3: bool,
}

/// Geo payload stored in both `Individual.geo` and `Family.geo`.
///
/// Only descendants visited by [`place_descendants`] receive an `Individual`
/// variant.  In-scope spouses of those descendants are inserted into the output
/// map with `geo = None` so the SVG renderer can look them up.
/// `Family` variants are assigned in a post-pass by [`build_family_geo`].
#[derive(Debug, Clone)]
pub enum BoxedCouplesGeo {
    Individual(IndividualGeo),
    Family(FamilyGeo),
}

/// Returns at most 3 in-scope spouses, preferring those with children.
///
/// The layout can represent at most 3 spouses (a 1-spouse box, a wide 2-spouse box,
/// or a triple-wide 3-spouse box).  When more exist, spouses without children are
/// dropped from the end of the list first until at most 3 remain.
fn prune_spouses<G>(ind_id: &str, genrep: &Genrep<G>) -> Vec<String> {
    let mut spouses = spouses_of(ind_id, genrep);
    if spouses.len() > 3 {
        eprintln!(
            "warning: {} has {} spouses; only 3 can be represented in boxed_couples layout",
            ind_id,
            spouses.len()
        );
        let mut i = spouses.len();
        while i > 0 && spouses.len() > 3 {
            i -= 1;
            if children_with_spouse(ind_id, &spouses[i].clone(), genrep).is_empty() {
                spouses.remove(i);
            }
        }
        spouses.truncate(3);
    }
    spouses
}

/// Returns the appropriate half-width for `id` based on its pruned spouse count.
fn half_width_of(id: &str, genrep: &Genrep, box_w: f64, box_w2: f64, box_w3: f64) -> f64 {
    let n = prune_spouses(id, genrep).len();
    (if n >= 3 {
        box_w3
    } else if n >= 2 {
        box_w2
    } else {
        box_w
    }) / 2.0
}

/// Returns the right-side envelope of `ind`'s placed subtree.
///
/// `result[0]` = right edge of `ind` itself (`x + width/2`).
/// `result[k]` = maximum right edge among all boxes placed k levels below `ind`.
///
/// Passing an accumulated right-envelope of all preceding siblings as `env_left`
/// to the next sibling ensures it and its entire subtree clear all previous
/// sibling subtrees at every generation.
fn get_right_envelope(
    ind_id: &str,
    genrep: &Genrep,
    out: &HashMap<String, Individual<BoxedCouplesGeo>>,
) -> Vec<f64> {
    let ind = match out.get(ind_id) {
        Some(i) => i,
        None => return vec![],
    };
    let geo = match &ind.geo {
        Some(BoxedCouplesGeo::Individual(g)) => g,
        _ => return vec![],
    };
    let mut result = vec![geo.x + geo.width / 2.0];

    let spouses = prune_spouses(ind_id, genrep);
    let child_ids: Vec<String> = spouses
        .iter()
        .flat_map(|sp| children_with_spouse(ind_id, sp, genrep))
        .filter(|cid| out.contains_key(cid.as_str()))
        .collect();

    let mut merged_children_env = Vec::new();
    for child_id in child_ids {
        let child_env = get_right_envelope(&child_id, genrep, out);
        merged_children_env = merge_max(merged_children_env, child_env);
    }

    result.extend(merged_children_env);
    result
}

/// Returns the left-side envelope of `ind`'s placed subtree.
///
/// `result[0]` = left edge of `ind` itself (`x - width/2`).
/// `result[k]` = minimum left edge among all boxes placed k levels below `ind`.
///
/// Used by [`compact_siblings`] to compute the safe shift at each depth.
fn get_left_envelope(
    ind_id: &str,
    genrep: &Genrep,
    out: &HashMap<String, Individual<BoxedCouplesGeo>>,
) -> Vec<f64> {
    let ind = match out.get(ind_id) {
        Some(i) => i,
        None => return vec![],
    };
    let geo = match &ind.geo {
        Some(BoxedCouplesGeo::Individual(g)) => g,
        _ => return vec![],
    };
    let mut result = vec![geo.x - geo.width / 2.0];

    let spouses = prune_spouses(ind_id, genrep);
    let child_ids: Vec<String> = spouses
        .iter()
        .flat_map(|sp| children_with_spouse(ind_id, sp, genrep))
        .filter(|cid| out.contains_key(cid.as_str()))
        .collect();

    let mut merged_children_env = Vec::new();
    for child_id in child_ids {
        let child_env = get_left_envelope(&child_id, genrep, out);
        merged_children_env = merge_min(merged_children_env, child_env);
    }
    result.extend(merged_children_env);
    result
}

/// Derives connector geometry for one family from its placed parent's [`IndividualGeo`].
///
/// Returns `None` if neither spouse has been placed (e.g. an out-of-scope family).
/// The `conn_out` x-values are offset left/right by half the wide-box difference
/// when the parent has two in-scope spouses.
fn build_family_geo(
    fam: &crate::parser::genrep::Family<()>, // full path to disambiguate from the Family enum variant defined above
    out: &HashMap<String, Individual<BoxedCouplesGeo>>,
    box_h: f64,
    box_w: f64,
    box_w2: f64,
    box_w3: f64,
) -> Option<BoxedCouplesGeo> {
    let is_placed = |id: &&str| {
        matches!(
            out.get(*id).and_then(|i| i.geo.as_ref()),
            Some(BoxedCouplesGeo::Individual(_))
        )
    };
    let parent_id = fam
        .husband_id
        .as_deref()
        .filter(is_placed)
        .or_else(|| fam.wife_id.as_deref().filter(is_placed))?;
    let parent = out.get(parent_id).unwrap(); // safe: is_placed guarantees presence
    let geo = match &parent.geo {
        Some(BoxedCouplesGeo::Individual(g)) => g,
        _ => return None, // unreachable; kept for exhaustiveness
    };

    let has_spouse3 = geo.width > box_w2 + 1.0;
    let has_spouse2 = geo.width > box_w + 1.0; // true for both 2- and 3-spouse boxes

    let conn_out_y = geo.y + box_h / 2.0;
    let off3 = box_w3 / 2.0 - box_w / 2.0; // = box_w + gap_w at default sizing
    let off2 = box_w2 / 2.0 - box_w / 2.0;
    let (conn_out1_x, conn_out2_x, conn_out3_x) = if has_spouse3 {
        (geo.x - off3, geo.x, geo.x + off3)
    } else if has_spouse2 {
        // conn_out3 ≡ conn_out2 (unused for 2-spouse families)
        (geo.x - off2, geo.x + off2, geo.x + off2)
    } else {
        (geo.x, geo.x, geo.x)
    };

    Some(BoxedCouplesGeo::Family(FamilyGeo {
        conn_out1_x,
        conn_out1_y: conn_out_y,
        conn_out2_x,
        conn_out2_y: conn_out_y,
        has_spouse2,
        conn_out3_x,
        conn_out3_y: conn_out_y,
        has_spouse3,
    }))
}

/// Panics if `id` is not yet placed — only call after the individual's subtree is done.
fn get_x_of(id: &str, out: &HashMap<String, Individual<BoxedCouplesGeo>>) -> f64 {
    match out.get(id).and_then(|i| i.geo.as_ref()) {
        Some(BoxedCouplesGeo::Individual(g)) => g.x,
        _ => panic!("get_x_of: individual {id:?} not yet placed — this is a bug"),
    }
}

/// Shifts `ind_id` and every placed descendant rightward by `dx`, and updates `global_right`.
///
/// Called by [`place_descendants`] when the parent's natural centre falls left of
/// `x_default`: rather than clamping the parent, we push the whole child subtree
/// right so the centring invariant is preserved.   `global_right` must be updated
/// here too, otherwise subsequent siblings see stale envelope values.
fn shift_subtree(
    ind_id: &str,
    dx: f64,
    generation: u32,
    genrep: &Genrep,
    out: &mut HashMap<String, Individual<BoxedCouplesGeo>>,
    global_right: &mut Vec<f64>,
) {
    let spouses = prune_spouses(ind_id, genrep);
    let child_ids: Vec<String> = spouses
        .iter()
        .flat_map(|sp| children_with_spouse(ind_id, sp, genrep))
        .collect();

    if let Some(ind) = out.get_mut(ind_id) {
        if let Some(BoxedCouplesGeo::Individual(g)) = &mut ind.geo {
            #[cfg(feature = "bc_debug")]
            let x_before = g.x;
            g.x += dx;
            g.conn_in_x += dx;
            bc_log_shift!(ind_id, x_before, g.x, generation);
            let gen_idx = generation as usize;
            if gen_idx < global_right.len() {
                global_right[gen_idx] = global_right[gen_idx].max(g.x + g.width / 2.0);
            }
        }
    }

    for child_id in child_ids {
        shift_subtree(&child_id, dx, generation + 1, genrep, out, global_right);
    }
}

/// Closes excess gaps between already-placed siblings by shifting left-packed ones right.
///
/// After the left-to-right sibling loop, a sibling that was pulled right by the parent-centring
/// rule leaves a larger-than-`gap_w` gap before it.  This right-to-left sweep shifts the
/// left group rightward to close that gap, but only by the *safe* amount — the minimum over
/// all depths of `left_env(children[i+1])[j] − right_env(children[i])[j] − gap_w`.
/// This prevents the shifted subtree from overlapping the next sibling's subtree at any depth.
fn compact_siblings(
    children: &[String],
    generation: u32,
    gap_w: f64,
    genrep: &Genrep,
    out: &mut HashMap<String, Individual<BoxedCouplesGeo>>,
    global_right: &mut Vec<f64>,
) {
    if children.len() < 2 {
        return;
    }
    for i in (0..children.len() - 1).rev() {
        // Compute merged right envelope for the entire block to the left of the gap.
        let mut block_right_env = Vec::new();
        #[allow(clippy::needless_range_loop)]
        for j in 0..=i {
            let child_right_env = get_right_envelope(&children[j], genrep, out);
            block_right_env = merge_max(block_right_env, child_right_env);
        }
        let left_env = get_left_envelope(&children[i + 1], genrep, out);

        if block_right_env.is_empty() || left_env.is_empty() {
            continue;
        }

        // Top-level gap excess — how much we want to shift.
        let desired_shift = left_env[0] - block_right_env[0] - gap_w;
        if desired_shift <= 1e-6 {
            continue;
        }

        // Safe shift: the desired shift capped by the tightest clearance at any depth.
        // zip stops at the shorter envelope, so leaf siblings (envelope length 1) are
        // unconstrained by deeper generations.
        let safe_shift = block_right_env
            .iter()
            .zip(left_env.iter())
            .map(|(r, l)| l - r - gap_w)
            .fold(desired_shift, f64::min)
            .max(0.0);

        if safe_shift > 1e-6 {
            bc_set_shift_ctx!("compact");
            #[allow(clippy::needless_range_loop)]
            for j in 0..=i {
                shift_subtree(
                    &children[j],
                    safe_shift,
                    generation,
                    genrep,
                    out,
                    global_right,
                );
            }
        }
    }
}

/// Top-down recursive compact pass: closes sibling gaps at each level before recursing.
///
/// Called once after [`place_descendants`] has finished.  Processing parent levels before
/// child levels ensures that ancestor-level compaction sees pre-shift child positions, so it
/// cannot over-shift a subtree into one that a lower-level compact will later shift right.
fn compact_pass(
    ind_id: &str,
    genrep: &Genrep,
    out: &mut HashMap<String, Individual<BoxedCouplesGeo>>,
    global_right: &mut Vec<f64>,
    gap_w: f64,
    generation: u32,
) {
    let spouses = prune_spouses(ind_id, genrep);

    if spouses.len() >= 3 {
        // 3-spouse parents: compact each spouse's children independently.
        // Never compact across group boundaries (would violate connector invariants).
        let children1: Vec<String> = children_with_spouse(ind_id, &spouses[0], genrep)
            .into_iter()
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();
        let children2: Vec<String> = children_with_spouse(ind_id, &spouses[1], genrep)
            .into_iter()
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();
        let children3: Vec<String> = children_with_spouse(ind_id, &spouses[2], genrep)
            .into_iter()
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();
        compact_siblings(&children1, generation + 1, gap_w, genrep, out, global_right);
        compact_siblings(&children2, generation + 1, gap_w, genrep, out, global_right);
        compact_siblings(&children3, generation + 1, gap_w, genrep, out, global_right);
        for child_id in children1
            .iter()
            .chain(children2.iter())
            .chain(children3.iter())
        {
            compact_pass(child_id, genrep, out, global_right, gap_w, generation + 1);
        }
    } else if spouses.len() >= 2 {
        // For 2-spouse parents, compact each spouse's children independently.
        // Compacting across the children1/children2 boundary would shift children1
        // past conn_out1_x, breaking the invariant set by place_descendants.
        let children1: Vec<String> = children_with_spouse(ind_id, &spouses[0], genrep)
            .into_iter()
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();
        let children2: Vec<String> = children_with_spouse(ind_id, &spouses[1], genrep)
            .into_iter()
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();
        compact_siblings(&children1, generation + 1, gap_w, genrep, out, global_right);
        compact_siblings(&children2, generation + 1, gap_w, genrep, out, global_right);
        for child_id in children1.iter().chain(children2.iter()) {
            compact_pass(child_id, genrep, out, global_right, gap_w, generation + 1);
        }
    } else {
        let all_children: Vec<String> = spouses
            .iter()
            .flat_map(|sp| children_with_spouse(ind_id, sp, genrep))
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();
        if all_children.is_empty() {
            return;
        }
        compact_siblings(
            &all_children,
            generation + 1,
            gap_w,
            genrep,
            out,
            global_right,
        );
        for child_id in all_children {
            compact_pass(&child_id, genrep, out, global_right, gap_w, generation + 1);
        }
    }
}

/// Bottom-up re-centering pass: corrects parent x after `compact_pass` shifts children.
///
/// `compact_siblings` can shift a left child rightward into a gap, changing the
/// midpoint of the sibling group without updating the already-stored parent x.
/// This pass recurses children-first (post-order) so that by the time a parent
/// is re-centered its children already have their final positions.
///
/// The centering formula mirrors [`place_descendants`]:
/// - 1-spouse: median child for odd n; average of two middle children for even n.
/// - 2-spouse: derived from `children1.last()` via `conn_out1_offset`, or
///   `children2.first()` via `conn_out2_offset` when `children1` is empty.
/// - 3-spouse: derived from `children2`'s median (center over middle spouse's children).
fn recenter_pass(
    ind_id: &str,
    genrep: &Genrep,
    box_w: f64,
    box_w2: f64,
    box_w3: f64,
    gap_w: f64,
    out: &mut HashMap<String, Individual<BoxedCouplesGeo>>,
    max_center_x: Option<f64>,
) {
    // Use prune_spouses (scope-filtered, sorted by date, at most 3).
    // Any warning was already emitted during place_descendants.
    let spouses = prune_spouses(ind_id, genrep);

    if spouses.len() >= 3 {
        let children1: Vec<String> = children_with_spouse(ind_id, &spouses[0], genrep)
            .into_iter()
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();
        let children2: Vec<String> = children_with_spouse(ind_id, &spouses[1], genrep)
            .into_iter()
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();
        let children3: Vec<String> = children_with_spouse(ind_id, &spouses[2], genrep)
            .into_iter()
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();

        for (i, child_id) in children1.iter().enumerate() {
            let max_x = if i + 1 < children1.len() {
                let rsib = &children1[i + 1];
                Some(
                    get_x_of(rsib, out)
                        - half_width_of(rsib, genrep, box_w, box_w2, box_w3)
                        - gap_w
                        - half_width_of(child_id, genrep, box_w, box_w2, box_w3),
                )
            } else {
                None
            };
            recenter_pass(child_id, genrep, box_w, box_w2, box_w3, gap_w, out, max_x);
        }
        for (i, child_id) in children2.iter().enumerate() {
            let max_x = if i + 1 < children2.len() {
                let rsib = &children2[i + 1];
                Some(
                    get_x_of(rsib, out)
                        - half_width_of(rsib, genrep, box_w, box_w2, box_w3)
                        - gap_w
                        - half_width_of(child_id, genrep, box_w, box_w2, box_w3),
                )
            } else {
                None
            };
            recenter_pass(child_id, genrep, box_w, box_w2, box_w3, gap_w, out, max_x);
        }
        for (i, child_id) in children3.iter().enumerate() {
            let max_x = if i + 1 < children3.len() {
                let rsib = &children3[i + 1];
                Some(
                    get_x_of(rsib, out)
                        - half_width_of(rsib, genrep, box_w, box_w2, box_w3)
                        - gap_w
                        - half_width_of(child_id, genrep, box_w, box_w2, box_w3),
                )
            } else {
                None
            };
            recenter_pass(child_id, genrep, box_w, box_w2, box_w3, gap_w, out, max_x);
        }

        if children1.is_empty() && children2.is_empty() && children3.is_empty() {
            return;
        }

        let conn_out3_offset = box_w3 / 2.0 - box_w / 2.0;
        let new_x = if !children2.is_empty() {
            let n2 = children2.len();
            if n2 % 2 == 1 {
                get_x_of(&children2[n2 / 2], out)
            } else {
                (get_x_of(&children2[n2 / 2 - 1], out) + get_x_of(&children2[n2 / 2], out)) / 2.0
            }
        } else if !children1.is_empty() {
            get_x_of(children1.last().unwrap(), out) + conn_out3_offset
        } else {
            get_x_of(children3.first().unwrap(), out) - conn_out3_offset
        };
        let final_x = max_center_x.map_or(new_x, |m| new_x.min(m));

        if let Some(ind) = out.get_mut(ind_id) {
            if let Some(BoxedCouplesGeo::Individual(g)) = &mut ind.geo {
                #[cfg(feature = "bc_debug")]
                let x_before = g.x;
                g.x = final_x;
                g.conn_in_x = final_x;
                bc_log_recenter!(ind_id, x_before, final_x, g.generation);
            }
        }
    } else if spouses.len() >= 2 {
        let children1: Vec<String> = children_with_spouse(ind_id, &spouses[0], genrep)
            .into_iter()
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();
        let children2: Vec<String> = children_with_spouse(ind_id, &spouses[1], genrep)
            .into_iter()
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();

        for (i, child_id) in children1.iter().enumerate() {
            let max_x = if i + 1 < children1.len() {
                let rsib = &children1[i + 1];
                Some(
                    get_x_of(rsib, out)
                        - half_width_of(rsib, genrep, box_w, box_w2, box_w3)
                        - gap_w
                        - half_width_of(child_id, genrep, box_w, box_w2, box_w3),
                )
            } else {
                None
            };
            recenter_pass(child_id, genrep, box_w, box_w2, box_w3, gap_w, out, max_x);
        }
        for (i, child_id) in children2.iter().enumerate() {
            let max_x = if i + 1 < children2.len() {
                let rsib = &children2[i + 1];
                Some(
                    get_x_of(rsib, out)
                        - half_width_of(rsib, genrep, box_w, box_w2, box_w3)
                        - gap_w
                        - half_width_of(child_id, genrep, box_w, box_w2, box_w3),
                )
            } else {
                None
            };
            recenter_pass(child_id, genrep, box_w, box_w2, box_w3, gap_w, out, max_x);
        }

        if children1.is_empty() && children2.is_empty() {
            return;
        }

        let conn_out1_offset = -(box_w2 / 2.0 - box_w / 2.0);
        let conn_out2_offset = box_w2 / 2.0 - box_w / 2.0;
        let new_x = if !children1.is_empty() {
            get_x_of(children1.last().unwrap(), out) - conn_out1_offset
        } else {
            get_x_of(children2.first().unwrap(), out) - conn_out2_offset
        };
        let final_x = max_center_x.map_or(new_x, |m| new_x.min(m));

        if let Some(ind) = out.get_mut(ind_id) {
            if let Some(BoxedCouplesGeo::Individual(g)) = &mut ind.geo {
                #[cfg(feature = "bc_debug")]
                let x_before = g.x;
                g.x = final_x;
                g.conn_in_x = final_x;
                bc_log_recenter!(ind_id, x_before, final_x, g.generation);
            }
        }
    } else {
        let all_children: Vec<String> = spouses
            .iter()
            .flat_map(|sp| children_with_spouse(ind_id, sp, genrep))
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();

        if all_children.is_empty() {
            return;
        }

        for (i, child_id) in all_children.iter().enumerate() {
            let max_x = if i + 1 < all_children.len() {
                let rsib = &all_children[i + 1];
                Some(
                    get_x_of(rsib, out)
                        - half_width_of(rsib, genrep, box_w, box_w2, box_w3)
                        - gap_w
                        - half_width_of(child_id, genrep, box_w, box_w2, box_w3),
                )
            } else {
                None
            };
            recenter_pass(child_id, genrep, box_w, box_w2, box_w3, gap_w, out, max_x);
        }

        let n = all_children.len();
        let new_x = if n % 2 == 1 {
            get_x_of(&all_children[n / 2], out)
        } else {
            (get_x_of(&all_children[n / 2 - 1], out) + get_x_of(&all_children[n / 2], out)) / 2.0
        };
        let final_x = max_center_x.map_or(new_x, |m| new_x.min(m));

        if let Some(ind) = out.get_mut(ind_id) {
            if let Some(BoxedCouplesGeo::Individual(g)) = &mut ind.geo {
                #[cfg(feature = "bc_debug")]
                let x_before = g.x;
                g.x = final_x;
                g.conn_in_x = final_x;
                bc_log_recenter!(ind_id, x_before, final_x, g.generation);
            }
        }
    }
}

/// Recursively places `ind_id` and all its in-scope descendants into `out`.
///
/// ## Parameters
/// - `env_left[j]` — minimum right-edge that must be cleared at depth
///   `generation + j` (i.e. `env_left[0]` constrains `ind_id` itself).
/// - `generation` — absolute depth from the root (root = 0).
/// - `global_right[g]` — rightmost right-edge placed so far at absolute
///   generation `g`; updated after each individual is inserted into `out`.
///
/// ## Algorithm (single-spouse case)
/// 1. Place the first child using `env_left[1..]`.
/// 2. Place each subsequent child using the right-envelope of the previous child
///    (extended via `fill_env_from_global`).
/// 3. Derive the parent's x as the horizontal midpoint of the children.
/// 4. If that midpoint is left of `x_default` (the column is squeezed), shift every child
///    subtree rightward by the difference (`shift_subtree`) so the parent can sit at
///    `x_default` while remaining centred.
///
/// After all descendants are placed, [`compact_pass`] closes sibling gaps in a top-down sweep.
/// The two-spouse case is identical but concatenates both spouses' children
/// and adjusts the midpoint calculation for the wide-box connector offsets.
#[allow(clippy::too_many_arguments)]
fn place_descendants(
    genrep: &Genrep,
    ind_id: &str,
    env_left: &[f64],
    generation: u32,
    box_w: f64,
    box_h: f64,
    box_w2: f64,
    box_w3: f64,
    gap_w: f64,
    gap_h: f64,
    out: &mut HashMap<String, Individual<BoxedCouplesGeo>>,
    global_right: &mut Vec<f64>,
) {
    let ind = match genrep.get_individual(ind_id) {
        Some(i) => i,
        None => return,
    };

    if !ind.in_scope {
        return;
    }

    let spouses = prune_spouses(ind_id, genrep);
    let width = if spouses.len() >= 3 {
        box_w3
    } else if spouses.len() == 2 {
        box_w2
    } else {
        box_w
    };
    let y = -(generation as f64 * (box_h + gap_h));

    let x_default = env_left.first().copied().unwrap_or(0.0) + gap_w + width / 2.0;

    let x = match spouses.len() {
        0 => x_default,

        1 => {
            let children = children_with_spouse(ind_id, &spouses[0], genrep);
            if children.is_empty() {
                x_default
            } else {
                place_descendants(
                    genrep,
                    &children[0],
                    &env_left[1..],
                    generation + 1,
                    box_w,
                    box_h,
                    box_w2,
                    box_w3,
                    gap_w,
                    gap_h,
                    out,
                    global_right,
                );
                for i in 1..children.len() {
                    let right_env = fill_env_from_global(
                        get_right_envelope(&children[i - 1], genrep, out),
                        env_left.len().saturating_sub(1),
                        global_right,
                        (generation as usize) + 1,
                    );
                    place_descendants(
                        genrep,
                        &children[i],
                        &right_env,
                        generation + 1,
                        box_w,
                        box_h,
                        box_w2,
                        box_w3,
                        gap_w,
                        gap_h,
                        out,
                        global_right,
                    );
                }

                let n = children.len();
                let x_mid = if n % 2 == 1 {
                    get_x_of(&children[n / 2], out)
                } else {
                    (get_x_of(&children[n / 2 - 1], out) + get_x_of(&children[n / 2], out)) / 2.0
                };
                if x_mid < x_default {
                    let shift = x_default - x_mid;
                    bc_set_shift_ctx!("place/align");
                    for child_id in &children {
                        shift_subtree(child_id, shift, generation + 1, genrep, out, global_right);
                    }
                    x_default
                } else {
                    x_mid
                }
            }
        }

        2 => {
            let children1 = children_with_spouse(ind_id, &spouses[0], genrep);
            let children2 = children_with_spouse(ind_id, &spouses[1], genrep);
            let all_children: Vec<String> =
                children1.iter().chain(children2.iter()).cloned().collect();

            if all_children.is_empty() {
                x_default
            } else {
                place_descendants(
                    genrep,
                    &all_children[0],
                    &env_left[1..],
                    generation + 1,
                    box_w,
                    box_h,
                    box_w2,
                    box_w3,
                    gap_w,
                    gap_h,
                    out,
                    global_right,
                );
                for i in 1..all_children.len() {
                    let right_env = fill_env_from_global(
                        get_right_envelope(&all_children[i - 1], genrep, out),
                        env_left.len().saturating_sub(1),
                        global_right,
                        (generation as usize) + 1,
                    );
                    place_descendants(
                        genrep,
                        &all_children[i],
                        &right_env,
                        generation + 1,
                        box_w,
                        box_h,
                        box_w2,
                        box_w3,
                        gap_w,
                        gap_h,
                        out,
                        global_right,
                    );
                }

                let conn_out1_offset = -(box_w2 / 2.0 - box_w / 2.0);
                let conn_out2_offset = box_w2 / 2.0 - box_w / 2.0;

                let x_from_children = if !children1.is_empty() {
                    get_x_of(children1.last().unwrap(), out) - conn_out1_offset
                } else {
                    get_x_of(children2.first().unwrap(), out) - conn_out2_offset
                };
                if x_from_children < x_default {
                    let shift = x_default - x_from_children;
                    bc_set_shift_ctx!("place/align");
                    for child_id in all_children.iter() {
                        shift_subtree(child_id, shift, generation + 1, genrep, out, global_right);
                    }
                    x_default
                } else {
                    x_from_children
                }
            }
        }

        _ => {
            // spouses.len() == 3
            let children1 = children_with_spouse(ind_id, &spouses[0], genrep);
            let children2 = children_with_spouse(ind_id, &spouses[1], genrep);
            let children3 = children_with_spouse(ind_id, &spouses[2], genrep);
            let all_children: Vec<String> = children1
                .iter()
                .chain(children2.iter())
                .chain(children3.iter())
                .cloned()
                .collect();

            if all_children.is_empty() {
                x_default
            } else {
                // Sequential left-to-right placement (identical structure to 2-spouse arm)
                place_descendants(
                    genrep,
                    &all_children[0],
                    &env_left[1..],
                    generation + 1,
                    box_w,
                    box_h,
                    box_w2,
                    box_w3,
                    gap_w,
                    gap_h,
                    out,
                    global_right,
                );
                for i in 1..all_children.len() {
                    let right_env = fill_env_from_global(
                        get_right_envelope(&all_children[i - 1], genrep, out),
                        env_left.len().saturating_sub(1),
                        global_right,
                        (generation as usize) + 1,
                    );
                    place_descendants(
                        genrep,
                        &all_children[i],
                        &right_env,
                        generation + 1,
                        box_w,
                        box_h,
                        box_w2,
                        box_w3,
                        gap_w,
                        gap_h,
                        out,
                        global_right,
                    );
                }

                // Parent x from children2's median — centers the triple box over the
                // middle spouse's children. Falls back to children1 / children3 when
                // children2 is empty (analogous to the 2-spouse fallback).
                let conn_out3_offset = box_w3 / 2.0 - box_w / 2.0;
                let x_from_children = if !children2.is_empty() {
                    let n2 = children2.len();
                    if n2 % 2 == 1 {
                        get_x_of(&children2[n2 / 2], out)
                    } else {
                        (get_x_of(&children2[n2 / 2 - 1], out) + get_x_of(&children2[n2 / 2], out))
                            / 2.0
                    }
                } else if !children1.is_empty() {
                    get_x_of(children1.last().unwrap(), out) + conn_out3_offset
                } else {
                    get_x_of(children3.first().unwrap(), out) - conn_out3_offset
                };

                if x_from_children < x_default {
                    let shift = x_default - x_from_children;
                    bc_set_shift_ctx!("place/align");
                    for child_id in all_children.iter() {
                        shift_subtree(child_id, shift, generation + 1, genrep, out, global_right);
                    }
                    x_default
                } else {
                    x_from_children
                }
            }
        }
    };

    let geo = IndividualGeo {
        x,
        y,
        width,
        height: box_h,
        conn_in_x: x,
        conn_in_y: y - box_h / 2.0,
        generation,
    };
    out.insert(
        ind_id.to_string(),
        copy_individual(ind, Some(BoxedCouplesGeo::Individual(geo))),
    );
    bc_log_place!(ind_id, x, generation);
    if (generation as usize) < global_right.len() {
        let right_edge = x + width / 2.0;
        global_right[generation as usize] = global_right[generation as usize].max(right_edge);
    }
}

/// Sweeps each generation left-to-right and pushes any overlapping right-side node
/// (and its subtree) far enough right to restore a `gap_w` gap.
///
/// Called once after `recenter_pass` to fix cross-family overlaps that can arise
/// when compact+recenter moves a node rightward after its right cousin was already
/// placed during `place_descendants`.
fn fix_overlaps_pass(
    genrep: &Genrep,
    gap_w: f64,
    out: &mut HashMap<String, Individual<BoxedCouplesGeo>>,
    global_right: &mut Vec<f64>,
) {
    let mut by_gen: HashMap<u32, Vec<(f64, f64, String)>> = HashMap::new();
    for (id, ind) in out.iter() {
        if let Some(BoxedCouplesGeo::Individual(g)) = &ind.geo {
            by_gen
                .entry(g.generation)
                .or_default()
                .push((g.x, g.width, id.clone()));
        }
    }
    for (generation, nodes) in &mut by_gen {
        nodes.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        for i in 0..nodes.len().saturating_sub(1) {
            let a_right = nodes[i].0 + nodes[i].1 / 2.0;
            let b_left = nodes[i + 1].0 - nodes[i + 1].1 / 2.0;
            let gap = b_left - a_right;
            if gap < gap_w - 1e-6 {
                let shift = gap_w - gap;
                bc_set_shift_ctx!("fix_overlap");
                shift_subtree(
                    &nodes[i + 1].2,
                    shift,
                    *generation,
                    genrep,
                    out,
                    global_right,
                );
                nodes[i + 1].0 += shift;
            }
        }
    }
}

pub struct BoxedCouplesLayout;

impl Layout for BoxedCouplesLayout {
    type Geo = BoxedCouplesGeo;

    fn compute(&self, genrep: &Genrep, prefs: &Prefs) -> Result<Genrep<BoxedCouplesGeo>> {
        let dir = prefs.scope.direction.to_lowercase();

        if matches_direction(&dir, "ancestors") || matches_direction(&dir, "pedigree") {
            return Err(anyhow::anyhow!(
                "boxed_couples layout does not support the ancestors direction; use 'boxes' instead"
            ));
        }

        if matches_direction(&dir, "forest") {
            eprintln!("warning: boxed_couples layout does not support direction=forest");
            return Ok(Genrep {
                individuals: HashMap::new(),
                families: HashMap::new(),
                first_individual_id: genrep.first_individual_id.clone(),
            });
        }

        let root_opt = resolve_root_id(genrep, prefs);
        let root_id = root_opt.as_deref().unwrap_or("");

        if root_id.is_empty() {
            return Ok(Genrep {
                individuals: HashMap::new(),
                families: HashMap::new(),
                first_individual_id: None,
            });
        }

        let bc = &prefs.layout.boxed_couples;
        let box_w = bc.box_width;
        let box_h = bc.box_height;
        let box_w2 = bc.box_width_2_spouses;
        let box_w3 = bc.box_width_3_spouses;
        let gap_w = bc.gap_width;
        let gap_h = bc.gap_height;

        let max_gen = if prefs.scope.generations == 0 {
            100
        } else {
            prefs.scope.generations
        };
        let env_left: Vec<f64> = vec![0.0; max_gen as usize];
        let mut global_right: Vec<f64> = vec![0.0; max_gen as usize];

        let mut individuals: HashMap<String, Individual<BoxedCouplesGeo>> = HashMap::new();

        place_descendants(
            genrep,
            root_id,
            &env_left,
            0,
            box_w,
            box_h,
            box_w2,
            box_w3,
            gap_w,
            gap_h,
            &mut individuals,
            &mut global_right,
        );
        compact_pass(
            root_id,
            genrep,
            &mut individuals,
            &mut global_right,
            gap_w,
            0,
        );
        recenter_pass(
            root_id,
            genrep,
            box_w,
            box_w2,
            box_w3,
            gap_w,
            &mut individuals,
            None,
        );
        fix_overlaps_pass(genrep, gap_w, &mut individuals, &mut global_right);
        recenter_pass(
            root_id,
            genrep,
            box_w,
            box_w2,
            box_w3,
            gap_w,
            &mut individuals,
            None,
        );

        // Add in-scope spouses of placed individuals to the output,
        // skipping spouses of the last (deepest) generation unless opted in.
        let placed_ids: Vec<String> = individuals.keys().cloned().collect();

        let max_gen: Option<u32> = if !prefs.show.last_gen_spouses {
            placed_ids
                .iter()
                .filter_map(|id| {
                    individuals.get(id).and_then(|i| {
                        if let Some(BoxedCouplesGeo::Individual(g)) = &i.geo {
                            Some(g.generation)
                        } else {
                            None
                        }
                    })
                })
                .max()
        } else {
            None
        };

        for ind_id in placed_ids {
            if let Some(last_gen) = max_gen {
                if let Some(i) = individuals.get(&ind_id) {
                    if let Some(BoxedCouplesGeo::Individual(g)) = &i.geo {
                        if g.generation == last_gen {
                            continue;
                        }
                    }
                }
            }
            let spouses = spouses_of(&ind_id, genrep);
            for spouse_id in spouses {
                #[allow(clippy::map_entry)]
                if !individuals.contains_key(&spouse_id) {
                    if let Some(spouse) = genrep.get_individual(&spouse_id) {
                        individuals.insert(spouse_id, copy_individual(spouse, None));
                    }
                }
            }
        }

        let families = copy_families(genrep, |fam| {
            build_family_geo(fam, &individuals, box_h, box_w, box_w2, box_w3)
        });

        Ok(Genrep {
            individuals,
            families,
            first_individual_id: genrep.first_individual_id.clone(),
        })
    }
}

// ── Scene IR emission ─────────────────────────────────────────────────────────

/// Parse the size (last whitespace-delimited token) from a font preference string.
///
/// Example: `"Georgia 14"` → `14.0`.  Falls back to `fallback` when parsing
/// fails or the string is empty.
fn parse_font_size(s: &str, fallback: f64) -> f64 {
    s.trim()
        .rsplit_once(' ')
        .and_then(|(_, last)| last.parse::<f64>().ok())
        .unwrap_or(fallback)
}

/// Returns the ID of the other member of a family given one member's ID.
fn spouse_id_from_family_bc<G>(
    ind_id: &str,
    fam: &crate::parser::genrep::Family<G>,
) -> Option<String> {
    if fam.husband_id.as_deref() == Some(ind_id) {
        fam.wife_id.clone()
    } else {
        fam.husband_id.clone()
    }
}

/// Emit a `Scene` from a fully-placed `boxed_couples` layout.
///
/// Translates all layout-space coordinates into display space (y=0 at top,
/// y increases downward) and assembles `Primitive`s in the order:
/// boxes, text, connectors.
pub fn emit_scene(genrep: &Genrep<BoxedCouplesGeo>, prefs: &Prefs) -> crate::scene::Scene {
    use crate::scene::{
        BoxPrimitive, ConnectorPrimitive, GroupPrimitive, Point, Primitive, Rect, Scene,
    };
    // ── 4a: load highlights ──────────────────────────────────────────────────
    // ── 4a: load highlights ──────────────────────────────────────────────────
    let highlighted_ids = crate::layout::common::highlight_set(prefs);
    // ── 4b: collect placed individuals ──────────────────────────────────────
    // Sorted by id so the SVG element order is stable across runs (the source
    // `individuals` is a HashMap with randomised iteration order); see the
    // "Deterministic emit order" note in CLAUDE.md.
    let mut placed: Vec<(&str, &IndividualGeo)> = genrep
        .individuals
        .iter()
        .filter(|(_, ind)| ind.in_scope)
        .filter_map(|(id, ind)| {
            if let Some(BoxedCouplesGeo::Individual(ref g)) = ind.geo {
                Some((id.as_str(), g))
            } else {
                None
            }
        })
        .collect();
    placed.sort_by(|a, b| a.0.cmp(b.0));

    if placed.is_empty() {
        return Scene {
            primitives: vec![],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            },
        };
    }

    // ── 4c: display-space transforms ────────────────────────────────────────
    let canvas_min_x = placed
        .iter()
        .map(|(_, g)| g.x - g.width / 2.0)
        .fold(f64::INFINITY, f64::min);
    let to_display_x = |lx: f64| lx - canvas_min_x;

    let root_pos_bottom =
        prefs.layout.root_pos.is_empty() || prefs.layout.root_pos.starts_with("bot");

    let to_display_y: Box<dyn Fn(f64) -> f64> = if root_pos_bottom {
        let cmin = placed
            .iter()
            .map(|(_, g)| g.y - g.height / 2.0)
            .fold(f64::INFINITY, f64::min);
        Box::new(move |ly: f64| ly - cmin)
    } else {
        let cmax = placed
            .iter()
            .map(|(_, g)| g.y + g.height / 2.0)
            .fold(f64::NEG_INFINITY, f64::max);
        Box::new(move |ly: f64| cmax - ly)
    };

    // ── 4d: font sizes ──────────────────────────────────────────────────────
    let font_size = parse_font_size(&prefs.output.style.fonts.names, 13.0);
    let date_font_size_raw = parse_font_size(&prefs.output.style.fonts.dates, font_size);
    let date_font_size = if date_font_size_raw <= 0.0 {
        font_size
    } else {
        date_font_size_raw
    };
    let _id_font_size = parse_font_size(&prefs.output.style.fonts.id, 8.0);

    let bc = &prefs.layout.boxed_couples;
    let spacing = &prefs.output.style.spacing.boxed_couples;

    let mut box_groups: Vec<Primitive> = Vec::new();

    // ── 4e: per-individual primitives ───────────────────────────────────────
    for (ind_id, geo) in &placed {
        // Box primitive
        let box_display_top = f64::min(
            to_display_y(geo.y + geo.height / 2.0),
            to_display_y(geo.y - geo.height / 2.0),
        );
        let box_bbox = Rect {
            x: to_display_x(geo.x - geo.width / 2.0),
            y: box_display_top,
            w: geo.width,
            h: geo.height,
        };
        let is_highlighted = highlighted_ids.contains(*ind_id);
        let ind_id_trimmed = ind_id
            .trim_start_matches('@')
            .trim_end_matches('@')
            .to_string();

        let ind = &genrep.individuals[*ind_id];
        let mut box_children: Vec<Primitive> = Vec::new();
        box_children.push(Primitive::Box(BoxPrimitive {
            bbox: box_bbox,
            two_spouses: geo.width > bc.box_width + 1.0,
        }));

        // Layout sections
        let center_display_y = to_display_y(geo.y);
        let region_height = (geo.height - bc.spouse_sep_height) / 2.0;
        let top_region_display = center_display_y - bc.spouse_sep_height / 2.0 - region_height;
        let bottom_region_display = center_display_y + bc.spouse_sep_height / 2.0;
        let sp_section_top = if root_pos_bottom {
            top_region_display
        } else {
            bottom_region_display
        };
        let ind_section_top = if root_pos_bottom {
            bottom_region_display
        } else {
            top_region_display
        };
        let marr_y = center_display_y + date_font_size / 2.0;

        let pruned_spouse_ids: std::collections::HashSet<String> =
            prune_spouses(ind_id, genrep).into_iter().collect();
        let sorted_fam_ids = sort_families_by_date(ind, genrep);
        let spouses: Vec<(&String, &crate::parser::genrep::Family<BoxedCouplesGeo>)> =
            sorted_fam_ids
                .iter()
                .filter_map(|fid| genrep.families.get(fid).map(|f| (fid, f)))
                .filter(|(_, f)| f.in_scope)
                .filter(|(_, f)| {
                    spouse_id_from_family_bc(ind_id, f)
                        .is_some_and(|id| pruned_spouse_ids.contains(&id))
                })
                .collect();
        let is_three_spouse = geo.width > bc.box_width_2_spouses + 1.0;
        let is_two_spouse = !is_three_spouse && geo.width > bc.box_width + 1.0;

        if is_three_spouse {
            let off3 = bc.box_width_3_spouses / 2.0 - bc.box_width / 2.0;
            let left_cx = to_display_x(geo.x - off3); // sp1 center = conn_out1_x
            let center_cx = to_display_x(geo.x); // sp2 + ind center = conn_out2_x
            let right_cx = to_display_x(geo.x + off3); // sp3 center = conn_out3_x
            let box_display_left = to_display_x(geo.x - geo.width / 2.0);

            // Individual section — bottom half of center region; name spans full box width
            let name_baseline = ind_section_top + spacing.name_above + font_size;
            emit_individual_section(
                &mut box_children,
                ind,
                &ind_id_trimmed,
                center_cx,
                box_display_left + 2.0,
                geo.width,
                name_baseline,
                font_size,
                date_font_size,
                spacing,
                prefs,
                is_highlighted,
            );

            // sp1 — top half of left region
            if let Some((fam1_id, fam1)) = spouses.first() {
                if let Some(sp1_id) = spouse_id_from_family_bc(ind_id, fam1) {
                    if let Some(sp1) = genrep.individuals.get(&sp1_id) {
                        box_children.extend(emit_spouse_primitives(
                            left_cx,
                            box_display_left + 2.0,
                            marr_y,
                            sp_section_top,
                            sp1,
                            fam1,
                            fam1_id,
                            prefs,
                            bc.box_width,
                            font_size,
                            date_font_size,
                            spacing,
                            highlighted_ids.contains(sp1_id.as_str()),
                        ));
                    }
                }
            }

            // sp2 — top half of center region
            if let Some((fam2_id, fam2)) = spouses.get(1) {
                if let Some(sp2_id) = spouse_id_from_family_bc(ind_id, fam2) {
                    if let Some(sp2) = genrep.individuals.get(&sp2_id) {
                        box_children.extend(emit_spouse_primitives(
                            center_cx,
                            to_display_x(geo.x - bc.box_width / 2.0) + 2.0,
                            marr_y,
                            sp_section_top,
                            sp2,
                            fam2,
                            fam2_id,
                            prefs,
                            bc.box_width,
                            font_size,
                            date_font_size,
                            spacing,
                            highlighted_ids.contains(sp2_id.as_str()),
                        ));
                    }
                }
            }

            // sp3 — top half of right region
            if let Some((fam3_id, fam3)) = spouses.get(2) {
                if let Some(sp3_id) = spouse_id_from_family_bc(ind_id, fam3) {
                    if let Some(sp3) = genrep.individuals.get(&sp3_id) {
                        box_children.extend(emit_spouse_primitives(
                            right_cx,
                            to_display_x(geo.x + off3 - bc.box_width / 2.0) + 2.0,
                            marr_y,
                            sp_section_top,
                            sp3,
                            fam3,
                            fam3_id,
                            prefs,
                            bc.box_width,
                            font_size,
                            date_font_size,
                            spacing,
                            highlighted_ids.contains(sp3_id.as_str()),
                        ));
                    }
                }
            }
        } else if is_two_spouse {
            let left_cx = to_display_x(geo.x - (bc.box_width_2_spouses / 2.0 - bc.box_width / 2.0));
            let right_cx =
                to_display_x(geo.x + (bc.box_width_2_spouses / 2.0 - bc.box_width / 2.0));
            let ind_cx = to_display_x(geo.x);
            let box_display_left = to_display_x(geo.x - geo.width / 2.0);

            let name_baseline = ind_section_top + spacing.name_above + font_size;
            emit_individual_section(
                &mut box_children,
                ind,
                &ind_id_trimmed,
                ind_cx,
                box_display_left + 2.0,
                geo.width,
                name_baseline,
                font_size,
                date_font_size,
                spacing,
                prefs,
                is_highlighted,
            );

            // First spouse in left section
            if let Some((fam1_id, fam1)) = spouses.first() {
                if let Some(sp1_id) = spouse_id_from_family_bc(ind_id, fam1) {
                    if let Some(sp1) = genrep.individuals.get(&sp1_id) {
                        let id_x = to_display_x(geo.x - bc.box_width_2_spouses / 2.0) + 2.0;
                        box_children.extend(emit_spouse_primitives(
                            left_cx,
                            id_x,
                            marr_y,
                            sp_section_top,
                            sp1,
                            fam1,
                            fam1_id,
                            prefs,
                            bc.box_width,
                            font_size,
                            date_font_size,
                            spacing,
                            highlighted_ids.contains(sp1_id.as_str()),
                        ));
                    }
                }
            }

            // Second spouse in right section
            if let Some((fam2_id, fam2)) = spouses.get(1) {
                if let Some(sp2_id) = spouse_id_from_family_bc(ind_id, fam2) {
                    if let Some(sp2) = genrep.individuals.get(&sp2_id) {
                        let id_x =
                            to_display_x(geo.x + bc.box_width_2_spouses / 2.0 - bc.box_width) + 2.0;
                        box_children.extend(emit_spouse_primitives(
                            right_cx,
                            id_x,
                            marr_y,
                            sp_section_top,
                            sp2,
                            fam2,
                            fam2_id,
                            prefs,
                            bc.box_width,
                            font_size,
                            date_font_size,
                            spacing,
                            highlighted_ids.contains(sp2_id.as_str()),
                        ));
                    }
                }
            }
        } else {
            // Single-spouse or no-spouse box
            let section_cx = to_display_x(geo.x);
            let box_display_left = to_display_x(geo.x - geo.width / 2.0);
            let name_baseline = ind_section_top + spacing.name_above + font_size;

            emit_individual_section(
                &mut box_children,
                ind,
                &ind_id_trimmed,
                section_cx,
                box_display_left + 2.0,
                geo.width,
                name_baseline,
                font_size,
                date_font_size,
                spacing,
                prefs,
                is_highlighted,
            );

            let spouse_emitted = if let Some((fam_id, fam)) = spouses.first() {
                if let Some(sp_id) = spouse_id_from_family_bc(ind_id, fam) {
                    if let Some(sp) = genrep.individuals.get(&sp_id) {
                        box_children.extend(emit_spouse_primitives(
                            section_cx,
                            box_display_left + 2.0,
                            marr_y,
                            sp_section_top,
                            sp,
                            fam,
                            fam_id,
                            prefs,
                            geo.width,
                            font_size,
                            date_font_size,
                            spacing,
                            highlighted_ids.contains(sp_id.as_str()),
                        ));
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            if !spouse_emitted {
                box_children.extend(emit_blank_spouse_section(
                    section_cx,
                    marr_y,
                    sp_section_top,
                    prefs,
                    geo.width,
                    font_size,
                    date_font_size,
                    spacing,
                ));
            }
        }

        // Double-wrap so that applications that strip one group level still see a group
        box_groups.push(Primitive::Group(GroupPrimitive {
            id: String::new(),
            children: vec![Primitive::Group(GroupPrimitive {
                id: ind_id_trimmed,
                children: box_children,
            })],
        }));
    }

    // ── 4f: connector primitives ─────────────────────────────────────────────
    let mut connector_groups: Vec<Primitive> = Vec::new();

    // Sorted by family id so connector (and realistic-tree branch) element order
    // is stable across runs; see the "Deterministic emit order" note in CLAUDE.md.
    let mut sorted_families: Vec<(&String, &crate::parser::genrep::Family<BoxedCouplesGeo>)> =
        genrep.families.iter().collect();
    sorted_families.sort_by(|a, b| a.0.cmp(b.0));
    for (fam_id, fam) in sorted_families {
        if !fam.in_scope {
            continue;
        }
        let fam_geo = match &fam.geo {
            Some(BoxedCouplesGeo::Family(g)) => g,
            _ => continue,
        };

        // Find parent (prefer husband, fall back to wife)
        let is_placed_individual = |id: &str| {
            matches!(
                genrep.individuals.get(id).and_then(|i| i.geo.as_ref()),
                Some(BoxedCouplesGeo::Individual(_))
            )
        };
        let parent_id = fam
            .husband_id
            .as_deref()
            .filter(|id| is_placed_individual(id))
            .or_else(|| fam.wife_id.as_deref().filter(|id| is_placed_individual(id)));
        let parent_id = match parent_id {
            Some(p) => p,
            None => continue,
        };

        let parent_ind = &genrep.individuals[parent_id];
        let parent_geo = match parent_ind.geo.as_ref() {
            Some(BoxedCouplesGeo::Individual(g)) => g,
            _ => continue,
        };

        // Determine which conn_out_x to use based on the family's position in the pruned list.
        let pruned_spouse_ids = prune_spouses(parent_id, genrep);
        let spouse_of_fam = spouse_id_from_family_bc(parent_id, fam);
        let pruned_idx = match spouse_of_fam
            .as_deref()
            .and_then(|sp| pruned_spouse_ids.iter().position(|s| s == sp))
        {
            Some(idx) => idx,
            None => continue, // family's spouse not in pruned list → skip
        };
        let conn_out_x = match pruned_idx {
            0 => fam_geo.conn_out1_x,
            1 if fam_geo.has_spouse2 || fam_geo.has_spouse3 => fam_geo.conn_out2_x,
            2 if fam_geo.has_spouse3 => fam_geo.conn_out3_x,
            _ => fam_geo.conn_out1_x,
        };

        // 2-channel routing: lower channel (1/3) for sp1/sp3, upper (2/3) for sp2,
        // only when the parent has a 3-spouse box AND all 3 spouses have ≥2 children.
        let bar_y_fraction = if fam_geo.has_spouse3 {
            let all_two_plus = pruned_spouse_ids
                .iter()
                .all(|sp| children_with_spouse(parent_id, sp, genrep).len() >= 2);
            if all_two_plus {
                if pruned_idx == 1 {
                    2.0 / 3.0
                } else {
                    1.0 / 3.0
                }
            } else {
                0.5
            }
        } else {
            0.5
        };

        // Parent exit point: the child-facing edge of the parent box
        let parent_point = Point {
            x: to_display_x(conn_out_x),
            y: to_display_y(parent_geo.y - parent_geo.height / 2.0),
        };

        // Child entry points
        let child_points: Vec<Point> = fam
            .children_ids
            .iter()
            .filter_map(|cid| {
                let child = genrep.individuals.get(cid)?;
                if let Some(BoxedCouplesGeo::Individual(cg)) = child.geo.as_ref() {
                    Some(Point {
                        x: to_display_x(cg.conn_in_x),
                        y: to_display_y(cg.y + cg.height / 2.0),
                    })
                } else {
                    None
                }
            })
            .collect();

        if child_points.is_empty() {
            continue;
        }

        let fam_id_trimmed = fam_id
            .trim_start_matches('@')
            .trim_end_matches('@')
            .to_string();
        connector_groups.push(Primitive::Group(GroupPrimitive {
            id: String::new(),
            children: vec![Primitive::Group(GroupPrimitive {
                id: format!("{fam_id_trimmed}-connectors"),
                children: vec![Primitive::Connector(ConnectorPrimitive {
                    parent_points: vec![parent_point],
                    child_points,
                    bar_y_fraction,
                })],
            })],
        }));
    }

    // ── 4g: assemble scene ──────────────────────────────────────────────────
    let content_w = placed
        .iter()
        .map(|(_, g)| to_display_x(g.x + g.width / 2.0))
        .fold(0.0_f64, f64::max);
    let content_h = placed
        .iter()
        .map(|(_, g)| {
            f64::max(
                to_display_y(g.y + g.height / 2.0),
                to_display_y(g.y - g.height / 2.0),
            )
        })
        .fold(0.0_f64, f64::max);

    let canvas_bounds = Rect {
        x: 0.0,
        y: 0.0,
        w: content_w,
        h: content_h,
    };

    // Connectors first, then boxes, so boxes (and their text) render on top of the
    // connectors. Thick connectors overshoot their nominal endpoints and would otherwise
    // visibly overlap the boxes.
    let mut primitives = connector_groups;
    primitives.extend(box_groups);

    Scene {
        primitives,
        canvas_bounds,
    }
}

/// Emit name, optional ID, and optional birth/death text primitives for one individual section.
///
/// Used for both the center section of a two-spouse box and the full width of a single-spouse
/// box — callers differ only in `center_x`, `id_left_x`, and `width`.
#[allow(clippy::too_many_arguments)]
fn emit_individual_section(
    children: &mut Vec<crate::scene::Primitive>,
    ind: &crate::parser::genrep::Individual<BoxedCouplesGeo>,
    ind_id_trimmed: &str,
    center_x: f64,
    id_left_x: f64,
    width: f64,
    name_baseline: f64,
    font_size: f64,
    date_font_size: f64,
    spacing: &crate::preferences::BoxedCouplesSpacingPrefs,
    prefs: &Prefs,
    is_highlighted: bool,
) {
    use crate::format::{format_event, format_name};
    use crate::scene::{GroupPrimitive, Primitive, Rect, TextAlign, TextAttr, TextPrimitive};

    children.push(Primitive::Group(GroupPrimitive {
        id: format!("{ind_id_trimmed}-name"),
        children: vec![Primitive::Text(TextPrimitive {
            content: format_name(ind, prefs),
            bbox: Rect {
                x: center_x - width / 2.0,
                y: name_baseline - font_size,
                w: width,
                h: font_size,
            },
            align: TextAlign::Center,
            attrs: crate::scene::label_attrs(TextAttr::IndividualName, is_highlighted),
        })],
    }));

    if prefs.show.id {
        children.push(Primitive::Text(TextPrimitive {
            content: ind_id_trimmed.to_string(),
            bbox: Rect {
                x: id_left_x,
                y: name_baseline - font_size,
                w: width,
                h: font_size,
            },
            align: TextAlign::Left,
            attrs: vec![TextAttr::IndividualId],
        }));
    }

    let mut y_pos = name_baseline;
    if prefs.show.birth {
        y_pos += spacing.date_above + date_font_size;
        let birth_content = ind
            .birth
            .as_ref()
            .and_then(|b| {
                format_event(
                    &prefs.format.birth,
                    b.date.as_ref(),
                    b.place.as_deref(),
                    &prefs.format.date_qualifiers,
                )
            })
            .unwrap_or_default();
        children.push(Primitive::Text(TextPrimitive {
            content: birth_content,
            bbox: Rect {
                x: center_x - width / 2.0,
                y: y_pos - date_font_size,
                w: width,
                h: date_font_size,
            },
            align: TextAlign::Center,
            attrs: vec![TextAttr::BirthData],
        }));
    }
    if prefs.show.death {
        y_pos += spacing.date_above + date_font_size;
        let death_content = ind
            .death
            .as_ref()
            .and_then(|d| {
                format_event(
                    &prefs.format.death,
                    d.date.as_ref(),
                    d.place.as_deref(),
                    &prefs.format.date_qualifiers,
                )
            })
            .unwrap_or_default();
        children.push(Primitive::Text(TextPrimitive {
            content: death_content,
            bbox: Rect {
                x: center_x - width / 2.0,
                y: y_pos - date_font_size,
                w: width,
                h: date_font_size,
            },
            align: TextAlign::Center,
            attrs: vec![TextAttr::DeathData],
        }));
    }
}

/// Emit blank placeholder primitives for the spouse section when no spouse is present.
/// This ensures the text backend allocates the same rows as a full spouse section,
/// keeping all individuals in the same generation vertically aligned.
#[allow(clippy::too_many_arguments)]
fn emit_blank_spouse_section(
    cx: f64,
    marr_y: f64,
    sp_section_top: f64,
    prefs: &Prefs,
    section_width: f64,
    font_size: f64,
    date_font_size: f64,
    spacing: &crate::preferences::BoxedCouplesSpacingPrefs,
) -> Vec<crate::scene::Primitive> {
    use crate::scene::{Primitive, Rect, TextAlign, TextAttr, TextPrimitive};
    let mut result: Vec<Primitive> = Vec::new();

    // Blank marriage row — causes text backend to allocate blank-before + blank-after rows
    if prefs.show.marriage {
        result.push(Primitive::Text(TextPrimitive {
            content: String::new(),
            bbox: Rect {
                x: cx - section_width / 2.0,
                y: marr_y - date_font_size,
                w: section_width,
                h: date_font_size,
            },
            align: TextAlign::Center,
            attrs: vec![TextAttr::MarriageData],
        }));
    }

    let sp_name_baseline = sp_section_top + spacing.name_above + font_size;
    result.push(Primitive::Text(TextPrimitive {
        content: String::new(),
        bbox: Rect {
            x: cx - section_width / 2.0,
            y: sp_name_baseline - font_size,
            w: section_width,
            h: font_size,
        },
        align: TextAlign::Center,
        attrs: vec![TextAttr::SpouseName],
    }));

    let mut y = sp_name_baseline;
    if prefs.show.birth {
        y += spacing.date_above + date_font_size;
        result.push(Primitive::Text(TextPrimitive {
            content: String::new(),
            bbox: Rect {
                x: cx - section_width / 2.0,
                y: y - date_font_size,
                w: section_width,
                h: date_font_size,
            },
            align: TextAlign::Center,
            attrs: vec![TextAttr::BirthData],
        }));
    }
    if prefs.show.death {
        y += spacing.date_above + date_font_size;
        result.push(Primitive::Text(TextPrimitive {
            content: String::new(),
            bbox: Rect {
                x: cx - section_width / 2.0,
                y: y - date_font_size,
                w: section_width,
                h: date_font_size,
            },
            align: TextAlign::Center,
            attrs: vec![TextAttr::DeathData],
        }));
    }
    result
}

/// Emit primitives for one spouse section of a boxed-couples box.
/// Returns primitives grouped by semantic role (marriage sub-group, spouse name sub-group).
#[allow(clippy::too_many_arguments)]
fn emit_spouse_primitives(
    cx: f64,
    id_x: f64,
    marr_y: f64,
    sp_section_top: f64,
    sp: &crate::parser::genrep::Individual<BoxedCouplesGeo>,
    fam: &crate::parser::genrep::Family<BoxedCouplesGeo>,
    fam_id: &str,
    prefs: &Prefs,
    section_width: f64,
    font_size: f64,
    date_font_size: f64,
    spacing: &crate::preferences::BoxedCouplesSpacingPrefs,
    is_highlighted: bool,
) -> Vec<crate::scene::Primitive> {
    use crate::format::{format_event, format_event_extra, format_name};
    use crate::scene::{GroupPrimitive, Primitive, Rect, TextAlign, TextAttr, TextPrimitive};
    let mut result: Vec<Primitive> = Vec::new();

    let fam_id_trimmed = fam_id
        .trim_start_matches('@')
        .trim_end_matches('@')
        .to_string();

    // Marriage data — wrapped in a sub-group so SVG editors see symbol + text as one unit
    if prefs.show.marriage {
        if let Some(marr) = &fam.marriage {
            if let Some(s) = format_event_extra(
                &prefs.format.marriage,
                marr.date.as_ref(),
                marr.place.as_deref(),
                &prefs.format.date_qualifiers,
                &[("relig_marr", fam.relig_marr.as_deref().unwrap_or(""))],
            ) {
                result.push(Primitive::Group(GroupPrimitive {
                    id: format!("{fam_id_trimmed}-marriage"),
                    children: vec![Primitive::Text(TextPrimitive {
                        content: s,
                        bbox: Rect {
                            x: cx - section_width / 2.0,
                            y: marr_y - date_font_size,
                            w: section_width,
                            h: date_font_size,
                        },
                        align: TextAlign::Center,
                        attrs: vec![TextAttr::MarriageData],
                    })],
                }));
            }
        }
    }

    // Family ID (plain text, not inside marriage sub-group)
    if prefs.show.id {
        result.push(Primitive::Text(TextPrimitive {
            content: fam_id_trimmed,
            bbox: Rect {
                x: id_x,
                y: marr_y - date_font_size,
                w: section_width,
                h: date_font_size,
            },
            align: TextAlign::Left,
            attrs: vec![TextAttr::IndividualId],
        }));
    }

    // Spouse name — wrapped in a sub-group so SVG editors see name + sex symbol as one unit
    let sp_id_trimmed = sp
        .id
        .trim_start_matches('@')
        .trim_end_matches('@')
        .to_string();
    let sp_name_baseline = sp_section_top + spacing.name_above + font_size;
    result.push(Primitive::Group(GroupPrimitive {
        id: format!("{sp_id_trimmed}-name"),
        children: vec![Primitive::Text(TextPrimitive {
            content: format_name(sp, prefs),
            bbox: Rect {
                x: cx - section_width / 2.0,
                y: sp_name_baseline - font_size,
                w: section_width,
                h: font_size,
            },
            align: TextAlign::Center,
            attrs: crate::scene::label_attrs(TextAttr::SpouseName, is_highlighted),
        })],
    }));

    // Spouse ID (plain text, not inside name sub-group)
    if prefs.show.id {
        result.push(Primitive::Text(TextPrimitive {
            content: sp_id_trimmed,
            bbox: Rect {
                x: id_x,
                y: sp_name_baseline - font_size,
                w: section_width,
                h: font_size,
            },
            align: TextAlign::Left,
            attrs: vec![TextAttr::IndividualId],
        }));
    }

    // Spouse birth
    let mut y = sp_name_baseline;
    if prefs.show.birth {
        y += spacing.date_above + date_font_size;
        let birth_content = sp
            .birth
            .as_ref()
            .and_then(|b| {
                format_event(
                    &prefs.format.birth,
                    b.date.as_ref(),
                    b.place.as_deref(),
                    &prefs.format.date_qualifiers,
                )
            })
            .unwrap_or_default();
        result.push(Primitive::Text(TextPrimitive {
            content: birth_content,
            bbox: Rect {
                x: cx - section_width / 2.0,
                y: y - date_font_size,
                w: section_width,
                h: date_font_size,
            },
            align: TextAlign::Center,
            attrs: vec![TextAttr::BirthData],
        }));
    }

    // Spouse death
    if prefs.show.death {
        y += spacing.date_above + date_font_size;
        let death_content = sp
            .death
            .as_ref()
            .and_then(|d| {
                format_event(
                    &prefs.format.death,
                    d.date.as_ref(),
                    d.place.as_deref(),
                    &prefs.format.date_qualifiers,
                )
            })
            .unwrap_or_default();
        result.push(Primitive::Text(TextPrimitive {
            content: death_content,
            bbox: Rect {
                x: cx - section_width / 2.0,
                y: y - date_font_size,
                w: section_width,
                h: date_font_size,
            },
            align: TextAlign::Center,
            attrs: vec![TextAttr::DeathData],
        }));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_genrep() -> Genrep {
        let mut individuals = HashMap::new();
        let mut families = HashMap::new();

        individuals.insert(
            "I1".to_string(),
            Individual {
                id: "I1".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec!["F1".to_string()],
                famc: vec![],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I2".to_string(),
            Individual {
                id: "I2".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec!["F1".to_string()],
                famc: vec![],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I3".to_string(),
            Individual {
                id: "I3".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec!["F2".to_string()],
                famc: vec!["F1".to_string()],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I4".to_string(),
            Individual {
                id: "I4".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec![],
                famc: vec!["F1".to_string()],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I5".to_string(),
            Individual {
                id: "I5".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec![],
                famc: vec!["F1".to_string()],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I6".to_string(),
            Individual {
                id: "I6".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec![],
                famc: vec!["F2".to_string()],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        families.insert(
            "F1".to_string(),
            crate::parser::genrep::Family {
                id: "F1".to_string(),
                husband_id: Some("I1".to_string()),
                wife_id: Some("I2".to_string()),
                children_ids: vec!["I3".to_string(), "I4".to_string(), "I5".to_string()],
                marriage: None,
                relig_marr: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        families.insert(
            "F2".to_string(),
            crate::parser::genrep::Family {
                id: "F2".to_string(),
                husband_id: Some("I3".to_string()),
                wife_id: None,
                children_ids: vec!["I6".to_string()],
                marriage: None,
                relig_marr: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        Genrep {
            individuals,
            families,
            first_individual_id: Some("I1".to_string()),
        }
    }

    fn desc_prefs() -> Prefs {
        let mut prefs = Prefs::default();
        prefs.scope.direction = "descendants".to_string();
        prefs.scope.root = "I1".to_string();
        prefs.scope.generations = 4;
        prefs
    }

    fn three_spouse_genrep() -> Genrep {
        let mut individuals = HashMap::new();
        let mut families = HashMap::new();

        individuals.insert(
            "I10".to_string(),
            Individual {
                id: "I10".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec!["F10".to_string(), "F11".to_string(), "F12".to_string()],
                famc: vec![],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I11".to_string(),
            Individual {
                id: "I11".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec!["F10".to_string()],
                famc: vec![],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I12".to_string(),
            Individual {
                id: "I12".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec!["F11".to_string()],
                famc: vec![],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I13".to_string(),
            Individual {
                id: "I13".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec!["F12".to_string()],
                famc: vec![],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I14".to_string(),
            Individual {
                id: "I14".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec![],
                famc: vec!["F10".to_string()],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I15".to_string(),
            Individual {
                id: "I15".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec![],
                famc: vec!["F11".to_string()],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        families.insert(
            "F10".to_string(),
            crate::parser::genrep::Family {
                id: "F10".to_string(),
                husband_id: Some("I10".to_string()),
                wife_id: Some("I11".to_string()),
                children_ids: vec!["I14".to_string()],
                marriage: None,
                relig_marr: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        families.insert(
            "F11".to_string(),
            crate::parser::genrep::Family {
                id: "F11".to_string(),
                husband_id: Some("I10".to_string()),
                wife_id: Some("I12".to_string()),
                children_ids: vec!["I15".to_string()],
                marriage: None,
                relig_marr: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        families.insert(
            "F12".to_string(),
            crate::parser::genrep::Family {
                id: "F12".to_string(),
                husband_id: Some("I10".to_string()),
                wife_id: Some("I13".to_string()),
                children_ids: vec![],
                marriage: None,
                relig_marr: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        Genrep {
            individuals,
            families,
            first_individual_id: Some("I10".to_string()),
        }
    }

    fn two_spouse_genrep() -> Genrep {
        let mut individuals = HashMap::new();
        let mut families = HashMap::new();

        individuals.insert(
            "I20".to_string(),
            Individual {
                id: "I20".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec!["F20".to_string(), "F21".to_string()],
                famc: vec![],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I21".to_string(),
            Individual {
                id: "I21".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec!["F20".to_string()],
                famc: vec![],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I22".to_string(),
            Individual {
                id: "I22".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec!["F21".to_string()],
                famc: vec![],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        individuals.insert(
            "I23".to_string(),
            Individual {
                id: "I23".to_string(),
                given: None,
                surname: None,
                sex: None,
                birth: None,
                death: None,
                fams: vec![],
                famc: vec!["F21".to_string()],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        families.insert(
            "F20".to_string(),
            crate::parser::genrep::Family {
                id: "F20".to_string(),
                husband_id: Some("I20".to_string()),
                wife_id: Some("I21".to_string()),
                children_ids: vec![],
                marriage: None,
                relig_marr: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        families.insert(
            "F21".to_string(),
            crate::parser::genrep::Family {
                id: "F21".to_string(),
                husband_id: Some("I20".to_string()),
                wife_id: Some("I22".to_string()),
                children_ids: vec!["I23".to_string()],
                marriage: None,
                relig_marr: None,
                notes: vec![],
                in_scope: true,
                geo: None,
            },
        );

        Genrep {
            individuals,
            families,
            first_individual_id: Some("I20".to_string()),
        }
    }

    fn two_spouse_prefs() -> Prefs {
        let mut prefs = Prefs::default();
        prefs.scope.direction = "descendants".to_string();
        prefs.scope.root = "I20".to_string();
        prefs.scope.generations = 4;
        prefs
    }

    fn ind_geo(result: &Genrep<BoxedCouplesGeo>, id: &str) -> IndividualGeo {
        match result.individuals[id].geo.as_ref().unwrap() {
            BoxedCouplesGeo::Individual(g) => g.clone(),
            _ => panic!("expected Individual geo for {id}"),
        }
    }

    #[test]
    fn no_overlap_generation_1() {
        let result = BoxedCouplesLayout
            .compute(&test_genrep(), &desc_prefs())
            .unwrap();
        let prefs = desc_prefs();
        let box_w = prefs.layout.boxed_couples.box_width;
        let gap_w = prefs.layout.boxed_couples.gap_width;

        let mut xs: Vec<f64> = ["I3", "I4", "I5"]
            .iter()
            .map(|id| ind_geo(&result, id).x)
            .collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());

        for pair in xs.windows(2) {
            assert!(
                pair[1] - pair[0] >= box_w + gap_w - 1e-6,
                "siblings overlap: gap = {}",
                pair[1] - pair[0]
            );
        }
    }

    #[test]
    fn root_centred_over_children() {
        let result = BoxedCouplesLayout
            .compute(&test_genrep(), &desc_prefs())
            .unwrap();
        let x_root = ind_geo(&result, "I1").x;
        let x_mid = ind_geo(&result, "I4").x;
        assert!(
            (x_root - x_mid).abs() < 1e-6,
            "root x={x_root} should equal middle child (I4) x={x_mid}"
        );
    }

    #[test]
    fn connector_points() {
        let result = BoxedCouplesLayout
            .compute(&test_genrep(), &desc_prefs())
            .unwrap();
        let box_h = desc_prefs().layout.boxed_couples.box_height;

        let g1 = ind_geo(&result, "I1");
        let g3 = ind_geo(&result, "I3");

        assert!(
            (g1.conn_in_y - (0.0 - box_h / 2.0)).abs() < 1e-6,
            "I1 conn_in_y wrong: got {}",
            g1.conn_in_y
        );

        assert!(
            (g3.conn_in_y - (g3.y - box_h / 2.0)).abs() < 1e-6,
            "I3 conn_in_y wrong: got {}",
            g3.conn_in_y
        );
    }

    #[test]
    fn prune_spouses_keeps_up_to_three() {
        // three_spouse_genrep has I10 with 3 spouses: I11 (1 child), I12 (1 child), I13 (0 children).
        // With a limit of 3, all three are returned (no pruning needed for ≤3).
        let genrep = three_spouse_genrep();
        let pruned = prune_spouses("I10", &genrep);
        assert_eq!(pruned.len(), 3);
        assert!(pruned.contains(&"I11".to_string()));
        assert!(pruned.contains(&"I12".to_string()));
        assert!(pruned.contains(&"I13".to_string()));
    }

    #[test]
    fn two_spouse_only_second_has_children() {
        let result = BoxedCouplesLayout
            .compute(&two_spouse_genrep(), &two_spouse_prefs())
            .unwrap();
        let prefs = two_spouse_prefs();
        let box_w = prefs.layout.boxed_couples.box_width;
        let box_w2 = prefs.layout.boxed_couples.box_width_2_spouses;

        let x_root = ind_geo(&result, "I20").x;
        let x_child = ind_geo(&result, "I23").x;

        let conn_out2_offset = box_w2 / 2.0 - box_w / 2.0;
        assert!(
            (x_root + conn_out2_offset - x_child).abs() < 1e-6,
            "expected x_root({x_root}) + offset({conn_out2_offset}) == x_child({x_child})"
        );
    }

    #[test]
    fn build_family_geo_wife_is_placed_individual() {
        // Family where WIFE is the placed descendant, HUSBAND is a spouse (None geo).
        let mut out: HashMap<String, Individual<BoxedCouplesGeo>> = HashMap::new();
        out.insert(
            "I_wife".to_string(),
            Individual {
                id: "I_wife".to_string(),
                given: None,
                surname: None,
                sex: Some('F'),
                birth: None,
                death: None,
                fams: vec!["F1".to_string()],
                famc: vec![],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: Some(BoxedCouplesGeo::Individual(IndividualGeo {
                    x: 0.0,
                    y: 0.0,
                    width: 220.0,
                    height: 160.0,
                    conn_in_x: 0.0,
                    conn_in_y: -80.0,
                    generation: 0,
                })),
            },
        );
        out.insert(
            "I_husb".to_string(),
            Individual {
                id: "I_husb".to_string(),
                given: None,
                surname: None,
                sex: Some('M'),
                birth: None,
                death: None,
                fams: vec!["F1".to_string()],
                famc: vec![],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: true,
                geo: None, // spouse — not placed
            },
        );
        let fam = crate::parser::genrep::Family {
            id: "F1".to_string(),
            husband_id: Some("I_husb".to_string()),
            wife_id: Some("I_wife".to_string()),
            children_ids: vec![],
            marriage: None,
            relig_marr: None,
            notes: vec![],
            in_scope: true,
            geo: None,
        };
        let result = build_family_geo(&fam, &out, 160.0, 220.0, 480.0, 800.0);
        assert!(
            result.is_some(),
            "build_family_geo must succeed when wife is the placed individual"
        );
    }

    #[test]
    fn test_last_sibling_children_placed() {
        use crate::parser::{compute_scope, parse_str};
        use crate::preferences::Prefs;

        const GED: &str = "\
0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME Root /R/\n1 SEX M\n1 FAMS @F1@\n\
0 @I2@ INDI\n1 NAME Spouse /S/\n1 SEX F\n1 FAMS @F1@\n\
0 @I3@ INDI\n1 NAME FirstChild /C/\n1 SEX M\n1 FAMC @F1@\n\
0 @I4@ INDI\n1 NAME SecondChild /C/\n1 SEX M\n1 FAMC @F1@\n1 FAMS @F2@\n\
0 @I5@ INDI\n1 NAME GrandparentSpouse /G/\n1 SEX F\n1 FAMS @F2@\n\
0 @I6@ INDI\n1 NAME Grandchild /G/\n1 SEX M\n1 FAMC @F2@\n\
0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I3@\n1 CHIL @I4@\n\
0 @F2@ FAM\n1 HUSB @I4@\n1 WIFE @I5@\n1 CHIL @I6@\n\
0 TRLR\n";

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.scope.generations = 3;
        prefs.layout.layout_type = "boxed_couples".into();

        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(3));
        let bc = BoxedCouplesLayout.compute(&genrep, &prefs).unwrap();

        let i6 = bc
            .individuals
            .get("I6")
            .expect("I6 (grandchild of last sibling) must be placed");
        assert!(
            matches!(i6.geo, Some(BoxedCouplesGeo::Individual(_))),
            "I6 must have an IndividualGeo, not None"
        );
    }

    #[test]
    fn test_global_right_tight_packing() {
        use crate::parser::{compute_scope, parse_str};
        use crate::preferences::Prefs;

        // I1+I2 → [I3, I4(leaf), I5]
        // I3+I6 → [I7]          (single gen-2 grandchild under I3)
        // I5+I8 → [I9, I10]     (two gen-2 grandchildren under I5)
        //
        // With global-right + shift: I5 is placed at x_default (right next to I4),
        // centred over its two shifted children I9 and I10.
        // With "pad last value" I5 would end up further right (I4's right edge becomes
        // the gen-2 constraint for I9, pushing I5's centre past x_default).
        const GED: &str = "\
0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME Root /R/\n1 SEX M\n1 FAMS @F1@\n\
0 @I2@ INDI\n1 NAME Spouse1 /S/\n1 SEX F\n1 FAMS @F1@\n\
0 @I3@ INDI\n1 NAME ChildA /C/\n1 SEX M\n1 FAMC @F1@\n1 FAMS @F2@\n\
0 @I4@ INDI\n1 NAME ChildB /C/\n1 SEX M\n1 FAMC @F1@\n\
0 @I5@ INDI\n1 NAME ChildC /C/\n1 SEX M\n1 FAMC @F1@\n1 FAMS @F3@\n\
0 @I6@ INDI\n1 NAME SpouseA /S/\n1 SEX F\n1 FAMS @F2@\n\
0 @I7@ INDI\n1 NAME GrandchildA /G/\n1 SEX M\n1 FAMC @F2@\n\
0 @I8@ INDI\n1 NAME SpouseC /S/\n1 SEX F\n1 FAMS @F3@\n\
0 @I9@ INDI\n1 NAME GrandchildC1 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @I10@ INDI\n1 NAME GrandchildC2 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I3@\n1 CHIL @I4@\n1 CHIL @I5@\n\
0 @F2@ FAM\n1 HUSB @I3@\n1 WIFE @I6@\n1 CHIL @I7@\n\
0 @F3@ FAM\n1 HUSB @I5@\n1 WIFE @I8@\n1 CHIL @I9@\n1 CHIL @I10@\n\
0 TRLR\n";

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.scope.generations = 3;
        prefs.layout.layout_type = "boxed_couples".into();

        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(3));
        let bc = BoxedCouplesLayout.compute(&genrep, &prefs).unwrap();
        let bc = &bc;

        let get_x = |id: &str| match &bc.individuals[id].geo {
            Some(BoxedCouplesGeo::Individual(g)) => g.x,
            _ => panic!("{id} not placed as Individual"),
        };

        let x4 = get_x("I4");
        let x5 = get_x("I5");
        let x9 = get_x("I9");
        let x10 = get_x("I10");

        let box_w = prefs.layout.boxed_couples.box_width;
        let gap_w = prefs.layout.boxed_couples.gap_width;

        // Parent-centring rule: I5 must be centred over its two children.
        assert!(
            (x5 - (x9 + x10) / 2.0).abs() < 1e-6,
            "I5 not centred over children: x5={x5}, x9={x9}, x10={x10}"
        );

        // Tight-packing: I5 must be at the minimum distance from I4.
        // (With "pad last value" I5 would be further right because I4's right
        // edge propagates as the gen-2 constraint for I9, pushing children
        // and parent rightward.)
        assert!(
            (x5 - (x4 + box_w + gap_w)).abs() < 1e-6,
            "I5 placed too far right — not using global right envelope: x4={x4}, x5={x5}"
        );
    }

    #[test]
    fn test_compact_left_packed_siblings() {
        use crate::parser::{compute_scope, parse_str};
        use crate::preferences::Prefs;

        // I1+I2 → [I3(leaf), I4(leaf), I5]
        // I5+I6 → [I7, I8, I9, I10, I11, I12] (6 grandchildren)
        //
        // With 6 grandchildren the median of I5's children exceeds I5's x_default,
        // so I5.x > x_default and the gap between I4 and I5 is larger than gap_w
        // before compaction. Since I3 and I4 are leaves, safe_shift == desired_shift
        // and the compact closes the gap exactly.
        const GED: &str = "\
0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME Root /R/\n1 SEX M\n1 FAMS @F1@\n\
0 @I2@ INDI\n1 NAME Spouse1 /S/\n1 SEX F\n1 FAMS @F1@\n\
0 @I3@ INDI\n1 NAME Child1 /C/\n1 SEX M\n1 FAMC @F1@\n\
0 @I4@ INDI\n1 NAME Child2 /C/\n1 SEX M\n1 FAMC @F1@\n\
0 @I5@ INDI\n1 NAME Child3 /C/\n1 SEX M\n1 FAMC @F1@\n1 FAMS @F2@\n\
0 @I6@ INDI\n1 NAME Spouse2 /S/\n1 SEX F\n1 FAMS @F2@\n\
0 @I7@ INDI\n1 NAME Grandchild1 /G/\n1 SEX M\n1 FAMC @F2@\n\
0 @I8@ INDI\n1 NAME Grandchild2 /G/\n1 SEX M\n1 FAMC @F2@\n\
0 @I9@ INDI\n1 NAME Grandchild3 /G/\n1 SEX M\n1 FAMC @F2@\n\
0 @I10@ INDI\n1 NAME Grandchild4 /G/\n1 SEX M\n1 FAMC @F2@\n\
0 @I11@ INDI\n1 NAME Grandchild5 /G/\n1 SEX M\n1 FAMC @F2@\n\
0 @I12@ INDI\n1 NAME Grandchild6 /G/\n1 SEX M\n1 FAMC @F2@\n\
0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I3@\n1 CHIL @I4@\n1 CHIL @I5@\n\
0 @F2@ FAM\n1 HUSB @I5@\n1 WIFE @I6@\n1 CHIL @I7@\n1 CHIL @I8@\n1 CHIL @I9@\n1 CHIL @I10@\n1 CHIL @I11@\n1 CHIL @I12@\n\
0 TRLR\n";

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.scope.generations = 3;
        prefs.layout.layout_type = "boxed_couples".into();

        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(3));
        let bc = BoxedCouplesLayout.compute(&genrep, &prefs).unwrap();
        let bc = &bc;

        let get_x = |id: &str| match &bc.individuals[id].geo {
            Some(BoxedCouplesGeo::Individual(g)) => g.x,
            _ => panic!("{id} not placed as Individual"),
        };

        let box_w = prefs.layout.boxed_couples.box_width;
        let gap_w = prefs.layout.boxed_couples.gap_width;

        let x_i3 = get_x("I3");
        let x_i4 = get_x("I4");
        let x_i5 = get_x("I5");

        let gap_45 = x_i5 - box_w / 2.0 - (x_i4 + box_w / 2.0);
        assert!(
            (gap_45 - gap_w).abs() < 1e-6,
            "gap I4→I5 should equal gap_w after compact pass (leaves), got {gap_45}"
        );

        let gap_34 = x_i4 - box_w / 2.0 - (x_i3 + box_w / 2.0);
        assert!(
            (gap_34 - gap_w).abs() < 1e-6,
            "relative gap I3→I4 must be preserved, got {gap_34}"
        );
    }

    #[test]
    fn test_compact_no_subtree_overlap() {
        use crate::parser::{compute_scope, parse_str};
        use crate::preferences::Prefs;

        // I1+I2 → [I3(leaf), I4, I5]
        // I4+I6 → [I7(leaf)]
        // I5+I8 → [I9, I10, I11, I12, I13, I14] (6 grandchildren)
        //
        // With I4 having child I7, the depth-1 clearance between I7 and I9 is
        // exactly gap_w before compaction, so safe_shift = 0. No shift should
        // occur, and I7 must not overlap I9. (Regression test: the WIP implementation
        // shifted I4 and I7 rightward into I5's subtree.)
        const GED: &str = "\
0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME Root /R/\n1 SEX M\n1 FAMS @F1@\n\
0 @I2@ INDI\n1 NAME Spouse1 /S/\n1 SEX F\n1 FAMS @F1@\n\
0 @I3@ INDI\n1 NAME Child1 /C/\n1 SEX M\n1 FAMC @F1@\n\
0 @I4@ INDI\n1 NAME Child2 /C/\n1 SEX M\n1 FAMC @F1@\n1 FAMS @F2@\n\
0 @I5@ INDI\n1 NAME Child3 /C/\n1 SEX M\n1 FAMC @F1@\n1 FAMS @F3@\n\
0 @I6@ INDI\n1 NAME Spouse2 /S/\n1 SEX F\n1 FAMS @F2@\n\
0 @I7@ INDI\n1 NAME Grandchild1 /G/\n1 SEX M\n1 FAMC @F2@\n\
0 @I8@ INDI\n1 NAME Spouse3 /S/\n1 SEX F\n1 FAMS @F3@\n\
0 @I9@ INDI\n1 NAME Grandchild2 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @I10@ INDI\n1 NAME Grandchild3 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @I11@ INDI\n1 NAME Grandchild4 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @I12@ INDI\n1 NAME Grandchild5 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @I13@ INDI\n1 NAME Grandchild6 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @I14@ INDI\n1 NAME Grandchild7 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I3@\n1 CHIL @I4@\n1 CHIL @I5@\n\
0 @F2@ FAM\n1 HUSB @I4@\n1 WIFE @I6@\n1 CHIL @I7@\n\
0 @F3@ FAM\n1 HUSB @I5@\n1 WIFE @I8@\n1 CHIL @I9@\n1 CHIL @I10@\n1 CHIL @I11@\n1 CHIL @I12@\n1 CHIL @I13@\n1 CHIL @I14@\n\
0 TRLR\n";

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.scope.generations = 3;
        prefs.layout.layout_type = "boxed_couples".into();

        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(3));
        let bc = BoxedCouplesLayout.compute(&genrep, &prefs).unwrap();
        let bc = &bc;

        let get_x = |id: &str| match &bc.individuals[id].geo {
            Some(BoxedCouplesGeo::Individual(g)) => g.x,
            _ => panic!("{id} not placed as Individual"),
        };

        let box_w = prefs.layout.boxed_couples.box_width;
        let gap_w = prefs.layout.boxed_couples.gap_width;

        let x_i7 = get_x("I7");
        let x_i9 = get_x("I9");

        let right_edge_i7 = x_i7 + box_w / 2.0;
        let left_edge_i9 = x_i9 - box_w / 2.0;
        assert!(
            right_edge_i7 <= left_edge_i9 - gap_w + 1e-6,
            "compact moved I7 into I9 at depth 1: I7.right={right_edge_i7}, I9.left={left_edge_i9}"
        );
    }

    #[test]
    fn test_compact_no_second_cousin_overlap() {
        use crate::parser::{compute_scope, parse_str};
        use crate::preferences::Prefs;

        // Reproduces the bedarida.ged pattern where bottom-up compact cascades into
        // second-cousin overlap.
        //
        // I1+I2 → [I3, I4]
        // I3+I5 → [I6]              (David subtree — left child)
        // I6+I7 → [I8..I13]         (6 grandchildren, forces I6 far right)
        // I4+I9b → [I14]            (Jacob subtree — right child)
        // I14+I15 → [I16..I21]      (6 grandchildren, forces I14 far right)
        //
        // Top-down compact: Abraham level runs first; right_env(I3)[2]=I13.right,
        // left_env(I4)[2]=I16.left — clearance exactly gap_w — safe_shift=0, no shift.
        // Bottom-up would shift I14's children right first, then over-shift I3's subtree.
        const GED: &str = "\
0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME Abraham /A/\n1 SEX M\n1 FAMS @F1@\n\
0 @I2@ INDI\n1 NAME Sarah /A/\n1 SEX F\n1 FAMS @F1@\n\
0 @I3@ INDI\n1 NAME David /A/\n1 SEX M\n1 FAMC @F1@\n1 FAMS @F2@\n\
0 @I4@ INDI\n1 NAME Jacob /A/\n1 SEX M\n1 FAMC @F1@\n1 FAMS @F3@\n\
0 @I5@ INDI\n1 NAME DavidW /A/\n1 SEX F\n1 FAMS @F2@\n\
0 @I6@ INDI\n1 NAME Abram /A/\n1 SEX M\n1 FAMC @F2@\n1 FAMS @F4@\n\
0 @I7@ INDI\n1 NAME AbramW /A/\n1 SEX F\n1 FAMS @F4@\n\
0 @I8@ INDI\n1 NAME GC1 /A/\n1 SEX M\n1 FAMC @F4@\n\
0 @I9@ INDI\n1 NAME GC2 /A/\n1 SEX M\n1 FAMC @F4@\n\
0 @I10@ INDI\n1 NAME GC3 /A/\n1 SEX M\n1 FAMC @F4@\n\
0 @I11@ INDI\n1 NAME GC4 /A/\n1 SEX M\n1 FAMC @F4@\n\
0 @I12@ INDI\n1 NAME GC5 /A/\n1 SEX M\n1 FAMC @F4@\n\
0 @I13@ INDI\n1 NAME GC6 /A/\n1 SEX M\n1 FAMC @F4@\n\
0 @I9b@ INDI\n1 NAME JacobW /A/\n1 SEX F\n1 FAMS @F3@\n\
0 @I14@ INDI\n1 NAME Samuel /A/\n1 SEX M\n1 FAMC @F3@\n1 FAMS @F5@\n\
0 @I15@ INDI\n1 NAME SamuelW /A/\n1 SEX F\n1 FAMS @F5@\n\
0 @I16@ INDI\n1 NAME GC7 /A/\n1 SEX M\n1 FAMC @F5@\n\
0 @I17@ INDI\n1 NAME GC8 /A/\n1 SEX M\n1 FAMC @F5@\n\
0 @I18@ INDI\n1 NAME GC9 /A/\n1 SEX M\n1 FAMC @F5@\n\
0 @I19@ INDI\n1 NAME GC10 /A/\n1 SEX M\n1 FAMC @F5@\n\
0 @I20@ INDI\n1 NAME GC11 /A/\n1 SEX M\n1 FAMC @F5@\n\
0 @I21@ INDI\n1 NAME GC12 /A/\n1 SEX M\n1 FAMC @F5@\n\
0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I3@\n1 CHIL @I4@\n\
0 @F2@ FAM\n1 HUSB @I3@\n1 WIFE @I5@\n1 CHIL @I6@\n\
0 @F3@ FAM\n1 HUSB @I4@\n1 WIFE @I9b@\n1 CHIL @I14@\n\
0 @F4@ FAM\n1 HUSB @I6@\n1 WIFE @I7@\n1 CHIL @I8@\n1 CHIL @I9@\n1 CHIL @I10@\n1 CHIL @I11@\n1 CHIL @I12@\n1 CHIL @I13@\n\
0 @F5@ FAM\n1 HUSB @I14@\n1 WIFE @I15@\n1 CHIL @I16@\n1 CHIL @I17@\n1 CHIL @I18@\n1 CHIL @I19@\n1 CHIL @I20@\n1 CHIL @I21@\n\
0 TRLR\n";

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.scope.generations = 4;
        prefs.layout.layout_type = "boxed_couples".into();

        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(4));
        let bc = BoxedCouplesLayout.compute(&genrep, &prefs).unwrap();
        let bc = &bc;

        let get_x = |id: &str| match &bc.individuals[id].geo {
            Some(BoxedCouplesGeo::Individual(g)) => g.x,
            _ => panic!("{id} not placed as Individual"),
        };

        let box_w = prefs.layout.boxed_couples.box_width;
        let gap_w = prefs.layout.boxed_couples.gap_width;

        let x_i13 = get_x("I13");
        let x_i16 = get_x("I16");

        let right_edge_i13 = x_i13 + box_w / 2.0;
        let left_edge_i16 = x_i16 - box_w / 2.0;
        assert!(
            right_edge_i13 <= left_edge_i16 - gap_w + 1e-6,
            "second-cousin overlap: I13.right={right_edge_i13}, I16.left={left_edge_i16}"
        );
    }

    // ── Spouse sorting regression test ──

    /// GEDCOM with two spouses out of chronological order.
    /// F10: marriage 1900 (I11), F11: marriage 1850 (I12).
    /// Children of F11 (1850) should appear before children of F10 (1900).
    #[test]
    fn test_spouses_sorted_by_marriage_date() {
        use crate::parser::{compute_scope, parse_str};
        use crate::preferences::Prefs;

        const GED: &str = "\
0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME Root /R/\n1 SEX M\n1 FAMS @F1@\n1 FAMS @F2@\n\
0 @I2@ INDI\n1 NAME LaterSpouse /S/\n1 SEX F\n1 FAMS @F1@\n\
0 @I3@ INDI\n1 NAME EarlierSpouse /S/\n1 SEX F\n1 FAMS @F2@\n\
0 @I4@ INDI\n1 NAME Child1 /C/\n1 SEX M\n1 FAMC @F1@\n\
0 @I5@ INDI\n1 NAME Child2 /C/\n1 SEX M\n1 FAMC @F2@\n\
0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I4@\n1 MARR\n2 DATE 1 JUN 1900\n\
0 @F2@ FAM\n1 HUSB @I1@\n1 WIFE @I3@\n1 CHIL @I5@\n1 MARR\n2 DATE 10 MAR 1850\n\
0 TRLR\n";

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.scope.generations = 2;
        prefs.layout.layout_type = "boxed_couples".into();
        prefs.layout.boxed_couples.box_width = 240.0;
        prefs.layout.boxed_couples.box_height = 140.0;
        prefs.layout.boxed_couples.gap_width = 40.0;
        prefs.layout.boxed_couples.gap_height = 80.0;

        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        let bc = BoxedCouplesLayout.compute(&genrep, &prefs).unwrap();
        let bc = &bc;

        // I5 (child of earlier marriage 1850) should appear before I4 (child of later marriage 1900).
        // In the boxed_couples layout, children are placed left-to-right by order.
        // The earlier spouse's children should be placed to the left.
        // Both children are at generation 2, so same y.
        // The x-coordinate reflects placement order.
        let x_i4 = match &bc.individuals["I4"].geo {
            Some(BoxedCouplesGeo::Individual(g)) => g.x,
            _ => panic!("I4 not placed"),
        };
        let x_i5 = match &bc.individuals["I5"].geo {
            Some(BoxedCouplesGeo::Individual(g)) => g.x,
            _ => panic!("I5 not placed"),
        };

        // I5 (earlier marriage 1850) should be placed to the left of I4 (later marriage 1900)
        assert!(
            x_i5 < x_i4,
            "Child of earlier marriage (I5, 1850) should be left of later marriage child (I4, 1900): \
            I4.x={x_i4}, I5.x={x_i5}"
        );
    }

    // ── 2-spouse compact isolation test ──

    /// Verifies that compact_pass does NOT shift children1 past conn_out1_x when
    /// children2 has a large subtree that would otherwise create a compactable gap.
    ///
    /// I1 (root): 2 spouses — I2 (F1, 1867, earlier) and I3 (F2, 1901, later).
    /// F1 child: I4 (leaf)  → children1, small subtree, placed LEFT.
    /// F2 child: I5 + 6 grandchildren → children2, large subtree, placed RIGHT.
    ///
    /// place_descendants pins I4 at conn_out1_x (= I1.x − 140).
    /// Without the fix, compact_siblings([I4, I5]) shifts I4 rightward by ~420 units
    /// to close the gap before I5, breaking the connector invariant.
    #[test]
    fn test_two_spouse_compact_isolates_groups() {
        use crate::parser::{compute_scope, parse_str};
        use crate::preferences::Prefs;

        const GED: &str = "\
0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME Root /R/\n1 SEX M\n1 FAMS @F1@\n1 FAMS @F2@\n\
0 @I2@ INDI\n1 NAME EarlierSpouse /S/\n1 SEX F\n1 FAMS @F1@\n\
0 @I3@ INDI\n1 NAME LaterSpouse /S/\n1 SEX F\n1 FAMS @F2@\n\
0 @I4@ INDI\n1 NAME Child1 /C/\n1 SEX M\n1 FAMC @F1@\n\
0 @I5@ INDI\n1 NAME Child2 /C/\n1 SEX M\n1 FAMC @F2@\n1 FAMS @F3@\n\
0 @I6@ INDI\n1 NAME Spouse2 /S/\n1 SEX F\n1 FAMS @F3@\n\
0 @I7@  INDI\n1 NAME GC1 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @I8@  INDI\n1 NAME GC2 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @I9@  INDI\n1 NAME GC3 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @I10@ INDI\n1 NAME GC4 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @I11@ INDI\n1 NAME GC5 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @I12@ INDI\n1 NAME GC6 /G/\n1 SEX M\n1 FAMC @F3@\n\
0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I4@\n1 MARR\n2 DATE 7 Jul 1867\n\
0 @F2@ FAM\n1 HUSB @I1@\n1 WIFE @I3@\n1 CHIL @I5@\n1 MARR\n2 DATE 30 Nov 1901\n\
0 @F3@ FAM\n1 HUSB @I5@\n1 WIFE @I6@\
\n1 CHIL @I7@\n1 CHIL @I8@\n1 CHIL @I9@\n1 CHIL @I10@\n1 CHIL @I11@\n1 CHIL @I12@\n\
0 TRLR\n";

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.scope.generations = 3;
        prefs.layout.layout_type = "boxed_couples".into();
        prefs.layout.boxed_couples.box_width = 240.0;
        prefs.layout.boxed_couples.box_height = 140.0;
        prefs.layout.boxed_couples.gap_width = 40.0;
        prefs.layout.boxed_couples.gap_height = 80.0;
        prefs.layout.boxed_couples.box_width_2_spouses = 520.0;

        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(3));
        let bc = BoxedCouplesLayout.compute(&genrep, &prefs).unwrap();
        let bc = &bc;

        let get_x = |id: &str| match &bc.individuals[id].geo {
            Some(BoxedCouplesGeo::Individual(g)) => g.x,
            _ => panic!("{id} not placed as Individual"),
        };

        let x_i1 = get_x("I1");
        let x_i4 = get_x("I4");
        let x_i5 = get_x("I5");

        let box_w = prefs.layout.boxed_couples.box_width;
        let box_w2 = prefs.layout.boxed_couples.box_width_2_spouses;
        let conn_offset = box_w2 / 2.0 - box_w / 2.0; // 140.0

        // I4 (children1.last()) must remain pinned at conn_out1_x = I1.x − 140.
        // Without the fix compact_siblings shifts I4 rightward by ~420 units.
        assert!(
            (x_i4 - (x_i1 - conn_offset)).abs() < 1e-6,
            "I4 should be pinned at conn_out1_x (I1.x - {conn_offset}): \
            I1.x={x_i1}, expected I4.x={}, got I4.x={x_i4}",
            x_i1 - conn_offset
        );

        // I5 (children2.first()) must be at or right of conn_out2_x = I1.x + 140.
        assert!(
            x_i5 >= x_i1 + conn_offset - 1e-6,
            "I5 should be at or right of conn_out2_x (I1.x + {conn_offset}): \
            I1.x={x_i1}, conn_out2_x={}, I5.x={x_i5}",
            x_i1 + conn_offset
        );
    }

    /// Load a large fixture, run the boxed_couples layout for
    /// several box_width values, and assert that no two boxes at the same
    /// generation (same y) overlap in x.
    ///
    /// This is a regression test for the recenter_pass sibling-overshoot bug
    /// where I514 was pushed past I515 when I514's child (I348) had a deep
    /// two-spouse subtree.
    #[test]
    fn no_overlap_real_tree() {
        use crate::parser::{compute_scope, parse};
        use std::collections::HashMap;

        let ged_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fixture_large.ged");

        // box_width values to test; for each: box_width_2_spouses = 2*w + gap_w
        let gap_w = 40.0;
        let box_widths: &[f64] = &[80.0, 140.0, 180.0, 240.0, 500.0];

        for &box_w in box_widths {
            let box_w2 = 2.0 * box_w + gap_w;

            let mut prefs = Prefs::default();
            prefs.scope.root = "I506".into();
            prefs.scope.direction = "descendants".into();
            prefs.scope.generations = 99;
            prefs.layout.layout_type = "boxed_couples".into();
            prefs.layout.boxed_couples.box_width = box_w;
            prefs.layout.boxed_couples.box_height = 140.0;
            prefs.layout.boxed_couples.gap_width = gap_w;
            prefs.layout.boxed_couples.gap_height = 80.0;
            prefs.layout.boxed_couples.box_width_2_spouses = box_w2;

            let mut genrep = parse(&ged_path, &crate::plugins::PluginEngine::disabled())
                .expect("could not parse fixture_large.ged");
            compute_scope(&mut genrep, Some("I506"), "descendants", Some(99));

            let bc = BoxedCouplesLayout
                .compute(&genrep, &prefs)
                .unwrap_or_else(|e| panic!("layout failed at box_width={box_w}: {e}"));

            // Group placed individuals by their y (generation row).
            // key = y rounded to nearest integer (all same-gen boxes share y exactly)
            let mut by_y: HashMap<i64, Vec<(String, f64, f64)>> = HashMap::new();
            for (id, ind) in &bc.individuals {
                if let Some(BoxedCouplesGeo::Individual(g)) = &ind.geo {
                    let y_key = g.y.round() as i64;
                    by_y.entry(y_key)
                        .or_default()
                        .push((id.clone(), g.x, g.width));
                }
            }

            for (_, mut row) in by_y {
                row.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                for w in row.windows(2) {
                    let (ref id_l, cx_l, w_l) = w[0];
                    let (ref id_r, cx_r, w_r) = w[1];
                    let right_edge = cx_l + w_l / 2.0;
                    let left_edge = cx_r - w_r / 2.0;
                    assert!(
                        right_edge <= left_edge - gap_w + 1e-4,
                        "box_width={box_w}: {id_l} right={right_edge:.1} overlaps \
                         {id_r} left={left_edge:.1} (required gap={gap_w})"
                    );
                }
            }
        }
    }

    /// With 3-spouse support, all three spouses are shown in a triple-wide box.
    /// I1 has 3 spouses: I2 (no children), I3 (no children), I4 (child I5).
    /// The box should use box_width_3_spouses and show all three spouses.
    #[test]
    fn test_three_spouses_all_shown_in_scene() {
        use crate::parser::{compute_scope, parse_str};
        use crate::scene::Primitive;

        const GED: &str = "\
0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME Root /R/\n1 SEX M\n1 FAMS @F1@\n1 FAMS @F2@\n1 FAMS @F3@\n\
0 @I2@ INDI\n1 NAME Spouse1 /S/\n1 SEX F\n1 FAMS @F1@\n\
0 @I3@ INDI\n1 NAME Spouse2 /S/\n1 SEX F\n1 FAMS @F2@\n\
0 @I4@ INDI\n1 NAME Spouse3 /S/\n1 SEX F\n1 FAMS @F3@\n\
0 @I5@ INDI\n1 NAME Child1 /C/\n1 SEX M\n1 FAMC @F3@\n\
0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n\
0 @F2@ FAM\n1 HUSB @I1@\n1 WIFE @I3@\n\
0 @F3@ FAM\n1 HUSB @I1@\n1 WIFE @I4@\n1 CHIL @I5@\n\
0 TRLR\n";

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.scope.generations = 2;
        prefs.layout.layout_type = "boxed_couples".into();

        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        let bc = BoxedCouplesLayout.compute(&genrep, &prefs).unwrap();

        // I1 must have a triple-wide box
        let i1_geo = ind_geo(&bc, "I1");
        assert!(
            (i1_geo.width - prefs.layout.boxed_couples.box_width_3_spouses).abs() < 1e-6,
            "I1 should have a 3-spouse box width; got {}",
            i1_geo.width
        );

        let scene = emit_scene(&bc, &prefs);

        fn collect_ids(prims: &[Primitive], ids: &mut Vec<String>) {
            for p in prims {
                if let Primitive::Group(g) = p {
                    if !g.id.is_empty() {
                        ids.push(g.id.clone());
                    }
                    collect_ids(&g.children, ids);
                }
            }
        }
        let mut ids = Vec::new();
        collect_ids(&scene.primitives, &mut ids);

        assert!(
            ids.contains(&"I2-name".to_string()),
            "I2 (sp1, left) must appear; found: {ids:?}"
        );
        assert!(
            ids.contains(&"I3-name".to_string()),
            "I3 (sp2, center) must appear; found: {ids:?}"
        );
        assert!(
            ids.contains(&"I4-name".to_string()),
            "I4 (sp3, right) must appear; found: {ids:?}"
        );
    }

    /// 3-spouse layout placement: no sibling overlap, correct box width.
    #[test]
    fn test_three_spouse_placement() {
        use crate::parser::{compute_scope, parse_str};

        const GED: &str = "\
0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME Root /R/\n1 SEX M\n1 FAMS @F1@\n1 FAMS @F2@\n1 FAMS @F3@\n\
0 @I2@ INDI\n1 NAME Sp1 /S/\n1 SEX F\n1 FAMS @F1@\n\
0 @I3@ INDI\n1 NAME Sp2 /S/\n1 SEX F\n1 FAMS @F2@\n\
0 @I4@ INDI\n1 NAME Sp3 /S/\n1 SEX F\n1 FAMS @F3@\n\
0 @I5@ INDI\n1 NAME C1a /C/\n1 SEX M\n1 FAMC @F1@\n\
0 @I6@ INDI\n1 NAME C1b /C/\n1 SEX M\n1 FAMC @F1@\n\
0 @I7@ INDI\n1 NAME C2a /C/\n1 SEX M\n1 FAMC @F2@\n\
0 @I8@ INDI\n1 NAME C2b /C/\n1 SEX M\n1 FAMC @F2@\n\
0 @I9@ INDI\n1 NAME C3a /C/\n1 SEX M\n1 FAMC @F3@\n\
0 @I10@ INDI\n1 NAME C3b /C/\n1 SEX M\n1 FAMC @F3@\n\
0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I5@\n1 CHIL @I6@\n\
0 @F2@ FAM\n1 HUSB @I1@\n1 WIFE @I3@\n1 CHIL @I7@\n1 CHIL @I8@\n\
0 @F3@ FAM\n1 HUSB @I1@\n1 WIFE @I4@\n1 CHIL @I9@\n1 CHIL @I10@\n\
0 TRLR\n";

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.scope.generations = 2;
        prefs.layout.layout_type = "boxed_couples".into();

        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        let bc = BoxedCouplesLayout.compute(&genrep, &prefs).unwrap();

        // I1 must have the 3-spouse box width
        let i1_geo = ind_geo(&bc, "I1");
        assert!(
            (i1_geo.width - prefs.layout.boxed_couples.box_width_3_spouses).abs() < 1e-6,
            "I1 width should be box_width_3_spouses; got {}",
            i1_geo.width
        );

        // No sibling overlap at generation 1
        let box_w = prefs.layout.boxed_couples.box_width;
        let gap_w = prefs.layout.boxed_couples.gap_width;
        let mut xs: Vec<f64> = ["I5", "I6", "I7", "I8", "I9", "I10"]
            .iter()
            .map(|id| ind_geo(&bc, id).x)
            .collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in xs.windows(2) {
            assert!(
                pair[1] - pair[0] >= box_w + gap_w - 1e-6,
                "children overlap: gap = {}",
                pair[1] - pair[0]
            );
        }
    }

    /// 2-channel bar_y_fraction: when all 3 spouses have ≥2 children, sp2 gets 2/3
    /// and sp1/sp3 get 1/3; otherwise all get 0.5.
    #[test]
    fn test_three_spouse_bar_y_fraction() {
        use crate::parser::{compute_scope, parse_str};
        use crate::scene::Primitive;

        const GED: &str = "\
0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME Root /R/\n1 SEX M\n1 FAMS @F1@\n1 FAMS @F2@\n1 FAMS @F3@\n\
0 @I2@ INDI\n1 NAME Sp1 /S/\n1 SEX F\n1 FAMS @F1@\n\
0 @I3@ INDI\n1 NAME Sp2 /S/\n1 SEX F\n1 FAMS @F2@\n\
0 @I4@ INDI\n1 NAME Sp3 /S/\n1 SEX F\n1 FAMS @F3@\n\
0 @I5@ INDI\n1 NAME C1a /C/\n1 SEX M\n1 FAMC @F1@\n\
0 @I6@ INDI\n1 NAME C1b /C/\n1 SEX M\n1 FAMC @F1@\n\
0 @I7@ INDI\n1 NAME C2a /C/\n1 SEX M\n1 FAMC @F2@\n\
0 @I8@ INDI\n1 NAME C2b /C/\n1 SEX M\n1 FAMC @F2@\n\
0 @I9@ INDI\n1 NAME C3a /C/\n1 SEX M\n1 FAMC @F3@\n\
0 @I10@ INDI\n1 NAME C3b /C/\n1 SEX M\n1 FAMC @F3@\n\
0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I5@\n1 CHIL @I6@\n\
0 @F2@ FAM\n1 HUSB @I1@\n1 WIFE @I3@\n1 CHIL @I7@\n1 CHIL @I8@\n\
0 @F3@ FAM\n1 HUSB @I1@\n1 WIFE @I4@\n1 CHIL @I9@\n1 CHIL @I10@\n\
0 TRLR\n";

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.scope.generations = 2;
        prefs.layout.layout_type = "boxed_couples".into();

        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        let bc = BoxedCouplesLayout.compute(&genrep, &prefs).unwrap();
        let scene = emit_scene(&bc, &prefs);

        fn collect_connectors(prims: &[Primitive], conns: &mut Vec<f64>) {
            for p in prims {
                match p {
                    Primitive::Connector(c) => conns.push(c.bar_y_fraction),
                    Primitive::Group(g) => collect_connectors(&g.children, conns),
                    _ => {}
                }
            }
        }
        let mut fractions = Vec::new();
        collect_connectors(&scene.primitives, &mut fractions);

        // Should have 3 connectors (one per family of I1)
        assert_eq!(
            fractions.len(),
            3,
            "expected 3 connectors, got {fractions:?}"
        );

        // With all spouses having ≥2 children: 2 outer at 1/3, 1 middle at 2/3
        let lower = 1.0_f64 / 3.0;
        let upper = 2.0_f64 / 3.0;
        let lower_count = fractions
            .iter()
            .filter(|&&f| (f - lower).abs() < 1e-6)
            .count();
        let upper_count = fractions
            .iter()
            .filter(|&&f| (f - upper).abs() < 1e-6)
            .count();
        assert_eq!(
            lower_count, 2,
            "expected 2 connectors at 1/3 (sp1+sp3); fractions: {fractions:?}"
        );
        assert_eq!(
            upper_count, 1,
            "expected 1 connector at 2/3 (sp2); fractions: {fractions:?}"
        );
    }
}
