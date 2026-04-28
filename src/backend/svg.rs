//! SVG back-end (simple layout).

use anyhow::Result;
use crate::backend::Renderer;
use crate::layout::LayoutOutput;
use std::collections::HashMap;
use crate::layout::simple::SimpleGeo;
use crate::layout::fan::FanGeo;
use crate::parser::genrep::{Genrep, Individual};
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

// ── Fan layout rendering ──────────────────────────────────────────────────────

fn format_fan_name(indi: &Individual<FanGeo>, prefs: &Prefs) -> String {
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

// SVG arc path for one wedge of the fan.
// Angles follow the math convention (0°=right, 90°=top, 180°=left).
// SVG y is flipped: svg_y = cy - math_y.
// Outer arc is drawn counterclockwise (sweep=0); inner return arc clockwise (sweep=1).
fn wedge_path(cx: f64, cy: f64, geo: &FanGeo) -> String {
    let a0 = (geo.angle_center - geo.angle_span / 2.0).to_radians();
    let a1 = (geo.angle_center + geo.angle_span / 2.0).to_radians();
    let ri = geo.radius_inner;
    let ro = geo.radius_outer;

    let ox0 = cx + ro * a0.cos();  let oy0 = cy - ro * a0.sin();
    let ox1 = cx + ro * a1.cos();  let oy1 = cy - ro * a1.sin();

    let laf = if geo.angle_span >= 180.0 { 1 } else { 0 };

    if ri < 1.0 {
        // Root: pie-slice (degenerate inner radius)
        format!("M {cx:.1} {cy:.1} L {ox0:.1} {oy0:.1} \
                 A {ro:.1} {ro:.1} 0 {laf} 0 {ox1:.1} {oy1:.1} Z")
    } else {
        let ix0 = cx + ri * a0.cos();  let iy0 = cy - ri * a0.sin();
        let ix1 = cx + ri * a1.cos();  let iy1 = cy - ri * a1.sin();
        format!("M {ox0:.1} {oy0:.1} \
                 A {ro:.1} {ro:.1} 0 {laf} 0 {ox1:.1} {oy1:.1} \
                 L {ix1:.1} {iy1:.1} \
                 A {ri:.1} {ri:.1} 0 {laf} 1 {ix0:.1} {iy0:.1} Z")
    }
}

fn render_fan(genrep: &Genrep<FanGeo>, prefs: &Prefs) -> String {
    let max_radius = genrep.individuals.values()
        .filter_map(|i| i.geo.as_ref())
        .map(|g| g.radius_outer)
        .fold(0.0_f64, f64::max);

    if max_radius < 1.0 {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                <svg xmlns=\"http://www.w3.org/2000/svg\" \
                width=\"100\" height=\"100\"></svg>\n".into();
    }

    // Half-circle canvas: root at bottom-center.
    let width  = 2.0 * (max_radius + MARGIN);
    let height = max_radius + 2.0 * MARGIN;
    let cx     = width / 2.0;
    let cy     = height - MARGIN;

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" \
        width=\"{width:.0}\" height=\"{height:.0}\">\n"
    ));

    // Draw from innermost to outermost so later rings paint over earlier strokes.
    let mut indis: Vec<_> = genrep.individuals.values()
        .filter_map(|i| i.geo.as_ref().map(|g| (i, g)))
        .collect();
    indis.sort_by(|(_, a), (_, b)|
        a.radius_inner.partial_cmp(&b.radius_inner).unwrap_or(std::cmp::Ordering::Equal));

    for (indi, geo) in &indis {
        let path = wedge_path(cx, cy, geo);
        out.push_str(&format!(
            "  <path d=\"{path}\" fill=\"white\" stroke=\"black\" stroke-width=\"0.5\"/>\n"
        ));

        // Text at arc midpoint, rotated to align radially.
        let label = format_fan_name(indi, prefs);
        let tx = cx + geo.x;
        let ty = cy - geo.y;
        let rotate = 90.0 - geo.angle_center;
        out.push_str(&format!(
            "  <text x=\"{tx:.1}\" y=\"{ty:.1}\" \
             font-family=\"{FONT_FAMILY}\" font-size=\"{FONT_SIZE}\" \
             text-anchor=\"middle\" dominant-baseline=\"middle\" \
             transform=\"rotate({rotate:.1},{tx:.1},{ty:.1})\" \
             xml:space=\"preserve\">{}</text>\n",
            xml_escape(&label)
        ));
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
        LayoutOutput::Fan(genrep)     => Ok(render_fan(genrep, prefs)),
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
