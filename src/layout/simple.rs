//! Text-like layout: descendants, ancestors, forest (stub).

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

fn matches_direction(input: &str, canonical: &str) -> bool {
    !input.is_empty() && canonical.starts_with(input)
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
    spacing: usize,
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
    *line += 1 + spacing;

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
                        *line += 1 + spacing;
                    }
                }
            }
        }

        let children = fam.children_ids.clone();
        for child_id in &children {
            visit(child_id, depth + 1, spacing, line, geo_map, visited, genrep);
        }
    }
}

fn layout_descendants(genrep: &Genrep, root: &str, spacing: usize, geo_map: &mut HashMap<String, SimpleGeo>) {
    let mut visited: HashSet<String> = HashSet::new();
    let mut line: usize = 0;
    visit(root, 0, spacing, &mut line, geo_map, &mut visited, genrep);
}

fn in_order(
    id: &str,
    depth: usize,
    genrep: &Genrep,
    visited: &mut HashSet<String>,
    ordered: &mut Vec<(String, usize)>,
) {
    if visited.contains(id) {
        return;
    }
    visited.insert(id.to_string());

    let Some(indi) = genrep.individuals.get(id) else { return };
    if !indi.in_scope {
        return;
    }

    let parents = indi.famc.first()
        .and_then(|fam_id| genrep.families.get(fam_id));

    let father_id = parents.and_then(|f| f.husband_id.as_deref());
    let mother_id = parents.and_then(|f| f.wife_id.as_deref());

    if let Some(fid) = father_id {
        in_order(fid, depth + 1, genrep, visited, ordered);
    }

    ordered.push((id.to_string(), depth));

    if let Some(mid) = mother_id {
        in_order(mid, depth + 1, genrep, visited, ordered);
    }
}

fn layout_ancestors(genrep: &Genrep, root: &str, spacing: usize, geo_map: &mut HashMap<String, SimpleGeo>) {
    let mut visited = HashSet::new();
    let mut ordered: Vec<(String, usize)> = Vec::new();
    in_order(root, 0, genrep, &mut visited, &mut ordered);

    // First pass: assign line numbers, expanding gaps by vert_spacing
    let mut id_to_line: HashMap<String, usize> = HashMap::new();
    for (seq, (id, depth)) in ordered.iter().enumerate() {
        let line_num = seq * (1 + spacing);
        id_to_line.insert(id.clone(), line_num);
        geo_map.insert(
            id.clone(),
            SimpleGeo {
                line: line_num,
                indent: *depth,
                generation: depth + 1,
                is_spouse: false,
                connectors_above: Vec::new(),
                connectors_below: Vec::new(),
            },
        );
    }

    // Second pass: compute connectors
    for (id, _depth) in &ordered {
        let Some(indi) = genrep.individuals.get(id.as_str()) else { continue };
        let self_line = id_to_line[id.as_str()];

        let parents = indi.famc.first()
            .and_then(|fam_id| genrep.families.get(fam_id));

        if let Some(fam) = parents {
            if let Some(fid) = &fam.husband_id {
                if let Some(&father_line) = id_to_line.get(fid.as_str()) {
                    let above: Vec<usize> = (father_line + 1..self_line).collect();
                    if let Some(geo) = geo_map.get_mut(id.as_str()) {
                        geo.connectors_above = above;
                    }
                }
            }
            if let Some(mid) = &fam.wife_id {
                if let Some(&mother_line) = id_to_line.get(mid.as_str()) {
                    let below: Vec<usize> = (self_line + 1..mother_line).collect();
                    if let Some(geo) = geo_map.get_mut(id.as_str()) {
                        geo.connectors_below = below;
                    }
                }
            }
        }
    }
}

pub struct SimpleLayout;

impl Layout for SimpleLayout {
    type Geo = SimpleGeo;

    fn compute(&self, genrep: &Genrep, prefs: &Prefs) -> Result<Genrep<SimpleGeo>> {
        let dir = prefs.scope.direction.as_str();
        let mut geo_map: HashMap<String, SimpleGeo> = HashMap::new();

        let spacing = prefs.layout.simple.vert_spacing as usize;
        match dir {
            d if matches_direction(d, "descendants") => {
                if let Some(root) = root_id(genrep, prefs) {
                    layout_descendants(genrep, &root, spacing, &mut geo_map);
                }
            }
            d if matches_direction(d, "ancestors") || matches_direction(d, "pedigree") => {
                if let Some(root) = root_id(genrep, prefs) {
                    layout_ancestors(genrep, &root, spacing, &mut geo_map);
                }
            }
            d if matches_direction(d, "forest") => {
                eprintln!(
                    "warning: forest direction is not yet implemented; output will be empty"
                );
            }
            other => {
                eprintln!("warning: unknown direction {other:?}, falling back to descendants");
                if let Some(root) = root_id(genrep, prefs) {
                    layout_descendants(genrep, &root, spacing, &mut geo_map);
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
                    alt_name: indi.alt_name.clone(),
                    name_heb: indi.name_heb.clone(),
                    living: indi.living,
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
                    jmar: fam.jmar.clone(),
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

    const GEDCOM_3GEN: &str = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME John /Ancestor/
1 SEX M
1 FAMS @F1@
1 FAMC @F2@
0 @I2@ INDI
1 NAME Jane /Ancestress/
1 SEX F
1 FAMS @F1@
0 @I3@ INDI
1 NAME Paul /Child/
1 SEX M
1 FAMC @F1@
0 @F1@ FAM
1 HUSB @I1@
1 WIFE @I2@
1 CHIL @I3@
0 @I4@ INDI
1 NAME Grandpa /Ancestor/
1 SEX M
1 FAMS @F2@
0 @I5@ INDI
1 NAME Grandma /Ancestor/
1 SEX F
1 FAMS @F2@
0 @F2@ FAM
1 HUSB @I4@
1 WIFE @I5@
1 CHIL @I1@
0 TRLR
";

    #[test]
    fn test_ancestors_two_generations() {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(2));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I3".to_string();
        prefs.scope.direction = "ancestors".to_string();

        let output = SimpleLayout.compute(&genrep, &prefs).unwrap();

        let i1 = output.individuals["I1"].geo.as_ref().unwrap();
        let i3 = output.individuals["I3"].geo.as_ref().unwrap();
        let i2 = output.individuals["I2"].geo.as_ref().unwrap();

        assert!(i1.line < i3.line, "father above root");
        assert!(i3.line < i2.line, "mother below root");
        assert_eq!(i1.indent, 1);
        assert_eq!(i3.indent, 0);
        assert_eq!(i2.indent, 1);
        assert_eq!(i1.generation, 2);
        assert_eq!(i3.generation, 1);
        assert_eq!(i2.generation, 2);
    }

    #[test]
    fn test_descendants_vert_spacing() {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".to_string();
        prefs.scope.direction = "descendants".to_string();
        prefs.layout.simple.vert_spacing = 1;

        let result = SimpleLayout.compute(&genrep, &prefs).unwrap();

        let i1_line = result.individuals["I1"].geo.as_ref().unwrap().line;
        let i2_line = result.individuals["I2"].geo.as_ref().unwrap().line;
        let i3_line = result.individuals["I3"].geo.as_ref().unwrap().line;
        assert_eq!(i2_line, i1_line + 2, "spouse should be 2 lines below root with spacing=1");
        assert_eq!(i3_line, i2_line + 2, "child should be 2 lines below spouse with spacing=1");
    }

    #[test]
    fn test_ancestors_vert_spacing() {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(2));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I3".to_string();
        prefs.scope.direction = "ancestors".to_string();
        prefs.layout.simple.vert_spacing = 1;

        let result = SimpleLayout.compute(&genrep, &prefs).unwrap();

        let father_line = result.individuals["I1"].geo.as_ref().unwrap().line;
        let root_line   = result.individuals["I3"].geo.as_ref().unwrap().line;
        let mother_line = result.individuals["I2"].geo.as_ref().unwrap().line;
        assert_eq!(root_line,   father_line + 2, "root should be 2 lines below father");
        assert_eq!(mother_line, root_line   + 2, "mother should be 2 lines below root");

        let root_geo = result.individuals["I3"].geo.as_ref().unwrap();
        assert!(root_geo.connectors_above.contains(&(father_line + 1)),
                "gap line between father and root must carry a connector");
    }

    #[test]
    fn test_ancestors_three_generations() {
        let mut genrep = parse_str(GEDCOM_3GEN).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(3));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I3".to_string();
        prefs.scope.direction = "ancestors".to_string();

        let output = SimpleLayout.compute(&genrep, &prefs).unwrap();

        assert_eq!(output.individuals["I4"].geo.as_ref().unwrap().line, 0);
        assert_eq!(output.individuals["I1"].geo.as_ref().unwrap().line, 1);
        assert_eq!(output.individuals["I5"].geo.as_ref().unwrap().line, 2);
        assert_eq!(output.individuals["I3"].geo.as_ref().unwrap().line, 3);
        assert_eq!(output.individuals["I2"].geo.as_ref().unwrap().line, 4);
    }
}
