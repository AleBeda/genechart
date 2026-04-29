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
// More conservative estimate used only for dot-leader x1 placement.
// Proportional fonts (Georgia, etc.) average closer to 0.50× font-size.
const CHAR_WIDTH_RATIO_TIGHT: f64 = 0.50;
// Fixed pixel gap between text and the start/end of a dot leader.
const DOT_LEADER_GAP: f64 = 3.0;
/// Font-family used for symbol characters rendered in their own <text> element.
/// Lists only symbol fonts so usvg doesn't try the primary Latin font first.
const SYMBOL_FONT_FAMILY: &str =
    "'Apple Symbols', 'Segoe UI Symbol', 'DejaVu Sans', sans-serif";

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
pub(crate) fn paper_size_mm(prefs: &Prefs) -> Option<(f64, f64)> {
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
pub(crate) fn hex_color(val: i64) -> String {
    let r = (val >> 8) & 0xF;
    let g = (val >> 4) & 0xF;
    let b =  val       & 0xF;
    format!("#{r:X}{r:X}{g:X}{g:X}{b:X}{b:X}")
}

/// Draw a dotted leader line from `x1` to `x2` at text baseline `y`.
/// Only emits the element when there is meaningful space (> font_size px).
fn dot_leader(out: &mut String, x1: f64, x2: f64, y: f64, font_size: f64, color: &str) {
    let x1 = x1 + DOT_LEADER_GAP;
    let x2 = x2 - DOT_LEADER_GAP;
    if x2 > x1 + font_size {
        out.push_str(&format!(
            "  <line x1=\"{x1:.1}\" y1=\"{y:.1}\" x2=\"{x2:.1}\" y2=\"{y:.1}\" \
             stroke=\"{color}\" stroke-width=\"{:.2}\" \
             stroke-dasharray=\"1,3\" stroke-linecap=\"round\"/>\n",
            font_size * 0.07
        ));
    }
}

/// Render `text` at (x, y), splitting runs of Unicode symbol characters
/// (codepoint ≥ U+2000) into separate `<text>` elements with SYMBOL_FONT_FAMILY.
///
/// This prevents a symbol character like ⚭ from sharing a `<text>` element with
/// Latin characters — svg2pdf 0.13 corrupts cross-font text runs in the PDF.
fn render_mixed_text(
    out: &mut String,
    x: f64, y: f64,
    text: &str,
    primary_family: &str,
    font_size: f64,
    cw: f64,
) {
    if text.is_empty() {
        out.push_str(&svg_text(x, y, text, primary_family, font_size));
        return;
    }

    let mut cur_x   = x;
    let mut seg_start = 0usize;
    let mut in_symbol = (text.chars().next().map_or(0, |c| c as u32)) >= 0x2000;

    for (byte_pos, c) in text.char_indices() {
        let is_sym = (c as u32) >= 0x2000;
        if is_sym != in_symbol {
            let seg = &text[seg_start..byte_pos];
            let fam = if in_symbol { SYMBOL_FONT_FAMILY } else { primary_family };
            out.push_str(&svg_text(cur_x, y, seg, fam, font_size));
            cur_x    += seg.chars().count() as f64 * cw;
            seg_start = byte_pos;
            in_symbol = is_sym;
        }
    }
    // flush final segment
    let seg = &text[seg_start..];
    if !seg.is_empty() {
        let fam = if in_symbol { SYMBOL_FONT_FAMILY } else { primary_family };
        out.push_str(&svg_text(cur_x, y, seg, fam, font_size));
    }
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
    let (font_family_base, font_size) = parsed_font(&prefs.output.style.fonts.names);
    // Include symbol-font fallbacks so PDF renderers can find glyphs for ⚭, ×, etc.
    let font_family = format!(
        "{font_family_base}, 'Apple Symbols', 'Segoe UI Symbol', 'DejaVu Sans', sans-serif"
    );
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

    let dot_leaders = prefs.output.style.dot_leaders;

    // ── Text elements ─────────────────────────────────────────────────────────
    for (indi, geo) in &entries {
        let y      = MARGIN + (geo.line as f64 + 1.0) * line_height;
        let x_base = MARGIN + geo.indent as f64 * indent_px;
        let gpw    = gen_prefix_w(geo.generation);

        // Pre-compute event strings (needed for dot-leader geometry).
        let birth_s: Option<String> = if prefs.show.birth {
            indi.birth.as_ref().and_then(|e| {
                format_event(&prefs.format.birth, e.date.as_ref(), e.place.as_deref())
            })
        } else { None };
        let death_s: Option<String> = if prefs.show.death {
            indi.death.as_ref().and_then(|e| {
                format_event(&prefs.format.death, e.date.as_ref(), e.place.as_deref())
            })
        } else { None };
        let marr_s: Option<String> = if geo.is_spouse && prefs.show.marriage {
            find_marriage(indi, genrep).and_then(|e| {
                format_event(&prefs.format.marriage, e.date.as_ref(), e.place.as_deref())
            })
        } else { None };

        // Generation number (non-spouse only)
        if prefs.show.generation_num && !geo.is_spouse {
            let prefix = format!("{}. ", geo.generation);
            out.push_str(&svg_text(x_base, y, &prefix, &font_family, font_size));
        }

        // Name
        let name = format_name(indi, prefs);
        render_mixed_text(&mut out, x_base + gpw, y, &name, &font_family, font_size, cw);

        // Tight estimate of actual rendered text end (for name→birth dot-leader x1 only).
        let name_end_tight = x_base + gpw
            + name.chars().count() as f64 * font_size * CHAR_WIDTH_RATIO_TIGHT;
        let mut last_x = x_base + gpw + text_w(&name);

        // Birth (with optional dot leader; use tight estimate for the left edge)
        if let Some(ref s) = birth_s {
            if dot_leaders {
                dot_leader(&mut out, name_end_tight, x_birth, y, font_size, &conn_color);
            }
            render_mixed_text(&mut out, x_birth, y, s, &font_family, font_size, cw);
            last_x = x_birth + text_w(s);
        }

        // Death (with optional dot leader)
        if let Some(ref s) = death_s {
            if dot_leaders { dot_leader(&mut out, last_x, x_death, y, font_size, &conn_color); }
            render_mixed_text(&mut out, x_death, y, s, &font_family, font_size, cw);
            last_x = x_death + text_w(s);
        }

        // Marriage — spouse only (with optional dot leader)
        if let Some(ref s) = marr_s {
            if dot_leaders { dot_leader(&mut out, last_x, x_marriage, y, font_size, &conn_color); }
            render_mixed_text(&mut out, x_marriage, y, s, &font_family, font_size, cw);
        }
    }

    // ── Connector <line> elements (ancestors mode) ────────────────────────────
    //
    // x: aligned with the first character of the parent's name (after gen-prefix).
    // y: lines stop at the TOP / BOTTOM of the child row so they do not cross the name.
    for (_, geo) in &entries {
        // x at the parent's name-start (parent is one generation deeper = geo.generation + 1).
        let x_conn = MARGIN
            + (geo.indent + 1) as f64 * indent_px
            + gen_prefix_w(geo.generation + 1);
        let y_ctr = |line: usize| MARGIN + (line as f64 + 0.5) * line_height;
        let y_top = |line: usize| MARGIN +  line as f64         * line_height;
        let y_bot = |line: usize| MARGIN + (line as f64 + 1.0)  * line_height;

        if !geo.connectors_above.is_empty() {
            let first = *geo.connectors_above.iter().min().unwrap();
            if first > 0 {
                let father_line = first - 1;
                out.push_str(&svg_line(
                    x_conn, y_ctr(father_line),
                    x_conn, y_top(geo.line),
                    &conn_color, conn_width,
                ));
            }
        }

        if !geo.connectors_below.is_empty() {
            let last = *geo.connectors_below.iter().max().unwrap();
            let mother_line = last + 1;
            out.push_str(&svg_line(
                x_conn, y_bot(geo.line),
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
        // Font-family includes the base name plus fallbacks; check for base name presence.
        assert!(out.contains("Helvetica"),      "custom font family: {out}");
        assert!(out.contains("font-size=\"16\""), "custom font size: {out}");
    }

    #[test]
    fn test_svg_default_font_fallback() {
        let prefs = simple_prefs();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("monospace"), "default font: {out}");
    }

    // ── Dot leaders ──

    #[test]
    fn test_svg_dot_leaders_present_when_enabled() {
        let mut prefs = simple_prefs();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.birth = true;
        prefs.format.birth = "* {date}".into();
        prefs.output.style.dot_leaders = true;
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("stroke-dasharray"), "dot-leader lines expected: {out}");
    }

    #[test]
    fn test_svg_dot_leaders_absent_when_disabled() {
        let mut prefs = simple_prefs();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.birth = true;
        prefs.format.birth = "* {date}".into();
        prefs.output.style.dot_leaders = false;
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(!out.contains("stroke-dasharray"), "no dot leaders expected: {out}");
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

    #[test]
    fn test_svg_symbol_in_separate_element() {
        // Verify that a marriage string starting with ⚭ (U+26AD, codepoint ≥ U+2000)
        // is split: ⚭ must appear in an element whose font-family is the symbol list,
        // and "JAN" (Latin) must appear in an element whose font-family starts with the
        // primary font.
        let mut prefs = simple_prefs();
        prefs.show.marriage = true;
        prefs.format.marriage = "⚭ {date}".into();
        prefs.output.style.fonts.names = "Georgia 14".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();

        // The ⚭ character must be in a text element that does NOT start with Georgia.
        // SYMBOL_FONT_FAMILY starts with "'Apple Symbols'".
        let symbol_in_apple = out.lines().any(|l| {
            l.contains("Apple Symbols") && l.contains("⚭")
        });
        assert!(symbol_in_apple,
            "⚭ should be in a text element using the symbol font: {out}");

        // Latin characters ("APR") must not be in a symbol-font element.
        let latin_in_georgia = out.lines().any(|l| {
            l.contains("Georgia") && l.contains("APR")
        });
        assert!(latin_in_georgia,
            "Latin text should be in the primary-font element: {out}");
    }

    #[test]
    fn test_svg_dot_leader_gap_is_small() {
        let mut prefs = simple_prefs();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.birth = true;
        prefs.format.birth = "* {date}".into();
        prefs.output.style.dot_leaders = true;
        prefs.output.style.fonts.names = "monospace 14".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("stroke-dasharray"));
        let has_leader_line = out.lines().any(|l| l.contains("stroke-dasharray") && l.contains("x1="));
        assert!(has_leader_line, "no dot leader line found: {out}");
    }
}
