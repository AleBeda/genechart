//! Plain-text output backend.

use std::collections::{BTreeSet, HashMap};
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

// ── Column layout ─────────────────────────────────────────────────────────────

struct Columns {
    birth:    usize,
    death:    usize,
    marriage: usize,
}

fn compute_columns(genrep: &Genrep<SimpleGeo>, prefs: &Prefs) -> Columns {
    let indent_chars = prefs.layout.simple.indent as usize;

    let max_name_col = genrep.individuals.values()
        .filter(|i| i.in_scope)
        .filter_map(|i| i.geo.as_ref().map(|g| (i, g)))
        .map(|(indi, geo)| {
            let indent = geo.indent * indent_chars;
            let gen_prefix = if !geo.is_spouse && prefs.show.generation_num {
                format!("{}. ", geo.generation).len()
            } else {
                0
            };
            indent + gen_prefix + format_name(indi, prefs).len()
        })
        .max()
        .unwrap_or(20);

    let max_birth = genrep.individuals.values()
        .filter(|i| i.in_scope)
        .filter_map(|i| i.birth.as_ref().and_then(|e| {
            format_event(&prefs.format.birth, e.date.as_ref(), e.place.as_deref())
        }))
        .map(|s| s.len())
        .max()
        .unwrap_or(24);

    let max_death = genrep.individuals.values()
        .filter(|i| i.in_scope)
        .filter_map(|i| i.death.as_ref().and_then(|e| {
            format_event(&prefs.format.death, e.date.as_ref(), e.place.as_deref())
        }))
        .map(|s| s.len())
        .max()
        .unwrap_or(24);

    let birth_col    = max_name_col + 4;
    let death_col    = birth_col  + max_birth  + 4;
    let marriage_col = death_col  + max_death  + 4;

    Columns { birth: birth_col, death: death_col, marriage: marriage_col }
}

// ── String helpers ────────────────────────────────────────────────────────────

fn write_at_col(s: &mut String, col: usize, text: &str) {
    if s.len() < col {
        s.extend(std::iter::repeat(' ').take(col - s.len()));
    } else {
        s.push_str("  ");
    }
    s.push_str(text);
}

fn pad_line_to(s: &mut String, min_len: usize) {
    if s.len() < min_len {
        s.extend(std::iter::repeat(' ').take(min_len - s.len()));
    }
}

fn set_char_at(s: &mut String, byte_pos: usize, ch: char) {
    if byte_pos < s.len() {
        s.replace_range(byte_pos..byte_pos + 1, &ch.to_string());
    }
}

// ── Line assembly ─────────────────────────────────────────────────────────────

fn build_lines(genrep: &Genrep<SimpleGeo>, prefs: &Prefs) -> Vec<String> {
    let indent_chars = prefs.layout.simple.indent as usize;
    let cols = compute_columns(genrep, prefs);

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
        if let Some(b) = birth_str  { write_at_col(&mut line, cols.birth,    &b); }
        if let Some(d) = death_str  { write_at_col(&mut line, cols.death,    &d); }
        if let Some(m) = marr_str   { write_at_col(&mut line, cols.marriage, &m); }

        lines[geo.line] = line;
    }

    // Connector pass: insert │ on blank lines between ancestors.
    // Collect per-line columns, then insert right-to-left to preserve byte positions.
    let mut conn_per_line: HashMap<usize, BTreeSet<usize>> = HashMap::new();
    for (_, _, geo) in &entries {
        let col = (geo.indent + 1) * indent_chars;
        for &lnum in geo.connectors_above.iter().chain(geo.connectors_below.iter()) {
            if lnum < lines.len() {
                conn_per_line.entry(lnum).or_default().insert(col);
            }
        }
    }
    for (lnum, col_set) in &conn_per_line {
        let max_col = *col_set.iter().max().unwrap();
        pad_line_to(&mut lines[*lnum], max_col + 1);
        for &col in col_set.iter().rev() {
            set_char_at(&mut lines[*lnum], col, '│');
        }
    }

    lines
}

// ── Public API ────────────────────────────────────────────────────────────────

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

        // Title
        if !prefs.output.text.title.is_empty() {
            let gedcom_name = std::path::Path::new(&prefs.files.gedcom)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let mut vars = HashMap::new();
            vars.insert("gedcom".to_string(), gedcom_name.to_string());
            let title = strfmt::strfmt(&prefs.output.text.title, &vars)
                .unwrap_or_else(|_| prefs.output.text.title.clone());
            writeln!(writer, "{title}")?;
            writeln!(writer)?;
        }

        // Body
        let lines = build_lines(genrep, prefs);
        for line in &lines {
            writeln!(writer, "{line}")?;
        }

        // Copyright
        if !prefs.output.text.copyright.is_empty() {
            writeln!(writer)?;
            writeln!(writer, "{}", prefs.output.text.copyright)?;
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

// ── Tests ─────────────────────────────────────────────────────────────────────

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
        assert!(!lines[1].contains("* "), "unexpected birth prefix on spouse line: {:?}", lines[1]);
    }

    #[test]
    fn test_column_alignment() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        let birth_pos = lines[0].find("* ").expect("birth not found on line 0");
        assert!(birth_pos > "1. John Ancestor".len(),
                "birth should be after name: {:?}", lines[0]);
    }

    #[test]
    fn test_title_and_copyright() {
        let mut prefs = make_prefs();
        prefs.output.text.title = "My Chart".into();
        prefs.output.text.copyright = "© 2026".into();

        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        let layout_out = run_layout(&genrep, &prefs).unwrap();

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&layout_out, &prefs, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();

        assert!(text.starts_with("My Chart\n"), "title should be first line");
        assert!(text.trim_end().ends_with("© 2026"), "copyright should be last line");
    }
}
