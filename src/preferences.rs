//! Multi-level TOML preference system.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use toml::Value;

const DEFAULTS_TOML: &str = include_str!("defaults.toml");

// ── Structs ──────────────────────────────────────────────────────────────────

fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticsPrefs {
    #[serde(default = "default_true")]
    pub errors: bool,
    #[serde(default)]
    pub warnings: bool,
    #[serde(default)]
    pub messages: bool,
    #[serde(default)]
    pub debug: bool,
}

impl Default for DiagnosticsPrefs {
    fn default() -> Self {
        Self { errors: true, warnings: false, messages: false, debug: false }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Prefs {
    pub title: Option<String>,
    #[serde(default)]
    pub files: FilesPrefs,
    #[serde(default)]
    pub scope: ScopePrefs,
    #[serde(default)]
    pub show: ShowPrefs,
    #[serde(default)]
    pub format: FormatPrefs,
    #[serde(default)]
    pub layout: LayoutPrefs,
    #[serde(default)]
    pub output: OutputPrefs,
    #[serde(default)]
    pub diagnostics: DiagnosticsPrefs,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FilesPrefs {
    #[serde(default)]
    pub gedcom: String,
    #[serde(default)]
    pub highlights: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ScopePrefs {
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub generations: u32,
    #[serde(default)]
    pub direction: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ShowPrefs {
    #[serde(default)]
    pub generation_num: bool,
    #[serde(default)]
    pub sex: bool,
    #[serde(default)]
    pub birth: bool,
    #[serde(default)]
    pub death: bool,
    #[serde(default)]
    pub marriage: bool,
    #[serde(default)]
    pub notes: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FormatPrefs {
    #[serde(default)]
    pub individual: String,
    #[serde(default)]
    pub birth: String,
    #[serde(default)]
    pub death: String,
    #[serde(default)]
    pub marriage: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LayoutPrefs {
    #[serde(default, rename = "type")]
    pub layout_type: String,
    #[serde(default)]
    pub root_pos: String,
    #[serde(default)]
    pub simple: SimpleLayoutPrefs,
    #[serde(default)]
    pub boxed_couples: BoxedCouplesPrefs,
    #[serde(default)]
    pub fan: FanPrefs,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FanPrefs {
    #[serde(default, deserialize_with = "de_f64")]
    pub ring_height: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub ring_gap: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SimpleLayoutPrefs {
    #[serde(default)]
    pub indent: u32,
    #[serde(default)]
    pub vert_spacing: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BoxedCouplesPrefs {
    #[serde(default, deserialize_with = "de_f64")]
    pub box_width: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub box_height: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub gap_width: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub gap_height: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub box_width_2_spouses: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputPrefs {
    #[serde(default, rename = "type")]
    pub output_type: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub paper: PaperPrefs,
    #[serde(default)]
    pub poster: PosterPrefs,
    #[serde(default)]
    pub text: TextPrefs,
    #[serde(default)]
    pub style: StylePrefs,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PaperPrefs {
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub orientation: String,
    #[serde(default)]
    pub custom: CustomPaperPrefs,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CustomPaperPrefs {
    #[serde(default, deserialize_with = "de_f64")]
    pub width: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PosterPrefs {
    #[serde(default)]
    pub rows: u32,
    #[serde(default)]
    pub columns: u32,
    #[serde(default, deserialize_with = "de_f64")]
    pub overlap_mm: f64,
    #[serde(default = "default_true")]
    pub alignment_lines: bool,
    #[serde(default)]
    pub alignment_lines_color: i64,
}

impl Default for PosterPrefs {
    fn default() -> Self {
        Self {
            rows: 0,
            columns: 0,
            overlap_mm: 0.0,
            alignment_lines: true,
            alignment_lines_color: 0xF80,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TextPrefs {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub copyright: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StylePrefs {
    #[serde(default = "default_true")]
    pub dot_leaders: bool,
    #[serde(default)]
    pub boxes: BoxStylePrefs,
    #[serde(default)]
    pub connectors: ConnectorStylePrefs,
    #[serde(default)]
    pub fonts: FontPrefs,
    #[serde(default)]
    pub alignment: AlignmentPrefs,
    #[serde(default)]
    pub spacing: SpacingPrefs,
}

impl Default for StylePrefs {
    fn default() -> Self {
        Self {
            dot_leaders: true,
            boxes: BoxStylePrefs::default(),
            connectors: ConnectorStylePrefs::default(),
            fonts: FontPrefs::default(),
            alignment: AlignmentPrefs::default(),
            spacing: SpacingPrefs::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BoxStylePrefs {
    #[serde(default, deserialize_with = "de_f64")]
    pub width: f64,
    #[serde(default)]
    pub border: i64,
    #[serde(default)]
    pub background: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ConnectorStylePrefs {
    #[serde(default, deserialize_with = "de_f64")]
    pub width: f64,
    #[serde(default)]
    pub border: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FontPrefs {
    #[serde(default)]
    pub names: String,
    #[serde(default)]
    pub dates: String,
    #[serde(default)]
    pub descendant: String,
    #[serde(default)]
    pub spouse: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub copyright: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AlignmentPrefs {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub date: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SpacingPrefs {
    #[serde(default)]
    pub boxed_couples: BoxedCouplesSpacingPrefs,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BoxedCouplesSpacingPrefs {
    #[serde(default, deserialize_with = "de_f64")]
    pub name_above: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub date_above: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub marriage_above: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub spouse_separation: f64,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Load and merge all preference sources, applying CLI overrides last.
pub fn load(gedcom_path: Option<&Path>, pref_overrides: &[String]) -> Result<Prefs> {
    let mut base: Value = DEFAULTS_TOML
        .parse::<Value>()
        .context("failed to parse embedded defaults.toml")?;

    // Level 2: ~/.genechart.toml
    if let Some(home_toml) = home_toml_path() {
        merge_file(&mut base, &home_toml);
    }

    // Level 3: <gedcom_dir>/genechart.toml
    if let Some(ged) = gedcom_path {
        if let Some(dir) = ged.parent() {
            merge_file(&mut base, &dir.join("genechart.toml"));
        }
    }

    // Level 4: <gedcom_basename>.toml
    if let Some(ged) = gedcom_path {
        merge_file(&mut base, &ged.with_extension("toml"));
    }

    // Level 5: --pref overrides
    for pref_str in pref_overrides {
        merge_pref_str(&mut base, pref_str);
    }

    let prefs: Prefs = base.try_into().context("failed to deserialize preferences")?;
    Ok(prefs)
}

/// Expand `{key}` placeholders in `template` using the `strfmt` crate.
pub fn expand(template: &str, vars: &HashMap<&str, &str>) -> String {
    let owned: HashMap<String, &str> = vars.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    strfmt::strfmt(template, &owned).unwrap_or_else(|_| template.to_string())
}

// ── Private helpers ──────────────────────────────────────────────────────────

fn home_toml_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| std::path::PathBuf::from(h).join(".genechart.toml"))
}

fn merge_file(base: &mut Value, path: &Path) {
    match std::fs::read_to_string(path) {
        Ok(content) => match content.parse::<Value>() {
            Ok(overlay) => merge_toml(base, overlay),
            Err(e) => eprintln!("warning: failed to parse {}: {e}", path.display()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("warning: failed to read {}: {e}", path.display()),
    }
}

fn merge_pref_str(base: &mut Value, pref_str: &str) {
    // Split comma-separated assignments, join with newlines to form valid TOML
    let toml_doc: String = pref_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    match toml_doc.parse::<Value>() {
        Ok(overlay) => merge_toml(base, overlay),
        Err(e) => eprintln!("warning: failed to parse --pref '{pref_str}': {e}"),
    }
}

/// Recursively overlay non-null keys from `overlay` onto `base`.
fn merge_toml(base: &mut Value, overlay: Value) {
    match overlay {
        Value::Table(overlay_map) => {
            if let Value::Table(base_map) = base {
                for (key, val) in overlay_map {
                    match base_map.get_mut(&key) {
                        Some(base_val) => merge_toml(base_val, val),
                        None => {
                            base_map.insert(key, val);
                        }
                    }
                }
            } else {
                *base = Value::Table(overlay_map);
            }
        }
        other => *base = other,
    }
}

/// Deserializes a TOML number (integer or float) as `f64`.
fn de_f64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    use serde::de::Visitor;
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = f64;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "a number")
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<f64, E> {
            Ok(v)
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<f64, E> {
            Ok(v as f64)
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<f64, E> {
            Ok(v as f64)
        }
    }
    d.deserialize_any(V)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn defaults_load() {
        let prefs = load(None, &[]).unwrap();
        assert_eq!(prefs.scope.generations, 4);
        assert_eq!(prefs.scope.direction, "descendants");
        assert_eq!(prefs.layout.layout_type, "simple");
        assert_eq!(prefs.output.output_type, "text");
        assert!(prefs.show.generation_num);
        assert_eq!(prefs.layout.boxed_couples.box_width, 220.0);
        assert_eq!(prefs.output.paper.size, "A4");
    }

    #[test]
    fn merge_order() {
        let tmp = std::env::temp_dir();
        let file_a = tmp.join("genechart_test_merge_a.toml");
        let file_b = tmp.join("genechart_test_merge_b.toml");

        fs::write(&file_a, "scope.generations = 10\n").unwrap();
        fs::write(&file_b, "scope.generations = 20\n").unwrap();

        let mut base = DEFAULTS_TOML.parse::<Value>().unwrap();
        merge_file(&mut base, &file_a);
        merge_file(&mut base, &file_b);

        let prefs: Prefs = base.try_into().unwrap();
        assert_eq!(prefs.scope.generations, 20);

        let _ = fs::remove_file(&file_a);
        let _ = fs::remove_file(&file_b);
    }

    #[test]
    fn cli_override_wins() {
        let mut base = DEFAULTS_TOML.parse::<Value>().unwrap();
        // file-level value
        merge_toml(&mut base, "scope.generations = 7".parse::<Value>().unwrap());
        // CLI override
        merge_pref_str(&mut base, "scope.generations = 99");
        let prefs: Prefs = base.try_into().unwrap();
        assert_eq!(prefs.scope.generations, 99);
    }

    #[test]
    fn missing_file_silent() {
        let result = load(Some(Path::new("/nonexistent/path/family.ged")), &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn toml_round_trip() {
        let prefs = Prefs::default();
        let serialized = toml::to_string(&prefs).unwrap();
        let deserialized: Prefs = toml::from_str(&serialized).unwrap();
        assert_eq!(prefs, deserialized);
    }
}
