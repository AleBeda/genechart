//! Recursive box-placement layout for couples with envelope-based spacing.

use anyhow::Result;
use crate::parser::genrep::{Genrep, Individual, Family};
use crate::preferences::Prefs;
use super::Layout;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct IndividualGeo {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub conn_in_x: f64,
    pub conn_in_y: f64,
}

#[derive(Debug, Clone)]
pub struct FamilyGeo {
    pub conn_out1_x: f64,
    pub conn_out1_y: f64,
    pub conn_out2_x: f64,
    pub conn_out2_y: f64,
    pub has_spouse2: bool,
}

#[derive(Debug, Clone)]
pub enum BoxedCouplesGeo {
    Individual(IndividualGeo),
    Family(FamilyGeo),
}

fn matches_direction(input: &str, canonical: &str) -> bool {
    !input.is_empty() && canonical.starts_with(input)
}

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

fn get_x_of(id: &str, out: &HashMap<String, Individual<BoxedCouplesGeo>>) -> f64 {
    match out.get(id).and_then(|i| i.geo.as_ref()) {
        Some(BoxedCouplesGeo::Individual(g)) => g.x,
        _ => panic!("get_x_of: individual {id:?} not yet placed — this is a bug"),
    }
}

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
}
