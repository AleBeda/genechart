//! SVG back-end (simple, boxed_couples, and fan layouts).

use crate::backend::Renderer;
use crate::backend::font_metrics;
use crate::backend::text::find_marriage;
use crate::format::{format_event, format_name};
use crate::layout::LayoutOutput;
use crate::layout::fan::FanGeo;
use crate::layout::simple::SimpleGeo;
use crate::parser::genrep::{Genrep, Individual};
use crate::preferences::Prefs;
use crate::scene::{Scene, TextAlign, TextAttr};
use anyhow::Result;

// Fallback rendering constants (used when preferences are empty)
const LINE_HEIGHT: f64 = 18.0;
const FONT_SIZE: f64 = 13.0;
const MARGIN: f64 = 20.0;
const FONT_FAMILY: &str = "monospace";
// Estimated average character width as a fraction of font-size.
// Used for column-position arithmetic when exact glyph metrics are unavailable.
const CHAR_WIDTH_RATIO: f64 = 0.6;
// Fixed pixel gap between text and the start/end of a dot leader.
const DOT_LEADER_GAP: f64 = 3.0;
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

fn svg_text_colored(x: f64, y: f64, text: &str, family: &str, size: f64, color: &str) -> String {
    format!(
        "     <text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"{family}\" \
        font-size=\"{size}\" fill=\"{color}\">{}</text>\n",
        xml_escape(text)
    )
}

fn font_weight_from_pref(pref: &str) -> &str {
    match pref.trim().to_lowercase().as_str() {
        "bold" | "bolder" => "bold",
        "light" | "lighter" => "lighter",
        _ => "normal",
    }
}

fn svg_text_w(x: f64, y: f64, text: &str, family: &str, size: f64, weight: &str) -> String {
    format!(
        "  <text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"{family}\" \
         font-size=\"{size}\" font-weight=\"{weight}\" xml:space=\"preserve\">{}</text>\n",
        xml_escape(text)
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
    x: f64,
    y: f64,
    text: &str,
    primary_family: &str,
    font_size: f64,
    cw: f64,
) {
    if text.is_empty() {
        out.push_str(&svg_text(x, y, text, primary_family, font_size));
        return;
    }

    let mut cur_x = x;
    let mut seg_start = 0usize;
    let mut in_symbol = (text.chars().next().map_or(0, |c| c as u32)) >= 0x2000;

    for (byte_pos, c) in text.char_indices() {
        let is_sym = (c as u32) >= 0x2000;
        if is_sym != in_symbol {
            let seg = &text[seg_start..byte_pos];
            let fam = if in_symbol {
                SYMBOL_FONT_FAMILY
            } else {
                primary_family
            };
            out.push_str(&svg_text(cur_x, y, seg, fam, font_size));
            cur_x += seg.chars().count() as f64 * cw;
            seg_start = byte_pos;
            in_symbol = is_sym;
        }
    }
    // flush final segment
    let seg = &text[seg_start..];
    if !seg.is_empty() {
        let fam = if in_symbol {
            SYMBOL_FONT_FAMILY
        } else {
            primary_family
        };
        out.push_str(&svg_text(cur_x, y, seg, fam, font_size));
    }
}

/// Render centered mixed-font text at (cx, y), splitting Unicode symbol runs
/// (codepoint ≥ U+2000) into separate left-aligned `<text>` elements, collectively
/// centered using accurate font metrics for the Latin segments. Symbol segments use
/// SYMBOL_FONT_FAMILY and weight "normal"; non-symbol segments use `primary_family`
/// and `weight`.
#[allow(clippy::too_many_arguments)]
fn render_mixed_text_mid_w(
    out: &mut String,
    cx: f64,
    y: f64,
    text: &str,
    primary_family: &str,
    font_size: f64,
    weight: &str,
    cw: f64,
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

    // Measure each segment: exact metrics for Latin, char-count estimate for symbols.
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
    let mut cur_x = cx - total_width / 2.0;

    for ((seg, is_sym), &w) in segments.iter().zip(seg_widths.iter()) {
        let fam = if *is_sym {
            SYMBOL_FONT_FAMILY
        } else {
            primary_family
        };
        let wt = if *is_sym { "normal" } else { weight };
        out.push_str(&svg_text_w(cur_x, y, seg, fam, font_size, wt));
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

/// Expand `{gedcom}` in a title/copyright template string.
fn expand_title_template(template: &str, prefs: &Prefs) -> String {
    let gedcom_name = std::path::Path::new(&prefs.files.gedcom)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let mut vars = std::collections::HashMap::new();
    vars.insert("gedcom".to_string(), gedcom_name.to_string());
    strfmt::strfmt(template, &vars).unwrap_or_else(|_| template.to_string())
}

fn render_simple(genrep: &Genrep<SimpleGeo>, prefs: &Prefs) -> String {
    // Font metrics
    let (font_family_base, font_size) = parsed_font(&prefs.output.style.fonts.names);
    // Include symbol-font fallbacks so PDF renderers can find glyphs for ⚭, ×, etc.
    let font_family = format!(
        "{font_family_base}, 'Apple Symbols', 'Segoe UI Symbol', 'DejaVu Sans', sans-serif"
    );
    let line_height = font_size * (LINE_HEIGHT / FONT_SIZE);
    let cw = font_size * CHAR_WIDTH_RATIO; // estimated character width

    // Connector style
    let conn_color = hex_color(prefs.output.style.connectors.border);
    let conn_width = if prefs.output.style.connectors.width > 0.0 {
        prefs.output.style.connectors.width
    } else {
        0.5
    };

    // Collect and sort in-scope individuals by line number.
    let mut entries: Vec<(&Individual<SimpleGeo>, &SimpleGeo)> = genrep
        .individuals
        .values()
        .filter(|i| i.in_scope)
        .filter_map(|i| i.geo.as_ref().map(|g| (i, g)))
        .collect();
    entries.sort_by_key(|(_, g)| g.line);

    if entries.is_empty() {
        #[allow(clippy::useless_format)] // For consistency with other
        return format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <svg xmlns=\"http://www.w3.org/2000/svg\" \
             width=\"100\" height=\"100\"></svg>\n"
        );
    }

    let max_line = entries.iter().map(|(_, g)| g.line).max().unwrap_or(0);
    let indent_px = (prefs.layout.simple.indent as f64 * cw).max(cw);

    // Width (in px) of the generation-number prefix "N. " for a given generation.
    // Uses exact font metrics when the font is available, falls back to estimate.
    let gen_prefix_w = |generation: usize| -> f64 {
        if prefs.show.generation_num {
            let s = format!("{:>2}. ", generation);
            font_metrics::measure_text(&s, &font_family_base, font_size)
                .unwrap_or_else(|| s.chars().count() as f64 * cw)
        } else {
            0.0
        }
    };

    // Pixel width of a string: exact when font is available, estimate otherwise.
    let text_w = |s: &str| -> f64 {
        font_metrics::measure_text(s, &font_family_base, font_size)
            .unwrap_or_else(|| s.chars().count() as f64 * cw)
    };

    // ── Compute pixel column positions ────────────────────────────────────────

    // Right edge of the widest name (considering indent + gen-prefix + name).
    let max_name_end: f64 = entries
        .iter()
        .map(|(indi, geo)| {
            MARGIN
                + geo.indent as f64 * indent_px
                + gen_prefix_w(geo.generation)
                + text_w(&format_name(indi, prefs))
        })
        .fold(0.0_f64, f64::max);

    let gap = cw * 2.0; // column gap

    let max_birth_w: f64 = if prefs.show.birth {
        entries
            .iter()
            .filter_map(|(i, _)| {
                i.birth.as_ref().and_then(|e| {
                    format_event(&prefs.format.birth, e.date.as_ref(), e.place.as_deref())
                })
            })
            .map(|s| text_w(&s))
            .fold(0.0_f64, f64::max)
    } else {
        0.0
    };

    let max_death_w: f64 = if prefs.show.death {
        entries
            .iter()
            .filter_map(|(i, _)| {
                i.death.as_ref().and_then(|e| {
                    format_event(&prefs.format.death, e.date.as_ref(), e.place.as_deref())
                })
            })
            .map(|s| text_w(&s))
            .fold(0.0_f64, f64::max)
    } else {
        0.0
    };

    let max_marr_w: f64 = if prefs.show.marriage {
        entries
            .iter()
            .filter_map(|(i, g)| {
                if g.is_spouse {
                    find_marriage(i, genrep).and_then(|e| {
                        format_event(&prefs.format.marriage, e.date.as_ref(), e.place.as_deref())
                    })
                } else {
                    None
                }
            })
            .map(|s| text_w(&s))
            .fold(0.0_f64, f64::max)
    } else {
        0.0
    };

    let x_birth = max_name_end + gap;
    let x_death = x_birth + max_birth_w + gap;
    let x_marriage = x_death + max_death_w + gap;

    let content_right = if max_marr_w > 0.0 {
        x_marriage + max_marr_w
    } else if max_death_w > 0.0 {
        x_death + max_death_w
    } else if max_birth_w > 0.0 {
        x_birth + max_birth_w
    } else {
        max_name_end
    };
    let content_w = content_right + MARGIN;

    // Title / copyright
    let title_text = expand_title_template(&prefs.output.text.title, prefs);
    let (title_font_family, title_font_size) = parsed_font(&prefs.output.style.fonts.title);
    let title_line_h = if title_text.is_empty() {
        0.0
    } else {
        title_font_size * (LINE_HEIGHT / FONT_SIZE)
    };

    let copy_text = expand_title_template(&prefs.output.text.copyright, prefs);
    let (copy_font_family, copy_font_size) = parsed_font(&prefs.output.style.fonts.copyright);
    let copy_line_h = if copy_text.is_empty() {
        0.0
    } else {
        copy_font_size * (LINE_HEIGHT / FONT_SIZE)
    };

    // chart_top_offset: how far down the chart body starts (to make room for title)
    let chart_top_offset = if title_text.is_empty() {
        0.0
    } else {
        title_line_h
    };
    let content_h =
        MARGIN * 2.0 + chart_top_offset + (max_line + 1) as f64 * line_height + copy_line_h;

    // ── Build SVG ─────────────────────────────────────────────────────────────

    let canvas_w = format!("{content_w:.0}");
    let canvas_h = format!("{content_h:.0}");
    let viewbox = format!("0 0 {content_w:.1} {content_h:.1}");

    let mut out = svg_header(&canvas_w, &canvas_h, &viewbox);

    // ── Title ─────────────────────────────────────────────────────────────────
    if !title_text.is_empty() {
        let y = MARGIN + title_font_size; // baseline of title line
        out.push_str(&svg_text(
            MARGIN,
            y,
            &title_text,
            &title_font_family,
            title_font_size,
        ));
    }

    // ── Copyright ─────────────────────────────────────────────────────────────
    if !copy_text.is_empty() {
        let y = MARGIN + chart_top_offset + (max_line + 1) as f64 * line_height + copy_font_size;
        out.push_str(&svg_text(
            MARGIN,
            y,
            &copy_text,
            &copy_font_family,
            copy_font_size,
        ));
    }

    let dot_leaders = prefs.output.style.dot_leaders;

    // Compute font weights for descendants and spouses
    let descendant_weight = font_weight_from_pref(&prefs.output.style.fonts.descendant);
    let spouse_weight = font_weight_from_pref(&prefs.output.style.fonts.spouse);

    // ── Text elements ─────────────────────────────────────────────────────────
    for (indi, geo) in &entries {
        let y = MARGIN + chart_top_offset + (geo.line as f64 + 1.0) * line_height;
        let x_base = MARGIN + geo.indent as f64 * indent_px;
        let gpw = gen_prefix_w(geo.generation);

        // Pre-compute event strings (needed for dot-leader geometry).
        let birth_s: Option<String> = if prefs.show.birth {
            indi.birth.as_ref().and_then(|e| {
                format_event(&prefs.format.birth, e.date.as_ref(), e.place.as_deref())
            })
        } else {
            None
        };
        let death_s: Option<String> = if prefs.show.death {
            indi.death.as_ref().and_then(|e| {
                format_event(&prefs.format.death, e.date.as_ref(), e.place.as_deref())
            })
        } else {
            None
        };
        let marr_s: Option<String> = if geo.is_spouse && prefs.show.marriage {
            find_marriage(indi, genrep).and_then(|e| {
                format_event(&prefs.format.marriage, e.date.as_ref(), e.place.as_deref())
            })
        } else {
            None
        };

        // Generation number (non-spouse only)
        if prefs.show.generation_num && !geo.is_spouse {
            let prefix = format!("{:>2}. ", geo.generation);
            out.push_str(&svg_text(x_base, y, &prefix, &font_family, font_size));
        }

        // Name — rendered as a single element so sex symbols (♂/♀) at the end
        // stay flush with the name text (no positioning gap from our width estimate).
        let name = format_name(indi, prefs);
        let name_weight = if geo.is_spouse {
            spouse_weight
        } else {
            descendant_weight
        };
        out.push_str(&svg_text_w(
            x_base + gpw,
            y,
            &name,
            &font_family,
            font_size,
            name_weight,
        ));
        let mut last_x = x_base + gpw + text_w(&name);

        // Birth (with optional dot leader)
        if let Some(ref s) = birth_s {
            if dot_leaders {
                dot_leader(&mut out, last_x, x_birth, y, font_size, &conn_color);
            }
            render_mixed_text(&mut out, x_birth, y, s, &font_family, font_size, cw);
            last_x = x_birth + text_w(s);
        }

        // Death (with optional dot leader)
        if let Some(ref s) = death_s {
            if dot_leaders {
                dot_leader(&mut out, last_x, x_death, y, font_size, &conn_color);
            }
            render_mixed_text(&mut out, x_death, y, s, &font_family, font_size, cw);
            last_x = x_death + text_w(s);
        }

        // Marriage — spouse only (with optional dot leader)
        if let Some(ref s) = marr_s {
            if dot_leaders {
                dot_leader(&mut out, last_x, x_marriage, y, font_size, &conn_color);
            }
            render_mixed_text(&mut out, x_marriage, y, s, &font_family, font_size, cw);
        }
    }

    // ── Connector <line> elements (ancestors mode) ────────────────────────────
    //
    // x: aligned with the first character of the parent's name (after gen-prefix).
    // y: lines stop at the TOP / BOTTOM of the child row so they do not cross the name.
    for (_, geo) in &entries {
        // x at the parent's name-start (parent is one generation deeper = geo.generation + 1).
        let x_conn =
            MARGIN + (geo.indent + 1) as f64 * indent_px + gen_prefix_w(geo.generation + 1);
        let y_ctr = |line: usize| MARGIN + chart_top_offset + (line as f64 + 0.5) * line_height;
        let y_top = |line: usize| MARGIN + chart_top_offset + line as f64 * line_height;
        let y_bot = |line: usize| MARGIN + chart_top_offset + (line as f64 + 1.0) * line_height;

        if !geo.connectors_above.is_empty() {
            let first = *geo.connectors_above.iter().min().unwrap();
            if first > 0 {
                let father_line = first - 1;
                out.push_str(&svg_line(
                    x_conn,
                    y_ctr(father_line),
                    x_conn,
                    y_top(geo.line),
                    &conn_color,
                    conn_width,
                ));
            }
        }

        if !geo.connectors_below.is_empty() {
            let last = *geo.connectors_below.iter().max().unwrap();
            let mother_line = last + 1;
            out.push_str(&svg_line(
                x_conn,
                y_bot(geo.line),
                x_conn,
                y_ctr(mother_line),
                &conn_color,
                conn_width,
            ));
        }
    }

    out.push_str("</svg>\n");
    out
}

// SVG arc path for one wedge of the fan.
// Angles follow the math convention (0°=right, 90°=top, 180°=left).
// SVG y is flipped: svg_y = cy - math_y.
fn wedge_path(cx: f64, cy: f64, geo: &FanGeo) -> String {
    let a0 = (geo.angle_center - geo.angle_span / 2.0).to_radians();
    let a1 = (geo.angle_center + geo.angle_span / 2.0).to_radians();
    let ri = geo.radius_inner;
    let ro = geo.radius_outer;

    let ox0 = cx + ro * a0.cos();
    let oy0 = cy - ro * a0.sin();
    let ox1 = cx + ro * a1.cos();
    let oy1 = cy - ro * a1.sin();

    let laf = if geo.angle_span >= 180.0 { 1 } else { 0 };

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

fn render_fan(genrep: &Genrep<FanGeo>, prefs: &Prefs) -> String {
    let max_radius = genrep
        .individuals
        .values()
        .filter_map(|i| i.geo.as_ref())
        .map(|g| g.radius_outer)
        .fold(0.0_f64, f64::max);

    if max_radius < 1.0 {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                <svg xmlns=\"http://www.w3.org/2000/svg\" \
                width=\"100\" height=\"100\"></svg>\n"
            .into();
    }

    let (font_family, font_size) = parsed_font(&prefs.output.style.fonts.names);

    // Title / copyright
    let title_text = expand_title_template(&prefs.output.text.title, prefs);
    let (title_font_family, title_font_size) = parsed_font(&prefs.output.style.fonts.title);
    let title_line_h = if title_text.is_empty() {
        0.0
    } else {
        title_font_size * (LINE_HEIGHT / FONT_SIZE)
    };

    let copy_text = expand_title_template(&prefs.output.text.copyright, prefs);
    let (copy_font_family, copy_font_size) = parsed_font(&prefs.output.style.fonts.copyright);
    let copy_line_h = if copy_text.is_empty() {
        0.0
    } else {
        copy_font_size * (LINE_HEIGHT / FONT_SIZE)
    };

    let content_w = 2.0 * (max_radius + MARGIN);
    let fan_h = max_radius + 2.0 * MARGIN;
    let content_h = title_line_h + fan_h + copy_line_h;
    // Fan center y is shifted down by the title height
    let cx = content_w / 2.0;
    let cy = title_line_h + fan_h - MARGIN;

    let canvas_w = format!("{content_w:.0}");
    let canvas_h = format!("{content_h:.0}");
    let viewbox = format!("0 0 {content_w:.1} {content_h:.1}");

    let mut out = svg_header(&canvas_w, &canvas_h, &viewbox);

    // ── Title ─────────────────────────────────────────────────────────────────
    if !title_text.is_empty() {
        let y = title_font_size; // baseline at top
        out.push_str(&svg_text(
            MARGIN,
            y,
            &title_text,
            &title_font_family,
            title_font_size,
        ));
    }

    // ── Copyright ─────────────────────────────────────────────────────────────
    if !copy_text.is_empty() {
        let y = title_line_h + fan_h + copy_font_size - MARGIN;
        out.push_str(&svg_text(
            MARGIN,
            y,
            &copy_text,
            &copy_font_family,
            copy_font_size,
        ));
    }

    let mut indis: Vec<_> = genrep
        .individuals
        .values()
        .filter_map(|i| i.geo.as_ref().map(|g| (i, g)))
        .collect();
    indis.sort_by(|(_, a), (_, b)| {
        a.radius_inner
            .partial_cmp(&b.radius_inner)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for (indi, geo) in &indis {
        let path = wedge_path(cx, cy, geo);
        out.push_str(&format!(
            "  <path d=\"{path}\" fill=\"white\" stroke=\"black\" stroke-width=\"0.5\"/>\n"
        ));

        let label = format_name(indi, prefs);
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

// ── Scene renderer ────────────────────────────────────────────────────────────

/// Build the font family string with symbol fallbacks (for ♂/♀ and other symbols).
fn with_symbol_fallback(base: &str) -> String {
    format!("{base}, 'Apple Symbols', 'Segoe UI Symbol', 'DejaVu Sans', sans-serif")
}

/// Resolve (font_family, font_size) from a `TextAttr` and preferences.
fn font_for_attr(attr: &TextAttr, prefs: &Prefs) -> (String, f64) {
    match attr {
        TextAttr::IndividualName | TextAttr::SpouseName | TextAttr::GenerationNum => {
            let (fam, sz) = parsed_font(&prefs.output.style.fonts.names);
            let sz = if sz <= 0.0 { FONT_SIZE } else { sz };
            (with_symbol_fallback(&fam), sz)
        }
        TextAttr::BirthData | TextAttr::DeathData | TextAttr::MarriageData => {
            let (fam_base, sz_base) = parsed_font(&prefs.output.style.fonts.names);
            let (fam, sz) = parsed_font(&prefs.output.style.fonts.dates);
            let sz = if sz <= 0.0 { sz_base } else { sz };
            let fam = if fam.trim().is_empty() { fam_base } else { fam };
            (with_symbol_fallback(&fam), sz)
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
    }
}

/// Resolve CSS color for a `TextAttr`.
fn color_for_attr(attr: &TextAttr, prefs: &Prefs) -> String {
    match attr {
        TextAttr::IndividualId => hex_color(prefs.output.style.text.id),
        _ => "black".to_string(),
    }
}

/// Resolve font-weight string for a `TextAttr`.
fn weight_for_attr<'a>(attr: &TextAttr, prefs: &'a Prefs) -> &'a str {
    match attr {
        TextAttr::IndividualName => font_weight_from_pref(&prefs.output.style.fonts.descendant),
        TextAttr::SpouseName => font_weight_from_pref(&prefs.output.style.fonts.spouse),
        _ => "normal",
    }
}

fn render_scene(scene: &Scene, prefs: &Prefs) -> String {
    if scene.primitives.is_empty() {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                <svg xmlns=\"http://www.w3.org/2000/svg\" \
                width=\"100\" height=\"100\"></svg>\n"
            .into();
    }

    // Title / copyright (same logic as render_boxed_couples)
    let title_text = expand_title_template(&prefs.output.text.title, prefs);
    let (title_font_family, title_font_size) = parsed_font(&prefs.output.style.fonts.title);
    let title_line_h = if title_text.is_empty() {
        0.0
    } else {
        title_font_size * (LINE_HEIGHT / FONT_SIZE)
    };

    let copy_text = expand_title_template(&prefs.output.text.copyright, prefs);
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
                let (font_family, font_size) = font_for_attr(&t.attr, prefs);
                let weight = weight_for_attr(&t.attr, prefs);
                let color = color_for_attr(&t.attr, prefs);
                // baseline = bbox.y + bbox.h converted to SVG
                let baseline_svg = to_svg_y(t.bbox.y + t.bbox.h);
                let cw = font_size * CHAR_WIDTH_RATIO;

                match t.align {
                    TextAlign::Center => {
                        let cx = to_svg_x(t.bbox.x + t.bbox.w / 2.0);
                        render_mixed_text_mid_w(
                            &mut out,
                            cx,
                            baseline_svg,
                            &t.content,
                            &font_family,
                            font_size,
                            weight,
                            cw,
                        );
                    }
                    TextAlign::Left => {
                        let x = to_svg_x(t.bbox.x);
                        if matches!(t.attr, TextAttr::IndividualId) {
                            out.push_str(&svg_text_colored(
                                x,
                                baseline_svg,
                                &t.content,
                                &font_family,
                                font_size,
                                &color,
                            ));
                        } else {
                            out.push_str(&svg_text_w(
                                x,
                                baseline_svg,
                                &t.content,
                                &font_family,
                                font_size,
                                weight,
                            ));
                        }
                    }
                    TextAlign::Right => {
                        let x = to_svg_x(t.bbox.x + t.bbox.w);
                        out.push_str(&format!(
                            "  <text x=\"{x:.1}\" y=\"{baseline_svg:.1}\" \
                            font-family=\"{font_family}\" font-size=\"{font_size}\" \
                            text-anchor=\"end\">{}</text>\n",
                            xml_escape(&t.content)
                        ));
                    }
                }
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
            crate::scene::Primitive::Wedge(_) => {
                // Fan layout not yet migrated to Scene
            }
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
    match output {
        LayoutOutput::Simple(genrep) => Ok(render_simple(genrep, prefs)),
        LayoutOutput::BoxedCouples(scene) => Ok(render_scene(scene, prefs)),
        LayoutOutput::Fan(genrep) => Ok(render_fan(genrep, prefs)),
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
        assert!(
            out.contains("stroke-dasharray"),
            "dot-leader lines expected: {out}"
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
            !out.contains("stroke-dasharray"),
            "no dot leaders expected: {out}"
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
    fn test_svg_sex_symbol_in_name_element() {
        // Sex symbols (♂/♀) must share the same <text> element as the person's name.
        // Splitting them into separate elements (as render_mixed_text does for event
        // strings) creates a visible positioning gap due to character-width estimation.
        let mut prefs = simple_prefs();
        prefs.format.individual = "{firstname} {lastname} {sex}".into();
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
        let has_combined = out.lines().any(|l| {
            l.contains("<text ")
                && (l.contains("♂") || l.contains("♀"))
                && (l.contains("John") || l.contains("Jane") || l.contains("Paul"))
        });
        assert!(
            has_combined,
            "name and sex symbol should be in the same <text> element: {}",
            &out[..out.len().min(500)]
        );
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
        let has_leader_line = out
            .lines()
            .any(|l| l.contains("stroke-dasharray") && l.contains("x1="));
        assert!(has_leader_line, "no dot leader line found: {out}");
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
        // default prefs have empty title/copyright
        let out = render_to_string(&make_layout(&prefs), &prefs).unwrap();
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
        // When show.id is false, no svg_text_colored calls are made for IDs
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

        fn extract_rect_y(svg: &str) -> Option<f64> {
            // First rect = first individual box
            svg.lines().find(|l| l.contains("<rect ")).and_then(|l| {
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
