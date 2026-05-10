//! Plain-text output backend.

use crate::backend::Renderer;
use crate::layout::LayoutOutput;
use crate::layout::simple::SimpleGeo;
use crate::parser::genrep::{GedDate, Genrep, Individual};
use crate::preferences::Prefs;
use std::collections::{BTreeSet, HashMap};

pub(crate) fn format_name<G>(indi: &Individual<G>, prefs: &Prefs) -> String {
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

pub(crate) fn format_event(
    template: &str,
    date: Option<&GedDate>,
    place: Option<&str>,
) -> Option<String> {
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

pub(crate) fn find_marriage<'a>(
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

fn display_len(s: &str) -> usize {
    s.chars().count()
}

/// Right-align the generation number in a 2-char field: " 1. ", " 9. ", "10. ".
/// Fixed width prevents column shift at the single-digit / double-digit boundary.
fn gen_prefix_str(generation: usize) -> String {
    format!("{:>2}. ", generation)
}

struct Columns {
    birth: usize,
    death: usize,
    marriage: usize,
}

fn compute_columns(genrep: &Genrep<SimpleGeo>, prefs: &Prefs) -> Columns {
    let indent_chars = prefs.layout.simple.indent as usize;

    let max_name_col = genrep
        .individuals
        .values()
        .filter(|i| i.in_scope)
        .filter_map(|i| i.geo.as_ref().map(|g| (i, g)))
        .map(|(indi, geo)| {
            let indent = geo.indent * indent_chars;
            let gen_prefix_len = if prefs.show.generation_num {
                gen_prefix_str(geo.generation).len()
            } else {
                0
            };
            indent + gen_prefix_len + display_len(&format_name(indi, prefs))
        })
        .max()
        .unwrap_or(20);

    let max_birth = genrep
        .individuals
        .values()
        .filter(|i| i.in_scope)
        .filter_map(|i| {
            i.birth.as_ref().and_then(|e| {
                format_event(&prefs.format.birth, e.date.as_ref(), e.place.as_deref())
            })
        })
        .map(|s| display_len(&s))
        .max()
        .unwrap_or(24);

    let max_death = genrep
        .individuals
        .values()
        .filter(|i| i.in_scope)
        .filter_map(|i| {
            i.death.as_ref().and_then(|e| {
                format_event(&prefs.format.death, e.date.as_ref(), e.place.as_deref())
            })
        })
        .map(|s| display_len(&s))
        .max()
        .unwrap_or(24);

    let birth_col = max_name_col + 2;
    let death_col = birth_col + max_birth + 2;
    let marriage_col = death_col + max_death + 2;

    Columns {
        birth: birth_col,
        death: death_col,
        marriage: marriage_col,
    }
}

// ── String helpers ────────────────────────────────────────────────────────────

fn write_at_col(s: &mut String, col: usize, text: &str, dot_leaders: bool) {
    let cur = display_len(s);
    if cur < col {
        let gap = col - cur;
        if dot_leaders && gap >= 4 {
            s.push(' ');
            s.extend(std::iter::repeat_n('.', gap - 2));
            s.push(' ');
        } else {
            s.extend(std::iter::repeat_n(' ', gap));
        }
    } else {
        s.push_str("  ");
    }
    s.push_str(text);
}

fn pad_line_to(s: &mut String, min_len: usize) {
    if s.len() < min_len {
        s.extend(std::iter::repeat_n(' ', min_len - s.len()));
    }
}

fn set_char_at(s: &mut String, byte_pos: usize, ch: char) {
    if byte_pos < s.len() {
        s.replace_range(byte_pos..byte_pos + 1, &ch.to_string());
    }
}

// ── Line assembly ─────────────────────────────────────────────────────────────

pub(crate) fn build_lines(genrep: &Genrep<SimpleGeo>, prefs: &Prefs) -> Vec<String> {
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

        let gen_prefix = if prefs.show.generation_num {
            if geo.is_spouse {
                " ".repeat(gen_prefix_str(geo.generation).len())
            } else {
                gen_prefix_str(geo.generation)
            }
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

        let dl = prefs.output.style.dot_leaders;
        let mut line = format!("{indent}{gen_prefix}{name}");
        if let Some(b) = birth_str {
            write_at_col(&mut line, cols.birth, &b, dl);
        }
        if let Some(d) = death_str {
            write_at_col(&mut line, cols.death, &d, dl);
        }
        if let Some(m) = marr_str {
            write_at_col(&mut line, cols.marriage, &m, dl);
        }

        lines[geo.line] = line;
    }

    // Connector pass: insert │ on blank lines between ancestors.
    // Collect per-line columns, then insert right-to-left to preserve byte positions.
    let mut conn_per_line: HashMap<usize, BTreeSet<usize>> = HashMap::new();
    for (_, _, geo) in &entries {
        // Align with the first character of the parent's name (after gen-prefix).
        let parent_gen_prefix = if prefs.show.generation_num {
            gen_prefix_str(geo.generation + 1).len()
        } else {
            0
        };
        let col = (geo.indent + 1) * indent_chars + parent_gen_prefix;
        for &lnum in geo
            .connectors_above
            .iter()
            .chain(geo.connectors_below.iter())
        {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{LayoutOutput, run_layout};
    use crate::parser::{compute_scope, parse_str};

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
        String::from_utf8(buf)
            .unwrap()
            .lines()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn test_correct_names_and_order() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        assert!(
            lines[0].contains("John") && lines[0].contains("Ancestor"),
            "line 0 should be John: {:?}",
            lines[0]
        );
        assert!(
            lines[1].contains("Jane"),
            "line 1 should be Jane (spouse): {:?}",
            lines[1]
        );
        assert!(
            lines[2].contains("Paul"),
            "line 2 should be Paul (child): {:?}",
            lines[2]
        );
    }

    #[test]
    fn test_birth_data_on_root_line() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        assert!(
            lines[0].contains("1 JAN 1812"),
            "birth date missing: {:?}",
            lines[0]
        );
        assert!(
            lines[0].contains("London"),
            "birth place missing: {:?}",
            lines[0]
        );
    }

    #[test]
    fn test_marriage_on_spouse_line() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        assert!(
            lines[1].contains("4 APR 1843"),
            "marriage date missing: {:?}",
            lines[1]
        );
    }

    #[test]
    fn test_no_birth_prefix_when_absent() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        assert!(
            !lines[1].contains("* "),
            "unexpected birth prefix on spouse line: {:?}",
            lines[1]
        );
    }

    #[test]
    fn test_spouse_name_aligned_with_non_spouse() {
        // With generation numbers on, spouse names must start at the same column
        // as the non-spouse name (i.e. after the "N. " prefix width).
        let prefs = make_prefs(); // generation_num = true
        let lines = render_text(&prefs);
        // John (non-spouse) line starts with "1. John…"
        let root_name_col = lines[0].find("John").expect("John not on line 0");
        // Jane (spouse) line should start "   Jane…" — same column as John
        let spouse_name_col = lines[1].find("Jane").expect("Jane not on line 1");
        assert_eq!(
            root_name_col, spouse_name_col,
            "spouse name column ({spouse_name_col}) must equal non-spouse name column ({root_name_col});\n  line0: {:?}\n  line1: {:?}",
            lines[0], lines[1]
        );
    }

    #[test]
    fn test_column_alignment() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        let birth_pos = lines[0].find("* ").expect("birth not found on line 0");
        assert!(
            birth_pos > "1. John Ancestor".len(),
            "birth should be after name: {:?}",
            lines[0]
        );
    }

    #[test]
    fn test_sex_unknown_column_aligned() {
        // Regression: unknown sex previously left a trailing space in the formatted
        // name, inflating the birth column for everyone by 1.
        const GED: &str = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME Big /Nameperson/
1 BIRT
2 DATE 1 JAN 1900
1 FAMS @F1@
0 @I2@ INDI
1 NAME Al /Bo/
1 SEX M
1 BIRT
2 DATE 2 FEB 1901
1 FAMS @F1@
0 @F1@ FAM
1 HUSB @I2@
1 WIFE @I1@
0 TRLR
";
        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I2"), "descendants", Some(2));
        let mut prefs = Prefs::default();
        prefs.scope.root = "I2".into();
        prefs.scope.direction = "descendants".into();
        prefs.layout.layout_type = "simple".into();
        prefs.format.individual = "{firstname} {lastname} {sex}".into();
        prefs.show.generation_num = false;
        prefs.show.birth = true;
        prefs.format.birth = "* {date}".into();
        prefs.show.death = false;
        prefs.show.marriage = false;

        let layout_out = run_layout(&genrep, &prefs).unwrap();
        let g = match &layout_out {
            LayoutOutput::Simple(g) => g,
            _ => panic!(),
        };
        let lines = build_lines(g, &prefs);

        // Use character counts (display columns), not byte offsets.
        // ♂ is 3 bytes but 1 display column — the test must measure visual alignment.
        let char_positions: Vec<usize> = lines
            .iter()
            .filter_map(|l| l.find("* ").map(|b| l[..b].chars().count()))
            .collect();
        assert_eq!(
            char_positions.len(),
            2,
            "expected birth on both lines: {:?}",
            lines
        );
        assert_eq!(
            char_positions[0], char_positions[1],
            "birth columns must align visually; lines:\n{:?}",
            lines
        );
        assert_eq!(
            char_positions[0],
            "Big Nameperson".chars().count() + 2,
            "birth column should equal display width of longest name + 2"
        );
    }

    #[test]
    fn test_gen_prefix_str_fixed_width() {
        // Single-digit and double-digit generation numbers must produce the same width
        // so that name columns stay aligned across the gen-9 / gen-10 boundary.
        assert_eq!(
            gen_prefix_str(1),
            " 1. ",
            "gen 1 should be right-aligned in 2 chars"
        );
        assert_eq!(
            gen_prefix_str(9),
            " 9. ",
            "gen 9 should be right-aligned in 2 chars"
        );
        assert_eq!(gen_prefix_str(10), "10. ", "gen 10 should be 4 chars total");
        assert_eq!(
            gen_prefix_str(1).len(),
            gen_prefix_str(10).len(),
            "gen-1 and gen-10 prefix must be the same byte length"
        );
    }

    #[test]
    fn test_gen_prefix_present_in_output() {
        // With generation numbers on, the root line must start with " 1. ".
        let prefs = make_prefs(); // show.generation_num = true
        let lines = render_text(&prefs);
        assert!(
            lines[0].starts_with(" 1. "),
            "root line should start with \" 1. \" (right-aligned); got: {:?}",
            lines[0]
        );
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
        assert!(
            text.trim_end().ends_with("© 2026"),
            "copyright should be last line"
        );
    }
}
