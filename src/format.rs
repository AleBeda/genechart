//! Shared text-formatting helpers used by multiple backends.

use crate::parser::genrep::{GedDate, Individual};
use crate::preferences::Prefs;
use std::collections::HashMap;

/// Format an individual's name using the `format.individual` preference template.
///
/// Falls back to `"given surname"` if the template expansion fails.
pub fn format_name<G>(indi: &Individual<G>, prefs: &Prefs) -> String {
    let mut vars: HashMap<String, String> = HashMap::new();
    vars.insert("firstname".into(), indi.given.clone().unwrap_or_default());
    vars.insert("lastname".into(), indi.surname.clone().unwrap_or_default());
    vars.insert(
        "sex".into(),
        match indi.sex {
            Some('M') => "♂".into(),
            Some('F') => "♀".into(),
            _ => String::new(),
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
/// Returns `None` when both `date` and `place` are absent, or when the
/// formatted string is empty after trimming trailing punctuation.
pub fn format_event(template: &str, date: Option<&GedDate>, place: Option<&str>) -> Option<String> {
    if date.is_none() && place.is_none() {
        return None;
    }
    let mut vars: HashMap<String, String> = HashMap::new();
    vars.insert(
        "date".into(),
        date.map(|d| d.raw.clone()).unwrap_or_default(),
    );
    vars.insert("location".into(), place.unwrap_or("").to_string());

    let s = strfmt::strfmt(template, &vars).unwrap_or_else(|_| template.to_string());

    let s = s.trim_end_matches([',', ' ']).to_string();
    if s.is_empty() {
        return None;
    }
    Some(s)
}
