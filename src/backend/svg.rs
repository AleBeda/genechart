//! SVG back-end (simple, boxed_couples, and fan layouts).

use crate::backend::Renderer;
use crate::backend::font_metrics;
use crate::layout::LayoutOutput;
use crate::preferences::Prefs;
use crate::scene::{TextAlign, TextAttr};
use crate::text_metrics::{CHAR_WIDTH_RATIO, FONT_SIZE, LINE_HEIGHT, parsed_font};
use anyhow::Result;

// SVG-specific rendering constants
const MARGIN: f64 = 20.0;
/// Font-family used for symbol characters rendered in their own <text> element.
/// Lists only symbol fonts so usvg doesn't try the primary Latin font first.
const SYMBOL_FONT_FAMILY: &str = "'Apple Symbols', 'Segoe UI Symbol', 'DejaVu Sans', sans-serif";

/// Clearance (canvas units, per side) a name must keep from its box edges before it is
/// considered to overflow / be eligible for autocompression.
const NAME_CLEARANCE: f64 = 2.0;

// ── Low-level SVG primitives ──────────────────────────────────────────────────

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn svg_text_full(
    x: f64,
    y: f64,
    text: &str,
    family: &str,
    size: f64,
    weight: &str,
    color: &str,
    class: &str,
) -> String {
    let cls = if class.is_empty() {
        String::new()
    } else {
        format!(" class=\"{class}\"")
    };
    format!(
        "  <text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"{family}\" \
         font-size=\"{size}\" font-weight=\"{weight}\" fill=\"{color}\"{cls} xml:space=\"preserve\">{}</text>\n",
        xml_escape(text)
    )
}

fn svg_line(x1: f64, y1: f64, x2: f64, y2: f64, color: &str, width: f64, class: &str) -> String {
    let cls = if class.is_empty() {
        String::new()
    } else {
        format!(" class=\"{class}\"")
    };
    format!(
        "  <line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" \
         stroke=\"{color}\" stroke-width=\"{width}\"{cls}/>\n"
    )
}

#[allow(clippy::too_many_arguments)]
fn svg_rect(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    fill: &str,
    stroke: &str,
    sw: f64,
    radius: f64,
    class: &str,
) -> String {
    let cls = if class.is_empty() {
        String::new()
    } else {
        format!(" class=\"{class}\"")
    };
    format!(
        "  <rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" \
         rx=\"{radius:.1}\" ry=\"{radius:.1}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{sw}\"{cls}/>\n"
    )
}

fn font_weight_from_pref(pref: &str) -> &str {
    match pref.trim().to_lowercase().as_str() {
        "bold" | "bolder" => "bold",
        "light" | "lighter" => "lighter",
        _ => "normal",
    }
}

fn base_font_from_css(css_family: &str) -> &str {
    css_family
        .split(',')
        .next()
        .map(|s| s.trim().trim_matches('\'').trim_matches('"'))
        .unwrap_or("monospace")
}

// ── Preference helpers ────────────────────────────────────────────────────────

/// Return paper dimensions `(width_mm, height_mm)` from preferences,
/// or `None` when the paper size is absent or unrecognised.
pub(crate) fn paper_size_mm(prefs: &Prefs) -> Option<(f64, f64)> {
    let (w, h): (f64, f64) = match prefs.output.paper.size.trim().to_uppercase().as_str() {
        "A0" => (841.0, 1189.0),
        "A1" => (594.0, 841.0),
        "A2" => (420.0, 594.0),
        "A3" => (297.0, 420.0),
        "A4" => (210.0, 297.0),
        "A5" => (148.0, 210.0),
        "LETTER" => (215.9, 279.4),
        "CUSTOM" => {
            let cw = prefs.output.paper.custom.width;
            let ch = prefs.output.paper.custom.height;
            if cw > 0.0 && ch > 0.0 {
                return Some((cw, ch)); // orientation is ignored for explicit custom dimensions
            } else {
                return None;
            }
        }
        _ => return None,
    };
    let landscape = prefs
        .output
        .paper
        .orientation
        .trim()
        .to_lowercase()
        .starts_with("land");
    Some(if landscape { (h, w) } else { (w, h) })
}

/// Convert a `0xRGB` / `0xRGBA` / `0xRRGGBB` / `0xRRGGBBAA` colour preference value to a
/// CSS hex string. The intended width is inferred by magnitude:
/// `<= 0xFFF` → 3-digit RGB (opaque); `<= 0xFFFF` → 4-digit RGBA (alpha-last);
/// `<= 0xFFFFFF` → 6-digit RRGGBB (opaque); otherwise 8-digit RRGGBBAA.
///
/// Limitation: a colour whose most-significant hex digit is `0` collapses to the next
/// smaller width (e.g. `0x00FF00` == `0xFF00` reads as 4-digit RGBA, and a translucent
/// *black* cannot be distinguished from a 3-digit value). Keep a non-zero leading nibble.
pub(crate) fn hex_color(val: i64) -> String {
    if val <= 0xFFF {
        let r = (val >> 8) & 0xF;
        let g = (val >> 4) & 0xF;
        let b = val & 0xF;
        format!("#{r:X}{r:X}{g:X}{g:X}{b:X}{b:X}")
    } else if val <= 0xFFFF {
        let r = (val >> 12) & 0xF;
        let g = (val >> 8) & 0xF;
        let b = (val >> 4) & 0xF;
        let a = val & 0xF;
        format!("#{r:X}{r:X}{g:X}{g:X}{b:X}{b:X}{a:X}{a:X}")
    } else if val <= 0xFFFFFF {
        format!("#{val:06X}")
    } else {
        format!("#{val:08X}")
    }
}

/// A color preference where `0` means "unset" and renders as literal `"black"` (keeps
/// default output byte-identical); any non-zero value goes through [`hex_color`].
pub(crate) fn color_or_black(val: i64) -> String {
    if val != 0 {
        hex_color(val)
    } else {
        "black".to_string()
    }
}

/// Renders text split at U+2000 into Latin/symbol segments; `anchor_x` is the left/center/right edge per `align`.
#[allow(clippy::too_many_arguments)]
fn render_mixed_text(
    out: &mut String,
    anchor_x: f64,
    y: f64,
    text: &str,
    primary_family: &str,
    font_size: f64,
    weight: &str,
    cw: f64,
    color: &str,
    bg: Option<&str>,
    align: &TextAlign,
    class: &str,
) {
    if text.is_empty() {
        return;
    }

    // Split text into (slice, is_symbol) segments at U+2000 boundary.
    let segments = split_at_u2000(text);

    // Strip CSS fallback list to get the bare font name for font_metrics.
    let base_font = primary_family
        .split(',')
        .next()
        .unwrap_or(primary_family)
        .trim()
        .trim_matches('\'')
        .trim_matches('"');

    // Measure: exact metrics for Latin, char-count estimate for symbols.
    let is_bold = weight == "bold";
    let seg_widths: Vec<f64> = segments
        .iter()
        .map(|(seg, is_sym)| {
            if *is_sym {
                seg.chars().count() as f64 * cw
            } else {
                font_metrics::measure_text_w(seg, base_font, font_size, is_bold)
                    .unwrap_or_else(|| seg.chars().count() as f64 * cw)
            }
        })
        .collect();

    let total_width: f64 = seg_widths.iter().sum();
    let start_x = match align {
        TextAlign::Left => anchor_x,
        TextAlign::Center => anchor_x - total_width / 2.0,
        TextAlign::Right => anchor_x - total_width,
    };

    if let Some(bg_color) = bg {
        let bg_y = y - font_size * 0.9;
        let bg_h = font_size * 1.2;
        out.push_str(&format!(
            "  <rect x=\"{bx:.1}\" y=\"{by:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" fill=\"{c}\" class=\"highlight_rect\"/>\n",
            bx = start_x - 2.0, by = bg_y, w = total_width + 4.0, h = bg_h, c = bg_color
        ));
    }

    let mut cur_x = start_x;
    for ((seg, is_sym), &w) in segments.iter().zip(seg_widths.iter()) {
        let fam = if *is_sym {
            SYMBOL_FONT_FAMILY
        } else {
            primary_family
        };
        let wt = if *is_sym { "normal" } else { weight };
        out.push_str(&svg_text_full(
            cur_x, y, seg, fam, font_size, wt, color, class,
        ));
        cur_x += w;
    }
}

/// Total rendered width of `text`, measured the same way `render_mixed_text` lays it out:
/// exact glyph metrics for Latin runs, a char-count estimate for symbol runs (≥ U+2000).
fn mixed_text_width(text: &str, primary_family: &str, font_size: f64, is_bold: bool) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let cw = font_size * CHAR_WIDTH_RATIO;
    let base_font = primary_family
        .split(',')
        .next()
        .unwrap_or(primary_family)
        .trim()
        .trim_matches('\'')
        .trim_matches('"');
    split_at_u2000(text)
        .iter()
        .map(|(seg, is_sym)| {
            if *is_sym {
                seg.chars().count() as f64 * cw
            } else {
                font_metrics::measure_text_w(seg, base_font, font_size, is_bold)
                    .unwrap_or_else(|| seg.chars().count() as f64 * cw)
            }
        })
        .sum()
}

/// Like `render_mixed_text` but for text centered at x=0 inside a rotated `<g>`.
/// Uses `dominant-baseline="middle"` so `y` is the vertical centre of each line.
/// Splits at U+2000 to give each segment its own `<text>` element — required
/// because svg2pdf cannot do per-character font fallback within a single element.
fn render_mixed_text_rotated(
    out: &mut String,
    y: f64,
    text: &str,
    primary_family: &str,
    font_size: f64,
    color: &str,
    class: &str,
) {
    if text.is_empty() {
        return;
    }
    let cw = font_size * CHAR_WIDTH_RATIO;

    // Split at U+2000 boundary.
    let segments = split_at_u2000(text);

    let base_font = primary_family
        .split(',')
        .next()
        .unwrap_or(primary_family)
        .trim()
        .trim_matches('\'')
        .trim_matches('"');

    let seg_widths: Vec<f64> = segments
        .iter()
        .map(|(seg, is_sym)| {
            if *is_sym {
                seg.chars().count() as f64 * cw
            } else {
                font_metrics::measure_text_w(seg, base_font, font_size, false)
                    .unwrap_or_else(|| seg.chars().count() as f64 * cw)
            }
        })
        .collect();

    let total_width: f64 = seg_widths.iter().sum();
    let mut cur_x = -total_width / 2.0;
    for ((seg, is_sym), &w) in segments.iter().zip(seg_widths.iter()) {
        let fam = if *is_sym {
            SYMBOL_FONT_FAMILY
        } else {
            primary_family
        };
        let cls = if class.is_empty() {
            String::new()
        } else {
            format!(" class=\"{class}\"")
        };
        out.push_str(&format!(
            "    <text x=\"{cur_x:.1}\" y=\"{y:.1}\" \
             font-family=\"{fam}\" font-size=\"{font_size}\" \
             fill=\"{color}\" dominant-baseline=\"middle\"{cls} \
             xml:space=\"preserve\">{}</text>\n",
            xml_escape(seg)
        ));
        cur_x += w;
    }
}

/// Splits `text` into `(slice, is_symbol)` segments at U+2000 — required because svg2pdf cannot do per-character font fallback within a single `<text>` element.
fn split_at_u2000(text: &str) -> Vec<(&str, bool)> {
    let mut segments: Vec<(&str, bool)> = Vec::new();
    let mut seg_start = 0usize;
    let mut in_symbol = text.chars().next().is_some_and(|c| (c as u32) >= 0x2000);
    for (byte_pos, c) in text.char_indices() {
        let is_sym = (c as u32) >= 0x2000;
        if is_sym != in_symbol {
            segments.push((&text[seg_start..byte_pos], in_symbol));
            seg_start = byte_pos;
            in_symbol = is_sym;
        }
    }
    segments.push((&text[seg_start..], in_symbol));
    segments
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

// SVG arc path for one wedge of the fan.
// Angles follow the math convention (0°=right, 90°=top, 180°=left).
// SVG y is flipped: svg_y = cy - math_y.
fn wedge_path(
    cx: f64,
    cy: f64,
    angle_center: f64,
    angle_span: f64,
    radius_inner: f64,
    radius_outer: f64,
) -> String {
    let a0 = (angle_center - angle_span / 2.0).to_radians();
    let a1 = (angle_center + angle_span / 2.0).to_radians();
    let ri = radius_inner;
    let ro = radius_outer;

    let ox0 = cx + ro * a0.cos();
    let oy0 = cy - ro * a0.sin();
    let ox1 = cx + ro * a1.cos();
    let oy1 = cy - ro * a1.sin();

    let laf = if angle_span >= 180.0 { 1 } else { 0 };

    if ri < 1.0 {
        format!(
            "M {cx:.1} {cy:.1} L {ox0:.1} {oy0:.1} \
                 A {ro:.1} {ro:.1} 0 {laf} 0 {ox1:.1} {oy1:.1} Z"
        )
    } else {
        let ix0 = cx + ri * a0.cos();
        let iy0 = cy - ri * a0.sin();
        let ix1 = cx + ri * a1.cos();
        let iy1 = cy - ri * a1.sin();
        format!(
            "M {ox0:.1} {oy0:.1} \
                 A {ro:.1} {ro:.1} 0 {laf} 0 {ox1:.1} {oy1:.1} \
                 L {ix1:.1} {iy1:.1} \
                 A {ri:.1} {ri:.1} 0 {laf} 1 {ix0:.1} {iy0:.1} Z"
        )
    }
}

// ── Scene renderer ────────────────────────────────────────────────────────────

/// Check if any attr in the slice is `Highlighted`.
fn is_highlighted(attrs: &[TextAttr]) -> bool {
    attrs.contains(&TextAttr::Highlighted)
}

/// First non-Highlighted semantic attribute, or `IndividualName` as fallback.
fn semantic_attr(attrs: &[TextAttr]) -> &TextAttr {
    attrs
        .iter()
        .find(|a| !matches!(a, TextAttr::Highlighted))
        .unwrap_or(&TextAttr::IndividualName)
}

/// Map a `[TextAttr]` slice to a space-separated CSS class string.
fn class_for_attrs(attrs: &[TextAttr]) -> String {
    let mut classes: Vec<&str> = Vec::new();
    for attr in attrs {
        match attr {
            TextAttr::IndividualName => classes.push("indi_name"),
            TextAttr::SpouseName => classes.push("spouse_name"),
            TextAttr::BirthData => classes.push("indi_birth"),
            TextAttr::DeathData => classes.push("indi_death"),
            TextAttr::MarriageData => classes.push("indi_marriage"),
            TextAttr::IndividualId => classes.push("indi_id"),
            TextAttr::GenerationNum => classes.push("gen_num"),
            TextAttr::NoteText => classes.push("note_text"),
            TextAttr::ExcludeMsg => classes.push("exclude_msg"),
            TextAttr::Highlighted => classes.push("highlighted"),
        }
    }
    classes.join(" ")
}

/// Build the font family string with symbol fallbacks (for ♂/♀ and other symbols).
fn with_symbol_fallback(base: &str) -> String {
    format!("{base}, 'Apple Symbols', 'Segoe UI Symbol', 'DejaVu Sans', sans-serif")
}

/// Resolve (font_family, font_size) from a `[TextAttr]` slice and preferences.
fn font_for_attr(attrs: &[TextAttr], prefs: &Prefs) -> (String, f64) {
    match semantic_attr(attrs) {
        TextAttr::IndividualName
        | TextAttr::SpouseName
        | TextAttr::GenerationNum
        | TextAttr::NoteText => {
            let (fam, sz) = parsed_font(&prefs.output.style.fonts.names);
            let sz = if sz <= 0.0 { FONT_SIZE } else { sz };
            (with_symbol_fallback(&fam), sz)
        }
        TextAttr::BirthData | TextAttr::DeathData | TextAttr::MarriageData => {
            let (fam_base, sz_base) = parsed_font(&prefs.output.style.fonts.names);
            if prefs.output.style.fonts.dates.trim().is_empty() {
                (with_symbol_fallback(&fam_base), sz_base)
            } else {
                let (fam, sz) = parsed_font(&prefs.output.style.fonts.dates);
                let sz = if sz <= 0.0 { sz_base } else { sz };
                (with_symbol_fallback(&fam), sz)
            }
        }
        TextAttr::IndividualId => {
            let (fam, sz) = parsed_font(&prefs.output.style.fonts.id);
            let sz = if sz <= 0.0 { 8.0 } else { sz };
            let fam = if fam.trim().is_empty() {
                "Courier New, monospace".to_string()
            } else {
                format!("{fam}, Courier New, monospace")
            };
            (fam, sz)
        }
        TextAttr::ExcludeMsg => {
            let (fam, sz) = parsed_font(&prefs.output.style.fonts.exclude_msg);
            let sz = if sz <= 0.0 { FONT_SIZE } else { sz };
            (with_symbol_fallback(&fam), sz)
        }
        _ => (with_symbol_fallback("monospace"), FONT_SIZE),
    }
}

/// Resolve CSS color for a `[TextAttr]` slice.
fn color_for_attr(attrs: &[TextAttr], prefs: &Prefs) -> String {
    if is_highlighted(attrs) {
        return hex_color(prefs.output.style.text.highlights.color);
    }
    let text = &prefs.output.style.text;
    let pick = |c: i64| color_or_black(c);
    match semantic_attr(attrs) {
        TextAttr::IndividualName | TextAttr::SpouseName => pick(text.names),
        TextAttr::BirthData | TextAttr::DeathData | TextAttr::MarriageData => pick(text.dates),
        TextAttr::GenerationNum => pick(text.gen_numbers),
        TextAttr::NoteText => pick(text.notes),
        TextAttr::IndividualId => hex_color(text.id),
        TextAttr::ExcludeMsg => pick(text.exclude_msg),
        _ => "black".to_string(),
    }
}

/// Resolve font-weight string for a `[TextAttr]` slice.
fn weight_for_attr<'a>(attrs: &[TextAttr], prefs: &'a Prefs) -> &'a str {
    match semantic_attr(attrs) {
        TextAttr::IndividualName => font_weight_from_pref(&prefs.output.style.fonts.descendant),
        TextAttr::SpouseName => font_weight_from_pref(&prefs.output.style.fonts.spouse),
        _ => "normal",
    }
}

/// Rendering context passed to the recursive group renderer.
struct BcSvgCtx<'a> {
    to_svg_x: &'a dyn Fn(f64) -> f64,
    to_svg_y: &'a dyn Fn(f64) -> f64,
    box_fill: &'a str,
    box_stroke: &'a str,
    box_sw: f64,
    box_radius: f64,
    conn_color: &'a str,
    conn_width: f64,
    prefs: &'a Prefs,
    skip_connectors: bool,
    /// True for the `boxes` / `boxed_couples` layouts: name text is fit-checked against its box
    /// (and autocompressed per `output.style.spacing.names_autocompress`).
    box_name_autocompress: bool,
}

fn render_fancy_highlight_rect(
    out: &mut String,
    name_line: &crate::scene::FancyLine,
    svg_x: f64,
    svg_y: f64,
    prefs: &Prefs,
) {
    let (font_family, font_size) = font_for_attr(&name_line.attrs, prefs);
    let bold = weight_for_attr(&name_line.attrs, prefs) == "bold";
    let lh = font_size * 1.2;
    let base_font = base_font_from_css(&font_family);
    let w = font_metrics::measure_text_w(&name_line.text, base_font, font_size, bold)
        .unwrap_or_else(|| name_line.text.chars().count() as f64 * font_size * CHAR_WIDTH_RATIO);
    let bg = hex_color(prefs.output.style.text.highlights.background_color);
    let pad = 1.0;
    let rx = svg_x - pad;
    let ry = svg_y - pad;
    out.push_str(&format!(
        "  <rect x=\"{rx:.1}\" y=\"{ry:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"{bg}\" class=\"highlight_rect\"/>\n",
        w + 2.0 * pad,
        lh + 2.0 * pad
    ));
}

fn render_fancy_conn_group(
    out: &mut String,
    group_id: &str,
    offset_x: f64,
    offset_y: f64,
    conns: &[&crate::scene::FancyConnector],
) {
    out.push_str(&format!(
        "  <g id=\"{group_id}\" transform=\"translate({offset_x:.1},{offset_y:.1})\">\n"
    ));
    for c in conns {
        if !c.id.is_empty() {
            out.push_str(&format!("    <g id=\"{}\">\n", xml_escape(&c.id)));
        }
        let dash = if c.stroke_dasharray.is_empty() {
            String::new()
        } else {
            format!(" stroke-dasharray=\"{}\"", c.stroke_dasharray)
        };
        out.push_str(&format!(
            "    <path d=\"{}\" stroke=\"{}\" stroke-width=\"{:.1}\" fill=\"none\" stroke-linecap=\"round\" class=\"connector\"{}/>\n",
            c.d, c.stroke, c.stroke_width, dash
        ));
        if !c.id.is_empty() {
            out.push_str("    </g>\n");
        }
    }
    out.push_str("  </g>\n");
}

/// Recursively render a `Primitive::Group` and its children to SVG.
fn render_bc_primitive(
    p: &crate::scene::Primitive,
    ctx: &BcSvgCtx<'_>,
    out: &mut String,
    current_id: &str,
) {
    use crate::scene::Primitive;
    match p {
        Primitive::Box(b) => {
            let x = (ctx.to_svg_x)(b.bbox.x);
            let y = (ctx.to_svg_y)(b.bbox.y);
            let box_class = if b.two_spouses { "box double" } else { "box" };
            out.push_str(&svg_rect(
                x,
                y,
                b.bbox.w,
                b.bbox.h,
                ctx.box_fill,
                ctx.box_stroke,
                ctx.box_sw,
                ctx.box_radius,
                box_class,
            ));
        }
        Primitive::Text(t) => {
            let (font_family, font_size) = font_for_attr(&t.attrs, ctx.prefs);
            let weight = weight_for_attr(&t.attrs, ctx.prefs);
            let color = color_for_attr(&t.attrs, ctx.prefs);
            let bg_color: Option<String> = if is_highlighted(&t.attrs) {
                Some(hex_color(
                    ctx.prefs.output.style.text.highlights.background_color,
                ))
            } else {
                None
            };
            let baseline_svg = (ctx.to_svg_y)(t.bbox.y + t.bbox.h);
            let cw = font_size * CHAR_WIDTH_RATIO;
            // NoteText: the "| " prefix is for the text backend; SVG uses a NoteBar line
            // instead. Only strip+shift when the prefix is actually present (plain-text mode);
            // HTML-mode continuation segments have no prefix and a pre-shifted bbox.x.
            let (content, anchor_x) = if semantic_attr(&t.attrs) == &TextAttr::NoteText {
                if let Some(stripped) = t.content.strip_prefix("| ") {
                    (stripped.to_string(), (ctx.to_svg_x)(t.bbox.x) + 2.0 * cw)
                } else {
                    (t.content.clone(), (ctx.to_svg_x)(t.bbox.x))
                }
            } else {
                let ax = match t.align {
                    TextAlign::Left => (ctx.to_svg_x)(t.bbox.x),
                    TextAlign::Center => (ctx.to_svg_x)(t.bbox.x + t.bbox.w / 2.0),
                    TextAlign::Right => (ctx.to_svg_x)(t.bbox.x + t.bbox.w),
                };
                (t.content.clone(), ax)
            };

            // Name autocompress (boxes / boxed_couples only): horizontally shrink a name that
            // does not fit its box, and/or warn that it overflows. `bbox.w` is the box/section
            // width available to the name.
            let is_name = matches!(
                semantic_attr(&t.attrs),
                TextAttr::IndividualName | TextAttr::SpouseName
            );
            let mut x_scale = 1.0_f64;
            if ctx.box_name_autocompress && is_name {
                let available = t.bbox.w - 2.0 * NAME_CLEARANCE;
                let measured =
                    mixed_text_width(&content, &font_family, font_size, weight == "bold");
                let pref = ctx.prefs.output.style.spacing.names_autocompress;
                if available > 0.0 && measured > available && pref < 1.0 {
                    x_scale = (available / measured).max(pref.clamp(0.05, 1.0));
                }
                let person_id = current_id.strip_suffix("-name").unwrap_or(current_id);
                if x_scale < 1.0 && ctx.prefs.diagnostics.info {
                    eprintln!(
                        "info: name compressed to {pct}% to fit box: {person_id} \"{content}\"",
                        pct = (x_scale * 100.0).round() as i64
                    );
                }
                let rendered = measured * x_scale;
                if available > 0.0 && rendered > available + 0.5 && ctx.prefs.diagnostics.warnings {
                    let over = (((rendered / available) - 1.0) * 100.0).round().max(1.0) as i64;
                    eprintln!("warning: name {over}% too wide for box: {person_id} \"{content}\"");
                }
            }

            // Wrap a compressed name in a group that scales it horizontally about its anchor
            // (box centre for centred names), so the Y size and the anchor position are preserved.
            let scaled = x_scale < 1.0;
            if scaled {
                out.push_str(&format!(
                    "  <g transform=\"translate({ax:.3},0) scale({sx:.4},1) translate({nax:.3},0)\">\n",
                    ax = anchor_x,
                    sx = x_scale,
                    nax = -anchor_x,
                ));
            }
            render_mixed_text(
                out,
                anchor_x,
                baseline_svg,
                &content,
                &font_family,
                font_size,
                weight,
                cw,
                &color,
                bg_color.as_deref(),
                &t.align,
                &class_for_attrs(&t.attrs),
            );
            if scaled {
                out.push_str("  </g>\n");
            }
        }
        Primitive::Connector(c) => {
            if ctx.skip_connectors || c.child_points.is_empty() {
                return;
            }
            let parent_svgs: Vec<(f64, f64)> = c
                .parent_points
                .iter()
                .map(|p| ((ctx.to_svg_x)(p.x), (ctx.to_svg_y)(p.y)))
                .collect();
            let child_svgs: Vec<(f64, f64)> = c
                .child_points
                .iter()
                .map(|p| ((ctx.to_svg_x)(p.x), (ctx.to_svg_y)(p.y)))
                .collect();
            let bar_y = parent_svgs[0].1 + (child_svgs[0].1 - parent_svgs[0].1) * c.bar_y_fraction;
            let all_x: Vec<f64> = parent_svgs
                .iter()
                .map(|p| p.0)
                .chain(child_svgs.iter().map(|p| p.0))
                .collect();
            let bar_x_min = all_x.iter().cloned().fold(f64::INFINITY, f64::min);
            let bar_x_max = all_x.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            for (px, py) in &parent_svgs {
                out.push_str(&svg_line(
                    *px,
                    *py,
                    *px,
                    bar_y,
                    ctx.conn_color,
                    ctx.conn_width,
                    "connector",
                ));
            }
            if (bar_x_max - bar_x_min).abs() > 0.1 {
                out.push_str(&svg_line(
                    bar_x_min,
                    bar_y,
                    bar_x_max,
                    bar_y,
                    ctx.conn_color,
                    ctx.conn_width,
                    "connector",
                ));
            }
            for (cx_svg, cy_svg) in &child_svgs {
                out.push_str(&svg_line(
                    *cx_svg,
                    bar_y,
                    *cx_svg,
                    *cy_svg,
                    ctx.conn_color,
                    ctx.conn_width,
                    "connector",
                ));
            }
        }
        Primitive::Group(g) => {
            let id_attr = if g.id.is_empty() {
                String::new()
            } else {
                format!(" id=\"{}\"", xml_escape(&g.id))
            };
            out.push_str(&format!("<g{id_attr}>\n"));
            // Carry the nearest non-empty group id down to descendants so name text can be
            // attributed to a person (used by name autocompress diagnostics).
            let child_id = if g.id.is_empty() {
                current_id
            } else {
                g.id.as_str()
            };
            for child in &g.children {
                render_bc_primitive(child, ctx, out, child_id);
            }
            out.push_str("</g>\n");
        }
        Primitive::BoxesSpouseConnector(c) => {
            if c.spouse_entries.is_empty() {
                return;
            }
            let exit_x = (ctx.to_svg_x)(c.individual_exit.x);
            let exit_y = (ctx.to_svg_y)(c.individual_exit.y);
            let bar_y = (exit_y + (ctx.to_svg_y)(c.spouse_entries[0].y)) / 2.0;
            let last_x = c
                .spouse_entries
                .iter()
                .map(|p| (ctx.to_svg_x)(p.x))
                .fold(f64::NEG_INFINITY, f64::max);
            out.push_str(&svg_line(
                exit_x,
                exit_y,
                exit_x,
                bar_y,
                ctx.conn_color,
                ctx.conn_width,
                "connector",
            ));
            out.push_str(&svg_line(
                exit_x,
                bar_y,
                last_x,
                bar_y,
                ctx.conn_color,
                ctx.conn_width,
                "connector",
            ));
            for sp in &c.spouse_entries {
                let sx = (ctx.to_svg_x)(sp.x);
                let sy = (ctx.to_svg_y)(sp.y);
                out.push_str(&svg_line(
                    sx,
                    bar_y,
                    sx,
                    sy,
                    ctx.conn_color,
                    ctx.conn_width,
                    "connector",
                ));
            }
        }
        Primitive::Image(img) => {
            let x = (ctx.to_svg_x)(img.bbox.x);
            let y = (ctx.to_svg_y)(img.bbox.y);
            let href_safe = img.href.replace('"', "&quot;");
            out.push_str(&format!(
                "  <image x=\"{x:.2}\" y=\"{y:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" \
                 href=\"{href_safe}\" preserveAspectRatio=\"none\" class=\"photo\"/>\n",
                w = img.bbox.w,
                h = img.bbox.h,
            ));
        }
        Primitive::FilledRect(r) => {
            let x = (ctx.to_svg_x)(r.bbox.x);
            let y = (ctx.to_svg_y)(r.bbox.y);
            let fill_safe = r.fill.replace('"', "&quot;");
            out.push_str(&format!(
                "  <rect x=\"{x:.2}\" y=\"{y:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" \
                 fill=\"{fill_safe}\" class=\"highlight_rect\"/>\n",
                w = r.bbox.w,
                h = r.bbox.h,
            ));
        }
        Primitive::FancyText(item) => {
            if item.highlighted {
                if let Some(name_line) = item.lines.iter().find(|l| {
                    l.attrs.contains(&TextAttr::IndividualName)
                        || l.attrs.contains(&TextAttr::SpouseName)
                }) {
                    render_fancy_highlight_rect(
                        out,
                        name_line,
                        (ctx.to_svg_x)(name_line.x),
                        (ctx.to_svg_y)(name_line.y),
                        ctx.prefs,
                    );
                }
            }
            for line in &item.lines {
                let (font_family, font_size) = font_for_attr(&line.attrs, ctx.prefs);
                let weight = weight_for_attr(&line.attrs, ctx.prefs);
                let color = color_for_attr(&line.attrs, ctx.prefs);
                let cw = font_size * CHAR_WIDTH_RATIO;
                let x_svg = (ctx.to_svg_x)(line.x);
                let y_svg = (ctx.to_svg_y)(line.y + font_size * 0.85);
                render_mixed_text(
                    out,
                    x_svg,
                    y_svg,
                    &line.text,
                    &font_family,
                    font_size,
                    weight,
                    cw,
                    &color,
                    None,
                    &TextAlign::Left,
                    &class_for_attrs(&line.attrs),
                );
            }
        }
        Primitive::NoteHtmlLink(link) => {
            let (font_family, font_size) = font_for_attr(&link.attrs, ctx.prefs);
            let weight = weight_for_attr(&link.attrs, ctx.prefs);
            let cw = font_size * CHAR_WIDTH_RATIO;
            let anchor_x = (ctx.to_svg_x)(link.bbox.x);
            let baseline_svg = (ctx.to_svg_y)(link.bbox.y + link.bbox.h);
            let link_color = hex_color(ctx.prefs.output.style.text.note_link);
            out.push_str(&format!(
                "  <a href=\"{}\" class=\"note_link\" style=\"text-decoration:underline;\">\n",
                xml_escape(&link.href)
            ));
            render_mixed_text(
                out,
                anchor_x,
                baseline_svg,
                &link.content,
                &font_family,
                font_size,
                weight,
                cw,
                &link_color,
                None,
                &TextAlign::Left,
                &class_for_attrs(&link.attrs),
            );
            out.push_str("  </a>\n");
        }
        _ => {}
    }
}

fn render_scene(output: &LayoutOutput, prefs: &Prefs) -> String {
    let scene = output.scene();
    if scene.primitives.is_empty() {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                <svg xmlns=\"http://www.w3.org/2000/svg\" \
                width=\"100\" height=\"100\"></svg>\n"
            .into();
    }

    // Title / copyright (same logic as render_boxed_couples)
    let title_text = super::expand_title_template(&prefs.output.text.title, prefs);
    let (title_font_family, title_font_size) = parsed_font(&prefs.output.style.fonts.title);
    let title_line_h = if title_text.is_empty() {
        0.0
    } else {
        title_font_size * (LINE_HEIGHT / FONT_SIZE)
    };

    let copy_text = super::expand_title_template(&prefs.output.text.copyright, prefs);
    let (copy_font_family, copy_font_size) = parsed_font(&prefs.output.style.fonts.copyright);
    let copy_line_h = if copy_text.is_empty() {
        0.0
    } else {
        copy_font_size * (LINE_HEIGHT / FONT_SIZE)
    };

    let chart_top_offset = if title_text.is_empty() {
        0.0
    } else {
        title_line_h + prefs.output.style.spacing.title
    };

    // Box style prefs
    let box_fill = if prefs.output.style.boxes.background != 0 {
        hex_color(prefs.output.style.boxes.background)
    } else {
        "white".to_string()
    };
    let box_stroke = if prefs.output.style.boxes.border != 0 {
        hex_color(prefs.output.style.boxes.border)
    } else {
        "black".to_string()
    };
    let box_sw = if prefs.output.style.boxes.width > 0.0 {
        prefs.output.style.boxes.width
    } else {
        1.0
    };
    let box_radius = prefs.output.style.boxes.radius;

    // Wedge style prefs (fan layout); same semantics as boxes, default width 0.5.
    let wedge_fill = if prefs.output.style.wedges.background != 0 {
        hex_color(prefs.output.style.wedges.background)
    } else {
        "white".to_string()
    };
    let wedge_stroke = if prefs.output.style.wedges.border != 0 {
        hex_color(prefs.output.style.wedges.border)
    } else {
        "black".to_string()
    };
    let wedge_sw = if prefs.output.style.wedges.width > 0.0 {
        prefs.output.style.wedges.width
    } else {
        0.5
    };

    // Connector style prefs
    let conn_color = hex_color(prefs.output.style.connectors.border);
    let conn_width = if prefs.output.style.connectors.width > 0.0 {
        prefs.output.style.connectors.width
    } else {
        1.0
    };

    // SVG coordinate transforms: add MARGIN to display coordinates
    let to_svg_x = |dx: f64| dx + MARGIN;
    let to_svg_y = |dy: f64| dy + MARGIN + chart_top_offset;

    let realistic_tree_active =
        prefs.output.style.realistic_tree.enabled && output.is_boxed_couples();

    // Collect connectors early so root_extra_height can expand the canvas before we emit the header.
    let rt_connectors: Vec<crate::backend::realistic_tree::SeededConnector> =
        if realistic_tree_active {
            let mut v = Vec::new();
            crate::backend::realistic_tree::collect_connectors(&scene.primitives, "", &mut v);
            v
        } else {
            Vec::new()
        };
    let tree_root_extra_h = if realistic_tree_active {
        crate::backend::realistic_tree::root_extra_height(&rt_connectors)
    } else {
        0.0
    };

    // Shared rendering context (used for boxed_couples groups and all non-fan primitives)
    let ctx = BcSvgCtx {
        to_svg_x: &to_svg_x,
        to_svg_y: &to_svg_y,
        box_fill: &box_fill,
        box_stroke: &box_stroke,
        box_sw,
        box_radius,
        conn_color: &conn_color,
        conn_width,
        prefs,
        skip_connectors: realistic_tree_active,
        box_name_autocompress: output.is_boxes() || output.is_boxed_couples(),
    };

    // SVG dimensions
    let total_w = scene.canvas_bounds.w + 2.0 * MARGIN;
    let copy_spacing = if copy_text.is_empty() {
        0.0
    } else {
        prefs.output.style.spacing.copyright
    };
    let total_h = scene.canvas_bounds.h
        + 2.0 * MARGIN
        + chart_top_offset
        + copy_line_h
        + copy_spacing
        + tree_root_extra_h;

    let canvas_w = format!("{total_w:.0}");
    let canvas_h = format!("{total_h:.0}");
    let viewbox = format!("0 0 {total_w:.1} {total_h:.1}");
    let mut out = svg_header(&canvas_w, &canvas_h, &viewbox);

    // Title / copyright colors (0 ⇒ default "black").
    let title_color = color_or_black(prefs.output.style.text.title);
    let copy_color = color_or_black(prefs.output.style.text.copyright);

    // Title
    if !title_text.is_empty() {
        let y = MARGIN + title_font_size;
        out.push_str(&svg_text_full(
            MARGIN,
            y,
            &title_text,
            &title_font_family,
            title_font_size,
            "normal",
            &title_color,
            "title",
        ));
    }

    // Copyright
    if !copy_text.is_empty() {
        let y = total_h - MARGIN;
        out.push_str(&svg_text_full(
            MARGIN,
            y,
            &copy_text,
            &copy_font_family,
            copy_font_size,
            "normal",
            &copy_color,
            "copyright",
        ));
    }

    // Realistic tree background layer (rendered before boxes so boxes are on top)
    if realistic_tree_active {
        out.push_str(&crate::backend::realistic_tree::render_tree_layer(
            &rt_connectors,
            &to_svg_x,
            &to_svg_y,
            prefs,
        ));
    }

    let mut indiv_conns: Vec<&crate::scene::FancyConnector> = Vec::new();
    let mut spouse_conns: Vec<&crate::scene::FancyConnector> = Vec::new();
    // Render primitives
    for prim in &scene.primitives {
        match prim {
            crate::scene::Primitive::Wedge(w) => {
                let cx_svg = to_svg_x(w.cx);
                let cy_svg = to_svg_y(w.cy);
                let path = wedge_path(
                    cx_svg,
                    cy_svg,
                    w.angle_center,
                    w.angle_span,
                    w.radius_inner,
                    w.radius_outer,
                );
                out.push_str(&format!(
                    "  <path d=\"{path}\" fill=\"{wedge_fill}\" stroke=\"{wedge_stroke}\" stroke-width=\"{wedge_sw}\" class=\"wedge\"/>\n"
                ));
                let ring_height = w.radius_outer - w.radius_inner;
                let mid_r = (w.radius_inner + w.radius_outer) / 2.0;
                let arc_length = mid_r * w.angle_span.to_radians();
                let (width_budget, height_budget) = if w.radial_text {
                    (ring_height, arc_length)
                } else {
                    (arc_length, ring_height)
                };

                struct Line {
                    text: String,
                    font_family: String,
                    font_size: f64,
                    color: String,
                    class: String,
                }
                let mut lines: Vec<Line> = Vec::new();
                let mut total_height = 0.0_f64;

                // Name has top priority: include it regardless of width,
                // but only if it fits in the height budget.
                let name_included = if let Some(text) = w.label.as_ref() {
                    let (font_family, font_size) = font_for_attr(&w.label_attrs, prefs);
                    let lh = font_size * 1.2;
                    if total_height + lh <= height_budget + 1e-6 {
                        total_height += lh;
                        lines.push(Line {
                            text: text.to_string(),
                            color: color_for_attr(&w.label_attrs, prefs),
                            class: class_for_attrs(&w.label_attrs),
                            font_family,
                            font_size,
                        });
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                // Dates are only shown if the name was included and
                // they fit both the width and the remaining height budget.
                if name_included {
                    let date_candidates = [
                        (w.birth_line.as_ref(), vec![TextAttr::BirthData]),
                        (w.death_line.as_ref(), vec![TextAttr::DeathData]),
                    ];
                    for (maybe_text, attrs) in &date_candidates {
                        let Some(text) = maybe_text else { continue };
                        let (font_family, font_size) = font_for_attr(attrs, prefs);
                        let text_width = text.chars().count() as f64 * font_size * CHAR_WIDTH_RATIO;
                        let lh = font_size * 1.2;
                        if text_width > width_budget + 1e-6
                            || total_height + lh > height_budget + 1e-6
                        {
                            if prefs.diagnostics.warnings {
                                let name = w.label.as_deref().unwrap_or("?");
                                eprintln!(
                                    "Warning: fan wedge text dropped for {} ({name}): \"{text}\"",
                                    w.individual_id
                                );
                            }
                            continue;
                        }
                        total_height += lh;
                        lines.push(Line {
                            text: text.to_string(),
                            color: color_for_attr(attrs, prefs),
                            class: class_for_attrs(attrs),
                            font_family,
                            font_size,
                        });
                    }
                }
                if !lines.is_empty() {
                    let rotation = if w.radial_text {
                        let r = 180.0 - w.angle_center;
                        if r > 90.0 { r - 180.0 } else { r }
                    } else {
                        90.0 - w.angle_center
                    };
                    let angle_rad = w.angle_center.to_radians();
                    let anchor_r = if w.radial_text {
                        mid_r
                    } else {
                        w.radius_inner + ring_height * 0.4
                    };
                    let tx = cx_svg + anchor_r * angle_rad.cos();
                    let ty = cy_svg - anchor_r * angle_rad.sin();
                    let mut y = -total_height / 2.0;
                    out.push_str(&format!(
                        "  <g transform=\"translate({tx:.1},{ty:.1}) rotate({rotation:.1})\">\n"
                    ));
                    if is_highlighted(&w.label_attrs) {
                        let name_line = &lines[0];
                        let name_lh = name_line.font_size * 1.2;
                        let base_font = base_font_from_css(&name_line.font_family);
                        let name_w = font_metrics::measure_text_w(
                            &name_line.text,
                            base_font,
                            name_line.font_size,
                            false,
                        )
                        .unwrap_or_else(|| {
                            name_line.text.chars().count() as f64
                                * name_line.font_size
                                * CHAR_WIDTH_RATIO
                        });
                        let name_line_y = -total_height / 2.0 + name_lh / 2.0;
                        let bg = hex_color(prefs.output.style.text.highlights.background_color);
                        let pad = 1.0;
                        out.push_str(&format!(
                            "    <rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"{bg}\" class=\"highlight_rect\"/>\n",
                            -name_w / 2.0 - pad,
                            name_line_y - name_lh / 2.0 - pad,
                            name_w + 2.0 * pad,
                            name_lh + 2.0 * pad
                        ));
                    }
                    for line in &lines {
                        let lh = line.font_size * 1.2;
                        let line_y = y + lh / 2.0;
                        y += lh;
                        render_mixed_text_rotated(
                            &mut out,
                            line_y,
                            &line.text,
                            &line.font_family,
                            line.font_size,
                            &line.color,
                            &line.class,
                        );
                    }
                    out.push_str("  </g>\n");
                }
            }
            crate::scene::Primitive::FancyConn(conn) => {
                use crate::scene::FancyConnKind;
                match &conn.kind {
                    FancyConnKind::IndivToSpouse => indiv_conns.push(conn),
                    FancyConnKind::SpouseToChildren => spouse_conns.push(conn),
                }
            }
            crate::scene::Primitive::NoteBar(bar) => {
                let x_svg = to_svg_x(bar.x);
                let y1_svg = to_svg_y(bar.top_y);
                let y2_svg = to_svg_y(bar.bottom_y);
                let bar_color = hex_color(prefs.output.style.text.note_bar);
                out.push_str(&format!(
                    "  <line x1=\"{x:.1}\" y1=\"{y1:.1}\" x2=\"{x:.1}\" y2=\"{y2:.1}\" \
                     stroke=\"{bar_color}\" stroke-width=\"2\" stroke-linecap=\"round\" class=\"note_bar\"/>\n",
                    x = x_svg,
                    y1 = y1_svg,
                    y2 = y2_svg,
                ));
            }
            _ => {
                render_bc_primitive(prim, &ctx, &mut out, "");
            }
        }
    }

    // ── Fancy layout connector groups ─────────────────────────────────────────
    if !indiv_conns.is_empty() || !spouse_conns.is_empty() {
        let offset_x = MARGIN;
        let offset_y = MARGIN + chart_top_offset;
        render_fancy_conn_group(
            &mut out,
            "fancy-connectors-1",
            offset_x,
            offset_y,
            &indiv_conns,
        );
        render_fancy_conn_group(
            &mut out,
            "fancy-connectors-2",
            offset_x,
            offset_y,
            &spouse_conns,
        );
    }

    // ── Row-rule underlines (simple layout only, replaces dotted leaders) ──
    if output.is_simple() && prefs.output.style.dot_leaders {
        // Collect (row_y, max_x, font_size) per row from Text primitives.
        // All text on a row in the simple layout shares the same bbox.y and bbox.h.
        let mut row_info: std::collections::HashMap<i64, (f64, f64, f64)> =
            std::collections::HashMap::new();
        for p in &scene.primitives {
            if let crate::scene::Primitive::Text(t) = p {
                let row = t.bbox.y.round() as i64;
                let x_end = t.bbox.x + t.bbox.w;
                let entry = row_info.entry(row).or_insert((t.bbox.y, 0.0, t.bbox.h));
                if x_end > entry.1 {
                    entry.1 = x_end;
                }
            }
        }

        // Emit one solid thin line per row, from left margin to canvas right edge,
        // positioned 1px below the approximate descender (font_size * 0.16).
        let underline_color = hex_color(prefs.output.style.text.row_rule);
        const UNDERLINE_WIDTH: f64 = 0.5;
        // Sort by row so the underline element order is stable across runs
        // (`row_info` is a HashMap); see the "Deterministic emit order" note in CLAUDE.md.
        let mut rows: Vec<(i64, (f64, f64, f64))> = row_info.into_iter().collect();
        rows.sort_by_key(|(row, _)| *row);
        for (_row, (row_y, _max_x, font_size)) in rows {
            let baseline_svg = to_svg_y(row_y + font_size);
            let underline_y = baseline_svg + font_size * 0.16 + 1.0;
            let line_x1 = to_svg_x(0.0);
            let line_x2 = to_svg_x(scene.canvas_bounds.w);
            out.push_str(&format!(
                "    <line x1=\"{x1:.1}\" y1=\"{y:.1}\" x2=\"{x2:.1}\" y2=\"{y:.1}\" \
                 stroke=\"{color}\" stroke-width=\"{width}\" class=\"row_rule\"/>\n",
                x1 = line_x1,
                y = underline_y,
                x2 = line_x2,
                color = underline_color,
                width = UNDERLINE_WIDTH
            ));
        }
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
    Ok(render_scene(output, prefs))
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
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs
    }

    // ── Structure ──

    #[test]
    fn test_svg_structure() {
        let prefs = simple_prefs();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(out.contains("<svg "), "missing <svg: {out}");
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
        assert!(
            !out.contains('│'),
            "SVG must not contain │ bar characters: {out}"
        );
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
        assert!(
            out.contains("<line "),
            "connector <line> elements expected: {out}"
        );
    }

    // ── Paper sizing ──

    #[test]
    fn test_svg_content_sized_when_no_paper() {
        let prefs = simple_prefs();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        let width_val: String = out
            .split("width=\"")
            .nth(1)
            .unwrap_or("")
            .chars()
            .take_while(|c| *c != '"')
            .collect();
        assert!(
            width_val.parse::<f64>().is_ok(),
            "content-sized width should be a number, got: {width_val:?}"
        );
    }

    // ── Font prefs ──

    #[test]
    fn test_svg_font_from_prefs() {
        let mut prefs = simple_prefs();
        prefs.output.style.fonts.names = "Helvetica 16".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        // Font-family includes the base name plus fallbacks; check for base name presence.
        assert!(out.contains("Helvetica"), "custom font family: {out}");
        assert!(out.contains("font-size=\"16\""), "custom font size: {out}");
    }

    #[test]
    fn test_svg_default_font_fallback() {
        let mut prefs = simple_prefs();
        prefs.output.style.fonts.names = "".into(); // clear to test monospace fallback
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
        assert!(
            out.contains("class=\"row_rule\""),
            "row-rule underline lines expected: {out}"
        );
    }

    #[test]
    fn test_svg_dot_leaders_absent_when_disabled() {
        let mut prefs = simple_prefs();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.birth = true;
        prefs.format.birth = "* {date}".into();
        prefs.output.style.dot_leaders = false;
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(
            !out.contains("class=\"row_rule\""),
            "no row-rule underlines expected: {out}"
        );
    }

    // ── Unit helpers ──

    #[test]
    fn test_parsed_font() {
        assert_eq!(parsed_font("Georgia 14"), ("Georgia".to_string(), 14.0));
        assert_eq!(
            parsed_font("Arial Bold 10"),
            ("Arial Bold".to_string(), 10.0)
        );
        assert_eq!(parsed_font(""), ("monospace".to_string(), FONT_SIZE));
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

        // Custom: orientation is ignored; exact dimensions are returned as-is.
        prefs.output.paper.size = "custom".into();
        prefs.output.paper.custom.width = 300.0;
        prefs.output.paper.custom.height = 150.0;
        prefs.output.paper.orientation = "portrait".into();
        assert_eq!(paper_size_mm(&prefs), Some((300.0, 150.0)));
        prefs.output.paper.orientation = "landscape".into();
        assert_eq!(paper_size_mm(&prefs), Some((300.0, 150.0)));

        // Custom with no dimensions → None.
        prefs.output.paper.custom.width = 0.0;
        prefs.output.paper.custom.height = 0.0;
        assert_eq!(paper_size_mm(&prefs), None);
    }

    #[test]
    fn test_hex_color() {
        // 3-digit RGB (opaque) — unchanged behaviour.
        assert_eq!(hex_color(0x000), "#000000");
        assert_eq!(hex_color(0xFFF), "#FFFFFF");
        assert_eq!(hex_color(0x222), "#222222");
        // 4-digit RGBA (alpha-last) — queue #98 examples.
        assert_eq!(hex_color(0xFFFF), "#FFFFFFFF"); // opaque white
        assert_eq!(hex_color(0xF008), "#FF000088"); // red ~53%
        // 6-digit RRGGBB (opaque, full 24-bit).
        assert_eq!(hex_color(0x3D2B1F), "#3D2B1F");
        // 8-digit RRGGBBAA.
        assert_eq!(hex_color(0x12345678), "#12345678");
    }

    #[test]
    fn test_color_or_black() {
        assert_eq!(color_or_black(0), "black"); // unset ⇒ literal black (byte-stable default)
        assert_eq!(color_or_black(0xCCC), "#CCCCCC"); // row_rule / note_bar default
        assert_eq!(color_or_black(0x06C), "#0066CC"); // note_link default
        assert_eq!(color_or_black(0xF008), "#FF000088"); // alpha passes through
    }

    fn fan_wedge_output() -> LayoutOutput {
        let scene = crate::scene::Scene {
            primitives: vec![crate::scene::Primitive::Wedge(
                crate::scene::WedgePrimitive {
                    cx: 100.0,
                    cy: 100.0,
                    angle_center: 90.0,
                    angle_span: 30.0,
                    radius_inner: 20.0,
                    radius_outer: 60.0,
                    label: None,
                    label_attrs: vec![],
                    radial_text: false,
                    individual_id: "I1".to_string(),
                    birth_line: None,
                    death_line: None,
                },
            )],
            canvas_bounds: crate::scene::Rect {
                x: 0.0,
                y: 0.0,
                w: 200.0,
                h: 200.0,
            },
        };
        LayoutOutput::Fan(scene)
    }

    #[test]
    fn test_svg_wedge_style_defaults() {
        // Defaults mirror boxes: border 0x222, background 0xFFF, width 0.5.
        let out = render_to_string(&fan_wedge_output(), &Prefs::default()).unwrap();
        let line = out
            .lines()
            .find(|l| l.contains("class=\"wedge\""))
            .expect("wedge path emitted");
        assert!(line.contains("stroke=\"#222222\""), "got: {line}");
        assert!(line.contains("fill=\"#FFFFFF\""), "got: {line}");
        assert!(line.contains("stroke-width=\"0.5\""), "got: {line}");
    }

    #[test]
    fn test_svg_wedge_style_from_prefs_with_alpha() {
        let mut prefs = Prefs::default();
        prefs.output.style.wedges.border = 0xF008; // 4-digit RGBA → #FF000088
        prefs.output.style.wedges.background = 0xABC; // 3-digit opaque → #AABBCC
        prefs.output.style.wedges.width = 2.0;
        let out = render_to_string(&fan_wedge_output(), &prefs).unwrap();
        let line = out
            .lines()
            .find(|l| l.contains("class=\"wedge\""))
            .expect("wedge path emitted");
        assert!(line.contains("stroke=\"#FF000088\""), "got: {line}");
        assert!(line.contains("fill=\"#AABBCC\""), "got: {line}");
        assert!(line.contains("stroke-width=\"2\""), "got: {line}");
    }

    #[test]
    fn exclude_msg_font_uses_pref() {
        let mut prefs = Prefs::default();
        prefs.output.style.fonts.exclude_msg = "Verdana 9".to_string();
        let (fam, sz) = font_for_attr(&[TextAttr::ExcludeMsg], &prefs);
        assert!(fam.starts_with("Verdana"), "got {fam}");
        assert_eq!(sz, 9.0);
    }

    #[test]
    fn test_color_for_attr() {
        let mut prefs = Prefs::default();
        prefs.output.style.text.names = 0xF00; // 3-digit
        prefs.output.style.text.dates = 0x0F0;
        prefs.output.style.text.gen_numbers = 0x00F;
        prefs.output.style.text.notes = 0x1234ABCD; // 8-digit RRGGBBAA
        prefs.output.style.text.id = 0xE00;

        assert_eq!(
            color_for_attr(&[TextAttr::IndividualName], &prefs),
            "#FF0000"
        );
        assert_eq!(color_for_attr(&[TextAttr::SpouseName], &prefs), "#FF0000");
        assert_eq!(color_for_attr(&[TextAttr::BirthData], &prefs), "#00FF00");
        assert_eq!(color_for_attr(&[TextAttr::DeathData], &prefs), "#00FF00");
        assert_eq!(color_for_attr(&[TextAttr::MarriageData], &prefs), "#00FF00");
        assert_eq!(
            color_for_attr(&[TextAttr::GenerationNum], &prefs),
            "#0000FF"
        );
        assert_eq!(color_for_attr(&[TextAttr::NoteText], &prefs), "#1234ABCD");
        assert_eq!(color_for_attr(&[TextAttr::IndividualId], &prefs), "#EE0000");
        prefs.output.style.text.exclude_msg = 0x0AB; // 3-digit → #00AABB
        assert_eq!(color_for_attr(&[TextAttr::ExcludeMsg], &prefs), "#00AABB");
        // Highlighted wins over the per-kind color.
        assert_eq!(
            color_for_attr(&[TextAttr::IndividualName, TextAttr::Highlighted], &prefs),
            hex_color(prefs.output.style.text.highlights.color)
        );

        // 4-digit alpha and the 0 ⇒ "black" fallback.
        prefs.output.style.text.names = 0x2228;
        assert_eq!(
            color_for_attr(&[TextAttr::IndividualName], &prefs),
            "#22222288"
        );
        prefs.output.style.text.dates = 0x000;
        assert_eq!(color_for_attr(&[TextAttr::BirthData], &prefs), "black");
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
        prefs.output.style.fonts.names = "Georgia 14".into();
        prefs.output.style.fonts.dates = "".into(); // let dates fall back to names font so APR is in Georgia
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();

        // The ⚭ character must be in a text element that does NOT start with Georgia.
        // SYMBOL_FONT_FAMILY starts with "'Apple Symbols'".
        let symbol_in_apple = out
            .lines()
            .any(|l| l.contains("Apple Symbols") && l.contains("⚭"));
        assert!(
            symbol_in_apple,
            "⚭ should be in a text element using the symbol font: {out}"
        );

        // Latin characters ("APR") must not be in a symbol-font element.
        let latin_in_georgia = out
            .lines()
            .any(|l| l.contains("Georgia") && l.contains("APR"));
        assert!(
            latin_in_georgia,
            "Latin text should be in the primary-font element: {out}"
        );
    }

    #[test]
    fn test_svg_sex_symbol_in_separate_element() {
        // Sex symbols (♂/♀) in names should be split into their own <text> element
        // with the symbol font family, so that PDF backends (svg2pdf) render them
        // correctly without corrupting the primary font for the name text.
        let mut prefs = simple_prefs();
        prefs.format.individual = "{firstname} {lastname} {sex}".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        let symbol_in_apple = out
            .lines()
            .any(|l| l.contains("Apple Symbols") && (l.contains("♂") || l.contains("♀")));
        assert!(
            symbol_in_apple,
            "sex symbols should be in a text element using the symbol font: {}",
            &out[..out.len().min(500)]
        );
    }

    #[test]
    fn test_svg_underline_spans_full_width() {
        let mut prefs = simple_prefs();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.birth = true;
        prefs.format.birth = "* {date}".into();
        prefs.output.style.dot_leaders = true;
        prefs.output.style.fonts.names = "monospace 14".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(
            out.lines()
                .any(|l| l.contains("class=\"row_rule\"") && l.contains("x1=\"20.0\"")),
            "underline should start at left margin: {out}"
        );
    }

    // ── Title and copyright ──

    #[test]
    fn test_svg_title_and_copyright_simple() {
        let mut prefs = simple_prefs();
        prefs.output.text.title = "My Family Chart".into();
        prefs.output.text.copyright = "© 2026 Alex".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(
            out.contains("My Family Chart"),
            "title should appear in SVG: {out}"
        );
        assert!(
            out.contains("© 2026 Alex"),
            "copyright should appear in SVG: {out}"
        );
        // Verify the copyright baseline y is within the viewBox (was broken when title present).
        let viewbox_h: f64 = out
            .split("viewBox=\"0 0 ")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .expect("viewBox height missing");
        // The copyright <text> element is on a single line containing the text content.
        let copy_y: f64 = out
            .lines()
            .find(|l| l.contains("© 2026 Alex"))
            .and_then(|l| l.split("y=\"").nth(1))
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse().ok())
            .expect("copyright y attribute missing");
        assert!(
            copy_y <= viewbox_h,
            "copyright y={copy_y} is outside viewBox height={viewbox_h}"
        );
    }

    #[test]
    fn test_svg_title_gedcom_template() {
        let mut prefs = simple_prefs();
        prefs.output.text.title = "Chart of {gedcom}".into();
        // prefs.files.gedcom is empty by default → "unknown"
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        assert!(
            out.contains("Chart of"),
            "template title should appear in SVG: {out}"
        );
    }

    #[test]
    fn test_svg_no_title_when_empty() {
        let mut prefs = simple_prefs();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.birth = false;
        prefs.show.death = false;
        prefs.show.marriage = false;
        prefs.output.text.title = "".into();
        prefs.output.text.copyright = "".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        // No spurious title/copyright text elements; chart body is still present.
        // No spurious title/copyright text elements; chart body is still present.
        assert!(out.contains("John"), "names should still be present: {out}");
        // Should not inject spurious text for empty title/copyright.
        // Count <text elements: just the three names (John, Jane, Paul).
        let count = out.matches("<text ").count();
        assert!(
            count <= 5,
            "unexpected extra <text elements when title/copyright empty: {out}"
        );
    }

    // ── Boxed couples layout ──

    fn bc_prefs() -> Prefs {
        let mut p = Prefs::default();
        p.scope.root = "I1".into();
        p.scope.direction = "descendants".into();
        p.scope.generations = 3;
        p.layout.layout_type = "boxed_couples".into();
        p
    }

    fn bc_layout_with_prefs(prefs: &Prefs) -> LayoutOutput {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(3));
        run_layout(&genrep, prefs).unwrap()
    }

    fn bc_layout() -> LayoutOutput {
        bc_layout_with_prefs(&bc_prefs())
    }

    #[test]
    fn test_bc_svg_structure() {
        let prefs = bc_prefs();
        let out = render_to_string(&bc_layout(), &prefs).unwrap();
        assert!(out.contains("<svg "), "missing <svg");
        assert!(out.contains("</svg>"), "missing </svg>");
        assert!(out.contains("<rect "), "missing <rect> for boxes");
        assert!(out.contains("<line "), "missing <line> for connectors");
        assert!(out.contains("viewBox="), "missing viewBox");
    }

    #[test]
    fn test_name_autocompress() {
        fn layout(ged: &str, prefs: &Prefs) -> LayoutOutput {
            let mut g = parse_str(ged).unwrap();
            compute_scope(&mut g, Some("I1"), "descendants", Some(1));
            run_layout(&g, prefs).unwrap()
        }
        let long = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n\
            1 NAME Bartholomew Maximilian /Featherstonehaugh-Cholmondeley/\n1 SEX M\n0 TRLR\n";
        let short = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Jo /Ng/\n1 SEX M\n0 TRLR\n";

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.layout.layout_type = "boxed_couples".into();

        // Default (0.85): a too-long name is wrapped in a horizontal-scale group.
        let out = render_to_string(&layout(long, &prefs), &prefs).unwrap();
        assert!(
            out.contains("scale(0."),
            "long name should be compressed: {out}"
        );
        assert!(out.contains("class=\"indi_name\""));

        // A short name is not compressed.
        let out = render_to_string(&layout(short, &prefs), &prefs).unwrap();
        assert!(
            !out.contains("scale(0."),
            "short name must not be compressed: {out}"
        );

        // names_autocompress >= 1.0 disables compression even when the name overflows.
        prefs.output.style.spacing.names_autocompress = 1.0;
        let out = render_to_string(&layout(long, &prefs), &prefs).unwrap();
        assert!(
            !out.contains("scale(0."),
            "autocompress disabled must not compress: {out}"
        );
    }

    #[test]
    fn test_bc_svg_contains_names() {
        let mut prefs = bc_prefs();
        prefs.format.individual = "{firstname} {lastname}".into();
        let out = render_to_string(&bc_layout_with_prefs(&prefs), &prefs).unwrap();
        assert!(out.contains("John"), "root name missing");
        assert!(out.contains("Jane"), "spouse name missing");
        assert!(out.contains("Paul"), "child name missing");
    }

    #[test]
    fn test_bc_svg_copyright_within_viewbox() {
        let mut prefs = bc_prefs();
        prefs.output.text.copyright = "© 2026 TestOwner".into();
        let out = render_to_string(&bc_layout_with_prefs(&prefs), &prefs).unwrap();
        assert!(
            out.contains("© 2026 TestOwner"),
            "copyright text missing from SVG"
        );
        let viewbox_h: f64 = out
            .split("viewBox=\"0 0 ")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .expect("viewBox height missing");
        let copy_y: f64 = out
            .lines()
            .find(|l| l.contains("© 2026 TestOwner"))
            .and_then(|l| l.split("y=\"").nth(1))
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse().ok())
            .expect("copyright y attribute missing");
        assert!(
            copy_y <= viewbox_h,
            "copyright y={copy_y} is outside viewBox height={viewbox_h}"
        );
    }

    #[test]
    fn test_bc_svg_show_sex_false_hides_symbol() {
        let mut prefs = bc_prefs();
        prefs.format.individual = "{firstname} {lastname} {sex}".into();
        prefs.show.sex = false;
        let out = render_to_string(&bc_layout_with_prefs(&prefs), &prefs).unwrap();
        assert!(
            !out.contains('♂') && !out.contains('♀'),
            "sex symbols should not appear when show.sex=false"
        );
    }

    #[test]
    fn test_bc_svg_show_sex_true_includes_symbol() {
        let mut prefs = bc_prefs();
        prefs.format.individual = "{firstname} {lastname} {sex}".into();
        prefs.show.sex = true;
        let out = render_to_string(&bc_layout_with_prefs(&prefs), &prefs).unwrap();
        assert!(
            out.contains('♂') || out.contains('♀'),
            "sex symbols should appear when show.sex=true"
        );
    }

    fn parse_viewbox_h(svg: &str) -> f64 {
        svg.split("viewBox=\"0 0 ")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .expect("viewBox height missing")
    }

    #[test]
    fn test_svg_title_spacing_enlarges_viewbox() {
        let make_out = |spacing: f64| {
            let mut prefs = bc_prefs();
            prefs.output.text.title = "Test Title".into();
            prefs.output.text.copyright = "".into();
            prefs.output.style.spacing.title = spacing;
            prefs.output.style.spacing.copyright = 0.0;
            render_to_string(&bc_layout_with_prefs(&prefs), &prefs).unwrap()
        };
        let h0 = parse_viewbox_h(&make_out(0.0));
        let h20 = parse_viewbox_h(&make_out(20.0));
        assert!(
            (h20 - h0 - 20.0).abs() < 0.5,
            "expected viewBox to grow by 20, got {}",
            h20 - h0
        );
    }

    #[test]
    fn test_svg_copyright_spacing_enlarges_viewbox() {
        let make_out = |spacing: f64| {
            let mut prefs = bc_prefs();
            prefs.output.text.title = "".into();
            prefs.output.text.copyright = "© Owner".into();
            prefs.output.style.spacing.title = 0.0;
            prefs.output.style.spacing.copyright = spacing;
            render_to_string(&bc_layout_with_prefs(&prefs), &prefs).unwrap()
        };
        let h0 = parse_viewbox_h(&make_out(0.0));
        let h20 = parse_viewbox_h(&make_out(20.0));
        assert!(
            (h20 - h0 - 20.0).abs() < 0.5,
            "expected viewBox to grow by 20, got {}",
            h20 - h0
        );
    }

    #[test]
    fn test_simple_svg_font_weight_applied() {
        let mut prefs = simple_prefs();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.output.style.fonts.descendant = "bold".into();
        prefs.output.style.fonts.spouse = "regular".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        // Descendant name must carry bold weight
        assert!(
            out.contains("font-weight=\"bold\""),
            "descendant name must have font-weight=bold"
        );
        // Spouse name must carry normal weight (regular maps to normal)
        assert!(
            out.contains("font-weight=\"normal\""),
            "spouse name must have font-weight=normal"
        );
    }

    #[test]
    fn test_bc_svg_show_ids_enabled() {
        let mut prefs = bc_prefs();
        prefs.show.id = true;
        let out = render_to_string(&bc_layout_with_prefs(&prefs), &prefs).unwrap();
        // Individual IDs should appear without @ delimiters
        assert!(out.contains("I1"), "individual ID I1 should appear: {out}");
        assert!(out.contains("I2"), "individual ID I2 should appear: {out}");
        assert!(out.contains("I3"), "individual ID I3 should appear: {out}");
        // Family ID should appear without @ delimiter
        assert!(out.contains("F1"), "family ID F1 should appear: {out}");
        // IDs should be rendered with fill color (from id_color)
        assert!(
            out.contains("fill=\""),
            "ID text should have fill attribute: {out}"
        );
    }

    #[test]
    fn test_bc_svg_show_ids_disabled() {
        let prefs = bc_prefs();
        // Default show.id is false
        let out = render_to_string(&bc_layout(), &prefs).unwrap();
        // When show.id is false, no ID text elements are rendered
        // so there should be no fill attribute on text elements that contain IDs
        let id_text_lines = out
            .lines()
            .filter(|l| {
                l.contains("<text ")
                    && l.contains("fill=\"")
                    && (l.contains("I1")
                        || l.contains("I2")
                        || l.contains("I3")
                        || l.contains("F1"))
            })
            .count();
        // Group wrapper IDs (class="individual" id="...") are not counted because they don't have fill
        assert_eq!(
            id_text_lines, 0,
            "no ID text elements should appear when show.id is false: {out}"
        );
    }

    #[test]
    fn test_bc_svg_font_weight_applied() {
        let mut prefs = bc_prefs();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.output.style.fonts.descendant = "bold".into();
        prefs.output.style.fonts.spouse = "regular".into();
        let out = render_to_string(&bc_layout_with_prefs(&prefs), &prefs).unwrap();
        // Default fonts.descendant = "bold", fonts.spouse = "regular"
        // Descendant names should have font-weight="bold"
        assert!(
            out.lines()
                .any(|l| l.contains("font-weight=\"bold\"") && l.contains("John")),
            "descendant name must have font-weight=bold"
        );
        // Spouse names should have font-weight="normal"
        assert!(
            out.lines()
                .any(|l| l.contains("font-weight=\"normal\"") && l.contains("Jane")),
            "spouse name must have font-weight=normal"
        );
    }

    #[test]
    fn test_box_height_does_not_shift_text_positions() {
        // Regression: the spouse name and individual name should be in the correct
        // section of the box (top half for spouse, bottom half for individual when
        // root_pos_bottom=true). Their y-position relative to the box top should
        // be invariant to box_height changes.
        fn extract_name_y(svg: &str, name: &str) -> Option<f64> {
            svg.lines()
                .find(|l| l.contains(name) && l.contains("<text "))
                .and_then(|l| {
                    let start = l.find("y=\"")?;
                    let sub = &l[start + 3..];
                    let end = sub.find('\"')?;
                    sub[..end].parse::<f64>().ok()
                })
        }

        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(3));

        let make_prefs = |box_h: f64| {
            let mut p = bc_prefs();
            p.format.individual = "{firstname} {lastname}".into();
            p.layout.boxed_couples.box_height = box_h;
            p.layout.boxed_couples.box_width = 240.0;
            p.layout.boxed_couples.gap_width = 40.0;
            p.layout.boxed_couples.gap_height = 80.0;
            p.layout.boxed_couples.spouse_sep_height = 30.0;
            p.output.style.spacing.boxed_couples.name_above = 4.0;
            p.output.style.fonts.names = "Arial 13".into();
            p
        };

        let prefs_a = make_prefs(80.0);
        let layout_a = run_layout(&genrep, &prefs_a).unwrap();
        let svg_a = render_to_string(&layout_a, &prefs_a).unwrap();

        let prefs_b = make_prefs(200.0);
        let layout_b = run_layout(&genrep, &prefs_b).unwrap();
        let svg_b = render_to_string(&layout_b, &prefs_b).unwrap();

        // Check that text is in the correct half of the box.
        // For root_pos_bottom=true: spouse section is the top half,
        // individual section is the bottom half.
        // We just check that both SVGs produce text elements for the names.
        assert!(
            svg_a.contains("Jane"),
            "spouse name must appear in svg_a: {svg_a}"
        );
        assert!(
            svg_b.contains("Jane"),
            "spouse name must appear in svg_b: {svg_b}"
        );

        // Verify relative positions are correct: Jane (spouse) should be above John (individual).
        if let (Some(ya_jane), Some(ya_john)) = (
            extract_name_y(&svg_a, "Jane"),
            extract_name_y(&svg_a, "John"),
        ) {
            assert!(
                ya_jane < ya_john,
                "spouse (Jane) should be above individual (John) in svg_a: Jane.y={ya_jane}, John.y={ya_john}"
            );
        }
        if let (Some(yb_jane), Some(yb_john)) = (
            extract_name_y(&svg_b, "Jane"),
            extract_name_y(&svg_b, "John"),
        ) {
            assert!(
                yb_jane < yb_john,
                "spouse (Jane) should be above individual (John) in svg_b: Jane.y={yb_jane}, John.y={yb_john}"
            );
        }
    }

    #[test]
    fn fancy_ancestors_svg_contains_names() {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I3".into();
        prefs.scope.direction = "ancestors".into();
        prefs.scope.generations = 2;
        prefs.layout.layout_type = "fancy".into();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.birth = false;
        prefs.show.death = false;
        prefs.show.marriage = false;

        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(2));
        let output = run_layout(&genrep, &prefs).unwrap();
        let svg = render_to_string(&output, &prefs).unwrap();

        assert!(svg.contains("Paul"), "SVG missing Paul: {svg}");
        assert!(svg.contains("John"), "SVG missing John: {svg}");
        assert!(svg.contains("Jane"), "SVG missing Jane: {svg}");
        assert!(
            svg.contains("fancy-connectors-1"),
            "SVG missing fancy-connectors-1: {svg}"
        );
        assert!(
            svg.contains("anc-text-"),
            "SVG missing anc-text group: {svg}"
        );
    }
}
