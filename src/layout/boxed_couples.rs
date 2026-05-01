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
    let parent_id = fam.husband_id.as_deref().or(fam.wife_id.as_deref())?;
    let parent = out.get(parent_id)?;
    let geo = match &parent.geo {
        Some(BoxedCouplesGeo::Individual(g)) => g,
        _ => return None,
    };

    let has_spouse2 = geo.width > box_w + 1.0;

    let conn_out_y = geo.y + box_h / 2.0;
    let (conn_out1_x, conn_out2_x) = if has_spouse2 {
        (geo.x - (box_w2 / 2.0 - box_w), geo.x + (box_w2 / 2.0 - box_w))
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
                place_descendants(genrep, &children[0], &env_left[1..], generation + 1, box_w, box_h, box_w2, gap_w, gap_h, out);
                for i in 1..children.len() {
                    let right_env = get_right_envelope(&children[i - 1], genrep, out);
                    place_descendants(genrep, &children[i], &right_env, generation + 1, box_w, box_h, box_w2, gap_w, gap_h, out);
                }

                let n = children.len();
                if n % 2 == 1 {
                    get_x_of(&children[n / 2], out)
                } else {
                    (get_x_of(&children[n / 2 - 1], out) + get_x_of(&children[n / 2], out)) / 2.0
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
                place_descendants(genrep, &all_children[0], &env_left[1..], generation + 1, box_w, box_h, box_w2, gap_w, gap_h, out);
                for i in 1..all_children.len() {
                    let right_env = get_right_envelope(&all_children[i - 1], genrep, out);
                    place_descendants(genrep, &all_children[i], &right_env, generation + 1, box_w, box_h, box_w2, gap_w, gap_h, out);
                }

                let conn_out1_offset = -(box_w2 / 2.0 - box_w);
                let conn_out2_offset = box_w2 / 2.0 - box_w;

                if !children1.is_empty() {
                    get_x_of(children1.last().unwrap(), out) - conn_out1_offset
                } else {
                    get_x_of(children2.first().unwrap(), out) - conn_out2_offset
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
) {
    // TODO: implement true ancestors traversal (walk famc, place parents above child)
    place_descendants(genrep, ind_id, env_left, generation, box_w, box_h, box_w2, gap_w, gap_h, out);
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

        let mut individuals: HashMap<String, Individual<BoxedCouplesGeo>> = HashMap::new();

        if matches_direction(&dir, "ancestors") {
            place_ancestors(genrep, root_id, &env_left, 0, box_w, box_h, box_w2, gap_w, gap_h, &mut individuals);
        } else {
            place_descendants(genrep, root_id, &env_left, 0, box_w, box_h, box_w2, gap_w, gap_h, &mut individuals);
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

        let conn_out2_offset = box_w2 / 2.0 - box_w;
        assert!(
            (x_root + conn_out2_offset - x_child).abs() < 1e-6,
            "expected x_root({x_root}) + offset({conn_out2_offset}) == x_child({x_child})"
        );
    }
}
