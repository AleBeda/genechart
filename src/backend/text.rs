//! Plain-text output backend.

use std::collections::HashMap;
use crate::backend::Renderer;
use crate::layout::LayoutOutput;
use crate::parser::genrep::{GedDate, Genrep, Individual};
use crate::layout::simple::SimpleGeo;
use crate::preferences::Prefs;

fn format_name(indi: &Individual<SimpleGeo>, prefs: &Prefs) -> String {
    let mut vars: HashMap<String, String> = HashMap::new();
    vars.insert("firstname".into(), indi.given.clone().unwrap_or_default());
    vars.insert("lastname".into(),  indi.surname.clone().unwrap_or_default());
    vars.insert("sex".into(), match indi.sex {
        Some('M') => "♂".into(),
        Some('F') => "♀".into(),
        _         => String::new(),
    });
    strfmt::strfmt(&prefs.format.individual, &vars)
        .unwrap_or_else(|_| format!("{} {}",
            indi.given.as_deref().unwrap_or(""),
            indi.surname.as_deref().unwrap_or("")))
        .trim()
        .to_string()
}

fn format_event(template: &str, date: Option<&GedDate>, place: Option<&str>) -> Option<String> {
    if date.is_none() && place.is_none() {
        return None;
    }
    let mut vars: HashMap<String, String> = HashMap::new();
    vars.insert("date".into(),     date.map(|d| d.raw.clone()).unwrap_or_default());
    vars.insert("location".into(), place.unwrap_or("").to_string());

    let s = strfmt::strfmt(template, &vars)
        .unwrap_or_else(|_| template.to_string());

    let s = s.trim_end_matches(|c| matches!(c, ',' | ' ')).to_string();
    if s.is_empty() {
        return None;
    }
    Some(s)
}

fn find_marriage<'a>(
    indi: &Individual<SimpleGeo>,
    genrep: &'a Genrep<SimpleGeo>,
) -> Option<&'a crate::parser::genrep::Event> {
    for fam_id in &indi.fams {
        if let Some(fam) = genrep.families.get(fam_id) {
            if fam.in_scope {
                return fam.marriage.as_ref();
            }
        }
    }
    None
}

fn build_lines(genrep: &Genrep<SimpleGeo>, prefs: &Prefs) -> Vec<String> {
    let indent_chars = prefs.layout.simple.indent as usize;

    let mut entries: Vec<(&str, &Individual<SimpleGeo>, &SimpleGeo)> = genrep
        .individuals
        .iter()
        .filter(|(_, i)| i.in_scope)
        .filter_map(|(id, i)| i.geo.as_ref().map(|g| (id.as_str(), i, g)))
        .collect();
    entries.sort_by_key(|(_, _, g)| g.line);

    if entries.is_empty() {
        return Vec::new();
    }

    let max_line = entries.iter().map(|(_, _, g)| g.line).max().unwrap_or(0);
    let mut lines: Vec<String> = vec![String::new(); max_line + 1];

    for (_, indi, geo) in &entries {
        let indent = " ".repeat(geo.indent * indent_chars);

        let gen_prefix = if !geo.is_spouse && prefs.show.generation_num {
            format!("{}. ", geo.generation)
        } else {
            String::new()
        };

        let name = format_name(indi, prefs);

        let birth_str = if prefs.show.birth {
            indi.birth.as_ref().and_then(|e| {
                format_event(&prefs.format.birth, e.date.as_ref(), e.place.as_deref())
            })
        } else {
            None
        };

        let death_str = if prefs.show.death {
            indi.death.as_ref().and_then(|e| {
                format_event(&prefs.format.death, e.date.as_ref(), e.place.as_deref())
            })
        } else {
            None
        };

        let marr_str = if geo.is_spouse && prefs.show.marriage {
            find_marriage(indi, genrep).and_then(|e| {
                format_event(&prefs.format.marriage, e.date.as_ref(), e.place.as_deref())
            })
        } else {
            None
        };

        let mut line = format!("{indent}{gen_prefix}{name}");
        if let Some(b) = birth_str { line.push_str("  "); line.push_str(&b); }
        if let Some(d) = death_str { line.push_str("  "); line.push_str(&d); }
        if let Some(m) = marr_str  { line.push_str("  "); line.push_str(&m); }

        lines[geo.line] = line;
    }

    lines
}

pub struct TextRenderer;

impl Renderer for TextRenderer {
    fn render(
        &self,
        output: &LayoutOutput,
        prefs: &Prefs,
        writer: &mut dyn std::io::Write,
    ) -> anyhow::Result<()> {
        let genrep = match output {
            LayoutOutput::Simple(g) => g,
            _ => anyhow::bail!("TextRenderer only supports Simple layout output"),
        };

        let lines = build_lines(genrep, prefs);
        for line in &lines {
            writeln!(writer, "{line}")?;
        }
        Ok(())
    }
}

pub fn render_to_file(
    output: &LayoutOutput,
    prefs: &Prefs,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    let mut f = std::fs::File::create(path)?;
    TextRenderer.render(output, prefs, &mut f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{compute_scope, parse_str};
    use crate::layout::run_layout;

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

    fn make_prefs() -> Prefs {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.layout.layout_type = "simple".into();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.generation_num = true;
        prefs.show.birth = true;
        prefs.format.birth = "* {date}, {location}".into();
        prefs.show.death = true;
        prefs.format.death = "× {date}, {location}".into();
        prefs.show.marriage = true;
        prefs.format.marriage = "⚭ {date}, {location}".into();
        prefs
    }

    fn render_text(prefs: &Prefs) -> Vec<String> {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        let layout_out = run_layout(&genrep, prefs).unwrap();
        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&layout_out, prefs, &mut buf).unwrap();
        String::from_utf8(buf).unwrap().lines().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_correct_names_and_order() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        assert!(lines[0].contains("John") && lines[0].contains("Ancestor"),
                "line 0 should be John: {:?}", lines[0]);
        assert!(lines[1].contains("Jane"),
                "line 1 should be Jane (spouse): {:?}", lines[1]);
        assert!(lines[2].contains("Paul"),
                "line 2 should be Paul (child): {:?}", lines[2]);
    }

    #[test]
    fn test_birth_data_on_root_line() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        assert!(lines[0].contains("1 JAN 1812"), "birth date missing: {:?}", lines[0]);
        assert!(lines[0].contains("London"),     "birth place missing: {:?}", lines[0]);
    }

    #[test]
    fn test_marriage_on_spouse_line() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        assert!(lines[1].contains("4 APR 1843"), "marriage date missing: {:?}", lines[1]);
    }

    #[test]
    fn test_no_birth_prefix_when_absent() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        // I2 has no birth data; "* " should not appear on her line
        assert!(!lines[1].contains("* "), "unexpected birth prefix on spouse line: {:?}", lines[1]);
    }
}
