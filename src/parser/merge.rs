use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use super::genrep::Genrep;

/// Parse alias file text into a map: further_gedcom_id → main_gedcom_id.
///
/// Format: two whitespace-separated ID fields per line; lines starting with
/// `#` or blank lines are ignored; anything beyond the second field is ignored.
/// IDs must be given without `@` delimiters (e.g. `I123`, `F456`).
pub(crate) fn parse_alias_content(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_ascii_whitespace();
        let main_id = match fields.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let further_id = match fields.next() {
            Some(s) => s.to_string(),
            None => {
                eprintln!("warning: alias file line ignored (only one field): {line}");
                continue;
            }
        };
        map.insert(further_id, main_id);
    }
    map
}

/// Read an alias file from disk and return a map: further_gedcom_id → main_gedcom_id.
pub fn read_alias_file(path: &Path) -> Result<HashMap<String, String>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read alias file: {}", path.display()))?;
    Ok(parse_alias_content(&content))
}

/// Insert a prefix letter after the first character of an ID.
/// `"I123"` with prefix `'B'` → `"IB123"`. Works for any non-empty string.
fn prefix_id(id: &str, prefix: char) -> String {
    let mut chars = id.chars();
    match chars.next() {
        None => prefix.to_string(),
        Some(first) => format!("{first}{prefix}{}", chars.as_str()),
    }
}

/// Remap a single ID: look up in the alias map first; otherwise insert the prefix letter.
fn remap_id(id: &str, alias: &HashMap<String, String>, prefix: char) -> String {
    alias
        .get(id)
        .cloned()
        .unwrap_or_else(|| prefix_id(id, prefix))
}

/// Remap all IDs in a further `Genrep` — both the record's own ID and all cross-references.
///
/// Returns the remapped `Genrep` and a map of `remapped_id → original_further_id` for
/// individuals (used to reconstruct the original ID in warning messages).
pub fn remap_genrep(
    genrep: Genrep,
    alias: &HashMap<String, String>,
    prefix: char,
) -> (Genrep, HashMap<String, String>) {
    let remap = |id: &str| remap_id(id, alias, prefix);
    let mut orig_ids: HashMap<String, String> = HashMap::new();

    let mut new_individuals = HashMap::new();
    for (_, mut ind) in genrep.individuals {
        let orig_id = ind.id.clone();
        let new_id = remap(&ind.id);
        orig_ids.insert(new_id.clone(), orig_id);
        ind.id = new_id.clone();
        ind.fams = ind.fams.iter().map(|id| remap(id)).collect();
        ind.famc = ind.famc.iter().map(|id| remap(id)).collect();
        new_individuals.insert(new_id, ind);
    }

    let mut new_families = HashMap::new();
    for (_, mut fam) in genrep.families {
        let new_id = remap(&fam.id);
        fam.id = new_id.clone();
        fam.husband_id = fam.husband_id.as_deref().map(remap);
        fam.wife_id = fam.wife_id.as_deref().map(remap);
        fam.children_ids = fam.children_ids.iter().map(|id| remap(id)).collect();
        new_families.insert(new_id, fam);
    }

    (
        Genrep {
            individuals: new_individuals,
            families: new_families,
            first_individual_id: genrep.first_individual_id,
        },
        orig_ids,
    )
}

/// Merge a remapped further `Genrep` into the main `Genrep`.
///
/// - **Aliased** individuals/families (ID already present in `main`): gaps in basic
///   fields are filled from `further`; `fams`/`famc`/`children_ids` and `notes` are
///   unioned. The main GEDCOM's non-`None` fields are never overwritten. A warning is
///   emitted to stderr when both records have a name but the names differ.
/// - **New** individuals/families: inserted directly into `main`.
pub fn merge_into(
    main: &mut Genrep,
    further: Genrep,
    main_filename: &str,
    further_filename: &str,
    orig_ids: &HashMap<String, String>,
) {
    for (id, fi) in further.individuals {
        if let Some(mi) = main.individuals.get_mut(&id) {
            // Name mismatch warning
            if let (Some(mg), Some(fg)) = (&mi.given, &fi.given) {
                let ms = mi.surname.as_deref().unwrap_or("");
                let fs = fi.surname.as_deref().unwrap_or("");
                if mg.trim() != fg.trim() || ms.trim() != fs.trim() {
                    let orig_id = orig_ids.get(&id).map(|s| s.as_str()).unwrap_or(&id);
                    eprintln!(
                        "warning: name mismatch: {main_filename} {id}=\"{mg} {ms}\", \
                         {further_filename} {orig_id}=\"{fg} {fs}\""
                    );
                }
            }
            // Fill gaps in basic fields
            if mi.given.is_none() {
                mi.given = fi.given;
            }
            if mi.surname.is_none() {
                mi.surname = fi.surname;
            }
            if mi.sex.is_none() {
                mi.sex = fi.sex;
            }
            if mi.birth.is_none() {
                mi.birth = fi.birth;
            }
            if mi.death.is_none() {
                mi.death = fi.death;
            }
            if mi.alt_name.is_none() {
                mi.alt_name = fi.alt_name;
            }
            if mi.name_heb.is_none() {
                mi.name_heb = fi.name_heb;
            }
            if mi.living.is_none() {
                mi.living = fi.living;
            }
            // Union family links (deduplicated)
            for fid in fi.fams {
                if !mi.fams.contains(&fid) {
                    mi.fams.push(fid);
                }
            }
            for fid in fi.famc {
                if !mi.famc.contains(&fid) {
                    mi.famc.push(fid);
                }
            }
            // Union notes
            mi.notes.extend(fi.notes);
        } else {
            main.individuals.insert(id, fi);
        }
    }

    for (id, ff) in further.families {
        if let Some(mf) = main.families.get_mut(&id) {
            if mf.husband_id.is_none() {
                mf.husband_id = ff.husband_id;
            }
            if mf.wife_id.is_none() {
                mf.wife_id = ff.wife_id;
            }
            if mf.marriage.is_none() {
                mf.marriage = ff.marriage;
            }
            if mf.jmar.is_none() {
                mf.jmar = ff.jmar;
            }
            for cid in ff.children_ids {
                if !mf.children_ids.contains(&cid) {
                    mf.children_ids.push(cid);
                }
            }
            mf.notes.extend(ff.notes);
        } else {
            main.families.insert(id, ff);
        }
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::genrep::{Event, Family, GedDate, Individual};
    use super::*;

    fn make_individual(id: &str, given: Option<&str>, surname: Option<&str>) -> Individual<()> {
        Individual {
            id: id.to_string(),
            given: given.map(|s| s.to_string()),
            surname: surname.map(|s| s.to_string()),
            sex: None,
            birth: None,
            death: None,
            fams: Vec::new(),
            famc: Vec::new(),
            alt_name: None,
            name_heb: None,
            living: None,
            notes: Vec::new(),
            in_scope: false,
            geo: None,
        }
    }

    fn make_family(id: &str) -> Family<()> {
        Family {
            id: id.to_string(),
            husband_id: None,
            wife_id: None,
            children_ids: Vec::new(),
            marriage: None,
            jmar: None,
            notes: Vec::new(),
            in_scope: false,
            geo: None,
        }
    }

    fn make_genrep(individuals: Vec<Individual<()>>, families: Vec<Family<()>>) -> Genrep {
        let first = individuals.first().map(|i| i.id.clone());
        Genrep {
            individuals: individuals.into_iter().map(|i| (i.id.clone(), i)).collect(),
            families: families.into_iter().map(|f| (f.id.clone(), f)).collect(),
            first_individual_id: first,
        }
    }

    // ── parse_alias_content ───────────────────────────────────────────────────

    #[test]
    fn alias_content_basic() {
        let content = "I1  I99  John Smith\nF1  F55\n";
        let map = parse_alias_content(content);
        assert_eq!(map.get("I99").map(|s| s.as_str()), Some("I1"));
        assert_eq!(map.get("F55").map(|s| s.as_str()), Some("F1"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn alias_content_ignores_comments_and_blank_lines() {
        let content = "# This is a comment\n\nI1  I99\n  # indented comment\n";
        let map = parse_alias_content(content);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("I99").map(|s| s.as_str()), Some("I1"));
    }

    #[test]
    fn alias_content_single_field_line_is_skipped() {
        let content = "I1\nI2  I88\n";
        let map = parse_alias_content(content);
        // Single-field line is skipped with a warning; only the valid line is kept.
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("I88").map(|s| s.as_str()), Some("I2"));
    }

    // ── prefix_id ─────────────────────────────────────────────────────────────

    #[test]
    fn prefix_id_standard() {
        assert_eq!(prefix_id("I123", 'B'), "IB123");
        assert_eq!(prefix_id("F456", 'C'), "FC456");
    }

    #[test]
    fn prefix_id_single_char() {
        assert_eq!(prefix_id("I", 'B'), "IB");
    }

    #[test]
    fn prefix_id_empty_string() {
        assert_eq!(prefix_id("", 'B'), "B");
    }

    // ── remap_id ──────────────────────────────────────────────────────────────

    #[test]
    fn remap_id_aliased() {
        let mut alias = HashMap::new();
        alias.insert("I99".to_string(), "I1".to_string());
        assert_eq!(remap_id("I99", &alias, 'B'), "I1");
    }

    #[test]
    fn remap_id_not_aliased() {
        let alias = HashMap::new();
        assert_eq!(remap_id("I5", &alias, 'B'), "IB5");
    }

    // ── remap_genrep ──────────────────────────────────────────────────────────

    #[test]
    fn remap_genrep_remaps_all_cross_references() {
        let mut alias = HashMap::new();
        alias.insert("I99".to_string(), "I1".to_string()); // I99 in further → I1 in main

        let mut ind = make_individual("I99", Some("John"), Some("Smith"));
        ind.fams = vec!["F55".to_string()];
        ind.famc = vec!["F10".to_string()];

        let mut fam = make_family("F55");
        fam.husband_id = Some("I99".to_string());
        fam.wife_id = Some("I100".to_string());
        fam.children_ids = vec!["I101".to_string()];

        let genrep = make_genrep(vec![ind], vec![fam]);
        let (remapped, _) = remap_genrep(genrep, &alias, 'B');

        // I99 → I1 (aliased)
        let i1 = remapped
            .individuals
            .get("I1")
            .expect("I1 missing after remap");
        assert_eq!(i1.id, "I1");
        assert_eq!(i1.fams, vec!["FB55"]); // F55 → FB55 (not aliased)
        assert_eq!(i1.famc, vec!["FB10"]); // F10 → FB10

        // F55 → FB55
        let fb55 = remapped
            .families
            .get("FB55")
            .expect("FB55 missing after remap");
        assert_eq!(fb55.id, "FB55");
        assert_eq!(fb55.husband_id.as_deref(), Some("I1")); // I99 → I1
        assert_eq!(fb55.wife_id.as_deref(), Some("IB100")); // I100 → IB100
        assert_eq!(fb55.children_ids, vec!["IB101"]); // I101 → IB101
    }

    // ── merge_into ────────────────────────────────────────────────────────────

    #[test]
    fn merge_into_new_individual_inserted() {
        let mut main = make_genrep(
            vec![make_individual("I1", Some("John"), Some("Smith"))],
            vec![],
        );
        let further = make_genrep(
            vec![make_individual("IB2", Some("Jane"), Some("Doe"))],
            vec![],
        );
        merge_into(
            &mut main,
            further,
            "main.ged",
            "further.ged",
            &HashMap::new(),
        );
        assert!(
            main.individuals.contains_key("IB2"),
            "IB2 should be inserted"
        );
        assert_eq!(main.individuals.len(), 2);
    }

    #[test]
    fn merge_into_main_wins_for_existing_fields() {
        let main_ind = make_individual("I1", Some("John"), Some("Main"));
        let further_ind = make_individual("I1", Some("John"), Some("Further"));

        let mut main = make_genrep(vec![main_ind], vec![]);
        let further = make_genrep(vec![further_ind], vec![]);
        merge_into(
            &mut main,
            further,
            "main.ged",
            "further.ged",
            &HashMap::new(),
        );

        let i1 = main.individuals.get("I1").unwrap();
        assert_eq!(i1.surname.as_deref(), Some("Main"), "main surname must win");
    }

    #[test]
    fn merge_into_fills_gaps_from_further() {
        let mut main_ind = make_individual("I1", None, None); // no birth
        main_ind.given = Some("John".to_string());
        // surname is None in main

        let mut further_ind = make_individual("I1", Some("John"), Some("Smith"));
        further_ind.birth = Some(Event {
            date: Some(GedDate {
                raw: "1 JAN 1900".to_string(),
            }),
            place: Some("Paris".to_string()),
        });

        let mut main = make_genrep(vec![main_ind], vec![]);
        let further = make_genrep(vec![further_ind], vec![]);
        merge_into(
            &mut main,
            further,
            "main.ged",
            "further.ged",
            &HashMap::new(),
        );

        let i1 = main.individuals.get("I1").unwrap();
        assert_eq!(
            i1.surname.as_deref(),
            Some("Smith"),
            "gap filled from further"
        );
        assert!(i1.birth.is_some(), "birth filled from further");
    }

    #[test]
    fn merge_into_unions_family_links() {
        let mut main_ind = make_individual("I1", Some("John"), Some("Smith"));
        main_ind.fams = vec!["F1".to_string()];

        let mut further_ind = make_individual("I1", Some("John"), Some("Smith"));
        further_ind.fams = vec!["FB2".to_string()]; // new family link from further
        further_ind.famc = vec!["FB10".to_string()];

        let mut main = make_genrep(vec![main_ind], vec![]);
        let further = make_genrep(vec![further_ind], vec![]);
        merge_into(
            &mut main,
            further,
            "main.ged",
            "further.ged",
            &HashMap::new(),
        );

        let i1 = main.individuals.get("I1").unwrap();
        assert!(i1.fams.contains(&"F1".to_string()), "F1 preserved");
        assert!(
            i1.fams.contains(&"FB2".to_string()),
            "FB2 added from further"
        );
        assert!(
            i1.famc.contains(&"FB10".to_string()),
            "FB10 added from further"
        );
    }

    #[test]
    fn merge_into_no_duplicate_family_links() {
        let mut main_ind = make_individual("I1", Some("John"), Some("Smith"));
        main_ind.fams = vec!["F1".to_string()];

        let mut further_ind = make_individual("I1", Some("John"), Some("Smith"));
        further_ind.fams = vec!["F1".to_string()]; // same link, must not duplicate

        let mut main = make_genrep(vec![main_ind], vec![]);
        let further = make_genrep(vec![further_ind], vec![]);
        merge_into(
            &mut main,
            further,
            "main.ged",
            "further.ged",
            &HashMap::new(),
        );

        let i1 = main.individuals.get("I1").unwrap();
        assert_eq!(i1.fams.len(), 1, "F1 must not be duplicated");
    }

    #[test]
    fn merge_into_unions_notes() {
        let mut main_ind = make_individual("I1", Some("John"), Some("Smith"));
        main_ind.notes = vec!["note from main".to_string()];

        let mut further_ind = make_individual("I1", Some("John"), Some("Smith"));
        further_ind.notes = vec!["note from further".to_string()];

        let mut main = make_genrep(vec![main_ind], vec![]);
        let further = make_genrep(vec![further_ind], vec![]);
        merge_into(
            &mut main,
            further,
            "main.ged",
            "further.ged",
            &HashMap::new(),
        );

        let i1 = main.individuals.get("I1").unwrap();
        assert_eq!(i1.notes.len(), 2);
        assert!(i1.notes.contains(&"note from main".to_string()));
        assert!(i1.notes.contains(&"note from further".to_string()));
    }

    #[test]
    fn merge_into_family_gap_filling_and_children_union() {
        let mut main_fam = make_family("F1");
        main_fam.husband_id = Some("I1".to_string());
        // wife_id and children are missing in main

        let mut further_fam = make_family("F1");
        further_fam.wife_id = Some("I2".to_string());
        further_fam.children_ids = vec!["I3".to_string(), "I4".to_string()];

        let mut main = make_genrep(vec![], vec![main_fam]);
        let further = make_genrep(vec![], vec![further_fam]);
        merge_into(
            &mut main,
            further,
            "main.ged",
            "further.ged",
            &HashMap::new(),
        );

        let f1 = main.families.get("F1").unwrap();
        assert_eq!(f1.husband_id.as_deref(), Some("I1"), "husband preserved");
        assert_eq!(
            f1.wife_id.as_deref(),
            Some("I2"),
            "wife filled from further"
        );
        assert_eq!(f1.children_ids.len(), 2, "both children present");
    }
}
