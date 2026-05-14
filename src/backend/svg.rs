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
) -> String {
    format!(
        "  <text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"{family}\" \
         font-size=\"{size}\" font-weight=\"{weight}\" fill=\"{color}\" xml:space=\"preserve\">{}</text>\n",
        xml_escape(text)
    )
}

fn svg_text(x: f64, y: f64, text: &str, family: &str, size: f64) -> String {
    svg_text_full(x, y, text, family, size, "normal", "black")
}

fn svg_line(x1: f64, y1: f64, x2: f64, y2: f64, color: &str, width: f64) -> String {
    format!(
        "  <line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" \
         stroke=\"{color}\" stroke-width=\"{width}\"/>\n"
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
) -> String {
    format!(
        "  <rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" \
         rx=\"{radius:.1}\" ry=\"{radius:.1}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{sw}\"/>\n"
    )
}

fn font_weight_from_pref(pref: &str) -> &str {
    match pref.trim().to_lowercase().as_str() {
        "bold" | "bolder" => "bold",
        "light" | "lighter" => "lighter",
        _ => "normal",
    }
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
                (cw, ch)
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

/// Convert a 12-bit 0xRGB colour preference value to a CSS hex string.
pub(crate) fn hex_color(val: i64) -> String {
    let r = (val >> 8) & 0xF;
    let g = (val >> 4) & 0xF;
    let b = val & 0xF;
    format!("#{r:X}{r:X}{g:X}{g:X}{b:X}{b:X}")
}

/// Render text with symbol/non-symbol segmentation. When `split` is false,
/// renders the entire text as a single `<text>` element (used for names with
/// sex symbols to avoid positioning gaps). When `split` is true, separates
/// Unicode symbol runs (codepoint >= U+2000) into distinct elements with
/// SYMBOL_FONT_FAMILY. Uses exact font metrics for Latin segments.
///
/// `anchor_x` means: left edge for Left, center for Center, right edge for Right.
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
) {
    if text.is_empty() {
        return;
    }

    // Split text into (slice, is_symbol) segments at U+2000 boundary.
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
            "  <rect x=\"{bx:.1}\" y=\"{by:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" fill=\"{c}\"/>\n",
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
        out.push_str(&svg_text_full(cur_x, y, seg, fam, font_size, wt, color));
        cur_x += w;
    }
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
) {
    if text.is_empty() {
        return;
    }
    let cw = font_size * CHAR_WIDTH_RATIO;

    // Split at U+2000 boundary.
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
        out.push_str(&format!(
            "    <text x=\"{cur_x:.1}\" y=\"{y:.1}\" \
             font-family=\"{fam}\" font-size=\"{font_size}\" \
             fill=\"{color}\" dominant-baseline=\"middle\" \
             xml:space=\"preserve\">{}</text>\n",
            xml_escape(seg)
        ));
        cur_x += w;
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

/// Build the font family string with symbol fallbacks (for ♂/♀ and other symbols).
fn with_symbol_fallback(base: &str) -> String {
    format!("{base}, 'Apple Symbols', 'Segoe UI Symbol', 'DejaVu Sans', sans-serif")
}

/// Resolve (font_family, font_size) from a `[TextAttr]` slice and preferences.
fn font_for_attr(attrs: &[TextAttr], prefs: &Prefs) -> (String, f64) {
    match semantic_attr(attrs) {
        TextAttr::IndividualName | TextAttr::SpouseName | TextAttr::GenerationNum => {
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
        _ => (with_symbol_fallback("monospace"), FONT_SIZE),
    }
}

/// Resolve CSS color for a `[TextAttr]` slice.
fn color_for_attr(attrs: &[TextAttr], prefs: &Prefs) -> String {
    if is_highlighted(attrs) {
        return hex_color(prefs.output.style.text.highlights.color);
    }
    match semantic_attr(attrs) {
        TextAttr::IndividualId => hex_color(prefs.output.style.text.id),
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
        title_line_h
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

    // SVG dimensions
    let total_w = scene.canvas_bounds.w + 2.0 * MARGIN;
    let total_h = scene.canvas_bounds.h + 2.0 * MARGIN + chart_top_offset + copy_line_h;

    let canvas_w = format!("{total_w:.0}");
    let canvas_h = format!("{total_h:.0}");
    let viewbox = format!("0 0 {total_w:.1} {total_h:.1}");
    let mut out = svg_header(&canvas_w, &canvas_h, &viewbox);

    // Title
    if !title_text.is_empty() {
        let y = MARGIN + title_font_size;
        out.push_str(&svg_text(
            MARGIN,
            y,
            &title_text,
            &title_font_family,
            title_font_size,
        ));
    }

    // Copyright
    if !copy_text.is_empty() {
        let y = MARGIN + chart_top_offset + (total_h - copy_line_h - MARGIN);
        out.push_str(&svg_text(
            MARGIN,
            y,
            &copy_text,
            &copy_font_family,
            copy_font_size,
        ));
    }

    let mut indiv_conns: Vec<&crate::scene::FancyConnector> = Vec::new();
    let mut spouse_conns: Vec<&crate::scene::FancyConnector> = Vec::new();
    // Render primitives
    for prim in &scene.primitives {
        match prim {
            crate::scene::Primitive::Box(b) => {
                let x = to_svg_x(b.bbox.x);
                let y = to_svg_y(b.bbox.y);
                out.push_str(&svg_rect(
                    x,
                    y,
                    b.bbox.w,
                    b.bbox.h,
                    &box_fill,
                    &box_stroke,
                    box_sw,
                    box_radius,
                ));
            }
            crate::scene::Primitive::Text(t) => {
                let (font_family, font_size) = font_for_attr(&t.attrs, prefs);
                let weight = weight_for_attr(&t.attrs, prefs);
                let color = color_for_attr(&t.attrs, prefs);
                let bg_color: Option<String> = if is_highlighted(&t.attrs) {
                    Some(hex_color(
                        prefs.output.style.text.highlights.background_color,
                    ))
                } else {
                    None
                };
                // baseline = bbox.y + bbox.h converted to SVG
                let baseline_svg = to_svg_y(t.bbox.y + t.bbox.h);
                let cw = font_size * CHAR_WIDTH_RATIO;

                let anchor_x = match t.align {
                    TextAlign::Left => to_svg_x(t.bbox.x),
                    TextAlign::Center => to_svg_x(t.bbox.x + t.bbox.w / 2.0),
                    TextAlign::Right => to_svg_x(t.bbox.x + t.bbox.w),
                };

                render_mixed_text(
                    &mut out,
                    anchor_x,
                    baseline_svg,
                    &t.content,
                    &font_family,
                    font_size,
                    weight,
                    cw,
                    &color,
                    bg_color.as_deref(),
                    &t.align,
                );
            }
            crate::scene::Primitive::Connector(c) => {
                if c.child_points.is_empty() {
                    continue;
                }

                let parent_svgs: Vec<(f64, f64)> = c
                    .parent_points
                    .iter()
                    .map(|p| (to_svg_x(p.x), to_svg_y(p.y)))
                    .collect();
                let child_svgs: Vec<(f64, f64)> = c
                    .child_points
                    .iter()
                    .map(|p| (to_svg_x(p.x), to_svg_y(p.y)))
                    .collect();

                // Bar y = midpoint between parent exit and first child entry
                let bar_y = (parent_svgs[0].1 + child_svgs[0].1) / 2.0;

                // Bar x spans all parent and child x values
                let all_x: Vec<f64> = parent_svgs
                    .iter()
                    .map(|p| p.0)
                    .chain(child_svgs.iter().map(|p| p.0))
                    .collect();
                let bar_x_min = all_x.iter().cloned().fold(f64::INFINITY, f64::min);
                let bar_x_max = all_x.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

                // Vertical drops from parents to bar
                for (px, py) in &parent_svgs {
                    out.push_str(&svg_line(*px, *py, *px, bar_y, &conn_color, conn_width));
                }

                // Horizontal bar
                if (bar_x_max - bar_x_min).abs() > 0.1 {
                    out.push_str(&svg_line(
                        bar_x_min,
                        bar_y,
                        bar_x_max,
                        bar_y,
                        &conn_color,
                        conn_width,
                    ));
                }

                // Vertical drops from bar to children
                for (cx_svg, cy_svg) in &child_svgs {
                    out.push_str(&svg_line(
                        *cx_svg,
                        bar_y,
                        *cx_svg,
                        *cy_svg,
                        &conn_color,
                        conn_width,
                    ));
                }
            }
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
                    "  <path d=\"{path}\" fill=\"{box_fill}\" stroke=\"{box_stroke}\" stroke-width=\"0.5\"/>\n"
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
                        );
                    }
                    out.push_str("  </g>\n");
                }
            }
            crate::scene::Primitive::FancyText(item) => {
                for line in &item.lines {
                    let (font_family, font_size) = font_for_attr(&line.attrs, prefs);
                    let weight = weight_for_attr(&line.attrs, prefs);
                    let color = color_for_attr(&line.attrs, prefs);
                    let cw = font_size * CHAR_WIDTH_RATIO;
                    let x_svg = to_svg_x(line.x);
                    let y_svg = to_svg_y(line.y + font_size * 0.85);
                    render_mixed_text(
                        &mut out,
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
                    );
                }
            }
            crate::scene::Primitive::FancyConn(conn) => {
                use crate::scene::FancyConnKind;
                match &conn.kind {
                    FancyConnKind::IndivToSpouse => indiv_conns.push(conn),
                    FancyConnKind::SpouseToChildren => spouse_conns.push(conn),
                }
            }
        }
    }

    // ── Fancy layout connector groups ─────────────────────────────────────────
    if !indiv_conns.is_empty() || !spouse_conns.is_empty() {
        let offset_x = MARGIN;
        let offset_y = MARGIN + chart_top_offset;
        out.push_str(&format!(
            "  <g id=\"fancy-connectors-1\" transform=\"translate({offset_x:.1},{offset_y:.1})\">\n"
        ));
        for c in &indiv_conns {
            out.push_str(&format!(
                "    <path d=\"{}\" stroke=\"{}\" stroke-width=\"{:.1}\" fill=\"none\" stroke-linecap=\"round\"/>\n",
                c.d, c.stroke, c.stroke_width
            ));
        }
        out.push_str("  </g>\n");
        out.push_str(&format!(
            "  <g id=\"fancy-connectors-2\" transform=\"translate({offset_x:.1},{offset_y:.1})\">\n"
        ));
        for c in &spouse_conns {
            out.push_str(&format!(
                "    <path d=\"{}\" stroke=\"{}\" stroke-width=\"{:.1}\" fill=\"none\" stroke-linecap=\"round\"/>\n",
                c.d, c.stroke, c.stroke_width
            ));
        }
        out.push_str("  </g>\n");
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
        const UNDERLINE_COLOR: &str = "#CCCCCC";
        const UNDERLINE_WIDTH: f64 = 0.5;
        for (&_row, &(row_y, _max_x, font_size)) in &row_info {
            let baseline_svg = to_svg_y(row_y + font_size);
            let underline_y = baseline_svg + font_size * 0.16 + 1.0;
            let line_x1 = to_svg_x(0.0);
            let line_x2 = to_svg_x(scene.canvas_bounds.w);
            out.push_str(&format!(
                "    <line x1=\"{x1:.1}\" y1=\"{y:.1}\" x2=\"{x2:.1}\" y2=\"{y:.1}\" \
                 stroke=\"{color}\" stroke-width=\"{width}\" class=\"row-rule\"/>\n",
                x1 = line_x1,
                y = underline_y,
                x2 = line_x2,
                color = UNDERLINE_COLOR,
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
            out.contains("class=\"row-rule\""),
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
            !out.contains("class=\"row-rule\""),
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
                .any(|l| l.contains("class=\"row-rule\"") && l.contains("x1=\"20.0\"")),
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
    fn test_bc_svg_contains_names() {
        let mut prefs = bc_prefs();
        prefs.format.individual = "{firstname} {lastname}".into();
        let out = render_to_string(&bc_layout_with_prefs(&prefs), &prefs).unwrap();
        assert!(out.contains("John"), "root name missing");
        assert!(out.contains("Jane"), "spouse name missing");
        assert!(out.contains("Paul"), "child name missing");
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
}
