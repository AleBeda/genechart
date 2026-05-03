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
//!    the natural centre falls left of the `x_default` constraint, shift the
//!    entire child subtree rightward rather than clamping the parent.

use anyhow::Result;
use crate::parser::genrep::{Genrep, Individual, Family};
use crate::preferences::Prefs;
use super::Layout;
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
    pub conn_in_y: f64,
}

/// Layout geometry for the outgoing connectors of a placed family (parent → children).
#[derive(Debug, Clone)]
pub struct FamilyGeo {
    /// x of the outgoing connector for the first spouse's children.
    /// For a 1-spouse box this equals the box centre; for a 2-spouse box it
    /// is offset left to the centre of the first spouse's column.
    pub conn_out1_x: f64,
    /// y of both outgoing connectors (bottom edge of the parent box).
    pub conn_out1_y: f64,
    /// x of the outgoing connector for the second spouse's children (right column).
    pub conn_out2_x: f64,
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

fn matches_direction(input: &str, canonical: &str) -> bool {
    !input.is_empty() && canonical.starts_with(input)
}

/// Returns the IDs of all in-scope spouses of `ind_id`, in FAMS order.
fn spouses_of(ind_id: &str, genrep: &Genrep) -> Vec<String> {
    let ind = match genrep.get_individual(ind_id) {
        Some(i) => i,
        None => return vec![],
    };
    ind.fams.iter()
        .filter_map(|fam_id| genrep.get_family(fam_id))
        .filter(|fam| fam.in_scope)
        .filter_map(|fam| {
            if fam.husband_id.as_deref() == Some(ind_id) {
                fam.wife_id.clone()
            } else {
                fam.husband_id.clone()
            }
        })
        .filter(|sp| genrep.get_individual(sp).map_or(false, |i| i.in_scope))
        .collect()
}

/// Returns the IDs of in-scope children born to the pairing of `ind_id` and `spouse_id`.
fn children_with_spouse(ind_id: &str, spouse_id: &str, genrep: &Genrep) -> Vec<String> {
    let ind = match genrep.get_individual(ind_id) {
        Some(i) => i,
        None => return vec![],
    };
    ind.fams.iter()
        .filter_map(|fam_id| genrep.get_family(fam_id))
        .filter(|fam| {
            fam.husband_id.as_deref() == Some(spouse_id)
                || fam.wife_id.as_deref() == Some(spouse_id)
        })
        .flat_map(|fam| fam.children_ids.iter().cloned())
        .filter(|cid| genrep.get_individual(cid).map_or(false, |c| c.in_scope))
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

/// Returns the right-side envelope of `ind`'s placed subtree.
///
/// `result[0]` = right edge of `ind` itself (`x + width/2`).
/// `result[k]` = right edge of the rightmost placed box k levels below `ind`.
///
/// Passing `get_right_envelope(prev_sibling)` as `env_left` to the next
/// sibling ensures the next sibling and its entire subtree clear the previous
/// sibling's entire subtree at every generation.
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
    let rightmost_child = spouses.iter()
        .flat_map(|sp| children_with_spouse(ind_id, sp, genrep))
        .filter(|cid| out.contains_key(cid.as_str()))
        .last();

    if let Some(child_id) = rightmost_child {
        result.extend(get_right_envelope(&child_id, genrep, out));
    }
    result
}

/// Returns the left-side envelope of `ind`'s placed subtree.
///
/// `result[0]` = left edge of `ind` itself (`x - width/2`).
/// `result[k]` = left edge of the leftmost placed box k levels below `ind`.
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
    let leftmost_child = spouses
        .iter()
        .flat_map(|sp| children_with_spouse(ind_id, sp, genrep))
        .filter(|cid| out.contains_key(cid.as_str()))
        .next();

    if let Some(child_id) = leftmost_child {
        result.extend(get_left_envelope(&child_id, genrep, out));
    }
    result
}

/// Converts an `Individual<()>` into `Individual<BoxedCouplesGeo>` with the supplied geo.
fn copy_individual(
    src: &Individual<()>,
    geo: Option<BoxedCouplesGeo>,
) -> Individual<BoxedCouplesGeo> {
    Individual {
        id: src.id.clone(),
        given: src.given.clone(),
        surname: src.surname.clone(),
        sex: src.sex,
        birth: src.birth.clone(),
        death: src.death.clone(),
        fams: src.fams.clone(),
        famc: src.famc.clone(),
        alt_name: src.alt_name.clone(),
        name_heb: src.name_heb.clone(),
        living: src.living,
        in_scope: src.in_scope,
        geo,
    }
}

/// Builds the complete family map by calling [`build_family_geo`] for every family.
fn copy_families(
    genrep: &Genrep,
    out: &HashMap<String, Individual<BoxedCouplesGeo>>,
    box_h: f64,
    box_w: f64,
    box_w2: f64,
) -> HashMap<String, Family<BoxedCouplesGeo>> {
    genrep.families.iter().map(|(id, fam)| {
        let geo = build_family_geo(fam, out, box_h, box_w, box_w2);
        (id.clone(), Family {
            id: fam.id.clone(),
            husband_id: fam.husband_id.clone(),
            wife_id: fam.wife_id.clone(),
            children_ids: fam.children_ids.clone(),
            marriage: fam.marriage.clone(),
            jmar: fam.jmar.clone(),
            in_scope: fam.in_scope,
            geo,
        })
    }).collect()
}

/// Derives connector geometry for one family from its placed parent's [`IndividualGeo`].
///
/// Returns `None` if neither spouse has been placed (e.g. an out-of-scope family).
/// The `conn_out` x-values are offset left/right by half the wide-box difference
/// when the parent has two in-scope spouses.
fn build_family_geo(
    fam: &Family<()>,
    out: &HashMap<String, Individual<BoxedCouplesGeo>>,
    box_h: f64,
    box_w: f64,
    box_w2: f64,
) -> Option<BoxedCouplesGeo> {
    let is_placed = |id: &&str| matches!(
        out.get(*id).and_then(|i| i.geo.as_ref()),
        Some(BoxedCouplesGeo::Individual(_))
    );
    let parent_id = fam.husband_id.as_deref()
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
        (geo.x - (box_w2 / 2.0 - box_w / 2.0), geo.x + (box_w2 / 2.0 - box_w / 2.0))
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
/// right so the centring invariant is preserved.  `global_right` must be updated
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
        let right_env = get_right_envelope(&children[i], genrep, out);
        let left_env  = get_left_envelope(&children[i + 1], genrep, out);

        if right_env.is_empty() || left_env.is_empty() {
            continue;
        }

        // Top-level gap excess — how much we want to shift.
        let desired_shift = left_env[0] - right_env[0] - gap_w;
        if desired_shift <= 1e-6 {
            continue;
        }

        // Safe shift: the desired shift capped by the tightest clearance at any depth.
        // zip stops at the shorter envelope, so leaf siblings (envelope length 1) are
        // unconstrained by deeper generations.
        let safe_shift = right_env
            .iter()
            .zip(left_env.iter())
            .map(|(r, l)| l - r - gap_w)
            .fold(desired_shift, f64::min)
            .max(0.0);

        if safe_shift > 1e-6 {
            for j in 0..=i {
                shift_subtree(&children[j], safe_shift, generation, genrep, out, global_right);
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
/// 3. Run a right-to-left compact pass (`compact_siblings`) to close excess gaps, limited
///    by the envelope clearance at every depth so no subtree overlap is introduced.
/// 4. Derive the parent's x as the horizontal midpoint of the children.
/// 5. If that midpoint is left of `x_default` (the column is squeezed), shift every child
///    subtree rightward by the difference (`shift_subtree`) so the parent can sit at
///    `x_default` while remaining centred.
///
/// The two-spouse case is identical but concatenates both spouses' children
/// and adjusts the midpoint calculation for the wide-box connector offsets.
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
            if children.is_empty() || env_left.len() <= 1 {
                x_default
            } else {
                place_descendants(genrep, &children[0], &env_left[1..], generation + 1, box_w, box_h, box_w2, gap_w, gap_h, out, global_right);
                for i in 1..children.len() {
                    let right_env = fill_env_from_global(
                        get_right_envelope(&children[i - 1], genrep, out),
                        env_left.len().saturating_sub(1),
                        global_right,
                        (generation as usize) + 1,
                    );
                    place_descendants(genrep, &children[i], &right_env, generation + 1, box_w, box_h, box_w2, gap_w, gap_h, out, global_right);
                }

                compact_siblings(&children, generation + 1, gap_w, genrep, out, global_right);

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
            let all_children: Vec<String> = children1.iter().chain(children2.iter()).cloned().collect();

            if all_children.is_empty() || env_left.len() <= 1 {
                x_default
            } else {
                place_descendants(genrep, &all_children[0], &env_left[1..], generation + 1, box_w, box_h, box_w2, gap_w, gap_h, out, global_right);
                for i in 1..all_children.len() {
                    let right_env = fill_env_from_global(
                        get_right_envelope(&all_children[i - 1], genrep, out),
                        env_left.len().saturating_sub(1),
                        global_right,
                        (generation as usize) + 1,
                    );
                    place_descendants(genrep, &all_children[i], &right_env, generation + 1, box_w, box_h, box_w2, gap_w, gap_h, out, global_right);
                }

                compact_siblings(&all_children, generation + 1, gap_w, genrep, out, global_right);

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
    };
    out.insert(ind_id.to_string(), copy_individual(ind, Some(BoxedCouplesGeo::Individual(geo))));
    if (generation as usize) < global_right.len() {
        let right_edge = x + width / 2.0;
        global_right[generation as usize] = global_right[generation as usize].max(right_edge);
    }
}

/// Stub for ancestor-direction layout; currently delegates to [`place_descendants`].
///
/// A true ancestors traversal would walk `famc` links and place parents above the child.
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
    // TODO: implement true ancestors traversal (walk famc, place parents above child)
    place_descendants(genrep, ind_id, env_left, generation, box_w, box_h, box_w2, gap_w, gap_h, out, global_right);
}

pub struct BoxedCouplesLayout;

impl Layout for BoxedCouplesLayout {
    type Geo = BoxedCouplesGeo;

    fn compute(
        &self,
        genrep: &Genrep,
        prefs: &Prefs,
    ) -> Result<Genrep<BoxedCouplesGeo>> {
        let dir = prefs.scope.direction.to_lowercase();

        if matches_direction(&dir, "forest") {
            eprintln!("warning: boxed_couples layout does not support direction=forest");
            return Ok(Genrep {
                individuals: HashMap::new(),
                families: HashMap::new(),
                first_individual_id: genrep.first_individual_id.clone(),
            });
        }

        let root_id = if prefs.scope.root.is_empty() {
            genrep.first_individual_id.as_deref().unwrap_or("")
        } else {
            prefs.scope.root.as_str()
        };

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

        let max_gen = if prefs.scope.generations == 0 { 100 } else { prefs.scope.generations };
        let env_left: Vec<f64> = vec![0.0; max_gen as usize];
        let mut global_right: Vec<f64> = vec![0.0; max_gen as usize];

        let mut individuals: HashMap<String, Individual<BoxedCouplesGeo>> = HashMap::new();

        if matches_direction(&dir, "ancestors") {
            place_ancestors(genrep, root_id, &env_left, 0, box_w, box_h, box_w2, gap_w, gap_h, &mut individuals, &mut global_right);
        } else {
            place_descendants(genrep, root_id, &env_left, 0, box_w, box_h, box_w2, gap_w, gap_h, &mut individuals, &mut global_right);
        }

        // Add in-scope spouses of placed individuals to the output
        let placed_ids: Vec<String> = individuals.keys().cloned().collect();
        for ind_id in placed_ids {
            let spouses = spouses_of(&ind_id, genrep);
            for spouse_id in spouses {
                if !individuals.contains_key(&spouse_id) {
                    if let Some(spouse) = genrep.get_individual(&spouse_id) {
                        individuals.insert(spouse_id, copy_individual(spouse, None));
                    }
                }
            }
        }

        let families = copy_families(genrep, &individuals, box_h, box_w, box_w2);

        Ok(Genrep {
            individuals,
            families,
            first_individual_id: genrep.first_individual_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_genrep() -> Genrep {
        let mut individuals = HashMap::new();
        let mut families = HashMap::new();

        individuals.insert("I1".to_string(), Individual {
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
        });

        individuals.insert("I2".to_string(), Individual {
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
        });

        individuals.insert("I3".to_string(), Individual {
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
        });

        individuals.insert("I4".to_string(), Individual {
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
        });

        individuals.insert("I5".to_string(), Individual {
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
        });

        individuals.insert("I6".to_string(), Individual {
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
        });

        families.insert("F1".to_string(), Family {
            id: "F1".to_string(),
            husband_id: Some("I1".to_string()),
            wife_id: Some("I2".to_string()),
            children_ids: vec!["I3".to_string(), "I4".to_string(), "I5".to_string()],
            marriage: None,
            jmar: None,
            in_scope: true,
            geo: None,
        });

        families.insert("F2".to_string(), Family {
            id: "F2".to_string(),
            husband_id: Some("I3".to_string()),
            wife_id: None,
            children_ids: vec!["I6".to_string()],
            marriage: None,
            jmar: None,
            in_scope: true,
            geo: None,
        });

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

        individuals.insert("I10".to_string(), Individual {
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
        });

        individuals.insert("I11".to_string(), Individual {
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
        });

        individuals.insert("I12".to_string(), Individual {
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
        });

        individuals.insert("I13".to_string(), Individual {
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
        });

        individuals.insert("I14".to_string(), Individual {
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
        });

        individuals.insert("I15".to_string(), Individual {
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
        });

        families.insert("F10".to_string(), Family {
            id: "F10".to_string(),
            husband_id: Some("I10".to_string()),
            wife_id: Some("I11".to_string()),
            children_ids: vec!["I14".to_string()],
            marriage: None,
            jmar: None,
            in_scope: true,
            geo: None,
        });

        families.insert("F11".to_string(), Family {
            id: "F11".to_string(),
            husband_id: Some("I10".to_string()),
            wife_id: Some("I12".to_string()),
            children_ids: vec!["I15".to_string()],
            marriage: None,
            jmar: None,
            in_scope: true,
            geo: None,
        });

        families.insert("F12".to_string(), Family {
            id: "F12".to_string(),
            husband_id: Some("I10".to_string()),
            wife_id: Some("I13".to_string()),
            children_ids: vec![],
            marriage: None,
            jmar: None,
            in_scope: true,
            geo: None,
        });

        Genrep {
            individuals,
            families,
            first_individual_id: Some("I10".to_string()),
        }
    }

    fn two_spouse_genrep() -> Genrep {
        let mut individuals = HashMap::new();
        let mut families = HashMap::new();

        individuals.insert("I20".to_string(), Individual {
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
        });

        individuals.insert("I21".to_string(), Individual {
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
        });

        individuals.insert("I22".to_string(), Individual {
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
        });

        individuals.insert("I23".to_string(), Individual {
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
        });

        families.insert("F20".to_string(), Family {
            id: "F20".to_string(),
            husband_id: Some("I20".to_string()),
            wife_id: Some("I21".to_string()),
            children_ids: vec![],
            marriage: None,
            jmar: None,
            in_scope: true,
            geo: None,
        });

        families.insert("F21".to_string(), Family {
            id: "F21".to_string(),
            husband_id: Some("I20".to_string()),
            wife_id: Some("I22".to_string()),
            children_ids: vec!["I23".to_string()],
            marriage: None,
            jmar: None,
            in_scope: true,
            geo: None,
        });

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
        let result = BoxedCouplesLayout.compute(&test_genrep(), &desc_prefs()).unwrap();
        let prefs = desc_prefs();
        let box_w = prefs.layout.boxed_couples.box_width;
        let gap_w = prefs.layout.boxed_couples.gap_width;

        let mut xs: Vec<f64> = ["I3", "I4", "I5"]
            .iter().map(|id| ind_geo(&result, id).x).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());

        for pair in xs.windows(2) {
            assert!(
                pair[1] - pair[0] >= box_w + gap_w - 1e-6,
                "siblings overlap: gap = {}", pair[1] - pair[0]
            );
        }
    }

    #[test]
    fn root_centred_over_children() {
        let result = BoxedCouplesLayout.compute(&test_genrep(), &desc_prefs()).unwrap();
        let x_root = ind_geo(&result, "I1").x;
        let x_mid = ind_geo(&result, "I4").x;
        assert!((x_root - x_mid).abs() < 1e-6,
            "root x={x_root} should equal middle child (I4) x={x_mid}");
    }

    #[test]
    fn connector_points() {
        let result = BoxedCouplesLayout.compute(&test_genrep(), &desc_prefs()).unwrap();
        let box_h = desc_prefs().layout.boxed_couples.box_height;

        let g1 = ind_geo(&result, "I1");
        let g3 = ind_geo(&result, "I3");

        assert!((g1.conn_in_y - (0.0 - box_h / 2.0)).abs() < 1e-6,
            "I1 conn_in_y wrong: got {}", g1.conn_in_y);

        assert!((g3.conn_in_y - (g3.y - box_h / 2.0)).abs() < 1e-6,
            "I3 conn_in_y wrong: got {}", g3.conn_in_y);
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
        let result = BoxedCouplesLayout.compute(&two_spouse_genrep(), &two_spouse_prefs()).unwrap();
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
        out.insert("I_wife".to_string(), Individual {
            id: "I_wife".to_string(),
            given: None, surname: None, sex: Some('F'),
            birth: None, death: None,
            fams: vec!["F1".to_string()], famc: vec![],
            alt_name: None, name_heb: None, living: None,
            in_scope: true,
            geo: Some(BoxedCouplesGeo::Individual(IndividualGeo {
                x: 0.0, y: 0.0, width: 220.0, height: 160.0,
                conn_in_x: 0.0, conn_in_y: -80.0,
            })),
        });
        out.insert("I_husb".to_string(), Individual {
            id: "I_husb".to_string(),
            given: None, surname: None, sex: Some('M'),
            birth: None, death: None,
            fams: vec!["F1".to_string()], famc: vec![],
            alt_name: None, name_heb: None, living: None,
            in_scope: true,
            geo: None, // spouse — not placed
        });
        let fam = Family {
            id: "F1".to_string(),
            husband_id: Some("I_husb".to_string()),
            wife_id: Some("I_wife".to_string()),
            children_ids: vec![],
            marriage: None, jmar: None,
            in_scope: true,
            geo: None,
        };
        let result = build_family_geo(&fam, &out, 160.0, 220.0, 480.0);
        assert!(result.is_some(), "build_family_geo must succeed when wife is the placed individual");
    }

    #[test]
    fn test_last_sibling_children_placed() {
        use crate::parser::{compute_scope, parse_str};
        use crate::preferences::Prefs;
        use crate::layout::{run_layout, LayoutOutput};

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
        let layout = run_layout(&genrep, &prefs).unwrap();

        let bc = match layout {
            LayoutOutput::BoxedCouples(ref g) => g,
            _ => panic!("expected BoxedCouples layout"),
        };

        let i6 = bc.individuals.get("I6")
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
        use crate::layout::{run_layout, LayoutOutput};

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
        let layout = run_layout(&genrep, &prefs).unwrap();

        let bc = match layout {
            LayoutOutput::BoxedCouples(ref g) => g,
            _ => panic!("expected BoxedCouples layout"),
        };

        let get_x = |id: &str| match &bc.individuals[id].geo {
            Some(BoxedCouplesGeo::Individual(g)) => g.x,
            _ => panic!("{id} not placed as Individual"),
        };

        let x4  = get_x("I4");
        let x5  = get_x("I5");
        let x9  = get_x("I9");
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
        use crate::layout::{run_layout, LayoutOutput};

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
        let layout = run_layout(&genrep, &prefs).unwrap();

        let bc = match layout {
            LayoutOutput::BoxedCouples(ref g) => g,
            _ => panic!("expected BoxedCouples layout"),
        };

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
        assert!((gap_45 - gap_w).abs() < 1e-6,
            "gap I4→I5 should equal gap_w after compact pass (leaves), got {gap_45}");

        let gap_34 = x_i4 - box_w / 2.0 - (x_i3 + box_w / 2.0);
        assert!((gap_34 - gap_w).abs() < 1e-6,
            "relative gap I3→I4 must be preserved, got {gap_34}");
    }

    #[test]
    fn test_compact_no_subtree_overlap() {
        use crate::parser::{compute_scope, parse_str};
        use crate::preferences::Prefs;
        use crate::layout::{run_layout, LayoutOutput};

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
        let layout = run_layout(&genrep, &prefs).unwrap();

        let bc = match layout {
            LayoutOutput::BoxedCouples(ref g) => g,
            _ => panic!("expected BoxedCouples layout"),
        };

        let get_x = |id: &str| match &bc.individuals[id].geo {
            Some(BoxedCouplesGeo::Individual(g)) => g.x,
            _ => panic!("{id} not placed as Individual"),
        };

        let box_w = prefs.layout.boxed_couples.box_width;
        let gap_w = prefs.layout.boxed_couples.gap_width;

        let x_i7 = get_x("I7");
        let x_i9 = get_x("I9");

        let right_edge_i7 = x_i7 + box_w / 2.0;
        let left_edge_i9  = x_i9 - box_w / 2.0;
        assert!(
            right_edge_i7 <= left_edge_i9 - gap_w + 1e-6,
            "compact moved I7 into I9 at depth 1: I7.right={right_edge_i7}, I9.left={left_edge_i9}"
        );
    }
}
