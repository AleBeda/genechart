//! Renderer trait and backend dispatcher.

pub mod text;
pub mod svg;
pub mod pdf;

pub trait Renderer {
    fn render(
        &self,
        output: &crate::layout::LayoutOutput,
        prefs: &crate::preferences::Prefs,
        writer: &mut dyn std::io::Write,
    ) -> anyhow::Result<()>;
}
