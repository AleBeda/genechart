//! Plain-text output backend.

use crate::backend::Renderer;
use crate::layout::LayoutOutput;
use crate::preferences::Prefs;
use crate::scene::{Primitive, Scene, TextAttr};
use crate::text_metrics::{CHAR_WIDTH_RATIO, FONT_SIZE, LINE_HEIGHT, parsed_font};
use std::collections::HashMap;

// ── String helpers ────────────────────────────────────────────────────────────

fn display_len(s: &str) -> usize {
    s.chars().count()
}

fn write_at_col(s: &mut String, col: usize, text: &str, dot_leaders: bool) {
    let cur = display_len(s);
    if cur <= col {
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

// ── Scene → text-grid rendering ───────────────────────────────────────────────

fn render_scene_text(scene: &Scene, prefs: &Prefs) -> String {
    let (_, font_size) = parsed_font(&prefs.output.style.fonts.names);
    let line_height_px = font_size * (LINE_HEIGHT / FONT_SIZE);
    let char_width_px = font_size * CHAR_WIDTH_RATIO;
    let total_lines = ((scene.canvas_bounds.h / line_height_px).ceil() as usize).max(1);
    let mut lines: Vec<String> = vec![String::new(); total_lines];

    let dot_leaders = prefs.output.style.dot_leaders;

    for prim in &scene.primitives {
        match prim {
            Primitive::Text(t) => {
                let line_idx = ((t.bbox.y / line_height_px).round() as usize).min(total_lines - 1);
                let col = (t.bbox.x / char_width_px).round() as usize;
                let use_dot_leaders = dot_leaders
                    && matches!(
                        t.attr,
                        TextAttr::BirthData | TextAttr::DeathData | TextAttr::MarriageData
                    );
                write_at_col(&mut lines[line_idx], col, &t.content, use_dot_leaders);
            }
            Primitive::Connector(c) => {
                if c.parent_points.is_empty() || c.child_points.is_empty() {
                    continue;
                }
                let x_col = (c.parent_points[0].x / char_width_px).round() as usize;
                let y_start = (c.parent_points[0].y / line_height_px).round() as usize;
                let y_end = (c.child_points[0].y / line_height_px).round() as usize;
                for row in y_start..y_end {
                    if row < total_lines {
                        pad_line_to(&mut lines[row], x_col + 1);
                        set_char_at(&mut lines[row], x_col, '│');
                    }
                }
            }
            Primitive::Box(_) | Primitive::Wedge(_) => {
                // Not used in simple layout text output
            }
        }
    }

    // Trim trailing empty lines
    while lines.last().map_or(false, |l| l.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
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
        if output.is_fan() {
            anyhow::bail!("fan layout does not support text output; use --svg or --pdf");
        }
        let scene = output.scene();

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
        let body = render_scene_text(scene, prefs);
        writeln!(writer, "{body}")?;

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
    use crate::layout::run_layout;
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
        prefs.show.last_gen_spouses = true;

        let layout_out = run_layout(&genrep, &prefs).unwrap();
        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&layout_out, &prefs, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = text.lines().collect();

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
        use crate::layout::simple::gen_prefix_str;
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
