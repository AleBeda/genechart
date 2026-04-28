//! PDF back-end: renders via the SVG back-end then converts with svg2pdf.

use anyhow::Result;
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
    svg_to_pdf(&svg_string)
}

fn svg_to_pdf(svg: &str) -> Result<Vec<u8>> {
    let mut options = svg2pdf::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = svg2pdf::usvg::Tree::from_str(svg, &options)
        .map_err(|e| anyhow::anyhow!("SVG parse error: {e}"))?;
    let pdf = svg2pdf::to_pdf(&tree, svg2pdf::ConversionOptions::default(), svg2pdf::PageOptions::default())
        .map_err(|e| anyhow::anyhow!("svg2pdf conversion failed: {e}"))?;
    Ok(pdf)
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
}
