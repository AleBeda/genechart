//! PDF output backend via SVG conversion (stub).

pub struct PdfRenderer;

impl crate::backend::Renderer for PdfRenderer {
    fn render(
        &self,
        _output: &crate::layout::LayoutOutput,
        _prefs: &crate::preferences::Prefs,
        _writer: &mut dyn std::io::Write,
    ) -> anyhow::Result<()> {
        anyhow::bail!("PDF backend is not yet implemented")
    }
}
