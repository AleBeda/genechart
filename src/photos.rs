//! Photo loading and processing for the `boxes` layout.
//!
//! `build_photo_map` resolves a set of individual IDs to image hrefs — either
//! base64 data URIs (for embedded output) or relative/absolute file paths (for
//! linked SVG output).  JPEG and PNG are supported; HEIC is not.

use crate::preferences::PhotosPrefs;
use base64::Engine as _;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Maps a bare individual ID (no `##n` suffix) → image href string.
pub type PhotoMap = HashMap<String, String>;

const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "JPG", "JPEG", "PNG"];

/// Build the photo map for a set of individual IDs.
///
/// - `individual_ids`: bare IDs (no `##n` consanguinity suffix)
/// - `gedcom_path`: path to the GEDCOM file (used to derive the base directory)
/// - `photos_prefs`: the `[photos]` preferences section
/// - `is_pdf`: when true, always embed regardless of `photos.embedded`
/// - `svg_output_path`: the SVG output path (empty = stdout); used for relative paths
pub fn build_photo_map(
    individual_ids: &[&str],
    gedcom_path: &str,
    photos_prefs: &PhotosPrefs,
    is_pdf: bool,
    svg_output_path: &str,
) -> PhotoMap {
    let mut map = PhotoMap::new();

    let gedcom_dir = if gedcom_path.is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        Path::new(gedcom_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    };

    let svg_dir: Option<PathBuf> = if svg_output_path.is_empty() {
        None
    } else {
        Path::new(svg_output_path).parent().map(|p| p.to_path_buf())
    };

    let index_map: HashMap<String, String> = if !photos_prefs.index.is_empty() {
        parse_index_file(&gedcom_dir.join(&photos_prefs.index))
    } else {
        HashMap::new()
    };

    let embed = photos_prefs.embedded || is_pdf;

    for &id in individual_ids {
        let photo_path = match find_photo_path(id, &gedcom_dir, photos_prefs, &index_map) {
            Some(p) => p,
            None => continue,
        };

        let img = match image::open(&photo_path) {
            Ok(img) => img,
            Err(e) => {
                eprintln!(
                    "warning: photos: cannot open {:?} for {id}: {e}",
                    photo_path
                );
                continue;
            }
        };

        let target_w = photos_prefs.width as u32;
        let target_h = photos_prefs.height as u32;
        let img = apply_scale(img, target_w, target_h, &photos_prefs.scale);
        let is_png = is_png_path(&photo_path);

        let href = if embed {
            build_embedded_href(img, is_png, photos_prefs)
        } else {
            build_linked_href(&photo_path, svg_dir.as_deref())
        };

        if !href.is_empty() {
            map.insert(id.to_string(), href);
        }
    }

    map
}

// ── Internal helpers ───────────────────────────────────────────────────────────

fn find_photo_path(
    id: &str,
    gedcom_dir: &Path,
    photos_prefs: &PhotosPrefs,
    index_map: &HashMap<String, String>,
) -> Option<PathBuf> {
    let photos_dir = gedcom_dir.join(&photos_prefs.directory);

    // Check index map first.
    if let Some(filename) = index_map.get(id) {
        let p = photos_dir.join(filename);
        if p.exists() {
            return Some(p);
        }
        // Also try relative to gedcom_dir directly.
        let p2 = gedcom_dir.join(filename);
        if p2.exists() {
            return Some(p2);
        }
    }

    // ID-based: <photos_dir>/<id>.<ext>
    for &ext in EXTENSIONS {
        let p = photos_dir.join(format!("{id}.{ext}"));
        if p.exists() {
            return Some(p);
        }
    }

    None
}

/// Parse an index file.  Format: `ID filename [anything...]`; `#` = comment.
fn parse_index_file(path: &Path) -> HashMap<String, String> {
    match std::fs::read_to_string(path) {
        Ok(content) => content
            .lines()
            .filter_map(|l| {
                // Strip inline comments.
                let l = match l.split_once('#') {
                    Some((before, _)) => before,
                    None => l,
                };
                let l = l.trim();
                if l.is_empty() {
                    return None;
                }
                let mut parts = l.split_whitespace();
                let id = parts.next()?.to_string();
                let filename = parts.next()?.to_string();
                Some((id, filename))
            })
            .collect(),
        Err(e) => {
            eprintln!("warning: photos: cannot read index {:?}: {e}", path);
            HashMap::new()
        }
    }
}

/// Apply the configured scale mode to resize `img` to `(tw, th)` pixels.
fn apply_scale(img: image::DynamicImage, tw: u32, th: u32, scale: &str) -> image::DynamicImage {
    if tw == 0 || th == 0 {
        return img;
    }
    match scale.trim().to_lowercase().as_str() {
        "fit" => img.resize(tw, th, image::imageops::FilterType::Lanczos3),
        "crop" => img.resize_to_fill(tw, th, image::imageops::FilterType::Lanczos3),
        _ => img, // "none" or unknown: no scaling
    }
}

fn is_png_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("png"))
        .unwrap_or(false)
}

/// Encode `img` as a base64 data URI, applying downsampling if configured.
fn build_embedded_href(img: image::DynamicImage, prefer_png: bool, prefs: &PhotosPrefs) -> String {
    // Downsample: max embed pixels = canvas_units * downsample_dpi / 96 css_dpi
    let img = if prefs.downsample > 0.0 {
        let max_w = (prefs.width * prefs.downsample / 96.0) as u32;
        let max_h = (prefs.height * prefs.downsample / 96.0) as u32;
        if max_w > 0 && max_h > 0 && (img.width() > max_w || img.height() > max_h) {
            img.resize(max_w, max_h, image::imageops::FilterType::Lanczos3)
        } else {
            img
        }
    } else {
        img
    };

    let mut buf = std::io::Cursor::new(Vec::<u8>::new());
    let (fmt, mime) = if prefer_png {
        (image::ImageFormat::Png, "image/png")
    } else {
        (image::ImageFormat::Jpeg, "image/jpeg")
    };
    if let Err(e) = img.write_to(&mut buf, fmt) {
        eprintln!("warning: photos: encode error: {e}");
        return String::new();
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());
    format!("data:{mime};base64,{encoded}")
}

/// Build a relative path href from `svg_dir` to `photo_path`.
/// Falls back to the absolute path when `svg_dir` is `None` (stdout mode)
/// or when relative path computation fails.
fn build_linked_href(photo_path: &Path, svg_dir: Option<&Path>) -> String {
    match svg_dir {
        None => photo_path.display().to_string(),
        Some(dir) => compute_relative_path(photo_path, dir)
            .unwrap_or_else(|| photo_path.display().to_string()),
    }
}

/// Compute a relative path from `base` (a directory) to `target`.
fn compute_relative_path(target: &Path, base: &Path) -> Option<String> {
    let target = target.canonicalize().ok()?;
    let base = base.canonicalize().ok()?;

    let mut t_components: Vec<_> = target.components().collect();
    let mut b_components: Vec<_> = base.components().collect();

    let common = t_components
        .iter()
        .zip(b_components.iter())
        .take_while(|(a, b)| a == b)
        .count();
    t_components.drain(..common);
    b_components.drain(..common);

    let mut rel = PathBuf::new();
    for _ in &b_components {
        rel.push("..");
    }
    for c in &t_components {
        rel.push(c);
    }
    Some(rel.display().to_string())
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_index_basic() {
        let dir = std::env::temp_dir();
        let path = dir.join("genechart_photos_test_index.txt");
        std::fs::write(
            &path,
            "# header\n\nI1 john.jpg\nI2 jane.png extra ignored\n",
        )
        .unwrap();
        let map = parse_index_file(&path);
        assert_eq!(map.get("I1").map(String::as_str), Some("john.jpg"));
        assert_eq!(map.get("I2").map(String::as_str), Some("jane.png"));
        assert!(!map.contains_key("# header"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parse_index_missing_file_is_empty() {
        assert!(parse_index_file(Path::new("/nonexistent/index.txt")).is_empty());
    }

    #[test]
    fn build_photo_map_no_photos_dir_returns_empty() {
        let map = build_photo_map(
            &["I1", "I2"],
            "/nonexistent/family.ged",
            &PhotosPrefs::default(),
            false,
            "",
        );
        assert!(map.is_empty());
    }

    #[test]
    fn apply_scale_none_keeps_original_size() {
        let img = image::DynamicImage::new_rgb8(200, 150);
        let out = apply_scale(img, 100, 80, "none");
        assert_eq!((out.width(), out.height()), (200, 150));
    }

    #[test]
    fn apply_scale_crop_hits_target() {
        let img = image::DynamicImage::new_rgb8(200, 150);
        let out = apply_scale(img, 100, 80, "crop");
        assert_eq!((out.width(), out.height()), (100, 80));
    }

    #[test]
    fn apply_scale_fit_bounded() {
        let img = image::DynamicImage::new_rgb8(200, 100);
        let out = apply_scale(img, 100, 80, "fit");
        assert!(out.width() <= 100 && out.height() <= 80);
    }
}
