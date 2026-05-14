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

/// Expand `{gedcom}` in a title/copyright template string.
pub(crate) fn expand_title_template(template: &str, prefs: &crate::preferences::Prefs) -> String {
    let gedcom_name = std::path::Path::new(&prefs.files.gedcom)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let mut vars = std::collections::HashMap::new();
    vars.insert("gedcom".to_string(), gedcom_name.to_string());
    strfmt::strfmt(template, &vars).unwrap_or_else(|_| template.to_string())
}
