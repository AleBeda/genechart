//! Half-circle pedigree fan layout.

use anyhow::{bail, Result};

use crate::util::matches_direction;
use std::collections::HashMap;
use std::f64::consts::PI;

use crate::parser::genrep::{Family, Genrep, Individual};
use crate::preferences::Prefs;
use super::Layout;

#[derive(Debug, Clone)]
pub struct FanGeo {
    pub angle_center: f64,
    pub angle_span: f64,
    pub radius_inner: f64,
    pub radius_outer: f64,
    pub x: f64,
    pub y: f64,
}

pub struct FanLayout;

impl Layout for FanLayout {
    type Geo = FanGeo;

    fn compute(&self, genrep: &Genrep, prefs: &Prefs) -> Result<Genrep<FanGeo>> {
        let dir = prefs.scope.direction.to_lowercase();
        if !matches_direction(&dir, "ancestors") && !matches_direction(&dir, "pedigree") {
            eprintln!("warning: fan layout requires direction=ancestors");
            bail!("fan layout requires direction=ancestors");
        }

        let root_id = if prefs.scope.root.is_empty() {
            genrep.first_individual_id.as_deref().unwrap_or("")
        } else {
            prefs.scope.root.as_str()
        };

        if root_id.is_empty() {
            return Ok(Genrep {
                individuals: HashMap::new(),
                families: copy_families(genrep),
                first_individual_id: genrep.first_individual_id.clone(),
            });
        }

        let ring_height = prefs.layout.fan.ring_height;
        let ring_gap = prefs.layout.fan.ring_gap;
        let max_gen = prefs.scope.generations;

        let mut individuals: HashMap<String, Individual<FanGeo>> = HashMap::new();

        if let Some(root) = genrep.get_individual(root_id) {
            let root_geo = FanGeo {
                angle_center: 90.0,
                angle_span: 180.0,
                radius_inner: 0.0,
                radius_outer: ring_height,
                x: 0.0,
                y: 0.0,
            };
            individuals.insert(root_id.to_string(), copy_individual(root, Some(root_geo)));

            place_ancestors(
                genrep, root_id, 90.0, 180.0, 0u32,
                ring_height, ring_gap, max_gen, &mut individuals,
            );
        }

        Ok(Genrep {
            individuals,
            families: copy_families(genrep),
            first_individual_id: genrep.first_individual_id.clone(),
        })
    }
}

fn place_ancestors(
    genrep: &Genrep,
    id: &str,
    angle_center: f64,
    angle_span: f64,
    depth: u32,
    ring_height: f64,
    ring_gap: f64,
    max_gen: u32,
    out: &mut HashMap<String, Individual<FanGeo>>,
) {
    if max_gen == 0 || depth + 1 >= max_gen {
        return;
    }

    let ind = match genrep.get_individual(id) {
        Some(i) => i,
        None => return,
    };

    let famc_id = match ind.famc.first() {
        Some(fid) => fid.clone(),
        None => return,
    };

    let fam = match genrep.get_family(&famc_id) {
        Some(f) => f,
        None => return,
    };

    let next_depth = depth + 1;
    let child_span = angle_span / 2.0;
    let radius_inner = next_depth as f64 * (ring_height + ring_gap);
    let radius_outer = radius_inner + ring_height;
    let radius_mid = (radius_inner + radius_outer) / 2.0;

    // Father: left side of chart = higher angle range → center at angle_center + angle_span/4
    let father_angle = angle_center + angle_span / 4.0;
    if let Some(father_id) = &fam.husband_id {
        if let Some(father) = genrep.get_individual(father_id) {
            if father.in_scope {
                let (x, y) = to_xy(radius_mid, father_angle);
                let geo = FanGeo {
                    angle_center: father_angle,
                    angle_span: child_span,
                    radius_inner,
                    radius_outer,
                    x,
                    y,
                };
                out.insert(father_id.clone(), copy_individual(father, Some(geo)));
                place_ancestors(
                    genrep, father_id, father_angle, child_span,
                    next_depth, ring_height, ring_gap, max_gen, out,
                );
            }
        }
    }

    // Mother: right side of chart = lower angle range → center at angle_center - angle_span/4
    let mother_angle = angle_center - angle_span / 4.0;
    if let Some(mother_id) = &fam.wife_id {
        if let Some(mother) = genrep.get_individual(mother_id) {
            if mother.in_scope {
                let (x, y) = to_xy(radius_mid, mother_angle);
                let geo = FanGeo {
                    angle_center: mother_angle,
                    angle_span: child_span,
                    radius_inner,
                    radius_outer,
                    x,
                    y,
                };
                out.insert(mother_id.clone(), copy_individual(mother, Some(geo)));
                place_ancestors(
                    genrep, mother_id, mother_angle, child_span,
                    next_depth, ring_height, ring_gap, max_gen, out,
                );
            }
        }
    }
}

fn to_xy(radius: f64, angle_deg: f64) -> (f64, f64) {
    let rad = angle_deg * PI / 180.0;
    (radius * rad.cos(), radius * rad.sin())
}

fn copy_individual(src: &Individual<()>, geo: Option<FanGeo>) -> Individual<FanGeo> {
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

fn copy_families(genrep: &Genrep) -> HashMap<String, Family<FanGeo>> {
    genrep.families.iter().map(|(id, fam)| {
        (id.clone(), Family {
            id: fam.id.clone(),
            husband_id: fam.husband_id.clone(),
            wife_id: fam.wife_id.clone(),
            children_ids: fam.children_ids.clone(),
            marriage: fam.marriage.clone(),
            jmar: fam.jmar.clone(),
            in_scope: fam.in_scope,
            geo: None,
        })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::genrep::{Family, Genrep, Individual};
    use crate::preferences::Prefs;

    fn make_individual(id: &str, famc: Vec<String>) -> Individual<()> {
        Individual {
            id: id.to_string(),
            given: None,
            surname: None,
            sex: None,
            birth: None,
            death: None,
            fams: vec![],
            famc,
            alt_name: None,
            name_heb: None,
            living: None,
            in_scope: true,
            geo: None,
        }
    }

    fn make_family(id: &str, husband: Option<&str>, wife: Option<&str>, child: &str) -> Family<()> {
        Family {
            id: id.to_string(),
            husband_id: husband.map(str::to_string),
            wife_id: wife.map(str::to_string),
            children_ids: vec![child.to_string()],
            marriage: None,
            jmar: None,
            in_scope: true,
            geo: None,
        }
    }

    fn test_genrep() -> Genrep {
        let mut individuals = HashMap::new();
        let mut families = HashMap::new();

        // I1 = root, I2 = father, I3 = mother, I4 = paternal grandfather, I5 = paternal grandmother
        individuals.insert("I1".to_string(), make_individual("I1", vec!["F1".to_string()]));
        individuals.insert("I2".to_string(), make_individual("I2", vec!["F2".to_string()]));
        individuals.insert("I3".to_string(), make_individual("I3", vec![]));
        individuals.insert("I4".to_string(), make_individual("I4", vec![]));
        individuals.insert("I5".to_string(), make_individual("I5", vec![]));

        families.insert("F1".to_string(), make_family("F1", Some("I2"), Some("I3"), "I1"));
        families.insert("F2".to_string(), make_family("F2", Some("I4"), Some("I5"), "I2"));

        Genrep {
            individuals,
            families,
            first_individual_id: Some("I1".to_string()),
        }
    }

    fn ancestors_prefs() -> Prefs {
        let mut prefs = Prefs::default();
        prefs.scope.direction = "ancestors".to_string();
        prefs.scope.root = "I1".to_string();
        prefs.scope.generations = 4;
        prefs.layout.fan.ring_height = 80.0;
        prefs.layout.fan.ring_gap = 20.0;
        prefs
    }

    #[test]
    fn root_placement() {
        let result = FanLayout.compute(&test_genrep(), &ancestors_prefs()).unwrap();
        let geo = result.individuals["I1"].geo.as_ref().unwrap();
        assert_eq!(geo.angle_center, 90.0);
        assert_eq!(geo.angle_span, 180.0);
        assert!(geo.x.abs() < 1e-10);
        assert!(geo.y.abs() < 1e-10);
    }

    #[test]
    fn father_arc() {
        let result = FanLayout.compute(&test_genrep(), &ancestors_prefs()).unwrap();
        let geo = result.individuals["I2"].geo.as_ref().unwrap();
        assert!((geo.angle_center - 135.0).abs() < 1e-10, "father angle_center={}", geo.angle_center);
        assert!((geo.angle_span - 90.0).abs() < 1e-10);
    }

    #[test]
    fn mother_arc() {
        let result = FanLayout.compute(&test_genrep(), &ancestors_prefs()).unwrap();
        let geo = result.individuals["I3"].geo.as_ref().unwrap();
        assert!((geo.angle_center - 45.0).abs() < 1e-10, "mother angle_center={}", geo.angle_center);
        assert!((geo.angle_span - 90.0).abs() < 1e-10);
    }

    #[test]
    fn paternal_grandfather_arc() {
        let result = FanLayout.compute(&test_genrep(), &ancestors_prefs()).unwrap();
        let geo = result.individuals["I4"].geo.as_ref().unwrap();
        assert!((geo.angle_center - 157.5).abs() < 1e-10, "paternal grandfather angle_center={}", geo.angle_center);
        assert!((geo.angle_span - 45.0).abs() < 1e-10);
    }

    #[test]
    fn no_overlap() {
        let result = FanLayout.compute(&test_genrep(), &ancestors_prefs()).unwrap();

        let fg = result.individuals["I2"].geo.as_ref().unwrap();
        let mg = result.individuals["I3"].geo.as_ref().unwrap();

        let father_min = fg.angle_center - fg.angle_span / 2.0; // 90
        let father_max = fg.angle_center + fg.angle_span / 2.0; // 180
        let mother_min = mg.angle_center - mg.angle_span / 2.0; // 0
        let mother_max = mg.angle_center + mg.angle_span / 2.0; // 90

        assert!(father_max <= 180.0 + 1e-10);
        assert!(mother_min >= -1e-10);
        // contiguous: father's lower edge meets mother's upper edge
        assert!((father_min - mother_max).abs() < 1e-10);
        // total span covers the full half-circle
        assert!((fg.angle_span + mg.angle_span - 180.0).abs() < 1e-10);
    }

    #[test]
    fn non_pedigree_direction_errors() {
        let mut prefs = ancestors_prefs();
        prefs.scope.direction = "descendants".to_string();
        assert!(FanLayout.compute(&test_genrep(), &prefs).is_err());
    }
}
