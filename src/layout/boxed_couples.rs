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
use super::common::{copy_families, copy_individual, resolve_root_id, sort_families_by_date};
use crate::parser::genrep::{Genrep, Individual};
use crate::preferences::Prefs;
use crate::util::matches_direction;
use anyhow::Result;
use std::collections::HashMap;

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

/// Returns the IDs of all in-scope spouses of `ind_id`, sorted by marriage date.
fn spouses_of(ind_id: &str, genrep: &Genrep) -> Vec<String> {
    let ind = match genrep.get_individual(ind_id) {
        Some(i) => i,
        None => return vec![],
    };
    let sorted_fams = sort_families_by_date(ind, genrep);
    sorted_fams
        .iter()
        .filter_map(|fam_id| genrep.get_family(fam_id))
        .filter(|fam| fam.in_scope)
        .filter_map(|fam| {
            if fam.husband_id.as_deref() == Some(ind_id) {
                fam.wife_id.clone()
            } else {
                fam.husband_id.clone()
            }
        })
        .filter(|sp| genrep.get_individual(sp).is_some_and(|i| i.in_scope))
        .collect()
}

/// Returns the IDs of in-scope children born to the pairing of `ind_id` and `spouse_id`.
fn children_with_spouse(ind_id: &str, spouse_id: &str, genrep: &Genrep) -> Vec<String> {
    let ind = match genrep.get_individual(ind_id) {
        Some(i) => i,
        None => return vec![],
    };
    ind.fams
        .iter()
        .filter_map(|fam_id| genrep.get_family(fam_id))
        .filter(|fam| {
            fam.husband_id.as_deref() == Some(spouse_id)
                || fam.wife_id.as_deref() == Some(spouse_id)
        })
        .flat_map(|fam| fam.children_ids.iter().cloned())
        .filter(|cid| genrep.get_individual(cid).is_some_and(|c| c.in_scope))
        .collect()
}

/// Returns at most 2 in-scope spouses, preferring those with children.
///
/// The layout can represent at most 2 spouses (a 1-spouse box or a wide
/// 2-spouse box).  When more exist, spouses without children are dropped from
/// the end of the list first until at most 2 remain.
fn prune_spouses(ind_id: &str, genrep: &Genrep) -> Vec<String> {
    let mut spouses = spouses_of(ind_id, genrep);
    if spouses.len() > 2 {
        eprintln!(
            "warning: {} has {} spouses; only 2 can be represented in boxed_couples layout",
            ind_id,
            spouses.len()
        );
        let mut i = spouses.len();
        while i > 0 && spouses.len() > 2 {
            i -= 1;
            if children_with_spouse(ind_id, &spouses[i].clone(), genrep).is_empty() {
                spouses.remove(i);
            }
        }
        spouses.truncate(2);
    }
    spouses
}

/// Extends `env` to `min_len` by filling missing slots from `global_right`.
///
/// `env[j]` is the minimum x right-edge that must be cleared by any box at
/// absolute generation `base_gen + j`.  When a previous sibling is a leaf its
/// right-envelope has length 1 (only its own right-edge), leaving the deeper
/// slots undefined.  Filling from `global_right[base_gen + j]` — the
/// rightmost right-edge already placed at that generation across the whole
/// traversal — gives the correct tight constraint without over-constraining by
/// propagating a shallower boundary downward.
fn fill_env_from_global(
    mut env: Vec<f64>,
    min_len: usize,
    global_right: &[f64],
    base_gen: usize,
) -> Vec<f64> {
    for j in env.len()..min_len {
        env.push(global_right.get(base_gen + j).copied().unwrap_or(0.0));
    }
    env
}

/// Merges two right-edge envelopes by taking the maximum x-coordinate at each depth level.
///
/// The resulting vector's length is the maximum of the two input lengths. If one vector
/// is shorter than the other, its missing values are treated as `f64::NEG_INFINITY`,
/// ensuring the merged contour accounts for the full depth and width of both subtrees.
fn merge_max(a: Vec<f64>, b: Vec<f64>) -> Vec<f64> {
    let new_len = a.len().max(b.len());
    let mut res = Vec::with_capacity(new_len);
    for i in 0..new_len {
        let val_a = a.get(i).copied().unwrap_or(f64::NEG_INFINITY);
        let val_b = b.get(i).copied().unwrap_or(f64::NEG_INFINITY);
        res.push(val_a.max(val_b));
    }
    res
}

/// Merges two left-edge envelopes by taking the minimum at each depth level.
fn merge_min(a: Vec<f64>, b: Vec<f64>) -> Vec<f64> {
    let new_len = a.len().max(b.len());
    let mut res = Vec::with_capacity(new_len);
    for i in 0..new_len {
        let val_a = a.get(i).copied().unwrap_or(f64::INFINITY);
        let val_b = b.get(i).copied().unwrap_or(f64::INFINITY);
        res.push(val_a.min(val_b));
    }
    res
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

    let has_spouse2 = geo.width > box_w + 1.0;

    let conn_out_y = geo.y + box_h / 2.0;
    let (conn_out1_x, conn_out2_x) = if has_spouse2 {
        (
            geo.x - (box_w2 / 2.0 - box_w / 2.0),
            geo.x + (box_w2 / 2.0 - box_w / 2.0),
        )
    } else {
        (geo.x, geo.x)
    };

    Some(BoxedCouplesGeo::Family(FamilyGeo {
        conn_out1_x,
        conn_out1_y: conn_out_y,
        conn_out2_x,
        conn_out2_y: conn_out_y,
        has_spouse2,
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
            g.x += dx;
            g.conn_in_x += dx;
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

    if spouses.len() >= 2 {
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
fn recenter_pass(
    ind_id: &str,
    genrep: &Genrep,
    box_w: f64,
    box_w2: f64,
    out: &mut HashMap<String, Individual<BoxedCouplesGeo>>,
) {
    // Use spouses_of (already scope-filtered, sorted by date) and take at most 2.
    // prune_spouses would give the same result for ≤2 spouses; any >2-spouse
    // warning was already emitted during place_descendants.
    let spouses: Vec<String> = spouses_of(ind_id, genrep).into_iter().take(2).collect();

    if spouses.len() >= 2 {
        let children1: Vec<String> = children_with_spouse(ind_id, &spouses[0], genrep)
            .into_iter()
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();
        let children2: Vec<String> = children_with_spouse(ind_id, &spouses[1], genrep)
            .into_iter()
            .filter(|cid| out.contains_key(cid.as_str()))
            .collect();

        for child_id in children1.iter().chain(children2.iter()) {
            recenter_pass(child_id, genrep, box_w, box_w2, out);
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

        if let Some(ind) = out.get_mut(ind_id) {
            if let Some(BoxedCouplesGeo::Individual(g)) = &mut ind.geo {
                g.x = new_x;
                g.conn_in_x = new_x;
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

        for child_id in &all_children {
            recenter_pass(child_id, genrep, box_w, box_w2, out);
        }

        let n = all_children.len();
        let new_x = if n % 2 == 1 {
            get_x_of(&all_children[n / 2], out)
        } else {
            (get_x_of(&all_children[n / 2 - 1], out) + get_x_of(&all_children[n / 2], out)) / 2.0
        };

        if let Some(ind) = out.get_mut(ind_id) {
            if let Some(BoxedCouplesGeo::Individual(g)) = &mut ind.geo {
                g.x = new_x;
                g.conn_in_x = new_x;
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
    let width = if spouses.len() >= 2 { box_w2 } else { box_w };
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
                    for child_id in &children {
                        shift_subtree(child_id, shift, generation + 1, genrep, out, global_right);
                    }
                    x_default
                } else {
                    x_mid
                }
            }
        }

        _ => {
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
    if (generation as usize) < global_right.len() {
        let right_edge = x + width / 2.0;
        global_right[generation as usize] = global_right[generation as usize].max(right_edge);
    }
}

/// Stub for ancestor-direction layout; currently delegates to [`place_descendants`].
///
/// A true ancestors traversal would walk `famc` links and place parents above the child.
#[allow(clippy::too_many_arguments)]
fn place_ancestors(
    genrep: &Genrep,
    ind_id: &str,
    env_left: &[f64],
    generation: u32,
    box_w: f64,
    box_h: f64,
    box_w2: f64,
    gap_w: f64,
    gap_h: f64,
    out: &mut HashMap<String, Individual<BoxedCouplesGeo>>,
    global_right: &mut Vec<f64>,
) {
    // TODO: implement true ancestors traversal (walk famc, place parents above the child)
    place_descendants(
        genrep,
        ind_id,
        env_left,
        generation,
        box_w,
        box_h,
        box_w2,
        gap_w,
        gap_h,
        out,
        global_right,
    );
}

pub struct BoxedCouplesLayout;

impl Layout for BoxedCouplesLayout {
    type Geo = BoxedCouplesGeo;

    fn compute(&self, genrep: &Genrep, prefs: &Prefs) -> Result<Genrep<BoxedCouplesGeo>> {
        let dir = prefs.scope.direction.to_lowercase();

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

        if matches_direction(&dir, "ancestors") {
            place_ancestors(
                genrep,
                root_id,
                &env_left,
                0,
                box_w,
                box_h,
                box_w2,
                gap_w,
                gap_h,
                &mut individuals,
                &mut global_right,
            );
        } else {
            place_descendants(
                genrep,
                root_id,
                &env_left,
                0,
                box_w,
                box_h,
                box_w2,
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
            recenter_pass(root_id, genrep, box_w, box_w2, &mut individuals);
        }

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
            build_family_geo(fam, &individuals, box_h, box_w, box_w2)
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
    use crate::format::{format_event, format_name};
    use crate::scene::{
        BoxPrimitive, ConnectorPrimitive, GroupPrimitive, Point, Primitive, Rect, Scene, TextAlign,
        TextAttr, TextPrimitive,
    };
    // ── 4a: load highlights ──────────────────────────────────────────────────
    // ── 4a: load highlights ──────────────────────────────────────────────────
    let highlighted_ids = crate::layout::common::highlight_set(prefs);
    // ── 4b: collect placed individuals ──────────────────────────────────────
    let placed: Vec<(&str, &IndividualGeo)> = genrep
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
        box_children.push(Primitive::Box(BoxPrimitive { bbox: box_bbox }));

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

        let sorted_fam_ids = sort_families_by_date(ind, genrep);
        let spouses: Vec<(&String, &crate::parser::genrep::Family<BoxedCouplesGeo>)> =
            sorted_fam_ids
                .iter()
                .filter_map(|fid| genrep.families.get(fid).map(|f| (fid, f)))
                .filter(|(_, f)| f.in_scope)
                .collect();
        let is_two_spouse = geo.width > bc.box_width + 1.0;

        if is_two_spouse {
            let left_cx = to_display_x(geo.x - (bc.box_width_2_spouses / 2.0 - bc.box_width / 2.0));
            let right_cx =
                to_display_x(geo.x + (bc.box_width_2_spouses / 2.0 - bc.box_width / 2.0));
            let ind_cx = to_display_x(geo.x);
            let box_display_left = to_display_x(geo.x - geo.width / 2.0);

            // Individual name (centered in wide box) — wrapped in name sub-group
            let name_baseline = ind_section_top + spacing.name_above + font_size;
            let name_bbox = Rect {
                x: box_display_left,
                y: name_baseline - font_size,
                w: geo.width,
                h: font_size,
            };
            // Override: center on ind_cx (use full width bbox but centered on ind_cx)
            let name_bbox = Rect {
                x: ind_cx - geo.width / 2.0,
                ..name_bbox
            };
            box_children.push(Primitive::Group(GroupPrimitive {
                id: format!("{ind_id_trimmed}-name"),
                children: vec![Primitive::Text(TextPrimitive {
                    content: format_name(ind, prefs),
                    bbox: name_bbox,
                    align: TextAlign::Center,
                    attrs: crate::scene::label_attrs(TextAttr::IndividualName, is_highlighted),
                })],
            }));

            if prefs.show.id {
                box_children.push(Primitive::Text(TextPrimitive {
                    content: ind_id_trimmed.clone(),
                    bbox: Rect {
                        x: box_display_left + 2.0,
                        y: name_baseline - font_size,
                        w: geo.width,
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
                box_children.push(Primitive::Text(TextPrimitive {
                    content: birth_content,
                    bbox: Rect {
                        x: ind_cx - geo.width / 2.0,
                        y: y_pos - date_font_size,
                        w: geo.width,
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
                box_children.push(Primitive::Text(TextPrimitive {
                    content: death_content,
                    bbox: Rect {
                        x: ind_cx - geo.width / 2.0,
                        y: y_pos - date_font_size,
                        w: geo.width,
                        h: date_font_size,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::DeathData],
                }));
            }

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

            // Individual name — wrapped in name sub-group
            box_children.push(Primitive::Group(GroupPrimitive {
                id: format!("{ind_id_trimmed}-name"),
                children: vec![Primitive::Text(TextPrimitive {
                    content: format_name(ind, prefs),
                    bbox: Rect {
                        x: section_cx - geo.width / 2.0,
                        y: name_baseline - font_size,
                        w: geo.width,
                        h: font_size,
                    },
                    align: TextAlign::Center,
                    attrs: crate::scene::label_attrs(TextAttr::IndividualName, is_highlighted),
                })],
            }));
            if prefs.show.id {
                box_children.push(Primitive::Text(TextPrimitive {
                    content: ind_id_trimmed.clone(),
                    bbox: Rect {
                        x: box_display_left + 2.0,
                        y: name_baseline - font_size,
                        w: geo.width,
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
                box_children.push(Primitive::Text(TextPrimitive {
                    content: birth_content,
                    bbox: Rect {
                        x: section_cx - geo.width / 2.0,
                        y: y_pos - date_font_size,
                        w: geo.width,
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
                box_children.push(Primitive::Text(TextPrimitive {
                    content: death_content,
                    bbox: Rect {
                        x: section_cx - geo.width / 2.0,
                        y: y_pos - date_font_size,
                        w: geo.width,
                        h: date_font_size,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::DeathData],
                }));
            }

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

    for (fam_id, fam) in &genrep.families {
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

        // Determine which conn_out_x to use based on family index
        let sorted_fams = sort_families_by_date(parent_ind, genrep);
        let fam_index = sorted_fams.iter().position(|f| f == fam_id).unwrap_or(0);
        let conn_out_x = if fam_index == 0 || !fam_geo.has_spouse2 {
            fam_geo.conn_out1_x
        } else {
            fam_geo.conn_out2_x
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

    let mut primitives = box_groups;
    primitives.extend(connector_groups);

    Scene {
        primitives,
        canvas_bounds,
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
    fam_id: &String,
    prefs: &Prefs,
    section_width: f64,
    font_size: f64,
    date_font_size: f64,
    spacing: &crate::preferences::BoxedCouplesSpacingPrefs,
    is_highlighted: bool,
) -> Vec<crate::scene::Primitive> {
    use crate::format::{format_event, format_name};
    use crate::scene::{GroupPrimitive, Primitive, Rect, TextAlign, TextAttr, TextPrimitive};
    let mut result: Vec<Primitive> = Vec::new();

    let fam_id_trimmed = fam_id
        .trim_start_matches('@')
        .trim_end_matches('@')
        .to_string();

    // Marriage data — wrapped in a sub-group so SVG editors see symbol + text as one unit
    if prefs.show.marriage {
        if let Some(marr) = &fam.marriage {
            if let Some(s) = format_event(
                &prefs.format.marriage,
                marr.date.as_ref(),
                marr.place.as_deref(),
                &prefs.format.date_qualifiers,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                jmar: None,
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
                jmar: None,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                jmar: None,
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
                jmar: None,
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
                jmar: None,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
                jmar: None,
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
                jmar: None,
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
    fn prune_spouses_keeps_two_with_children() {
        let genrep = three_spouse_genrep();
        let pruned = prune_spouses("I10", &genrep);
        assert_eq!(pruned.len(), 2);
        assert!(pruned.contains(&"I11".to_string()));
        assert!(pruned.contains(&"I12".to_string()));
        assert!(!pruned.contains(&"I13".to_string()));
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
                name_heb: None,
                living: None,
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
                name_heb: None,
                living: None,
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
            jmar: None,
            in_scope: true,
            geo: None,
        };
        let result = build_family_geo(&fam, &out, 160.0, 220.0, 480.0);
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
}
