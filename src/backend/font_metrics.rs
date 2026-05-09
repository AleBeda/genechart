//! Exact text advance-width measurement via fontdb + ttf-parser.
//!
//! Falls back silently to `None` when the named font is absent from the system,
//! so callers can use the estimate-based fallback (CHAR_WIDTH_RATIO × font_size).

use fontdb::{Database, Family, Query, Weight};
use std::sync::OnceLock;

static FONT_DB: OnceLock<Database> = OnceLock::new();

fn font_db() -> &'static Database {
    FONT_DB.get_or_init(|| {
        let mut db = Database::new();
        db.load_system_fonts();
        db
    })
}

/// Measure the advance width in pixels of `text` rendered in `font_family` at `font_size` px
/// with normal (400) weight.
///
/// Returns `None` when the font is not found on the system or metrics are unavailable.
/// Characters absent from the font's cmap use the `.notdef` (glyph 0) advance.
pub fn measure_text(text: &str, font_family: &str, font_size: f64) -> Option<f64> {
    measure_text_w(text, font_family, font_size, false)
}

/// Like [`measure_text`] but queries the bold (700) variant of the font when `bold` is true.
pub fn measure_text_w(text: &str, font_family: &str, font_size: f64, bold: bool) -> Option<f64> {
    if text.is_empty() {
        return Some(0.0);
    }
    let db = font_db();
    let query = Query {
        families: &[Family::Name(font_family)],
        weight: if bold { Weight::BOLD } else { Weight::NORMAL },
        ..Default::default()
    };
    let id = db.query(&query)?;
    db.with_face_data(id, |data, index| -> Option<f64> {
        let face = ttf_parser::Face::parse(data, index).ok()?;
        let upem = face.units_per_em() as f64;
        if upem <= 0.0 {
            return None;
        }
        let mut total = 0.0f64;
        for ch in text.chars() {
            let gid = face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
            let advance = face.glyph_hor_advance(gid).unwrap_or(0) as f64;
            total += advance;
        }
        Some(total / upem * font_size)
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_measure_empty_string() {
        assert_eq!(measure_text("", "monospace", 14.0), Some(0.0));
    }

    #[test]
    fn test_measure_nonexistent_font_returns_none() {
        assert_eq!(
            measure_text("hello", "ThisFontSurelyDoesNotExist_XYZ", 14.0),
            None
        );
    }

    #[test]
    fn test_measure_consistent_for_same_input() {
        // If a system font is available, two calls must return identical results.
        let a = measure_text("Hello World", "monospace", 14.0);
        let b = measure_text("Hello World", "monospace", 14.0);
        assert_eq!(a, b);
    }

    #[test]
    fn test_measure_scales_with_font_size() {
        // Width at 28pt should be exactly 2× width at 14pt.
        let w14 = measure_text("ABC", "monospace", 14.0);
        let w28 = measure_text("ABC", "monospace", 28.0);
        if let (Some(w14), Some(w28)) = (w14, w28) {
            let ratio = w28 / w14;
            assert!(
                (ratio - 2.0).abs() < 0.001,
                "expected 2× scaling, got {ratio}"
            );
        }
    }

    #[test]
    fn test_measure_longer_string_is_wider() {
        let short = measure_text("AB", "monospace", 14.0);
        let long = measure_text("ABCD", "monospace", 14.0);
        if let (Some(s), Some(l)) = (short, long) {
            assert!(l > s, "ABCD should be wider than AB: {s} vs {l}");
        }
    }

    #[test]
    fn test_bold_wider_than_normal() {
        // Bold glyphs should be at least as wide as normal glyphs.
        let normal = measure_text_w("Hello World", "Georgia", 14.0, false);
        let bold = measure_text_w("Hello World", "Georgia", 14.0, true);
        if let (Some(n), Some(b)) = (normal, bold) {
            assert!(b >= n, "bold should be >= normal width: {n} vs {b}");
        }
    }
}
