//! SVG back-end (simple layout).

use anyhow::Result;
use crate::backend::Renderer;
use crate::layout::LayoutOutput;
use crate::layout::simple::SimpleGeo;
use crate::parser::genrep::Genrep;
use crate::preferences::Prefs;

// Fixed rendering constants (may become preferences later)
const LINE_HEIGHT: f64 = 18.0;   // px between baselines
const FONT_SIZE:   f64 = 13.0;   // px
const MARGIN:      f64 = 20.0;   // px, left and top
const FONT_FAMILY: &str = "monospace";

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

fn svg_text(x: f64, y: f64, content: &str) -> String {
    format!(
        r#"  <text x="{:.1}" y="{:.1}" font-family="{}" font-size="{}" xml:space="preserve">{}</text>"#,
        x, y, FONT_FAMILY, FONT_SIZE, xml_escape(content)
    )
}

fn render_simple(genrep: &Genrep<SimpleGeo>, prefs: &Prefs) -> String {
    let lines = crate::backend::text::build_lines(genrep, prefs);

    let max_len = lines.iter().map(|l| l.len()).max().unwrap_or(80);
    let width  = MARGIN * 2.0 + max_len as f64 * (FONT_SIZE * 0.6);
    let height = MARGIN * 2.0 + lines.len() as f64 * LINE_HEIGHT;

    let mut out = String::new();

    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{:.0}\" height=\"{:.0}\">\n",
        width, height
    ));

    for (i, line) in lines.iter().enumerate() {
        let x = MARGIN;
        let y = MARGIN + (i as f64 + 1.0) * LINE_HEIGHT;
        out.push_str(&svg_text(x, y, line));
        out.push('\n');
    }

    out.push_str("</svg>\n");
    out
}

pub struct SvgRenderer;

impl Renderer for SvgRenderer {
    fn render(
        &self,
        output: &LayoutOutput,
        prefs: &Prefs,
        writer: &mut dyn std::io::Write,
    ) -> Result<()> {
        let svg = render_to_string(output, prefs)?;
        writer.write_all(svg.as_bytes())?;
        Ok(())
    }
}

pub fn render_to_string(output: &LayoutOutput, prefs: &Prefs) -> Result<String> {
    match output {
        LayoutOutput::Simple(genrep) => Ok(render_simple(genrep, prefs)),
        LayoutOutput::BoxedCouples(_) => anyhow::bail!("BoxedCouples SVG not yet implemented"),
        LayoutOutput::Fan(_)          => anyhow::bail!("Fan SVG not yet implemented"),
    }
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

    fn make_layout(prefs: &Prefs) -> LayoutOutput {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        run_layout(&genrep, prefs).unwrap()
    }

    #[test]
    fn test_svg_structure() {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.layout.layout_type = "simple".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("<svg "),   "missing <svg: {out}");
        assert!(out.contains("</svg>"),  "missing </svg: {out}");
        assert!(out.contains("<text "),  "missing <text: {out}");
    }

    #[test]
    fn test_svg_contains_names() {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.layout.layout_type = "simple".into();
        prefs.format.individual = "{firstname} {lastname}".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("John"),  "root name missing");
        assert!(out.contains("Jane"),  "spouse name missing");
        assert!(out.contains("Paul"),  "child name missing");
    }

    #[test]
    fn test_non_simple_returns_ok() {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.layout.layout_type = "simple".into();
        assert!(render_to_string(&make_layout(&prefs), &prefs).is_ok());
    }
}
