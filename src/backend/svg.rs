//! SVG output backend (stub).

pub struct SvgRenderer;

impl crate::backend::Renderer for SvgRenderer {
    fn render(
        &self,
        _output: &crate::layout::LayoutOutput,
        _prefs: &crate::preferences::Prefs,
        _writer: &mut dyn std::io::Write,
    ) -> anyhow::Result<()> {
        anyhow::bail!("SVG backend is not yet implemented")
    }
}
