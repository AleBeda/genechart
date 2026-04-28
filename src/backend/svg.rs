//! SVG back-end (simple and fan layouts).

use anyhow::Result;
use crate::backend::Renderer;
use crate::layout::LayoutOutput;
use std::collections::HashMap;
use crate::layout::simple::SimpleGeo;
use crate::layout::fan::FanGeo;
use crate::parser::genrep::{Genrep, Individual};
use crate::preferences::Prefs;

// Fallback rendering constants (used when preferences are empty)
const LINE_HEIGHT: f64 = 18.0;
const FONT_SIZE:   f64 = 13.0;
const MARGIN:      f64 = 20.0;
const FONT_FAMILY: &str = "monospace";

// ── Helpers ───────────────────────────────────────────────────────────────────

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

/// Parse "Family Name Size" preference string into (family, size).
/// The last token is tried as a number; everything before it is the family.
fn parsed_font(font_pref: &str) -> (String, f64) {
    if font_pref.trim().is_empty() {
        return (FONT_FAMILY.to_string(), FONT_SIZE);
    }
    let mut parts = font_pref.trim().rsplitn(2, ' ');
    let last = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or(font_pref.trim());
    if let Ok(size) = last.parse::<f64>() {
        (rest.to_string(), size)
    } else {
        (font_pref.trim().to_string(), FONT_SIZE)
    }
}

/// Return paper dimensions `(width_mm, height_mm)` from preferences,
/// or `None` when the paper size is absent or unrecognised.
fn paper_size_mm(prefs: &Prefs) -> Option<(f64, f64)> {
    let (w, h): (f64, f64) = match prefs.output.paper.size.trim().to_uppercase().as_str() {
        "A0"     => (841.0, 1189.0),
        "A1"     => (594.0,  841.0),
        "A2"     => (420.0,  594.0),
        "A3"     => (297.0,  420.0),
        "A4"     => (210.0,  297.0),
        "A5"     => (148.0,  210.0),
        "LETTER" => (215.9,  279.4),
        "CUSTOM" => {
            let cw = prefs.output.paper.custom.width;
            let ch = prefs.output.paper.custom.height;
            if cw > 0.0 && ch > 0.0 { (cw, ch) } else { return None; }
        }
        _ => return None,
    };
    let landscape = prefs.output.paper.orientation.trim().to_lowercase().starts_with("land");
    Some(if landscape { (h, w) } else { (w, h) })
}

// ── Simple layout rendering ───────────────────────────────────────────────────

fn render_simple(genrep: &Genrep<SimpleGeo>, prefs: &Prefs) -> String {
    let lines = crate::backend::text::build_lines(genrep, prefs);

    // Font from preferences; fall back to constants when empty.
    let (font_family, font_size) = parsed_font(&prefs.output.style.fonts.names);
    let line_height = font_size * (LINE_HEIGHT / FONT_SIZE);
    let char_width  = font_size * 0.6; // monospace estimate

    // Content bounding box in user-space units.
    let max_chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(80);
    let content_w = MARGIN * 2.0 + max_chars as f64 * char_width;
    let content_h = MARGIN * 2.0 + lines.len() as f64 * line_height;

    // SVG canvas: paper-sized when paper.size is set; content-sized otherwise.
    let (canvas_w, canvas_h) = match paper_size_mm(prefs) {
        Some((pw, ph)) => (format!("{pw}mm"), format!("{ph}mm")),
        None           => (format!("{content_w:.0}"), format!("{content_h:.0}")),
    };
    let viewbox = format!("0 0 {content_w:.1} {content_h:.1}");

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" \
         width=\"{canvas_w}\" height=\"{canvas_h}\" \
         viewBox=\"{viewbox}\">\n"
    ));

    for (i, line) in lines.iter().enumerate() {
        let x = MARGIN;
        let y = MARGIN + (i as f64 + 1.0) * line_height;
        out.push_str(&format!(
            "  <text x=\"{x:.1}\" y=\"{y:.1}\" \
             font-family=\"{font_family}\" font-size=\"{font_size}\" \
             xml:space=\"preserve\">{}</text>\n",
            xml_escape(line)
        ));
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

    let (font_family, font_size) = parsed_font(&prefs.output.style.fonts.names);

    // Half-circle canvas: root at bottom-center.
    let content_w = 2.0 * (max_radius + MARGIN);
    let content_h = max_radius + 2.0 * MARGIN;
    let cx = content_w / 2.0;
    let cy = content_h - MARGIN;

    let (canvas_w, canvas_h) = match paper_size_mm(prefs) {
        Some((pw, ph)) => (format!("{pw}mm"), format!("{ph}mm")),
        None           => (format!("{content_w:.0}"), format!("{content_h:.0}")),
    };
    let viewbox = format!("0 0 {content_w:.1} {content_h:.1}");

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" \
        width=\"{canvas_w}\" height=\"{canvas_h}\" \
        viewBox=\"{viewbox}\">\n"
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

        let label = format_fan_name(indi, prefs);
        let tx = cx + geo.x;
        let ty = cy - geo.y;
        let rotate = 90.0 - geo.angle_center;
        out.push_str(&format!(
            "  <text x=\"{tx:.1}\" y=\"{ty:.1}\" \
             font-family=\"{font_family}\" font-size=\"{font_size}\" \
             text-anchor=\"middle\" dominant-baseline=\"middle\" \
             transform=\"rotate({rotate:.1},{tx:.1},{ty:.1})\" \
             xml:space=\"preserve\">{}</text>\n",
            xml_escape(&label)
        ));
    }

    out.push_str("</svg>\n");
    out
}

// ── Public API ────────────────────────────────────────────────────────────────

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
        LayoutOutput::Simple(genrep)      => Ok(render_simple(genrep, prefs)),
        LayoutOutput::BoxedCouples(_)     => anyhow::bail!("BoxedCouples SVG not yet implemented"),
        LayoutOutput::Fan(genrep)         => Ok(render_fan(genrep, prefs)),
    }
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

    fn make_layout(prefs: &Prefs) -> LayoutOutput {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        run_layout(&genrep, prefs).unwrap()
    }

    fn simple_prefs() -> Prefs {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.layout.layout_type = "simple".into();
        prefs
    }

    #[test]
    fn test_svg_structure() {
        let prefs = simple_prefs();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("<svg "),  "missing <svg: {out}");
        assert!(out.contains("</svg>"), "missing </svg: {out}");
        assert!(out.contains("<text "), "missing <text: {out}");
        assert!(out.contains("viewBox="), "missing viewBox: {out}");
    }

    #[test]
    fn test_svg_contains_names() {
        let mut prefs = simple_prefs();
        prefs.format.individual = "{firstname} {lastname}".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("John"), "root name missing");
        assert!(out.contains("Jane"), "spouse name missing");
        assert!(out.contains("Paul"), "child name missing");
    }

    #[test]
    fn test_non_simple_returns_ok() {
        let prefs = simple_prefs();
        assert!(render_to_string(&make_layout(&prefs), &prefs).is_ok());
    }

    #[test]
    fn test_svg_content_sized_when_no_paper() {
        // With empty paper.size the SVG canvas should be numeric (pixel) dimensions.
        let prefs = simple_prefs(); // paper.size = "" via Prefs::default()
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        // Width attribute should be a number, not "Xmm"
        assert!(!out.contains("mm\"") || out.contains("viewBox="),
            "content-sized SVG must not have mm dimensions without paper.size: {out}");
        let width_val: String = out
            .split("width=\"").nth(1).unwrap_or("")
            .chars().take_while(|c| *c != '"').collect();
        assert!(width_val.parse::<f64>().is_ok(),
            "content-sized width should be a number, got: {width_val:?}");
    }

    #[test]
    fn test_svg_paper_a4_portrait() {
        let mut prefs = simple_prefs();
        prefs.output.paper.size = "A4".into();
        prefs.output.paper.orientation = "portrait".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("width=\"210mm\""),  "A4 portrait width: {out}");
        assert!(out.contains("height=\"297mm\""), "A4 portrait height: {out}");
        assert!(out.contains("viewBox="),         "A4 SVG needs viewBox: {out}");
    }

    #[test]
    fn test_svg_paper_a4_landscape() {
        let mut prefs = simple_prefs();
        prefs.output.paper.size = "A4".into();
        prefs.output.paper.orientation = "landscape".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("width=\"297mm\""),  "A4 landscape width: {out}");
        assert!(out.contains("height=\"210mm\""), "A4 landscape height: {out}");
    }

    #[test]
    fn test_svg_paper_letter() {
        let mut prefs = simple_prefs();
        prefs.output.paper.size = "letter".into();
        prefs.output.paper.orientation = "portrait".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("width=\"215.9mm\""),  "letter portrait width: {out}");
        assert!(out.contains("height=\"279.4mm\""), "letter portrait height: {out}");
    }

    #[test]
    fn test_svg_font_from_prefs() {
        let mut prefs = simple_prefs();
        prefs.output.style.fonts.names = "Helvetica 16".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("font-family=\"Helvetica\""), "custom font family: {out}");
        assert!(out.contains("font-size=\"16\""),          "custom font size: {out}");
    }

    #[test]
    fn test_svg_default_font_fallback() {
        // Empty fonts.names should fall back to the monospace constant.
        let prefs = simple_prefs(); // fonts.names = ""
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("font-family=\"monospace\""), "default font: {out}");
    }

    #[test]
    fn test_parsed_font() {
        assert_eq!(parsed_font("Georgia 14"), ("Georgia".to_string(), 14.0));
        assert_eq!(parsed_font("Arial Bold 10"), ("Arial Bold".to_string(), 10.0));
        assert_eq!(parsed_font(""), (FONT_FAMILY.to_string(), FONT_SIZE));
        assert_eq!(parsed_font("Courier"), ("Courier".to_string(), FONT_SIZE));
    }

    #[test]
    fn test_paper_size_mm() {
        let mut prefs = Prefs::default();
        prefs.output.paper.size = "A4".into();
        prefs.output.paper.orientation = "portrait".into();
        assert_eq!(paper_size_mm(&prefs), Some((210.0, 297.0)));

        prefs.output.paper.orientation = "landscape".into();
        assert_eq!(paper_size_mm(&prefs), Some((297.0, 210.0)));

        prefs.output.paper.size = "".into();
        assert_eq!(paper_size_mm(&prefs), None);
    }
}
