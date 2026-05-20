//! `boxes` layout: one individual per box, both ancestors and descendants directions.
//!
//! ## Coordinate system
//! Same as `boxed_couples`:
//! - x increases rightward; y = 0 is the root's row.
//! - y decreases (more negative) as generations move away from the root.
//! - All x/y values in [`BoxesIndividualGeo`] are box centres.
//!
//! ## Ancestors direction
//! Root at one edge; parents above (or below, depending on `root_pos`).
//! One box per individual; father to the left, mother to the right.
//! Consanguinity is handled via instance keys (`@I1@`, `@I1@##1`, …).
//!
//! ## Descendants direction
//! Root at one edge; each individual has their own box.
//! Spouses are placed to the right of the individual, slightly lower on the page
//! (`couple_y_offset`). Children of each spouse are placed below that spouse.

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

// ── Geo types ─────────────────────────────────────────────────────────────────

/// Layout geometry for a placed individual (both directions).
#[derive(Debug, Clone)]
pub struct BoxesIndividualGeo {
    /// Horizontal centre of the box (layout space).
    pub x: f64,
    /// Vertical centre of the box (layout space; root = 0, further generations more negative).
    pub y: f64,
    /// Box width (always `box_w`).
    pub width: f64,
    /// Box height (`box_h`).
    pub height: f64,
    /// x of the incoming-connector attach point (equals x).
    pub conn_in_x: f64,
    /// Absolute generation depth (root = 0).
    pub generation: u32,
    /// Ancestors mode only: instance keys of placed parents [father_key, mother_key].
    /// Empty in descendants mode.
    pub parent_keys: Vec<String>,
}

/// Geo variant stored on each placed individual.
#[derive(Debug, Clone)]
pub enum BoxesGeo {
    Individual(BoxesIndividualGeo),
}

// ── Shared helpers ─────────────────────────────────────────────────────────────

/// Returns a unique instance key for an individual.
/// First visit returns the raw id; subsequent visits append `##n`.
fn instance_key(id: &str, count: usize) -> String {
    if count == 0 {
        id.to_string()
    } else {
        format!("{id}##{count}")
    }
}

/// Returns (father_id, mother_id) from the first in-scope FAMC family of `ind_id`.
fn parents_of<G>(ind_id: &str, genrep: &Genrep<G>) -> (Option<String>, Option<String>) {
    let ind = match genrep.get_individual(ind_id) {
        Some(i) => i,
        None => return (None, None),
    };
    for fam_id in &ind.famc {
        let fam = match genrep.get_family(fam_id) {
            Some(f) if f.in_scope => f,
            _ => continue,
        };
        let father = fam
            .husband_id
            .as_ref()
            .filter(|id| genrep.get_individual(id).is_some_and(|i| i.in_scope))
            .cloned();
        let mother = fam
            .wife_id
            .as_ref()
            .filter(|id| genrep.get_individual(id).is_some_and(|i| i.in_scope))
            .cloned();
        if father.is_some() || mother.is_some() {
            return (father, mother);
        }
    }
    (None, None)
}

fn get_x_of(key: &str, out: &HashMap<String, Individual<BoxesGeo>>) -> f64 {
    match out.get(key).and_then(|i| i.geo.as_ref()) {
        Some(BoxesGeo::Individual(g)) => g.x,
        _ => panic!("boxes get_x_of: {key:?} not yet placed — this is a bug"),
    }
}

// ── Ancestors placement ────────────────────────────────────────────────────────

fn get_right_envelope_anc(
    instance_key: &str,
    out: &HashMap<String, Individual<BoxesGeo>>,
) -> Vec<f64> {
    let geo = match out.get(instance_key).and_then(|i| i.geo.as_ref()) {
        Some(BoxesGeo::Individual(g)) => g,
        _ => return vec![],
    };
    let mut result = vec![geo.x + geo.width / 2.0];
    let mut merged: Vec<f64> = Vec::new();
    for pk in &geo.parent_keys {
        merged = merge_max(merged, get_right_envelope_anc(pk, out));
    }
    result.extend(merged);
    result
}

fn shift_anc_subtree(
    instance_key: &str,
    dx: f64,
    generation: u32,
    out: &mut HashMap<String, Individual<BoxesGeo>>,
    global_right: &mut Vec<f64>,
) {
    let parent_keys: Vec<String> = match out.get(instance_key) {
        Some(i) => match &i.geo {
            Some(BoxesGeo::Individual(g)) => g.parent_keys.clone(),
            _ => vec![],
        },
        None => vec![],
    };
    if let Some(ind) = out.get_mut(instance_key) {
        if let Some(BoxesGeo::Individual(g)) = &mut ind.geo {
            g.x += dx;
            g.conn_in_x += dx;
            let gi = generation as usize;
            if gi < global_right.len() {
                global_right[gi] = global_right[gi].max(g.x + g.width / 2.0);
            }
        }
    }
    for pk in parent_keys {
        shift_anc_subtree(&pk, dx, generation + 1, out, global_right);
    }
}

/// Places a single known parent in the ancestors layout and returns `(x, parent_keys)`.
///
/// Shared by the `(Some(pid), None)` and `(None, Some(pid))` match arms of `place_ancestors`;
/// both arms are identical — only the source of `pid` differs.
#[allow(clippy::too_many_arguments)]
fn place_single_parent_anc<G>(
    genrep: &Genrep<G>,
    pid: &str,
    env_left: &[f64],
    generation: u32,
    box_w: f64,
    box_h: f64,
    gap_w: f64,
    gap_h: f64,
    out: &mut HashMap<String, Individual<BoxesGeo>>,
    global_right: &mut Vec<f64>,
    visit_count: &mut HashMap<String, usize>,
    x_default: f64,
) -> (f64, Vec<String>) {
    let pkey = place_ancestors(
        genrep,
        pid,
        &env_left[1..],
        generation + 1,
        box_w,
        box_h,
        gap_w,
        gap_h,
        out,
        global_right,
        visit_count,
    );
    if pkey.is_empty() {
        (x_default, vec![])
    } else {
        let px = get_x_of(&pkey, out);
        if px < x_default {
            shift_anc_subtree(&pkey, x_default - px, generation + 1, out, global_right);
        }
        (x_default.max(get_x_of(&pkey, out)), vec![pkey])
    }
}

/// Recursively places `ind_id` and its in-scope ancestors.
/// Returns the instance key assigned (empty string if not placed).
#[allow(clippy::too_many_arguments)]
fn place_ancestors<G>(
    genrep: &Genrep<G>,
    ind_id: &str,
    env_left: &[f64],
    generation: u32,
    box_w: f64,
    box_h: f64,
    gap_w: f64,
    gap_h: f64,
    out: &mut HashMap<String, Individual<BoxesGeo>>,
    global_right: &mut Vec<f64>,
    visit_count: &mut HashMap<String, usize>,
) -> String {
    let ind = match genrep.get_individual(ind_id) {
        Some(i) if i.in_scope => i,
        _ => return String::new(),
    };

    let count = *visit_count.get(ind_id).unwrap_or(&0);
    *visit_count.entry(ind_id.to_string()).or_insert(0) += 1;
    let ikey = instance_key(ind_id, count);

    let y = -(generation as f64 * (box_h + gap_h));
    let x_default = env_left.first().copied().unwrap_or(0.0) + gap_w + box_w / 2.0;

    let (father_id, mother_id) = parents_of(ind_id, genrep);

    let (x, parent_keys) = match (father_id, mother_id) {
        (None, None) => (x_default, vec![]),

        (Some(pid), None) => place_single_parent_anc(
            genrep,
            &pid,
            env_left,
            generation,
            box_w,
            box_h,
            gap_w,
            gap_h,
            out,
            global_right,
            visit_count,
            x_default,
        ),

        (None, Some(pid)) => place_single_parent_anc(
            genrep,
            &pid,
            env_left,
            generation,
            box_w,
            box_h,
            gap_w,
            gap_h,
            out,
            global_right,
            visit_count,
            x_default,
        ),

        (Some(fid), Some(mid)) => {
            let fkey = place_ancestors(
                genrep,
                &fid,
                &env_left[1..],
                generation + 1,
                box_w,
                box_h,
                gap_w,
                gap_h,
                out,
                global_right,
                visit_count,
            );
            let right_env = if fkey.is_empty() {
                vec![]
            } else {
                fill_env_from_global(
                    get_right_envelope_anc(&fkey, out),
                    env_left.len().saturating_sub(1),
                    global_right,
                    (generation as usize) + 1,
                )
            };
            let mkey = place_ancestors(
                genrep,
                &mid,
                &right_env,
                generation + 1,
                box_w,
                box_h,
                gap_w,
                gap_h,
                out,
                global_right,
                visit_count,
            );

            let x_mid = match (fkey.is_empty(), mkey.is_empty()) {
                (true, true) => x_default,
                (false, true) => get_x_of(&fkey, out),
                (true, false) => get_x_of(&mkey, out),
                (false, false) => (get_x_of(&fkey, out) + get_x_of(&mkey, out)) / 2.0,
            };

            if x_mid < x_default {
                let shift = x_default - x_mid;
                if !fkey.is_empty() {
                    shift_anc_subtree(&fkey, shift, generation + 1, out, global_right);
                }
                if !mkey.is_empty() {
                    shift_anc_subtree(&mkey, shift, generation + 1, out, global_right);
                }
            }

            let keys: Vec<String> = [fkey, mkey].into_iter().filter(|k| !k.is_empty()).collect();
            let x_final = x_default.max(x_mid);
            (x_final, keys)
        }
    };

    let geo = BoxesIndividualGeo {
        x,
        y,
        width: box_w,
        height: box_h,
        conn_in_x: x,
        generation,
        parent_keys,
    };
    out.insert(
        ikey.clone(),
        copy_individual(ind, Some(BoxesGeo::Individual(geo))),
    );
    let gi = generation as usize;
    if gi < global_right.len() {
        global_right[gi] = global_right[gi].max(x + box_w / 2.0);
    }
    ikey
}

// ── Descendants placement ──────────────────────────────────────────────────────

/// Right edge of the family unit (individual + all its in-scope spouses) at this generation.
fn family_right_edge(
    ind_id: &str,
    geo: &BoxesIndividualGeo,
    genrep: &Genrep,
    box_w: f64,
    gap_w: f64,
) -> f64 {
    let n = spouses_of(ind_id, genrep).len();
    geo.x + (n as f64) * (box_w + gap_w) + box_w / 2.0
}

fn get_right_envelope_desc(
    ind_id: &str,
    genrep: &Genrep,
    out: &HashMap<String, Individual<BoxesGeo>>,
    box_w: f64,
    gap_w: f64,
) -> Vec<f64> {
    let geo = match out.get(ind_id).and_then(|i| i.geo.as_ref()) {
        Some(BoxesGeo::Individual(g)) => g.clone(),
        _ => return vec![],
    };
    let right = family_right_edge(ind_id, &geo, genrep, box_w, gap_w);
    let mut result = vec![right];

    let mut merged: Vec<f64> = Vec::new();
    for sp in spouses_of(ind_id, genrep) {
        for child in children_with_spouse(ind_id, &sp, genrep) {
            if out.contains_key(child.as_str()) {
                merged = merge_max(
                    merged,
                    get_right_envelope_desc(&child, genrep, out, box_w, gap_w),
                );
            }
        }
    }
    result.extend(merged);
    result
}

fn get_left_envelope_desc(
    ind_id: &str,
    genrep: &Genrep,
    out: &HashMap<String, Individual<BoxesGeo>>,
    box_w: f64,
    gap_w: f64,
) -> Vec<f64> {
    let geo = match out.get(ind_id).and_then(|i| i.geo.as_ref()) {
        Some(BoxesGeo::Individual(g)) => g.clone(),
        _ => return vec![],
    };
    let mut result = vec![geo.x - box_w / 2.0];

    let mut merged: Vec<f64> = Vec::new();
    for sp in spouses_of(ind_id, genrep) {
        for child in children_with_spouse(ind_id, &sp, genrep) {
            if out.contains_key(child.as_str()) {
                merged = merge_min(
                    merged,
                    get_left_envelope_desc(&child, genrep, out, box_w, gap_w),
                );
            }
        }
    }
    result.extend(merged);
    result
}

fn shift_subtree_desc(
    ind_id: &str,
    dx: f64,
    generation: u32,
    genrep: &Genrep,
    box_w: f64,
    gap_w: f64,
    out: &mut HashMap<String, Individual<BoxesGeo>>,
    global_right: &mut Vec<f64>,
) {
    let children: Vec<String> = spouses_of(ind_id, genrep)
        .iter()
        .flat_map(|sp| children_with_spouse(ind_id, sp, genrep))
        .collect();

    if let Some(ind) = out.get_mut(ind_id) {
        if let Some(BoxesGeo::Individual(g)) = &mut ind.geo {
            g.x += dx;
            g.conn_in_x += dx;
            let n_spouses = spouses_of(ind_id, genrep).len();
            let right = g.x + (n_spouses as f64) * (box_w + gap_w) + box_w / 2.0;
            let gi = generation as usize;
            if gi < global_right.len() {
                global_right[gi] = global_right[gi].max(right);
            }
        }
    }

    for child in children {
        shift_subtree_desc(
            &child,
            dx,
            generation + 1,
            genrep,
            box_w,
            gap_w,
            out,
            global_right,
        );
    }
}

fn compact_siblings_desc(
    children: &[String],
    generation: u32,
    gap_w: f64,
    genrep: &Genrep,
    box_w: f64,
    out: &mut HashMap<String, Individual<BoxesGeo>>,
    global_right: &mut Vec<f64>,
) {
    if children.len() < 2 {
        return;
    }
    for i in (0..children.len() - 1).rev() {
        let mut block_right_env: Vec<f64> = Vec::new();
        #[allow(clippy::needless_range_loop)]
        for j in 0..=i {
            let env = get_right_envelope_desc(&children[j], genrep, out, box_w, gap_w);
            block_right_env = merge_max(block_right_env, env);
        }
        let left_env = get_left_envelope_desc(&children[i + 1], genrep, out, box_w, gap_w);

        if block_right_env.is_empty() || left_env.is_empty() {
            continue;
        }

        let desired_shift = left_env[0] - block_right_env[0] - gap_w;
        if desired_shift <= 1e-6 {
            continue;
        }

        let safe_shift = block_right_env
            .iter()
            .zip(left_env.iter())
            .map(|(r, l)| l - r - gap_w)
            .fold(desired_shift, f64::min)
            .max(0.0);

        if safe_shift > 1e-6 {
            #[allow(clippy::needless_range_loop)]
            for j in 0..=i {
                shift_subtree_desc(
                    &children[j],
                    safe_shift,
                    generation,
                    genrep,
                    box_w,
                    gap_w,
                    out,
                    global_right,
                );
            }
        }
    }
}

fn compact_pass_desc(
    ind_id: &str,
    genrep: &Genrep,
    out: &mut HashMap<String, Individual<BoxesGeo>>,
    global_right: &mut Vec<f64>,
    gap_w: f64,
    box_w: f64,
    generation: u32,
) {
    let all_children: Vec<String> = spouses_of(ind_id, genrep)
        .iter()
        .flat_map(|sp| children_with_spouse(ind_id, sp, genrep))
        .filter(|cid| out.contains_key(cid.as_str()))
        .collect();

    if all_children.is_empty() {
        return;
    }

    compact_siblings_desc(
        &all_children,
        generation + 1,
        gap_w,
        genrep,
        box_w,
        out,
        global_right,
    );

    for child in all_children {
        compact_pass_desc(
            &child,
            genrep,
            out,
            global_right,
            gap_w,
            box_w,
            generation + 1,
        );
    }
}

fn recenter_pass_desc(
    ind_id: &str,
    genrep: &Genrep,
    box_w: f64,
    gap_w: f64,
    out: &mut HashMap<String, Individual<BoxesGeo>>,
) {
    let spouses = spouses_of(ind_id, genrep);
    let all_children: Vec<String> = spouses
        .iter()
        .flat_map(|sp| children_with_spouse(ind_id, sp, genrep))
        .filter(|cid| out.contains_key(cid.as_str()))
        .collect();

    for child in &all_children {
        recenter_pass_desc(child, genrep, box_w, gap_w, out);
    }

    if spouses.is_empty() || all_children.is_empty() {
        return;
    }

    // Children of spouse1 determine where spouse1 goes, which sets the individual's x.
    let children1: Vec<String> = children_with_spouse(ind_id, &spouses[0], genrep)
        .into_iter()
        .filter(|cid| out.contains_key(cid.as_str()))
        .collect();

    if children1.is_empty() {
        return;
    }

    let n = children1.len();
    let x_spouse1 = if n % 2 == 1 {
        get_x_of(&children1[n / 2], out)
    } else {
        (get_x_of(&children1[n / 2 - 1], out) + get_x_of(&children1[n / 2], out)) / 2.0
    };

    let new_x = x_spouse1 - (box_w + gap_w);

    if let Some(ind) = out.get_mut(ind_id) {
        if let Some(BoxesGeo::Individual(g)) = &mut ind.geo {
            g.x = new_x;
            g.conn_in_x = new_x;
        }
    }
}

/// Left-to-right sweep that pushes overlapping siblings apart after recentering.
/// Must be called bottom-up so inner overlaps are resolved before outer ones.
fn separate_pass_desc(
    ind_id: &str,
    genrep: &Genrep,
    box_w: f64,
    gap_w: f64,
    out: &mut HashMap<String, Individual<BoxesGeo>>,
    global_right: &mut Vec<f64>,
) {
    let generation = match out.get(ind_id).and_then(|ind| ind.geo.as_ref()) {
        Some(BoxesGeo::Individual(g)) => g.generation,
        _ => return,
    };
    let child_generation = generation + 1;

    let all_children: Vec<String> = spouses_of(ind_id, genrep)
        .iter()
        .flat_map(|sp| children_with_spouse(ind_id, sp, genrep))
        .filter(|cid| out.contains_key(cid.as_str()))
        .collect();

    for child in &all_children {
        separate_pass_desc(child, genrep, box_w, gap_w, out, global_right);
    }

    for i in 1..all_children.len() {
        let right_env = get_right_envelope_desc(&all_children[i - 1], genrep, out, box_w, gap_w);
        let left_env = get_left_envelope_desc(&all_children[i], genrep, out, box_w, gap_w);
        if right_env.is_empty() || left_env.is_empty() {
            continue;
        }
        let overlap = right_env[0] + gap_w - left_env[0];
        if overlap > 1e-6 {
            shift_subtree_desc(
                &all_children[i],
                overlap,
                child_generation,
                genrep,
                box_w,
                gap_w,
                out,
                global_right,
            );
        }
    }
}

/// Recursively places `ind_id` and its in-scope descendants.
#[allow(clippy::too_many_arguments)]
fn place_descendants(
    genrep: &Genrep,
    ind_id: &str,
    env_left: &[f64],
    generation: u32,
    box_w: f64,
    box_h: f64,
    gap_w: f64,
    gap_h: f64,
    out: &mut HashMap<String, Individual<BoxesGeo>>,
    global_right: &mut Vec<f64>,
    visit_count: &mut HashMap<String, usize>,
) {
    let ind = match genrep.get_individual(ind_id) {
        Some(i) if i.in_scope => i,
        _ => return,
    };

    let count = *visit_count.get(ind_id).unwrap_or(&0);
    *visit_count.entry(ind_id.to_string()).or_insert(0) += 1;
    let ikey = instance_key(ind_id, count);

    let y = -(generation as f64 * (box_h + gap_h));
    let x_default = env_left.first().copied().unwrap_or(0.0) + gap_w + box_w / 2.0;

    let spouses = spouses_of(ind_id, genrep);

    let x = if spouses.is_empty() {
        x_default
    } else {
        // Collect all children across all spouses in order
        let children1 = children_with_spouse(ind_id, &spouses[0], genrep);
        let all_children: Vec<String> = spouses
            .iter()
            .flat_map(|sp| children_with_spouse(ind_id, sp, genrep))
            .collect();

        if all_children.is_empty() {
            x_default
        } else {
            // Place children of spouse1 first
            if !children1.is_empty() {
                place_descendants(
                    genrep,
                    &children1[0],
                    &env_left[1..],
                    generation + 1,
                    box_w,
                    box_h,
                    gap_w,
                    gap_h,
                    out,
                    global_right,
                    visit_count,
                );
                for i in 1..children1.len() {
                    let right_env = fill_env_from_global(
                        get_right_envelope_desc(&children1[i - 1], genrep, out, box_w, gap_w),
                        env_left.len().saturating_sub(1),
                        global_right,
                        (generation as usize) + 1,
                    );
                    place_descendants(
                        genrep,
                        &children1[i],
                        &right_env,
                        generation + 1,
                        box_w,
                        box_h,
                        gap_w,
                        gap_h,
                        out,
                        global_right,
                        visit_count,
                    );
                }
            }

            // Place children of subsequent spouses
            let mut last_placed: Option<String> = children1.last().cloned();
            for k in 1..spouses.len() {
                let children_k = children_with_spouse(ind_id, &spouses[k], genrep);
                if children_k.is_empty() {
                    continue;
                }
                let right_env = if let Some(ref prev) = last_placed {
                    fill_env_from_global(
                        get_right_envelope_desc(prev, genrep, out, box_w, gap_w),
                        env_left.len().saturating_sub(1),
                        global_right,
                        (generation as usize) + 1,
                    )
                } else {
                    env_left[1..].to_vec()
                };
                // Also bump right_env[0] by spouse k-1's box extent (conservative)
                let right_env = if let Some(ref renv_0) = right_env.first().cloned() {
                    let spouse_right = global_right
                        .get(generation as usize)
                        .copied()
                        .unwrap_or(0.0)
                        + (k as f64) * (box_w + gap_w);
                    let mut env = right_env.clone();
                    env[0] = env[0].max(spouse_right);
                    let _ = renv_0;
                    env
                } else {
                    right_env
                };

                place_descendants(
                    genrep,
                    &children_k[0],
                    &right_env,
                    generation + 1,
                    box_w,
                    box_h,
                    gap_w,
                    gap_h,
                    out,
                    global_right,
                    visit_count,
                );
                for i in 1..children_k.len() {
                    let r = fill_env_from_global(
                        get_right_envelope_desc(&children_k[i - 1], genrep, out, box_w, gap_w),
                        env_left.len().saturating_sub(1),
                        global_right,
                        (generation as usize) + 1,
                    );
                    place_descendants(
                        genrep,
                        &children_k[i],
                        &r,
                        generation + 1,
                        box_w,
                        box_h,
                        gap_w,
                        gap_h,
                        out,
                        global_right,
                        visit_count,
                    );
                }
                last_placed = children_k.last().cloned();
            }

            // Derive individual's x: spouse1's x = center of children1
            let x_spouse1 = if !children1.is_empty() {
                let placed_c1: Vec<String> = children1
                    .iter()
                    .filter(|c| out.contains_key(c.as_str()))
                    .cloned()
                    .collect();
                if placed_c1.is_empty() {
                    x_default + (box_w + gap_w)
                } else {
                    let n = placed_c1.len();
                    if n % 2 == 1 {
                        get_x_of(&placed_c1[n / 2], out)
                    } else {
                        (get_x_of(&placed_c1[n / 2 - 1], out) + get_x_of(&placed_c1[n / 2], out))
                            / 2.0
                    }
                }
            } else {
                x_default + (box_w + gap_w)
            };

            let x_candidate = x_spouse1 - (box_w + gap_w);
            if x_candidate < x_default {
                let shift = x_default - x_candidate;
                for child in &all_children {
                    if out.contains_key(child.as_str()) {
                        shift_subtree_desc(
                            child,
                            shift,
                            generation + 1,
                            genrep,
                            box_w,
                            gap_w,
                            out,
                            global_right,
                        );
                    }
                }
                x_default
            } else {
                x_candidate
            }
        }
    };

    let geo = BoxesIndividualGeo {
        x,
        y,
        width: box_w,
        height: box_h,
        conn_in_x: x,
        generation,
        parent_keys: vec![],
    };
    out.insert(ikey, copy_individual(ind, Some(BoxesGeo::Individual(geo))));

    let n_spouses = spouses.len();
    let right = x + (n_spouses as f64) * (box_w + gap_w) + box_w / 2.0;
    let gi = generation as usize;
    if gi < global_right.len() {
        global_right[gi] = global_right[gi].max(right);
    }
}

// ── Layout implementation ──────────────────────────────────────────────────────

pub struct BoxesLayout;

impl Layout for BoxesLayout {
    type Geo = BoxesGeo;

    fn compute(&self, genrep: &Genrep, prefs: &Prefs) -> Result<Genrep<BoxesGeo>> {
        let dir = prefs.scope.direction.to_lowercase();

        if matches_direction(&dir, "forest") {
            eprintln!("warning: boxes layout does not support direction=forest");
            return Ok(Genrep {
                individuals: HashMap::new(),
                families: HashMap::new(),
                first_individual_id: genrep.first_individual_id.clone(),
            });
        }

        let root_opt = resolve_root_id(genrep, prefs);
        let root_id = match root_opt.as_deref() {
            Some(r) if !r.is_empty() => r,
            _ => {
                return Ok(Genrep {
                    individuals: HashMap::new(),
                    families: HashMap::new(),
                    first_individual_id: None,
                });
            }
        };

        let bx = &prefs.layout.boxes;
        let box_w = bx.box_width;
        let photo_section_h = if prefs.show.photo && prefs.photos.box_resize {
            prefs.photos.height + 2.0 * prefs.photos.margin
        } else {
            0.0
        };
        let box_h = bx.box_height + photo_section_h;
        let gap_w = bx.gap_width;
        let gap_h = bx.gap_height;

        let max_gen = if prefs.scope.generations == 0 {
            100
        } else {
            prefs.scope.generations
        };
        let env_left: Vec<f64> = vec![0.0; max_gen as usize];
        let mut global_right: Vec<f64> = vec![0.0; max_gen as usize];
        let mut individuals: HashMap<String, Individual<BoxesGeo>> = HashMap::new();

        if matches_direction(&dir, "ancestors") {
            let mut visit_count: HashMap<String, usize> = HashMap::new();
            place_ancestors(
                genrep,
                root_id,
                &env_left,
                0,
                box_w,
                box_h,
                gap_w,
                gap_h,
                &mut individuals,
                &mut global_right,
                &mut visit_count,
            );
        } else {
            let mut visit_count: HashMap<String, usize> = HashMap::new();
            place_descendants(
                genrep,
                root_id,
                &env_left,
                0,
                box_w,
                box_h,
                gap_w,
                gap_h,
                &mut individuals,
                &mut global_right,
                &mut visit_count,
            );
            compact_pass_desc(
                root_id,
                genrep,
                &mut individuals,
                &mut global_right,
                gap_w,
                box_w,
                0,
            );
            recenter_pass_desc(root_id, genrep, box_w, gap_w, &mut individuals);
            separate_pass_desc(
                root_id,
                genrep,
                box_w,
                gap_w,
                &mut individuals,
                &mut global_right,
            );
        }

        // Copy any in-scope individuals not yet placed (e.g. spouses in descendants mode).
        // This lets emit_scene look up their data (name, birth, death) when rendering spouse boxes.
        for (id, ind) in &genrep.individuals {
            if ind.in_scope && !individuals.contains_key(id.as_str()) {
                individuals.insert(id.clone(), copy_individual(ind, None::<BoxesGeo>));
            }
        }

        let families = copy_families(genrep, |_| None::<BoxesGeo>);

        Ok(Genrep {
            individuals,
            families,
            first_individual_id: genrep.first_individual_id.clone(),
        })
    }
}

// ── Scene emission ─────────────────────────────────────────────────────────────

/// Parse the size (last whitespace-delimited token) from a font preference string.
fn parse_font_size(s: &str, fallback: f64) -> f64 {
    s.trim()
        .rsplit_once(' ')
        .and_then(|(_, last)| last.parse::<f64>().ok())
        .unwrap_or(fallback)
}

/// Trim `@` delimiters and replace `##` with `-dup-` for SVG id attributes.
fn trim_id(id: &str) -> String {
    id.replace('@', "").replace("##", "-dup-")
}

pub fn emit_scene(genrep: &Genrep<BoxesGeo>, prefs: &Prefs) -> crate::scene::Scene {
    use crate::format::{format_event, format_name};
    use crate::scene::{
        BoxPrimitive, BoxesSpouseConnector, ConnectorPrimitive, FilledRectPrimitive,
        GroupPrimitive, ImagePrimitive, Point, Primitive, Rect, Scene, TextAlign, TextAttr,
        TextPrimitive,
    };

    let highlighted_ids = crate::layout::common::highlight_set(prefs);

    let placed: Vec<(&str, &BoxesIndividualGeo)> = genrep
        .individuals
        .iter()
        .filter(|(_, ind)| ind.in_scope)
        .filter_map(|(id, ind)| {
            if let Some(BoxesGeo::Individual(ref g)) = ind.geo {
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

    // Display-space transforms
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

    let font_size = parse_font_size(&prefs.output.style.fonts.names, 13.0);
    let date_font_size_raw = parse_font_size(&prefs.output.style.fonts.dates, font_size);
    let date_font_size = if date_font_size_raw <= 0.0 {
        font_size
    } else {
        date_font_size_raw
    };

    let bx = &prefs.layout.boxes;
    let spacing = &prefs.output.style.spacing.boxed_couples;

    let photo_section_h = if prefs.show.photo && prefs.photos.box_resize {
        prefs.photos.height + 2.0 * prefs.photos.margin
    } else {
        0.0
    };
    let effective_box_h = bx.box_height + photo_section_h;

    let is_pdf = prefs.output.output_type.to_lowercase() == "pdf";
    let photo_map: crate::photos::PhotoMap = if prefs.show.photo {
        let bare_ids: Vec<&str> = genrep
            .individuals
            .keys()
            .filter(|id| !id.contains("##"))
            .map(|s| s.as_str())
            .collect();
        crate::photos::build_photo_map(
            &bare_ids,
            &prefs.files.gedcom,
            &prefs.photos,
            is_pdf,
            &prefs.output.path,
        )
    } else {
        crate::photos::PhotoMap::new()
    };

    let is_ancestors = matches_direction(&prefs.scope.direction.to_lowercase(), "ancestors");

    let mut box_groups: Vec<Primitive> = Vec::new();
    let mut connector_groups: Vec<Primitive> = Vec::new();

    // ── Helper: emit one individual box (shared between ancestors and descendants) ──
    let emit_individual_box = |id_trimmed: &str,
                               bare_id: &str,
                               is_dup: bool,
                               is_highlighted: bool,
                               ind: &Individual<BoxesGeo>,
                               box_display_top: f64,
                               cx_display: f64,
                               box_w: f64,
                               box_h: f64|
     -> Vec<Primitive> {
        let box_bbox = Rect {
            x: cx_display - box_w / 2.0,
            y: box_display_top,
            w: box_w,
            h: box_h,
        };
        let mut children: Vec<Primitive> = Vec::new();

        // Outer box
        children.push(Primitive::Box(BoxPrimitive {
            bbox: box_bbox.clone(),
        }));

        // Double border for duplicates when pref is set
        if is_dup && prefs.show.duplicated_individual {
            let inset = 2.5;
            children.push(Primitive::Box(BoxPrimitive {
                bbox: Rect {
                    x: box_bbox.x + inset,
                    y: box_bbox.y + inset,
                    w: (box_bbox.w - 2.0 * inset).max(0.0),
                    h: (box_bbox.h - 2.0 * inset).max(0.0),
                },
            }));
        }

        // Photo (boxes layout only)
        let actual_photo_h = if prefs.show.photo {
            let fits = prefs.photos.box_resize || {
                let min_text_h = spacing.name_above + font_size;
                prefs.photos.height + 2.0 * prefs.photos.margin + min_text_h <= box_h
            };
            if fits {
                let ph_w = prefs.photos.width.min(box_w - 2.0 * prefs.photos.margin);
                let photo_bbox = Rect {
                    x: cx_display - ph_w / 2.0,
                    y: box_display_top + prefs.photos.margin,
                    w: ph_w,
                    h: prefs.photos.height,
                };
                match photo_map.get(bare_id).filter(|h| !h.is_empty()) {
                    Some(href) => {
                        children.push(Primitive::Image(ImagePrimitive {
                            bbox: photo_bbox,
                            href: href.clone(),
                        }));
                    }
                    None => {
                        children.push(Primitive::FilledRect(FilledRectPrimitive {
                            bbox: photo_bbox,
                            fill: "#e8e8e8".to_string(),
                        }));
                    }
                }
                prefs.photos.height + 2.0 * prefs.photos.margin
            } else {
                0.0
            }
        } else {
            0.0
        };

        // Name (wrapped in name sub-group)
        let name_baseline = box_display_top + actual_photo_h + spacing.name_above + font_size;
        children.push(Primitive::Group(GroupPrimitive {
            id: format!("{id_trimmed}-name"),
            children: vec![Primitive::Text(TextPrimitive {
                content: format_name(ind, prefs),
                bbox: Rect {
                    x: cx_display - box_w / 2.0,
                    y: name_baseline - font_size,
                    w: box_w,
                    h: font_size,
                },
                align: TextAlign::Center,
                attrs: crate::scene::label_attrs(TextAttr::IndividualName, is_highlighted),
            })],
        }));

        // ID (if show.id)
        if prefs.show.id {
            children.push(Primitive::Text(TextPrimitive {
                content: id_trimmed.to_string(),
                bbox: Rect {
                    x: cx_display - box_w / 2.0 + 2.0,
                    y: name_baseline - font_size,
                    w: box_w,
                    h: font_size,
                },
                align: TextAlign::Left,
                attrs: vec![TextAttr::IndividualId],
            }));
        }

        // Birth
        let mut y_pos = name_baseline;
        if prefs.show.birth {
            y_pos += spacing.date_above + date_font_size;
            let content = ind
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
                content,
                bbox: Rect {
                    x: cx_display - box_w / 2.0,
                    y: y_pos - date_font_size,
                    w: box_w,
                    h: date_font_size,
                },
                align: TextAlign::Center,
                attrs: vec![TextAttr::BirthData],
            }));
        }

        // Death
        if prefs.show.death {
            y_pos += spacing.date_above + date_font_size;
            let content = ind
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
                content,
                bbox: Rect {
                    x: cx_display - box_w / 2.0,
                    y: y_pos - date_font_size,
                    w: box_w,
                    h: date_font_size,
                },
                align: TextAlign::Center,
                attrs: vec![TextAttr::DeathData],
            }));
        }

        // No marriage data (by design)
        children
    };

    if is_ancestors {
        // ── Ancestors: emit one box per placed individual ────────────────────────
        for (ind_id, geo) in &placed {
            let ind_id_trimmed = trim_id(ind_id);
            let bare_id = ind_id.split("##").next().unwrap_or(ind_id);
            let is_dup = ind_id.contains("##");
            let is_highlighted = highlighted_ids.contains(*ind_id);

            let box_display_top = f64::min(
                to_display_y(geo.y + geo.height / 2.0),
                to_display_y(geo.y - geo.height / 2.0),
            );
            let cx = to_display_x(geo.x);
            let ind = &genrep.individuals[*ind_id];

            let box_children = emit_individual_box(
                &ind_id_trimmed,
                bare_id,
                is_dup,
                is_highlighted,
                ind,
                box_display_top,
                cx,
                geo.width,
                geo.height,
            );

            box_groups.push(Primitive::Group(GroupPrimitive {
                id: String::new(),
                children: vec![Primitive::Group(GroupPrimitive {
                    id: ind_id_trimmed,
                    children: box_children,
                })],
            }));
        }

        // ── Ancestors: connectors (child → parents) ──────────────────────────────
        for (ind_id, geo) in &placed {
            if geo.parent_keys.is_empty() {
                continue;
            }

            // Child's parent-facing edge
            let ind_point = Point {
                x: to_display_x(geo.conn_in_x),
                y: to_display_y(geo.y - geo.height / 2.0),
            };

            // Each parent's child-facing edge
            let parent_points_display: Vec<Point> = geo
                .parent_keys
                .iter()
                .filter_map(|pk| {
                    let parent = genrep.individuals.get(pk)?;
                    if let Some(BoxesGeo::Individual(pg)) = parent.geo.as_ref() {
                        Some(Point {
                            x: to_display_x(pg.conn_in_x),
                            y: to_display_y(pg.y + pg.height / 2.0),
                        })
                    } else {
                        None
                    }
                })
                .collect();

            if parent_points_display.is_empty() {
                continue;
            }

            let child_trimmed = trim_id(ind_id);
            connector_groups.push(Primitive::Group(GroupPrimitive {
                id: String::new(),
                children: vec![Primitive::Group(GroupPrimitive {
                    id: format!("{child_trimmed}-connectors"),
                    children: vec![Primitive::Connector(ConnectorPrimitive {
                        parent_points: vec![ind_point],
                        child_points: parent_points_display,
                    })],
                })],
            }));
        }
    } else {
        // ── Descendants: determine which generation is the deepest ───────────────
        let max_gen_placed: Option<u32> = if !prefs.show.last_gen_spouses {
            placed.iter().map(|(_, g)| g.generation).max()
        } else {
            None
        };

        // ── Descendants: emit individual boxes ───────────────────────────────────
        for (ind_id, geo) in &placed {
            let ind_id_trimmed = trim_id(ind_id);
            let bare_id = ind_id.split("##").next().unwrap_or(ind_id);
            let is_dup = ind_id.contains("##");
            let is_highlighted = highlighted_ids.contains(*ind_id);

            let box_display_top = f64::min(
                to_display_y(geo.y + geo.height / 2.0),
                to_display_y(geo.y - geo.height / 2.0),
            );
            let cx = to_display_x(geo.x);
            let ind = &genrep.individuals[*ind_id];

            let box_children = emit_individual_box(
                &ind_id_trimmed,
                bare_id,
                is_dup,
                is_highlighted,
                ind,
                box_display_top,
                cx,
                geo.width,
                geo.height,
            );

            box_groups.push(Primitive::Group(GroupPrimitive {
                id: String::new(),
                children: vec![Primitive::Group(GroupPrimitive {
                    id: ind_id_trimmed,
                    children: box_children,
                })],
            }));
        }

        // ── Descendants: emit spouse boxes and connectors ────────────────────────
        for (ind_id, geo) in &placed {
            // Skip spouses of the last generation unless opted in
            if let Some(last_gen) = max_gen_placed {
                if geo.generation == last_gen {
                    continue;
                }
            }

            let ind = &genrep.individuals[*ind_id];
            let sorted_fams = sort_families_by_date(ind, genrep);
            let spouses: Vec<(usize, String)> = sorted_fams
                .iter()
                .filter_map(|fid| genrep.families.get(fid))
                .filter(|f| f.in_scope)
                .enumerate()
                .filter_map(|(k, fam)| {
                    let sp_id = if fam.husband_id.as_deref() == Some(ind_id) {
                        fam.wife_id.clone()?
                    } else {
                        fam.husband_id.clone()?
                    };
                    if genrep.get_individual(&sp_id).is_some_and(|i| i.in_scope) {
                        Some((k, sp_id))
                    } else {
                        None
                    }
                })
                .collect();

            if spouses.is_empty() {
                continue;
            }

            let ind_box_display_top = f64::min(
                to_display_y(geo.y + geo.height / 2.0),
                to_display_y(geo.y - geo.height / 2.0),
            );
            let sp_box_display_top = ind_box_display_top + bx.couple_y_offset;

            // Emit spouse boxes and collect their display positions
            let mut spouse_entries: Vec<Point> = Vec::new();
            for (k, sp_id) in &spouses {
                let sp_x_display =
                    to_display_x(geo.x + (*k as f64 + 1.0) * (bx.box_width + bx.gap_width));
                let sp_id_trimmed = trim_id(sp_id);
                let is_highlighted = highlighted_ids.contains(sp_id.as_str());

                let sp_ind = match genrep.individuals.get(sp_id) {
                    Some(i) => i,
                    None => continue,
                };

                let sp_box_children = emit_individual_box(
                    &sp_id_trimmed,
                    sp_id.as_str(), // spouses are never consanguinity duplicates in this pass
                    false,
                    is_highlighted,
                    sp_ind,
                    sp_box_display_top,
                    sp_x_display,
                    bx.box_width,
                    effective_box_h,
                );

                box_groups.push(Primitive::Group(GroupPrimitive {
                    id: String::new(),
                    children: vec![Primitive::Group(GroupPrimitive {
                        id: sp_id_trimmed,
                        children: sp_box_children,
                    })],
                }));

                spouse_entries.push(Point {
                    x: sp_x_display,
                    y: sp_box_display_top,
                });

                // Spouse → children connector
                let children_ids = children_with_spouse(ind_id, sp_id, genrep);
                // Attach at the spouse edge facing the children, and the children edge facing the spouse.
                let sp_exit_y = if root_pos_bottom {
                    sp_box_display_top // top of spouse box faces upward children
                } else {
                    sp_box_display_top + effective_box_h // bottom of spouse box faces downward children
                };
                let child_points: Vec<Point> = children_ids
                    .iter()
                    .filter_map(|cid| {
                        let child = genrep.individuals.get(cid)?;
                        if let Some(BoxesGeo::Individual(cg)) = child.geo.as_ref() {
                            let child_top = f64::min(
                                to_display_y(cg.y + cg.height / 2.0),
                                to_display_y(cg.y - cg.height / 2.0),
                            );
                            // Attach at child edge facing the spouse.
                            let child_y = if root_pos_bottom {
                                child_top + cg.height // bottom edge faces spouse below
                            } else {
                                child_top // top edge faces spouse above
                            };
                            Some(Point {
                                x: to_display_x(cg.conn_in_x),
                                y: child_y,
                            })
                        } else {
                            None
                        }
                    })
                    .collect();

                if !child_points.is_empty() {
                    let sp_id_trimmed2 = trim_id(sp_id);
                    connector_groups.push(Primitive::Group(GroupPrimitive {
                        id: String::new(),
                        children: vec![Primitive::Group(GroupPrimitive {
                            id: format!("{sp_id_trimmed2}-connectors"),
                            children: vec![Primitive::Connector(ConnectorPrimitive {
                                parent_points: vec![Point {
                                    x: sp_x_display,
                                    y: sp_exit_y,
                                }],
                                child_points,
                            })],
                        })],
                    }));
                }
            }

            // Individual → spouses connector (BoxesSpouseConnector)
            if !spouse_entries.is_empty() {
                let first_spouse_id = &spouses[0].1;
                let first_sp_trimmed = trim_id(first_spouse_id);
                connector_groups.push(Primitive::Group(GroupPrimitive {
                    id: String::new(),
                    children: vec![Primitive::Group(GroupPrimitive {
                        id: format!("{first_sp_trimmed}-ind-connectors"),
                        children: vec![Primitive::BoxesSpouseConnector(BoxesSpouseConnector {
                            individual_exit: Point {
                                x: to_display_x(geo.x + geo.width / 2.0),
                                y: ind_box_display_top,
                            },
                            spouse_entries,
                        })],
                    })],
                }));
            }
        }
    }

    // Canvas bounds
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
        h: content_h + bx.couple_y_offset + effective_box_h, // extra room for spouse boxes
    };

    let mut primitives = box_groups;
    primitives.extend(connector_groups);

    Scene {
        primitives,
        canvas_bounds,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::genrep::Genrep;
    use crate::parser::{compute_scope, parse_str};
    use crate::preferences::Prefs;
    use crate::scene::{Primitive, TextAttr};

    const CONSANG_GED: &str = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME Common /Ancestor/\n1 SEX M\n\
0 @I2@ INDI\n1 NAME Father /Person/\n1 SEX M\n1 FAMS @F1@\n1 FAMC @F2@\n\
0 @I3@ INDI\n1 NAME Mother /Person/\n1 SEX F\n1 FAMS @F1@\n1 FAMC @F3@\n\
0 @I4@ INDI\n1 NAME Root /Person/\n1 SEX M\n1 FAMC @F1@\n\
0 @F1@ FAM\n1 HUSB @I2@\n1 WIFE @I3@\n1 CHIL @I4@\n\
0 @F2@ FAM\n1 HUSB @I1@\n1 CHIL @I2@\n\
0 @F3@ FAM\n1 HUSB @I1@\n1 CHIL @I3@\n\
0 TRLR\n";

    const SIMPLE_DESC_GED: &str = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME Root /Person/\n1 SEX M\n1 FAMS @F1@\n\
0 @I2@ INDI\n1 NAME Spouse /Person/\n1 SEX F\n1 FAMS @F1@\n\
0 @I3@ INDI\n1 NAME Child1 /Person/\n1 SEX M\n1 FAMC @F1@\n\
0 @I4@ INDI\n1 NAME Child2 /Person/\n1 SEX F\n1 FAMC @F1@\n\
0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I3@\n1 CHIL @I4@\n\
0 TRLR\n";

    fn parse_ged(ged: &str, direction: &str, root: &str) -> Genrep {
        let mut genrep = parse_str(ged).unwrap();
        compute_scope(&mut genrep, Some(root), direction, Some(10));
        genrep
    }

    fn make_prefs_anc() -> Prefs {
        let mut p = Prefs::default();
        p.scope.direction = "ancestors".to_string();
        p.scope.root = "I4".to_string();
        p.scope.generations = 10;
        p
    }

    fn make_prefs_desc() -> Prefs {
        let mut p = Prefs::default();
        p.scope.direction = "descendants".to_string();
        p.scope.root = "I1".to_string();
        p.scope.generations = 10;
        p
    }

    fn get_geo(result: &Genrep<BoxesGeo>, key: &str) -> BoxesIndividualGeo {
        result
            .individuals
            .get(key)
            .and_then(|i| i.geo.as_ref())
            .and_then(|g| {
                let BoxesGeo::Individual(geo) = g;
                Some(geo.clone())
            })
            .unwrap_or_else(|| panic!("individual {key} not found or has no geo"))
    }

    fn scene_has_attr(scene: &crate::scene::Scene, attr: &TextAttr) -> bool {
        fn check_primitives(prims: &[Primitive], attr: &TextAttr) -> bool {
            prims.iter().any(|p| match p {
                Primitive::Text(t) => t.attrs.contains(attr),
                Primitive::Group(g) => check_primitives(&g.children, attr),
                _ => false,
            })
        }
        check_primitives(&scene.primitives, attr)
    }

    fn count_box_primitives_in_group(prims: &[Primitive], target_id: &str) -> usize {
        for p in prims {
            if let Primitive::Group(g) = p {
                if g.id == target_id {
                    return g
                        .children
                        .iter()
                        .filter(|c| matches!(c, Primitive::Box(_)))
                        .count();
                }
                let n = count_box_primitives_in_group(&g.children, target_id);
                if n > 0 {
                    return n;
                }
            }
        }
        0
    }

    fn group_ids_in_scene(scene: &crate::scene::Scene) -> Vec<String> {
        fn collect(prims: &[Primitive], ids: &mut Vec<String>) {
            for p in prims {
                if let Primitive::Group(g) = p {
                    if !g.id.is_empty() {
                        ids.push(g.id.clone());
                    }
                    collect(&g.children, ids);
                }
            }
        }
        let mut ids = Vec::new();
        collect(&scene.primitives, &mut ids);
        ids
    }

    #[test]
    fn boxes_anc_place_basic_positions() {
        let prefs = make_prefs_anc();
        let raw = parse_ged(CONSANG_GED, "ancestors", "I4");
        let result = BoxesLayout.compute(&raw, &prefs).unwrap();

        let g4 = get_geo(&result, "I4");
        let g2 = get_geo(&result, "I2");
        let g3 = get_geo(&result, "I3");

        assert_eq!(g4.generation, 0);
        assert_eq!(g2.generation, 1);
        assert_eq!(g3.generation, 1);

        // I4 should be centered between I2 and I3
        let mid = (g2.x + g3.x) / 2.0;
        assert!(
            (g4.x - mid).abs() < 1.0,
            "I4.x={} should equal midpoint of I2/I3={}",
            g4.x,
            mid
        );

        // Both I1 instances should be at generation 2
        let g1_first = get_geo(&result, "I1");
        let g1_dup = get_geo(&result, "I1##1");
        assert_eq!(g1_first.generation, 2);
        assert_eq!(g1_dup.generation, 2);
    }

    #[test]
    fn boxes_anc_consanguinity_instances() {
        let prefs = make_prefs_anc();
        let raw = parse_ged(CONSANG_GED, "ancestors", "I4");
        let result = BoxesLayout.compute(&raw, &prefs).unwrap();

        assert!(result.individuals.contains_key("I1"), "I1 should be placed");
        assert!(
            result.individuals.contains_key("I1##1"),
            "I1##1 should be placed"
        );

        let x1 = get_geo(&result, "I1").x;
        let x2 = get_geo(&result, "I1##1").x;
        assert!(
            (x1 - x2).abs() > 1.0,
            "duplicate instances should be at different x positions"
        );
    }

    #[test]
    fn boxes_anc_emit_no_marriage() {
        let mut prefs = make_prefs_anc();
        prefs.show.marriage = true;
        let raw = parse_ged(CONSANG_GED, "ancestors", "I4");
        let result = BoxesLayout.compute(&raw, &prefs).unwrap();
        let scene = emit_scene(&result, &prefs);

        assert!(
            !scene_has_attr(&scene, &TextAttr::MarriageData),
            "ancestors mode must not emit MarriageData"
        );
    }

    #[test]
    fn boxes_anc_emit_double_border() {
        let mut prefs = make_prefs_anc();
        prefs.show.duplicated_individual = true;
        let raw = parse_ged(CONSANG_GED, "ancestors", "I4");
        let result = BoxesLayout.compute(&raw, &prefs).unwrap();
        let scene = emit_scene(&result, &prefs);

        let n = count_box_primitives_in_group(&scene.primitives, "I1-dup-1");
        assert_eq!(n, 2, "duplicate box should have 2 BoxPrimitives, got {n}");
    }

    #[test]
    fn boxes_anc_emit_group_ids() {
        let prefs = make_prefs_anc();
        let raw = parse_ged(CONSANG_GED, "ancestors", "I4");
        let result = BoxesLayout.compute(&raw, &prefs).unwrap();
        let scene = emit_scene(&result, &prefs);

        let ids = group_ids_in_scene(&scene);
        for expected in &["I4", "I4-connectors", "I2", "I3"] {
            assert!(
                ids.contains(&expected.to_string()),
                "scene should contain group id {expected}"
            );
        }
    }

    #[test]
    fn boxes_desc_place_centering() {
        let prefs = make_prefs_desc();
        let raw = parse_ged(SIMPLE_DESC_GED, "descendants", "I1");
        let result = BoxesLayout.compute(&raw, &prefs).unwrap();

        let bx = &prefs.layout.boxes;
        let g1 = get_geo(&result, "I1");
        // Spouse I2 should be at x = g1.x + box_w + gap_w
        let expected_spouse_x = g1.x + bx.box_width + bx.gap_width;
        // Children I3 and I4 should be centered under spouse
        let g3 = get_geo(&result, "I3");
        let g4 = get_geo(&result, "I4");
        let children_center = (g3.x + g4.x) / 2.0;
        assert!(
            (children_center - expected_spouse_x).abs() < 2.0,
            "children should be centered under spouse; center={children_center:.1}, spouse_x={expected_spouse_x:.1}"
        );
    }

    #[test]
    fn boxes_desc_emit_spouse_connector() {
        let prefs = make_prefs_desc();
        let raw = parse_ged(SIMPLE_DESC_GED, "descendants", "I1");
        let result = BoxesLayout.compute(&raw, &prefs).unwrap();
        let scene = emit_scene(&result, &prefs);

        fn has_spouse_connector(prims: &[Primitive]) -> bool {
            prims.iter().any(|p| match p {
                Primitive::BoxesSpouseConnector(_) => true,
                Primitive::Group(g) => has_spouse_connector(&g.children),
                _ => false,
            })
        }
        assert!(
            has_spouse_connector(&scene.primitives),
            "scene should contain a BoxesSpouseConnector"
        );
    }

    #[test]
    fn boxes_desc_emit_no_marriage() {
        let mut prefs = make_prefs_desc();
        prefs.show.marriage = true;
        let raw = parse_ged(SIMPLE_DESC_GED, "descendants", "I1");
        let result = BoxesLayout.compute(&raw, &prefs).unwrap();
        let scene = emit_scene(&result, &prefs);

        assert!(
            !scene_has_attr(&scene, &TextAttr::MarriageData),
            "descendants mode must not emit MarriageData"
        );
    }

    #[test]
    fn boxes_compute_photo_section_increases_height() {
        let raw = parse_ged(SIMPLE_DESC_GED, "descendants", "I1");

        let mut no_photo = make_prefs_desc();
        no_photo.show.photo = false;
        let base_h = no_photo.layout.boxes.box_height;
        let r = BoxesLayout.compute(&raw, &no_photo).unwrap();
        assert_eq!(
            get_geo(&r, "I1").height,
            base_h,
            "without photo, height should equal box_height"
        );

        let mut with_photo = make_prefs_desc();
        with_photo.show.photo = true;
        with_photo.photos.box_resize = true;
        with_photo.photos.height = 80.0;
        with_photo.photos.margin = 4.0;
        let r2 = BoxesLayout.compute(&raw, &with_photo).unwrap();
        let expected = base_h + 80.0 + 2.0 * 4.0;
        assert_eq!(
            get_geo(&r2, "I1").height,
            expected,
            "with photo+box_resize, height should equal box_height + photo_section_h"
        );
    }

    #[test]
    fn boxes_emit_scene_placeholder_when_no_photo() {
        let raw = parse_ged(SIMPLE_DESC_GED, "descendants", "I1");
        let mut prefs = make_prefs_desc();
        prefs.show.photo = true;
        prefs.photos.box_resize = true;
        // No gedcom_path → photo_map is empty → every individual gets a placeholder
        let result = BoxesLayout.compute(&raw, &prefs).unwrap();
        let scene = emit_scene(&result, &prefs);

        fn count_filled_rects(prims: &[Primitive]) -> usize {
            prims
                .iter()
                .map(|p| match p {
                    Primitive::FilledRect(_) => 1,
                    Primitive::Group(g) => count_filled_rects(&g.children),
                    _ => 0,
                })
                .sum()
        }
        fn count_images(prims: &[Primitive]) -> usize {
            prims
                .iter()
                .map(|p| match p {
                    Primitive::Image(_) => 1,
                    Primitive::Group(g) => count_images(&g.children),
                    _ => 0,
                })
                .sum()
        }
        assert!(
            count_filled_rects(&scene.primitives) > 0,
            "should have placeholders when no photos found"
        );
        assert_eq!(
            count_images(&scene.primitives),
            0,
            "should have no Image primitives when photo_map is empty"
        );
    }

    #[test]
    fn boxes_emit_scene_no_crash_no_images_when_photo_map_empty() {
        let raw = parse_ged(SIMPLE_DESC_GED, "descendants", "I1");
        let mut prefs = make_prefs_desc();
        prefs.show.photo = true;
        prefs.photos.box_resize = true;
        // gedcom_path is empty → no photos directory → photo_map will be empty
        let result = BoxesLayout.compute(&raw, &prefs).unwrap();
        let scene = emit_scene(&result, &prefs);

        fn has_image(prims: &[Primitive]) -> bool {
            prims.iter().any(|p| match p {
                Primitive::Image(_) => true,
                Primitive::Group(g) => has_image(&g.children),
                _ => false,
            })
        }
        assert!(
            !has_image(&scene.primitives),
            "no photos found should produce no Image primitives"
        );
    }
}
