//! SVG back-end (simple and fan layouts).

use anyhow::Result;
use crate::backend::Renderer;
use crate::layout::LayoutOutput;
use std::collections::HashMap;
use crate::layout::simple::SimpleGeo;
use crate::layout::fan::FanGeo;
use crate::parser::genrep::{Genrep, Individual};
use crate::preferences::Prefs;
use crate::backend::text::{find_marriage, format_event, format_name};

// Fallback rendering constants (used when preferences are empty)
const LINE_HEIGHT: f64 = 18.0;
const FONT_SIZE:   f64 = 13.0;
const MARGIN:      f64 = 20.0;
const FONT_FAMILY: &str = "monospace";
// Estimated average character width as a fraction of font-size.
// Used for column-position arithmetic when exact glyph metrics are unavailable.
const CHAR_WIDTH_RATIO: f64 = 0.6;

// ── Low-level SVG primitives ──────────────────────────────────────────────────

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

fn svg_text(x: f64, y: f64, text: &str, family: &str, size: f64) -> String {
    format!(
        "  <text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"{family}\" \
         font-size=\"{size}\" xml:space=\"preserve\">{}</text>\n",
        xml_escape(text)
    )
}

fn svg_line(x1: f64, y1: f64, x2: f64, y2: f64, color: &str, width: f64) -> String {
    format!(
        "  <line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" \
         stroke=\"{color}\" stroke-width=\"{width}\"/>\n"
    )
}

// ── Preference helpers ────────────────────────────────────────────────────────

/// Parse "Family Name Size" preference string → (family, size).
/// The last whitespace-delimited token is tried as a float; the rest is the family.
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

/// Convert a 12-bit 0xRGB colour preference value to a CSS hex string.
fn hex_color(val: i64) -> String {
    let r = (val >> 8) & 0xF;
    let g = (val >> 4) & 0xF;
    let b =  val       & 0xF;
    format!("#{r:X}{r:X}{g:X}{g:X}{b:X}{b:X}")
}

fn svg_header(canvas_w: &str, canvas_h: &str, viewbox: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" \
         width=\"{canvas_w}\" height=\"{canvas_h}\" \
         viewBox=\"{viewbox}\">\n"
    )
}

// ── Simple layout — pixel-accurate SVG rendering ──────────────────────────────
//
// Unlike the text backend (which calls build_lines() and renders pre-formatted
// strings with spaces), this renderer places every element at a computed pixel
// x-coordinate and draws connector lines as <line> elements.  This is correct
// for variable-width (proportional) fonts.

fn render_simple(genrep: &Genrep<SimpleGeo>, prefs: &Prefs) -> String {
    // Font metrics
    let (font_family, font_size) = parsed_font(&prefs.output.style.fonts.names);
    let line_height = font_size * (LINE_HEIGHT / FONT_SIZE);
    let cw          = font_size * CHAR_WIDTH_RATIO; // estimated character width

    // Connector style
    let conn_color = hex_color(prefs.output.style.connectors.border);
    let conn_width = if prefs.output.style.connectors.width > 0.0 {
        prefs.output.style.connectors.width
    } else {
        0.5
    };

    // Collect and sort in-scope individuals by line number.
    let mut entries: Vec<(&Individual<SimpleGeo>, &SimpleGeo)> = genrep.individuals.values()
        .filter(|i| i.in_scope)
        .filter_map(|i| i.geo.as_ref().map(|g| (i, g)))
        .collect();
    entries.sort_by_key(|(_, g)| g.line);

    if entries.is_empty() {
        return format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <svg xmlns=\"http://www.w3.org/2000/svg\" \
             width=\"100\" height=\"100\"></svg>\n"
        );
    }

    let max_line   = entries.iter().map(|(_, g)| g.line).max().unwrap_or(0);
    let indent_px  = (prefs.layout.simple.indent as f64 * cw).max(cw);

    // Width (in px) of the generation-number prefix "N. " for a given generation.
    let gen_prefix_w = |generation: usize| -> f64 {
        if prefs.show.generation_num {
            format!("{}. ", generation).chars().count() as f64 * cw
        } else {
            0.0
        }
    };

    // Estimated pixel width of a string.
    let text_w = |s: &str| -> f64 { s.chars().count() as f64 * cw };

    // ── Compute pixel column positions ────────────────────────────────────────

    // Right edge of the widest name (considering indent + gen-prefix + name).
    let max_name_end: f64 = entries.iter().map(|(indi, geo)| {
        MARGIN
            + geo.indent as f64 * indent_px
            + gen_prefix_w(geo.generation)
            + text_w(&format_name(indi, prefs))
    }).fold(0.0_f64, f64::max);

    let gap = cw * 2.0; // column gap

    let max_birth_w: f64 = if prefs.show.birth {
        entries.iter()
            .filter_map(|(i, _)| i.birth.as_ref().and_then(|e| {
                format_event(&prefs.format.birth, e.date.as_ref(), e.place.as_deref())
            }))
            .map(|s| text_w(&s))
            .fold(0.0_f64, f64::max)
    } else { 0.0 };

    let max_death_w: f64 = if prefs.show.death {
        entries.iter()
            .filter_map(|(i, _)| i.death.as_ref().and_then(|e| {
                format_event(&prefs.format.death, e.date.as_ref(), e.place.as_deref())
            }))
            .map(|s| text_w(&s))
            .fold(0.0_f64, f64::max)
    } else { 0.0 };

    let max_marr_w: f64 = if prefs.show.marriage {
        entries.iter()
            .filter_map(|(i, g)| {
                if g.is_spouse {
                    find_marriage(i, genrep).and_then(|e| {
                        format_event(&prefs.format.marriage, e.date.as_ref(), e.place.as_deref())
                    })
                } else { None }
            })
            .map(|s| text_w(&s))
            .fold(0.0_f64, f64::max)
    } else { 0.0 };

    let x_birth    = max_name_end + gap;
    let x_death    = x_birth  + max_birth_w + gap;
    let x_marriage = x_death  + max_death_w + gap;

    let content_right = if max_marr_w  > 0.0 { x_marriage + max_marr_w  }
                   else if max_death_w > 0.0 { x_death    + max_death_w }
                   else if max_birth_w > 0.0 { x_birth    + max_birth_w }
                   else                      { max_name_end };
    let content_w = content_right + MARGIN;
    let content_h = MARGIN * 2.0 + (max_line + 1) as f64 * line_height;

    // ── Build SVG ─────────────────────────────────────────────────────────────

    let (canvas_w, canvas_h) = match paper_size_mm(prefs) {
        Some((pw, ph)) => (format!("{pw}mm"), format!("{ph}mm")),
        None           => (format!("{content_w:.0}"), format!("{content_h:.0}")),
    };
    let viewbox = format!("0 0 {content_w:.1} {content_h:.1}");

    let mut out = svg_header(&canvas_w, &canvas_h, &viewbox);

    // ── Text elements ─────────────────────────────────────────────────────────
    for (indi, geo) in &entries {
        let y      = MARGIN + (geo.line as f64 + 1.0) * line_height;
        let x_base = MARGIN + geo.indent as f64 * indent_px;
        let gpw    = gen_prefix_w(geo.generation);

        // Generation number (non-spouse only)
        if prefs.show.generation_num && !geo.is_spouse {
            let prefix = format!("{}. ", geo.generation);
            out.push_str(&svg_text(x_base, y, &prefix, &font_family, font_size));
        }

        // Name (starts after gen-prefix, same column for spouse and non-spouse)
        let name = format_name(indi, prefs);
        out.push_str(&svg_text(x_base + gpw, y, &name, &font_family, font_size));

        // Birth
        if prefs.show.birth {
            if let Some(s) = indi.birth.as_ref().and_then(|e| {
                format_event(&prefs.format.birth, e.date.as_ref(), e.place.as_deref())
            }) {
                out.push_str(&svg_text(x_birth, y, &s, &font_family, font_size));
            }
        }

        // Death
        if prefs.show.death {
            if let Some(s) = indi.death.as_ref().and_then(|e| {
                format_event(&prefs.format.death, e.date.as_ref(), e.place.as_deref())
            }) {
                out.push_str(&svg_text(x_death, y, &s, &font_family, font_size));
            }
        }

        // Marriage (spouse only)
        if geo.is_spouse && prefs.show.marriage {
            if let Some(evt) = find_marriage(indi, genrep) {
                if let Some(s) = format_event(&prefs.format.marriage, evt.date.as_ref(), evt.place.as_deref()) {
                    out.push_str(&svg_text(x_marriage, y, &s, &font_family, font_size));
                }
            }
        }
    }

    // ── Connector <line> elements (ancestors mode) ────────────────────────────
    //
    // connectors_above[i] / connectors_below[i] hold the logical line numbers
    // of blank rows between an individual and their father / mother.
    // We draw one vertical line per connector group from the centre of the
    // parent row to the centre of the child row.
    //
    // x position: one indent step to the right of the individual (= parent indent).
    for (_, geo) in &entries {
        let x_conn = MARGIN + (geo.indent + 1) as f64 * indent_px;
        let y_ctr  = |line: usize| MARGIN + (line as f64 + 0.5) * line_height;

        if !geo.connectors_above.is_empty() {
            // Derive parent (father) line from the first blank line above.
            let first = *geo.connectors_above.iter().min().unwrap();
            if first > 0 {
                let father_line = first - 1;
                out.push_str(&svg_line(
                    x_conn, y_ctr(father_line),
                    x_conn, y_ctr(geo.line),
                    &conn_color, conn_width,
                ));
            }
        }

        if !geo.connectors_below.is_empty() {
            // Derive parent (mother) line from the last blank line below.
            let last = *geo.connectors_below.iter().max().unwrap();
            let mother_line = last + 1;
            out.push_str(&svg_line(
                x_conn, y_ctr(geo.line),
                x_conn, y_ctr(mother_line),
                &conn_color, conn_width,
            ));
        }
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
fn wedge_path(cx: f64, cy: f64, geo: &FanGeo) -> String {
    let a0 = (geo.angle_center - geo.angle_span / 2.0).to_radians();
    let a1 = (geo.angle_center + geo.angle_span / 2.0).to_radians();
    let ri = geo.radius_inner;
    let ro = geo.radius_outer;

    let ox0 = cx + ro * a0.cos();  let oy0 = cy - ro * a0.sin();
    let ox1 = cx + ro * a1.cos();  let oy1 = cy - ro * a1.sin();

    let laf = if geo.angle_span >= 180.0 { 1 } else { 0 };

    if ri < 1.0 {
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

    let content_w = 2.0 * (max_radius + MARGIN);
    let content_h = max_radius + 2.0 * MARGIN;
    let cx = content_w / 2.0;
    let cy = content_h - MARGIN;

    let (canvas_w, canvas_h) = match paper_size_mm(prefs) {
        Some((pw, ph)) => (format!("{pw}mm"), format!("{ph}mm")),
        None           => (format!("{content_w:.0}"), format!("{content_h:.0}")),
    };
    let viewbox = format!("0 0 {content_w:.1} {content_h:.1}");

    let mut out = svg_header(&canvas_w, &canvas_h, &viewbox);

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

        let label  = format_fan_name(indi, prefs);
        let tx     = cx + geo.x;
        let ty     = cy - geo.y;
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
        LayoutOutput::Simple(genrep)  => Ok(render_simple(genrep, prefs)),
        LayoutOutput::BoxedCouples(_) => anyhow::bail!("BoxedCouples SVG not yet implemented"),
        LayoutOutput::Fan(genrep)     => Ok(render_fan(genrep, prefs)),
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

    // ── Structure ──

    #[test]
    fn test_svg_structure() {
        let prefs = simple_prefs();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("<svg "),   "missing <svg: {out}");
        assert!(out.contains("</svg>"),  "missing </svg: {out}");
        assert!(out.contains("<text "),  "missing <text: {out}");
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

    // ── Pixel-accurate layout ──

    #[test]
    fn test_svg_separate_text_elements_per_field() {
        // With pixel layout, each field (name, birth) is a separate <text> element.
        // Count <text elements to verify name and birth are rendered separately.
        let mut prefs = simple_prefs();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.birth = true;
        prefs.format.birth = "* {date}".into();
        // I1 has a birth date; render 3 individuals → at least 4 text elements
        // (3 names + 1 birth event for John)
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        let count = out.matches("<text ").count();
        assert!(count >= 4, "expected ≥4 <text elements, got {count}: {out}");
    }

    #[test]
    fn test_svg_no_bar_characters() {
        // Vertical connectors must never appear as │ characters in SVG output.
        let mut prefs = simple_prefs();
        prefs.scope.direction = "ancestors".into();
        prefs.scope.root = "I3".into();
        prefs.layout.simple.vert_spacing = 1;
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(2));
        let layout_out = run_layout(&genrep, &prefs).unwrap();
        let out = render_to_string(&layout_out, &prefs).unwrap();
        assert!(!out.contains('│'), "SVG must not contain │ bar characters: {out}");
    }

    #[test]
    fn test_svg_connector_lines_present() {
        // When vert_spacing > 0 and direction is ancestors, <line> elements must appear.
        let mut prefs = simple_prefs();
        prefs.scope.direction = "ancestors".into();
        prefs.scope.root = "I3".into();
        prefs.layout.simple.vert_spacing = 1;
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(2));
        let layout_out = run_layout(&genrep, &prefs).unwrap();
        let out = render_to_string(&layout_out, &prefs).unwrap();
        assert!(out.contains("<line "), "connector <line> elements expected: {out}");
    }

    // ── Paper sizing ──

    #[test]
    fn test_svg_content_sized_when_no_paper() {
        let prefs = simple_prefs();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
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

    // ── Font prefs ──

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
        let prefs = simple_prefs();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("font-family=\"monospace\""), "default font: {out}");
    }

    // ── Unit helpers ──

    #[test]
    fn test_parsed_font() {
        assert_eq!(parsed_font("Georgia 14"),    ("Georgia".to_string(), 14.0));
        assert_eq!(parsed_font("Arial Bold 10"), ("Arial Bold".to_string(), 10.0));
        assert_eq!(parsed_font(""),              (FONT_FAMILY.to_string(), FONT_SIZE));
        assert_eq!(parsed_font("Courier"),       ("Courier".to_string(), FONT_SIZE));
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

    #[test]
    fn test_hex_color() {
        assert_eq!(hex_color(0x000), "#000000");
        assert_eq!(hex_color(0xFFF), "#FFFFFF");
        assert_eq!(hex_color(0x222), "#222222");
    }
}
