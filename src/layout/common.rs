//! Shared utilities for layout algorithms.

use std::collections::HashMap;
use crate::parser::genrep::{Family, Genrep, Individual};
use crate::preferences::Prefs;

/// Parse a GEDCOM raw date string into a sortable `(year, month, day)` key.
///
/// Supported formats: `"1 JAN 1812"`, `"JAN 1812"`, `"1812"`, `"BEF 1900"`,
/// `"ABT 1850"`, `"FROM 1800 TO 1850"`, etc.
/// Prefix qualifiers (BEF, AFT, ABT, CAL, EST, FROM, TO, AND, INT, …) are ignored.
/// Dates with no year return `(u32::MAX, 0, 0)` so they sort last.
pub(crate) fn date_sort_key(raw: &str) -> (u32, u32, u32) {
    const MONTHS: &[&str] = &[
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN",
        "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    const QUALIFIERS: &[&str] = &[
        "BEF", "AFT", "ABT", "CAL", "EST", "FROM", "TO", "AND", "INT", "ABOUT",
        "BEFORE", "AFTER", "BETWEEN", "CALCULATED", "ESTIMATED",
    ];

    let mut year: Option<u32>   = None;
    let mut month: u32          = 0;
    let mut day: u32            = 0;

    for token in raw.split_whitespace() {
        let up = token.to_uppercase();
        if QUALIFIERS.contains(&up.as_str()) {
            continue;
        }
        if let Some(pos) = MONTHS.iter().position(|&m| m == up.as_str()) {
            month = (pos + 1) as u32;
        } else if let Ok(n) = token.parse::<u32>() {
            if n > 31 {
                // Likely a year (GEDCOM years are 4-digit).
                // Only record the first year seen (FROM 1800 TO 1850 → 1800).
                if year.is_none() {
                    year = Some(n);
                }
            } else if year.is_none() {
                // Small number before a year token → day
                day = n;
            }
        }
    }

    match year {
        Some(y) => (y, month, day),
        None     => (u32::MAX, 0, 0),
    }
}

/// Sort an individual's families by marriage date.
///
/// If ALL families have a marriage date, returns them sorted chronologically.
/// Otherwise preserves the original FAMS tag order (which may reflect
/// the GEDCOM author's intended sequencing).
pub(crate) fn sort_families_by_date<G>(ind: &Individual<G>, genrep: &Genrep<G>) -> Vec<String> {
    let fams = &ind.fams;
    let all_have_dates = fams.iter().all(|fam_id| {
        genrep.get_family(fam_id)
            .and_then(|f| f.marriage.as_ref())
            .and_then(|e| e.date.as_ref())
            .is_some()
    });
    let mut sorted = fams.clone();
    if all_have_dates {
        sorted.sort_by_key(|fam_id| {
            genrep.get_family(fam_id)
                .and_then(|f| f.marriage.as_ref())
                .and_then(|e| e.date.as_ref())
                .map(|d| date_sort_key(&d.raw))
                .unwrap_or((u32::MAX, 0, 0))
        });
    }
    sorted
}

/// Resolve the root individual ID from preferences, with fallback.
///
/// Validates the root ID against the genrep and warns on stderr if not found,
/// falling back to the first individual encountered during parsing.
pub(crate) fn resolve_root_id(genrep: &Genrep, prefs: &Prefs) -> Option<String> {
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

/// Copy an [`Individual`] with a different geo type.
///
/// Used by layout algorithms to convert `Individual<()>` to `Individual<LayoutGeo>`.
pub(crate) fn copy_individual<G, GH>(src: &Individual<G>, geo: Option<GH>) -> Individual<GH> {
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

/// Copy all families with a new geo type, computing geo via a closure.
///
/// The closure can return `None` (e.g. fan layout) or compute geo from
/// placed individuals (e.g. boxed_couples layout).
pub(crate) fn copy_families<G, GH, F>(
    genrep: &Genrep<G>,
    compute_geo: F,
) -> HashMap<String, Family<GH>>
where
    F: Fn(&Family<G>) -> Option<GH>,
{
    genrep.families.iter().map(|(id, fam)| {
        let geo = compute_geo(fam);
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── date_sort_key ──

    #[test]
    fn test_date_sort_key_full_date() {
        assert_eq!(date_sort_key("1 JAN 1812"), (1812, 1, 1));
        assert_eq!(date_sort_key("4 APR 1843"), (1843, 4, 4));
    }

    #[test]
    fn test_date_sort_key_partial_dates() {
        assert_eq!(date_sort_key("JAN 1812"), (1812, 1, 0));
        assert_eq!(date_sort_key("1812"),      (1812, 0, 0));
    }

    #[test]
    fn test_date_sort_key_qualifiers_ignored() {
        assert_eq!(date_sort_key("BEF 1900"),          (1900, 0, 0));
        assert_eq!(date_sort_key("ABT 1850"),           (1850, 0, 0));
        assert_eq!(date_sort_key("CAL 1760"),           (1760, 0, 0));
        assert_eq!(date_sort_key("EST 1800"),           (1800, 0, 0));
        assert_eq!(date_sort_key("FROM 1800 TO 1850"), (1800, 0, 0));
    }

    #[test]
    fn test_date_sort_key_no_year() {
        assert_eq!(date_sort_key(""),             (u32::MAX, 0, 0));
        assert_eq!(date_sort_key("JAN"),          (u32::MAX, 0, 0));
        assert_eq!(date_sort_key("unknown"),      (u32::MAX, 0, 0));
    }

    // ── sort_families_by_date ──

    #[test]
    fn test_sort_families_all_dates_sorted() {
        let mut individuals: HashMap<String, Individual<()>> = HashMap::new();
        let mut families: HashMap<String, Family<()>> = HashMap::new();

        individuals.insert("I1".to_string(), Individual {
            id: "I1".to_string(), given: None, surname: None, sex: None,
            birth: None, death: None,
            fams: vec!["F1".to_string(), "F2".to_string()],
            famc: vec![], alt_name: None, name_heb: None, living: None,
            in_scope: true, geo: None,
        });

        families.insert("F1".to_string(), Family {
            id: "F1".to_string(),
            husband_id: Some("I1".to_string()), wife_id: None,
            children_ids: vec![],
            marriage: Some(crate::parser::genrep::Event {
                date: Some(crate::parser::genrep::GedDate { raw: "1 JUN 1900".to_string() }),
                place: None,
            }),
            jmar: None, in_scope: true, geo: None,
        });

        families.insert("F2".to_string(), Family {
            id: "F2".to_string(),
            husband_id: Some("I1".to_string()), wife_id: None,
            children_ids: vec![],
            marriage: Some(crate::parser::genrep::Event {
                date: Some(crate::parser::genrep::GedDate { raw: "10 MAR 1850".to_string() }),
                place: None,
            }),
            jmar: None, in_scope: true, geo: None,
        });

        let genrep = Genrep { individuals, families, first_individual_id: Some("I1".to_string()) };
        let ind = genrep.individuals.get("I1").unwrap();
        let sorted = sort_families_by_date(ind, &genrep);

        // F2 (1850) should come before F1 (1900)
        assert_eq!(sorted, vec!["F2", "F1"]);
    }

    #[test]
    fn test_sort_families_missing_date_preserves_order() {
        let mut individuals: HashMap<String, Individual<()>> = HashMap::new();
        let mut families: HashMap<String, Family<()>> = HashMap::new();

        individuals.insert("I1".to_string(), Individual {
            id: "I1".to_string(), given: None, surname: None, sex: None,
            birth: None, death: None,
            fams: vec!["F1".to_string(), "F2".to_string()],
            famc: vec![], alt_name: None, name_heb: None, living: None,
            in_scope: true, geo: None,
        });

        // F1 has no marriage date
        families.insert("F1".to_string(), Family {
            id: "F1".to_string(),
            husband_id: Some("I1".to_string()), wife_id: None,
            children_ids: vec![],
            marriage: None,
            jmar: None, in_scope: true, geo: None,
        });

        families.insert("F2".to_string(), Family {
            id: "F2".to_string(),
            husband_id: Some("I1".to_string()), wife_id: None,
            children_ids: vec![],
            marriage: Some(crate::parser::genrep::Event {
                date: Some(crate::parser::genrep::GedDate { raw: "10 MAR 1850".to_string() }),
                place: None,
            }),
            jmar: None, in_scope: true, geo: None,
        });

        let genrep = Genrep { individuals, families, first_individual_id: Some("I1".to_string()) };
        let ind = genrep.individuals.get("I1").unwrap();
        let sorted = sort_families_by_date(ind, &genrep);

        // F1 has no date, so original order is preserved
        assert_eq!(sorted, vec!["F1", "F2"]);
    }

    #[test]
    fn test_sort_families_no_families_returns_empty() {
        let mut individuals: HashMap<String, Individual<()>> = HashMap::new();
        individuals.insert("I1".to_string(), Individual {
            id: "I1".to_string(), given: None, surname: None, sex: None,
            birth: None, death: None,
            fams: vec![],
            famc: vec![], alt_name: None, name_heb: None, living: None,
            in_scope: true, geo: None,
        });

        let genrep = Genrep {
            individuals, families: HashMap::new(), first_individual_id: Some("I1".to_string()),
        };
        let ind = genrep.individuals.get("I1").unwrap();
        let sorted = sort_families_by_date(ind, &genrep);
        assert!(sorted.is_empty());
    }
}
