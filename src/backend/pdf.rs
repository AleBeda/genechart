//! PDF back-end: renders via the SVG back-end then converts with svg2pdf.

use anyhow::Result;
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref};
use crate::backend::Renderer;
use crate::layout::LayoutOutput;
use crate::preferences::Prefs;

pub struct PdfRenderer;

impl Renderer for PdfRenderer {
    fn render(
        &self,
        output: &LayoutOutput,
        prefs: &Prefs,
        writer: &mut dyn std::io::Write,
    ) -> Result<()> {
        let pdf_bytes = render_to_bytes(output, prefs)?;
        writer.write_all(&pdf_bytes)?;
        Ok(())
    }
}

pub fn render_to_bytes(output: &LayoutOutput, prefs: &Prefs) -> Result<Vec<u8>> {
    let svg_string = crate::backend::svg::render_to_string(output, prefs)?;
    let usvg_opts  = make_usvg_options();

    let rows    = prefs.output.poster.rows.max(1) as usize;
    let columns = prefs.output.poster.columns.max(1) as usize;

    if rows == 1 && columns == 1 {
        // Apply paper sizing if specified.
        let final_svg = if let Some((pw_mm, ph_mm)) = crate::backend::svg::paper_size_mm(prefs) {
            const MM_TO_USER: f64 = 96.0 / 25.4;
            let (vbx, vby, vbw, vbh) = parse_viewbox(&svg_string)
                .unwrap_or((0.0, 0.0, pw_mm * MM_TO_USER, ph_mm * MM_TO_USER));
            let w_str = format!("{pw_mm}mm");
            let h_str = format!("{ph_mm}mm");
            patch_svg_header(&svg_string, vbx, vby, vbw, vbh, &w_str, &h_str)
        } else {
            svg_string.clone()
        };
        let tree = svg2pdf::usvg::Tree::from_str(&final_svg, &usvg_opts)
            .map_err(|e| anyhow::anyhow!("SVG parse error: {e}"))?;
        let pdf = svg2pdf::to_pdf(
            &tree,
            svg2pdf::ConversionOptions { embed_text: true, ..svg2pdf::ConversionOptions::default() },
            svg2pdf::PageOptions { dpi: 96.0 },
        ).map_err(|e| anyhow::anyhow!("svg2pdf conversion failed: {e}"))?;
        return Ok(pdf);
    }

    // Multi-page tiling — requires a known paper size.
    let (page_w_mm, page_h_mm) = match crate::backend::svg::paper_size_mm(prefs) {
        Some(dims) => dims,
        None => {
            eprintln!("warning: multi-page tiling requires output.paper.size to be set; \
                       falling back to single page");
            let tree = svg2pdf::usvg::Tree::from_str(&svg_string, &usvg_opts)
                .map_err(|e| anyhow::anyhow!("SVG parse error: {e}"))?;
            let pdf = svg2pdf::to_pdf(
                &tree,
                svg2pdf::ConversionOptions { embed_text: true, ..svg2pdf::ConversionOptions::default() },
                svg2pdf::PageOptions { dpi: 96.0 },
            ).map_err(|e| anyhow::anyhow!("svg2pdf conversion failed: {e}"))?;
            return Ok(pdf);
        }
    };

    // Coordinate conversions.
    const MM_TO_USER: f64 = 96.0 / 25.4; // SVG user units at 96 DPI
    const MM_TO_PT:   f32 = 72.0 / 25.4; // PDF points at 72 DPI
    let page_user_w = page_w_mm * MM_TO_USER;
    let page_user_h = page_h_mm * MM_TO_USER;
    let page_pt_w   = page_w_mm as f32 * MM_TO_PT;
    let page_pt_h   = page_h_mm as f32 * MM_TO_PT;

    let overlap_user = prefs.output.poster.overlap_mm * MM_TO_USER;
    let step_x = (page_user_w - overlap_user).max(1.0);
    let step_y = (page_user_h - overlap_user).max(1.0);

    let (vbx0, vby0, _vbw, _vbh) = parse_viewbox(&svg_string)
        .unwrap_or((0.0, 0.0, page_user_w, page_user_h));

    let w_str = format!("{page_w_mm}mm");
    let h_str = format!("{page_h_mm}mm");

    let conv_opts = svg2pdf::ConversionOptions { embed_text: true, ..svg2pdf::ConversionOptions::default() };
    let mut tiles: Vec<(String, f32, f32)> = Vec::new();

    for r in 0..rows {
        for c in 0..columns {
            let tile_x = vbx0 + c as f64 * step_x;
            let tile_y = vby0 + r as f64 * step_y;
            let mut tile_svg = patch_svg_header(
                &svg_string,
                tile_x, tile_y, page_user_w, page_user_h,
                &w_str, &h_str,
            );

            // Alignment lines mark the start of unique (non-overlapping) content.
            // Skip tile (0, 0) — no overlap region to mark there.
            if prefs.output.poster.alignment_lines && (r > 0 || c > 0) {
                let color = crate::backend::svg::hex_color(
                    prefs.output.poster.alignment_lines_color
                );
                let mut al_svg = String::new();
                if c > 0 {
                    let ax = tile_x + overlap_user;
                    al_svg.push_str(&format!(
                        "  <line x1=\"{ax:.3}\" y1=\"{:.3}\" x2=\"{ax:.3}\" y2=\"{:.3}\" \
                         stroke=\"{color}\" stroke-width=\"0.5\" opacity=\"0.5\"/>\n",
                        tile_y, tile_y + page_user_h
                    ));
                }
                if r > 0 {
                    let ay = tile_y + overlap_user;
                    al_svg.push_str(&format!(
                        "  <line x1=\"{:.3}\" y1=\"{ay:.3}\" x2=\"{:.3}\" y2=\"{ay:.3}\" \
                         stroke=\"{color}\" stroke-width=\"0.5\" opacity=\"0.5\"/>\n",
                        tile_x, tile_x + page_user_w
                    ));
                }
                tile_svg = tile_svg.replace("</svg>", &format!("{al_svg}</svg>"));
            }

            tiles.push((tile_svg, page_pt_w, page_pt_h));
        }
    }

    assemble_multipage(&tiles, &usvg_opts, conv_opts)
}

fn make_usvg_options() -> svg2pdf::usvg::Options<'static> {
    let mut options = svg2pdf::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    options
}

/// Extract (vbx, vby, vbw, vbh) from the viewBox attribute of the SVG header.
fn parse_viewbox(svg: &str) -> Option<(f64, f64, f64, f64)> {
    let start = svg.find("viewBox=\"")? + "viewBox=\"".len();
    let end = svg[start..].find('"')? + start;
    let parts: Vec<f64> = svg[start..end]
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    if parts.len() == 4 {
        Some((parts[0], parts[1], parts[2], parts[3]))
    } else {
        None
    }
}

/// Replace the viewBox, width, and height attributes in the SVG header.
/// The SVG produced by svg.rs always starts with the XML declaration on line 1
/// and the `<svg ...>` element on line 2.
fn patch_svg_header(svg: &str, vbx: f64, vby: f64, vbw: f64, vbh: f64,
                    new_w: &str, new_h: &str) -> String {
    let after_line1 = svg.find('\n').map(|i| i + 1).unwrap_or(0);
    let after_line2 = svg[after_line1..].find('\n')
        .map(|i| after_line1 + i + 1)
        .unwrap_or(svg.len());
    let body = &svg[after_line2..];
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" \
         width=\"{new_w}\" height=\"{new_h}\" \
         viewBox=\"{vbx:.3} {vby:.3} {vbw:.3} {vbh:.3}\">\n\
         {body}"
    )
}

/// Assemble `tiles` into a multi-page PDF.
/// Each tile is `(svg_string, page_w_pt, page_h_pt)`.
fn assemble_multipage(
    tiles: &[(String, f32, f32)],
    usvg_opts: &svg2pdf::usvg::Options<'static>,
    conv_opts: svg2pdf::ConversionOptions,
) -> Result<Vec<u8>> {
    let mut alloc = Ref::new(1);
    let catalog_ref   = alloc.bump();
    let page_tree_ref = alloc.bump();

    struct TileInfo {
        chunk: pdf_writer::Chunk,
        xobj_ref: Ref,
        page_ref: Ref,
        content_ref: Ref,
        page_w_pt: f32,
        page_h_pt: f32,
    }

    let mut infos: Vec<TileInfo> = Vec::new();

    for (svg_str, page_w_pt, page_h_pt) in tiles {
        let tree = svg2pdf::usvg::Tree::from_str(svg_str, usvg_opts)
            .map_err(|e| anyhow::anyhow!("SVG parse error: {e}"))?;
        let (chunk, xobj_orig) = svg2pdf::to_chunk(&tree, conv_opts)
            .map_err(|e| anyhow::anyhow!("svg2pdf chunk error: {e}"))?;

        // Renumber chunk refs into our allocation space; track the XObject ref.
        let mut xobj_new = xobj_orig;
        let mut map = std::collections::HashMap::new();
        let renumbered = chunk.renumber(|old| {
            let new = *map.entry(old).or_insert_with(|| alloc.bump());
            if old == xobj_orig { xobj_new = new; }
            new
        });

        let page_ref    = alloc.bump();
        let content_ref = alloc.bump();

        infos.push(TileInfo {
            chunk: renumbered,
            xobj_ref: xobj_new,
            page_ref,
            content_ref,
            page_w_pt: *page_w_pt,
            page_h_pt: *page_h_pt,
        });
    }

    let mut pdf = Pdf::new();
    pdf.catalog(catalog_ref).pages(page_tree_ref);
    pdf.pages(page_tree_ref)
        .count(infos.len() as i32)
        .kids(infos.iter().map(|t| t.page_ref));

    for (i, t) in infos.iter().enumerate() {
        // to_chunk produces a 1×1 pt XObject; scale it to fill the page.
        let name_buf = format!("S{i}");
        let name = Name(name_buf.as_bytes());

        let mut content = Content::new();
        content.transform([t.page_w_pt, 0.0, 0.0, t.page_h_pt, 0.0, 0.0]);
        content.x_object(name);
        let content_data = content.finish();

        pdf.stream(t.content_ref, &content_data);

        let mut page = pdf.page(t.page_ref);
        let mut res = page.resources();
        res.x_objects().pair(name, t.xobj_ref);
        res.finish();
        page.media_box(Rect::new(0.0, 0.0, t.page_w_pt, t.page_h_pt));
        page.parent(page_tree_ref);
        page.contents(t.content_ref);
        page.finish();

        pdf.extend(&t.chunk);
    }

    Ok(pdf.finish())
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
1 NAME John /Smith/
1 SEX M
1 FAMS @F1@
0 @I2@ INDI
1 NAME Jane /Smith/
1 SEX F
1 FAMS @F1@
0 @F1@ FAM
1 HUSB @I1@
1 WIFE @I2@
0 TRLR
";

    fn make_layout() -> LayoutOutput {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.layout.layout_type = "simple".into();
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        run_layout(&genrep, &prefs).unwrap()
    }

    #[test]
    fn test_pdf_magic_bytes() {
        let prefs = {
            let mut p = Prefs::default();
            p.scope.root = "I1".into();
            p.layout.layout_type = "simple".into();
            p
        };
        let bytes = render_to_bytes(&make_layout(), &prefs).unwrap();
        assert!(bytes.starts_with(b"%PDF-"), "missing PDF magic bytes");
    }

    #[test]
    fn test_pdf_non_empty() {
        let prefs = {
            let mut p = Prefs::default();
            p.scope.root = "I1".into();
            p.layout.layout_type = "simple".into();
            p
        };
        let bytes = render_to_bytes(&make_layout(), &prefs).unwrap();
        assert!(bytes.len() > 100, "PDF output suspiciously small: {} bytes", bytes.len());
    }

    #[test]
    fn test_pdf_page_size_a4() {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.layout.layout_type = "simple".into();
        prefs.output.paper.size = "A4".into();
        prefs.output.paper.orientation = "portrait".into();
        let bytes = render_to_bytes(&make_layout(), &prefs).unwrap();
        let pdf_str = String::from_utf8_lossy(&bytes);
        // A4 in pt: 595.28 × 841.89 — look for rounded values in the MediaBox
        assert!(
            pdf_str.contains("595") && pdf_str.contains("841"),
            "A4 MediaBox not found in PDF"
        );
    }

    #[test]
    fn test_pdf_multipage() {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.layout.layout_type = "simple".into();
        prefs.output.paper.size = "A4".into();
        prefs.output.paper.orientation = "portrait".into();
        prefs.output.poster.rows    = 1;
        prefs.output.poster.columns = 2;
        prefs.output.poster.overlap_mm = 10.0;
        let bytes = render_to_bytes(&make_layout(), &prefs).unwrap();
        assert!(bytes.starts_with(b"%PDF-"), "missing PDF header");
        let pdf_str = String::from_utf8_lossy(&bytes);
        assert!(pdf_str.contains("/Count 2"), "expected 2 pages in PDF");
    }

    #[test]
    fn test_alignment_lines_in_multipage() {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.layout.layout_type = "simple".into();
        prefs.output.paper.size = "A4".into();
        prefs.output.paper.orientation = "portrait".into();
        prefs.output.poster.rows    = 1;
        prefs.output.poster.columns = 2;
        prefs.output.poster.overlap_mm = 10.0;
        prefs.output.poster.alignment_lines = true;
        let bytes = render_to_bytes(&make_layout(), &prefs).unwrap();
        assert!(bytes.len() > 500, "PDF with alignment lines unexpectedly small");
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn test_pdf_unicode_marriage_symbol() {
        const GED_MARR: &str = "\
0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
0 @I1@ INDI\n1 NAME John /Smith/\n1 SEX M\n1 FAMS @F1@\n\
0 @I2@ INDI\n1 NAME Jane /Smith/\n1 SEX F\n1 FAMS @F1@\n\
0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 MARR\n2 DATE 1 JAN 1900\n0 TRLR\n";

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.layout.layout_type = "simple".into();
        prefs.show.marriage = true;
        prefs.format.marriage = "⚭ {date}, {location}".into();

        let mut genrep = crate::parser::parse_str(GED_MARR).unwrap();
        crate::parser::compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        let layout_out = crate::layout::run_layout(&genrep, &prefs).unwrap();
        let bytes = render_to_bytes(&layout_out, &prefs).unwrap();
        assert!(bytes.starts_with(b"%PDF-"), "missing PDF magic bytes");
        assert!(bytes.len() > 200, "PDF suspiciously small");
    }
}
