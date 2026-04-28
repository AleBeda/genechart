//! Text-like layout: descendants, ancestors (stub), forest (stub).

use anyhow::Result;
use crate::parser::genrep::{Family, Genrep, Individual};
use crate::preferences::Prefs;
use super::Layout;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub struct SimpleGeo {
    pub line: usize,
    pub indent: usize,
    pub generation: usize,
    pub is_spouse: bool,
    pub connectors_above: Vec<usize>,
    pub connectors_below: Vec<usize>,
}

fn root_id(genrep: &Genrep, prefs: &Prefs) -> Option<String> {
    let r = prefs.scope.root.trim();
    if !r.is_empty() {
        if genrep.individuals.contains_key(r) {
            Some(r.to_string())
        } else {
            eprintln!("warning: root '{r}' not found, falling back to first individual");
            genrep.first_individual_id.clone()
        }
    } else {
        genrep.first_individual_id.clone()
    }
}

fn visit(
    id: &str,
    depth: usize,
    line: &mut usize,
    geo_map: &mut HashMap<String, SimpleGeo>,
    visited: &mut HashSet<String>,
    genrep: &Genrep,
) {
    if visited.contains(id) {
        return;
    }
    visited.insert(id.to_string());

    let indi = match genrep.individuals.get(id) {
        Some(i) => i,
        None => return,
    };

    if !indi.in_scope {
        return;
    }

    geo_map.insert(
        id.to_string(),
        SimpleGeo {
            line: *line,
            indent: depth,
            generation: depth + 1,
            is_spouse: false,
            ..Default::default()
        },
    );
    *line += 1;

    let fams = indi.fams.clone();

    for fam_id in &fams {
        let fam = match genrep.families.get(fam_id) {
            Some(f) => f,
            None => continue,
        };

        if !fam.in_scope {
            continue;
        }

        let spouse_id: Option<String> = if fam.husband_id.as_deref() == Some(id) {
            fam.wife_id.clone()
        } else if fam.wife_id.as_deref() == Some(id) {
            fam.husband_id.clone()
        } else {
            None
        };

        if let Some(ref sid) = spouse_id {
            if !visited.contains(sid.as_str()) {
                if let Some(s) = genrep.individuals.get(sid.as_str()) {
                    if s.in_scope {
                        visited.insert(sid.clone());
                        geo_map.insert(
                            sid.clone(),
                            SimpleGeo {
                                line: *line,
                                indent: depth,
                                generation: depth + 1,
                                is_spouse: true,
                                ..Default::default()
                            },
                        );
                        *line += 1;
                    }
                }
            }
        }

        let children = fam.children_ids.clone();
        for child_id in &children {
            visit(child_id, depth + 1, line, geo_map, visited, genrep);
        }
    }
}

fn layout_descendants(genrep: &Genrep, root: &str, geo_map: &mut HashMap<String, SimpleGeo>) {
    let mut visited: HashSet<String> = HashSet::new();
    let mut line: usize = 0;
    visit(root, 0, &mut line, geo_map, &mut visited, genrep);
}

pub struct SimpleLayout;

impl Layout for SimpleLayout {
    type Geo = SimpleGeo;

    fn compute(&self, genrep: &Genrep, prefs: &Prefs) -> Result<Genrep<SimpleGeo>> {
        let dir = prefs.scope.direction.as_str();
        let mut geo_map: HashMap<String, SimpleGeo> = HashMap::new();

        match dir {
            d if d.starts_with("desc") => {
                if let Some(root) = root_id(genrep, prefs) {
                    layout_descendants(genrep, &root, &mut geo_map);
                }
            }
            d if d.starts_with("anc") || d.starts_with("ped") => {
                eprintln!("warning: ancestors direction not yet implemented in simple layout");
            }
            d if d.starts_with("for") => {
                eprintln!(
                    "warning: forest direction is not yet implemented; output will be empty"
                );
            }
            other => {
                eprintln!("warning: unknown direction {other:?}, falling back to descendants");
                if let Some(root) = root_id(genrep, prefs) {
                    layout_descendants(genrep, &root, &mut geo_map);
                }
            }
        }

        let mut out_individuals = HashMap::new();
        for (id, indi) in &genrep.individuals {
            out_individuals.insert(
                id.clone(),
                Individual {
                    id: indi.id.clone(),
                    given: indi.given.clone(),
                    surname: indi.surname.clone(),
                    sex: indi.sex,
                    birth: indi.birth.clone(),
                    death: indi.death.clone(),
                    fams: indi.fams.clone(),
                    famc: indi.famc.clone(),
                    in_scope: indi.in_scope,
                    geo: geo_map.get(id).cloned(),
                },
            );
        }

        let mut out_families = HashMap::new();
        for (id, fam) in &genrep.families {
            out_families.insert(
                id.clone(),
                Family {
                    id: fam.id.clone(),
                    husband_id: fam.husband_id.clone(),
                    wife_id: fam.wife_id.clone(),
                    children_ids: fam.children_ids.clone(),
                    marriage: fam.marriage.clone(),
                    in_scope: fam.in_scope,
                    geo: None,
                },
            );
        }

        Ok(Genrep {
            individuals: out_individuals,
            families: out_families,
            first_individual_id: genrep.first_individual_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{compute_scope, parse_str};
    use crate::preferences::Prefs;

    const GEDCOM: &str = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME John /Ancestor/
1 SEX M
1 BIRT
2 DATE 1 JAN 1812
2 PLAC London
1 FAMS @F1@
0 @I2@ INDI
1 NAME Jane /Ancestress/
1 SEX F
1 FAMS @F1@
0 @I3@ INDI
1 NAME Paul /Ancestor/
1 SEX M
1 FAMC @F1@
0 @F1@ FAM
1 HUSB @I1@
1 WIFE @I2@
1 CHIL @I3@
1 MARR
2 DATE 4 APR 1843
2 PLAC London
0 TRLR
";

    #[test]
    fn test_descendants_two_generations() {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".to_string();
        prefs.scope.direction = "descendants".to_string();

        let result = SimpleLayout.compute(&genrep, &prefs).unwrap();

        let i1_geo = result.individuals["I1"].geo.as_ref().unwrap();
        assert_eq!(i1_geo.line, 0);
        assert_eq!(i1_geo.indent, 0);
        assert_eq!(i1_geo.generation, 1);
        assert!(!i1_geo.is_spouse);

        let i2_geo = result.individuals["I2"].geo.as_ref().unwrap();
        assert_eq!(i2_geo.line, 1);
        assert_eq!(i2_geo.indent, 0);
        assert_eq!(i2_geo.generation, 1);
        assert!(i2_geo.is_spouse);

        let i3_geo = result.individuals["I3"].geo.as_ref().unwrap();
        assert_eq!(i3_geo.line, 2);
        assert_eq!(i3_geo.indent, 1);
        assert_eq!(i3_geo.generation, 2);
        assert!(!i3_geo.is_spouse);
    }

    #[test]
    fn test_forest_direction_no_panic() {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, None, "forest", None);

        let mut prefs = Prefs::default();
        prefs.scope.direction = "forest".to_string();

        let result = SimpleLayout.compute(&genrep, &prefs);
        assert!(result.is_ok());
    }
}
