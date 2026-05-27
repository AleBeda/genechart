//! Shared text-formatting helpers used by multiple backends.

use crate::parser::genrep::{GedDate, Individual};
use crate::preferences::Prefs;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// GEDCOM date parsing
// ---------------------------------------------------------------------------

struct PartialDate {
    day: Option<u32>,
    month: Option<u32>, // 1-12
    year: Option<u32>,
}

enum DateQualifier {
    None,
    Abt,   // ABT, CAL, EST
    Aft,   // AFT
    Bef,   // BEF
    Range, // BET...AND or FROM...TO
}

struct ParsedGedDate {
    qualifier: DateQualifier,
    date1: PartialDate,
    date2: Option<PartialDate>,
}

const MONTHS: &[&str] = &[
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

fn parse_month(token: &str) -> Option<u32> {
    MONTHS
        .iter()
        .position(|&m| m == token)
        .map(|i| (i + 1) as u32)
}

fn parse_ged_date(raw: &str) -> ParsedGedDate {
    let mut qualifier = DateQualifier::None;
    let mut date1 = PartialDate {
        day: None,
        month: None,
        year: None,
    };
    let mut date2: Option<PartialDate> = None;
    let mut in_second = false;

    for token in raw.split_whitespace() {
        let up = token.to_uppercase();
        match up.as_str() {
            "ABT" | "CAL" | "EST" | "ABOUT" | "CALCULATED" | "ESTIMATED" => {
                qualifier = DateQualifier::Abt;
            }
            "AFT" | "AFTER" => qualifier = DateQualifier::Aft,
            "BEF" | "BEFORE" => qualifier = DateQualifier::Bef,
            "BET" | "BETWEEN" | "FROM" => {
                qualifier = DateQualifier::Range;
                date2 = Some(PartialDate {
                    day: None,
                    month: None,
                    year: None,
                });
            }
            "AND" | "TO" => in_second = true,
            _ => {
                let target = if in_second {
                    date2.get_or_insert(PartialDate {
                        day: None,
                        month: None,
                        year: None,
                    })
                } else {
                    &mut date1
                };
                if let Some(m) = parse_month(&up) {
                    target.month = Some(m);
                } else if let Ok(n) = token.parse::<u32>() {
                    if n > 31 {
                        if target.year.is_none() {
                            target.year = Some(n);
                        }
                    } else if target.day.is_none() {
                        target.day = Some(n);
                    }
                }
            }
        }
    }

    ParsedGedDate {
        qualifier,
        date1,
        date2,
    }
}

// ---------------------------------------------------------------------------
// Date formatting helpers
// ---------------------------------------------------------------------------

/// Apply a strftime-like pattern to a parsed date component.
/// Missing components are replaced with `""` and extra spaces are collapsed.
fn apply_date_format(date: &PartialDate, pattern: &str) -> String {
    const MONTHS_SHORT: &[&str] = &[
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const MONTHS_LONG: &[&str] = &[
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];

    let mut result = pattern.to_string();

    let day_zero = date.day.map_or(String::new(), |d| format!("{d:02}"));
    let day_bare = date.day.map_or(String::new(), |d| format!("{d}"));
    let month_num = date.month.map_or(String::new(), |m| format!("{m:02}"));
    let month_short = date.month.map_or(String::new(), |m| {
        MONTHS_SHORT[(m - 1) as usize].to_string()
    });
    let month_long = date
        .month
        .map_or(String::new(), |m| MONTHS_LONG[(m - 1) as usize].to_string());
    let year_full = date.year.map_or(String::new(), |y| format!("{y:04}"));
    let year_short = date
        .year
        .map_or(String::new(), |y| format!("{:02}", y % 100));

    result = result.replace("%d", &day_zero);
    result = result.replace("%e", &day_bare);
    result = result.replace("%m", &month_num);
    result = result.replace("%B", &month_long); // must come before %b
    result = result.replace("%b", &month_short);
    result = result.replace("%Y", &year_full);
    result = result.replace("%y", &year_short);

    // Collapse spaces introduced by missing components.
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reconstruct a GEDCOM-style date string from parsed components, without any qualifier.
/// Used for `none` mode when no strftime pattern is given.
fn gedcom_natural(date: &PartialDate) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(d) = date.day {
        parts.push(format!("{d}"));
    }
    if let Some(m) = date.month {
        parts.push(MONTHS[(m - 1) as usize].to_string());
    }
    if let Some(y) = date.year {
        parts.push(format!("{y}"));
    }
    parts.join(" ")
}

/// Extract `{date:FORMAT}` from a template, returning the rewritten template
/// (with `{date}` in place of the full `{date:FORMAT}`) and the format string.
/// Returns `(template, None)` if no format specifier is present.
fn extract_date_format(template: &str) -> (String, Option<String>) {
    if let Some(start) = template.find("{date:") {
        let rest = &template[start + 6..];
        if let Some(end) = rest.find('}') {
            let fmt = rest[..end].to_string();
            let new_tmpl = format!("{}{{date}}{}", &template[..start], &rest[end + 1..]);
            return (new_tmpl, Some(fmt));
        }
    }
    (template.to_string(), None)
}

/// Format a raw GEDCOM date string for display.
///
/// - `pattern`: optional strftime-like format (e.g. `"%d %b %Y"`).
///   When `None`, the raw GEDCOM component order is preserved.
/// - `qualifier_mode`: `"none"` | `"gedcom"` | `"compact"`.
pub(crate) fn format_ged_date(raw: &str, pattern: Option<&str>, qualifier_mode: &str) -> String {
    // Fast path: gedcom mode + no pattern → raw string unchanged.
    if qualifier_mode == "gedcom" && pattern.is_none() {
        return raw.to_string();
    }

    let parsed = parse_ged_date(raw);

    let fmt_date = |date: &PartialDate| -> String {
        match pattern {
            Some(pat) => apply_date_format(date, pat),
            None => gedcom_natural(date),
        }
    };

    let d1 = fmt_date(&parsed.date1);

    match qualifier_mode {
        "none" => d1,

        "compact" => match parsed.qualifier {
            DateQualifier::None => d1,
            DateQualifier::Abt => format!("~{d1}"),
            DateQualifier::Aft => format!(">{d1}"),
            DateQualifier::Bef => format!("<{d1}"),
            DateQualifier::Range => {
                let d2 = fmt_date(parsed.date2.as_ref().unwrap_or(&parsed.date1));
                if d1 == d2 { d1 } else { format!("{d1}-{d2}") }
            }
        },

        // "gedcom" with a format spec: reconstruct GEDCOM-style qualifier with formatted dates.
        _ => match parsed.qualifier {
            DateQualifier::None => d1,
            DateQualifier::Abt => format!("ABT {d1}"),
            DateQualifier::Aft => format!("AFT {d1}"),
            DateQualifier::Bef => format!("BEF {d1}"),
            DateQualifier::Range => {
                let d2 = fmt_date(parsed.date2.as_ref().unwrap_or(&parsed.date1));
                if d1 == d2 {
                    d1
                } else {
                    format!("BET {d1} AND {d2}")
                }
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Format an individual's name using the `format.individual` preference template.
///
/// Falls back to `"given surname"` if the template expansion fails.
pub fn format_name<G>(indi: &Individual<G>, prefs: &Prefs) -> String {
    let given = indi.given.as_deref().unwrap_or("").trim();
    let surname = indi.surname.as_deref().unwrap_or("").trim();
    if given.is_empty() && surname.is_empty() {
        return prefs.format.no_name.clone();
    }
    let mut vars: HashMap<String, String> = HashMap::new();
    vars.insert("firstname".into(), indi.given.clone().unwrap_or_default());
    vars.insert("lastname".into(), indi.surname.clone().unwrap_or_default());
    vars.insert(
        "sex".into(),
        if prefs.show.sex {
            match indi.sex {
                Some('M') => "♂".into(),
                Some('F') => "♀".into(),
                _ => String::new(),
            }
        } else {
            String::new()
        },
    );
    strfmt::strfmt(&prefs.format.individual, &vars)
        .unwrap_or_else(|_| {
            format!(
                "{} {}",
                indi.given.as_deref().unwrap_or(""),
                indi.surname.as_deref().unwrap_or("")
            )
        })
        .trim()
        .to_string()
}

/// Format a birth/death/marriage event line using the given template.
///
/// The template may use `{date:%d %b %Y}` (strftime-like pattern) or plain `{date}`.
/// `qualifier_mode` controls how GEDCOM qualifier tokens are displayed:
/// `"none"` strips them, `"gedcom"` preserves them, `"compact"` uses symbols.
///
/// Returns `None` when both `date` and `place` are absent, or when the formatted
/// string is empty after trimming trailing punctuation.
pub fn format_event(
    template: &str,
    date: Option<&GedDate>,
    place: Option<&str>,
    qualifier_mode: &str,
) -> Option<String> {
    if date.is_none() && place.is_none() {
        return None;
    }

    let (processed_template, date_pattern) = extract_date_format(template);

    let date_str = date
        .map(|d| format_ged_date(&d.raw, date_pattern.as_deref(), qualifier_mode))
        .unwrap_or_default();

    let mut vars: HashMap<String, String> = HashMap::new();
    vars.insert("date".into(), date_str);
    vars.insert("location".into(), place.unwrap_or("").to_string());

    let s =
        strfmt::strfmt(&processed_template, &vars).unwrap_or_else(|_| processed_template.clone());

    let s = s.trim_end_matches([',', ' ']).to_string();
    if s.is_empty() {
        return None;
    }
    Some(s)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::genrep::GedDate;

    // --- compact qualifier symbols ---

    #[test]
    fn compact_abt() {
        assert_eq!(format_ged_date("ABT 1850", Some("%Y"), "compact"), "~1850");
    }
    #[test]
    fn compact_cal() {
        assert_eq!(format_ged_date("CAL 1850", Some("%Y"), "compact"), "~1850");
    }
    #[test]
    fn compact_est() {
        assert_eq!(format_ged_date("EST 1850", Some("%Y"), "compact"), "~1850");
    }
    #[test]
    fn compact_bef() {
        assert_eq!(format_ged_date("BEF 1900", Some("%Y"), "compact"), "<1900");
    }
    #[test]
    fn compact_aft() {
        assert_eq!(format_ged_date("AFT 1800", Some("%Y"), "compact"), ">1800");
    }

    // --- ranges ---

    #[test]
    fn compact_range_different() {
        assert_eq!(
            format_ged_date("BET APR 1880 AND JUL 1890", Some("%Y"), "compact"),
            "1880-1890"
        );
    }
    #[test]
    fn compact_range_same() {
        // corner case: both dates format to the same string → output once, no qualifier
        assert_eq!(
            format_ged_date("BET APR 1880 AND JUL 1880", Some("%Y"), "compact"),
            "1880"
        );
    }
    #[test]
    fn from_to_range() {
        assert_eq!(
            format_ged_date("FROM 1800 TO 1850", Some("%Y"), "compact"),
            "1800-1850"
        );
    }

    // --- none mode ---

    #[test]
    fn none_strips_qualifier() {
        assert_eq!(format_ged_date("ABT 1850", Some("%Y"), "none"), "1850");
    }
    #[test]
    fn none_range_first_date_only() {
        assert_eq!(
            format_ged_date("BET 1880 AND 1890", Some("%Y"), "none"),
            "1880"
        );
    }

    // --- gedcom mode ---

    #[test]
    fn gedcom_no_pattern_passthrough() {
        assert_eq!(format_ged_date("ABT 1850", None, "gedcom"), "ABT 1850");
    }
    #[test]
    fn gedcom_with_pattern_same_range() {
        assert_eq!(
            format_ged_date("BET APR 1880 AND JUL 1880", Some("%Y"), "gedcom"),
            "1880"
        );
    }
    #[test]
    fn gedcom_with_pattern_different_range() {
        assert_eq!(
            format_ged_date("BET APR 1880 AND JUL 1890", Some("%Y"), "gedcom"),
            "BET 1880 AND 1890"
        );
    }

    // --- strftime patterns ---

    #[test]
    fn format_full_date() {
        assert_eq!(
            format_ged_date("1 JAN 1812", Some("%d %b %Y"), "none"),
            "01 Jan 1812"
        );
    }
    #[test]
    fn format_year_month_only() {
        assert_eq!(
            format_ged_date("JAN 1812", Some("%d %b %Y"), "none"),
            "Jan 1812"
        );
    }
    #[test]
    fn format_year_only() {
        assert_eq!(format_ged_date("1812", Some("%d %b %Y"), "none"), "1812");
    }

    // --- format_event integration ---

    #[test]
    fn format_event_with_date_pattern() {
        let result = format_event(
            "* {date:%Y}, {location}",
            Some(&GedDate {
                raw: "ABT 1850".into(),
            }),
            None,
            "compact",
        );
        assert_eq!(result, Some("* ~1850".to_string()));
    }
    #[test]
    fn format_event_no_date_pattern_gedcom() {
        let result = format_event(
            "* {date}, {location}",
            Some(&GedDate {
                raw: "ABT 1850".into(),
            }),
            None,
            "gedcom",
        );
        assert_eq!(result, Some("* ABT 1850".to_string()));
    }
    #[test]
    fn format_event_none_mode_strips() {
        let result = format_event(
            "* {date:%Y}, {location}",
            Some(&GedDate {
                raw: "BEF 1900".into(),
            }),
            Some("London"),
            "none",
        );
        assert_eq!(result, Some("* 1900, London".to_string()));
    }

    fn make_test_individual(
        given: &str,
        surname: &str,
        sex: char,
    ) -> crate::parser::genrep::Individual<()> {
        crate::parser::genrep::Individual {
            id: "I1".into(),
            given: Some(given.into()),
            surname: Some(surname.into()),
            sex: Some(sex),
            birth: None,
            death: None,
            fams: vec![],
            famc: vec![],
            alt_name: None,
            name_heb: None,
            living: None,
            notes: vec![],
            in_scope: true,
            geo: None,
        }
    }

    #[test]
    fn show_sex_false_suppresses_symbol() {
        let mut prefs = Prefs::default();
        prefs.format.individual = "{firstname} {lastname} {sex}".into();
        prefs.show.sex = false;
        let indi = make_test_individual("John", "Doe", 'M');
        let name = format_name(&indi, &prefs);
        assert!(
            !name.contains('♂'),
            "sex symbol should be absent when show.sex=false: {name}"
        );
        assert!(
            name.contains("John"),
            "name should still contain given name: {name}"
        );
    }

    #[test]
    fn show_sex_true_includes_symbol() {
        let mut prefs = Prefs::default();
        prefs.format.individual = "{firstname} {lastname} {sex}".into();
        prefs.show.sex = true;
        let indi = make_test_individual("Jane", "Doe", 'F');
        let name = format_name(&indi, &prefs);
        assert!(
            name.contains('♀'),
            "sex symbol should be present when show.sex=true: {name}"
        );
    }

    #[test]
    fn no_name_placeholder_shown() {
        let mut prefs = Prefs::default();
        prefs.format.no_name = "N.N.".into();
        let indi = make_test_individual("", "", 'M');
        assert_eq!(format_name(&indi, &prefs), "N.N.");
    }

    #[test]
    fn no_name_empty_produces_empty_string() {
        let mut prefs = Prefs::default();
        prefs.format.no_name = "".into();
        let indi = make_test_individual("", "", 'M');
        assert_eq!(format_name(&indi, &prefs), "");
    }

    #[test]
    fn no_name_not_triggered_when_surname_present() {
        let mut prefs = Prefs::default();
        prefs.format.no_name = "N.N.".into();
        prefs.format.individual = "{firstname} {lastname}".into();
        let indi = make_test_individual("", "Smith", 'M');
        assert!(
            format_name(&indi, &prefs).contains("Smith"),
            "surname-only individual should not use no_name placeholder"
        );
    }
}
