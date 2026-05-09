//! Renderer trait and backend dispatcher.

pub(crate) mod font_metrics;
pub mod pdf;
pub mod svg;
pub mod text;

pub trait Renderer {
    fn render(
        &self,
        output: &crate::layout::LayoutOutput,
        prefs: &crate::preferences::Prefs,
        writer: &mut dyn std::io::Write,
    ) -> anyhow::Result<()>;
}
