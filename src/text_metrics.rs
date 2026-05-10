//! Shared text-metric constants used by the layout and text-backend layers.
//!
//! These values are also used in `backend/svg.rs` (imported via `use crate::text_metrics::*`).

/// Default line height in pixels (at default font size).
pub const LINE_HEIGHT: f64 = 18.0;

/// Default font size in pixels.
pub const FONT_SIZE: f64 = 13.0;

/// Estimated average character width as a fraction of font size.
/// Used for column-position arithmetic when exact glyph metrics are unavailable.
pub const CHAR_WIDTH_RATIO: f64 = 0.6;

/// Font-family fallback used when the preference is empty.
const FONT_FAMILY: &str = "monospace";

/// Parse "Family Name Size" preference string → (family, size).
/// The last whitespace-delimited token is tried as a float; the rest is the family.
pub fn parsed_font(font_pref: &str) -> (String, f64) {
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
