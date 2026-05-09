//! Multi-level TOML preference system.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use toml::Value;

const DEFAULTS_TOML: &str = include_str!("defaults.toml");

// ── Structs ──────────────────────────────────────────────────────────────────

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticsPrefs {
    #[serde(default = "default_true")]
    pub errors: bool,
    #[serde(default)]
    pub warnings: bool,
    #[serde(default)]
    pub info: bool,
    #[serde(default)]
    pub debug: bool,
}

impl Default for DiagnosticsPrefs {
    fn default() -> Self {
        Self {
            errors: true,
            warnings: false,
            info: false,
            debug: false,
        }
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
    #[serde(default)]
    pub last_gen_spouses: bool,
    #[serde(default)]
    pub id: bool,
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
    pub spouse_sep_height: f64,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TextStylePrefs {
    #[serde(default)]
    pub names: i64,
    #[serde(default)]
    pub dates: i64,
    #[serde(default)]
    pub id: i64,
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
    #[serde(default)]
    pub text: TextStylePrefs,
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
            text: TextStylePrefs::default(),
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
    #[serde(default, deserialize_with = "de_f64")]
    pub radius: f64,
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
    #[serde(default)]
    pub id: String,
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
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Load and merge all preference sources, applying CLI overrides last.
///
/// Merge order (lowest → highest priority):
/// 1. Embedded `defaults.toml`
/// 2. `~/.genechart.toml`
/// 3. `<gedcom_dir>/genechart.toml`
/// 4. `<gedcom_basename>.toml`
/// 5. `preff_path` (the `--preff` file, if supplied — errors if the path does not exist)
/// 6. `pref_overrides` (each `--pref` assignment string)
pub fn load(
    gedcom_path: Option<&Path>,
    preff_path: Option<&Path>,
    pref_overrides: &[String],
    tracer: &crate::trace::Tracer,
) -> Result<Prefs> {
    // Level 1: embedded defaults — start from an empty table so all default
    // keys appear as KEY-VALUE (new) in the trace.
    let mut base = Value::Table(toml::map::Map::new());
    let defaults: Value = DEFAULTS_TOML
        .parse::<Value>()
        .context("failed to parse embedded defaults.toml")?;
    tracer.emit("prefs", "PREF SOURCE <embedded defaults>");
    merge_toml_tracked(&mut base, defaults, "", tracer);

    // Level 2: ~/.genechart.toml
    if let Some(home_toml) = home_toml_path() {
        merge_file(&mut base, &home_toml, tracer);
    }

    // Level 3: <gedcom_dir>/genechart.toml
    if let Some(ged) = gedcom_path {
        if let Some(dir) = ged.parent() {
            merge_file(&mut base, &dir.join("genechart.toml"), tracer);
        }
    }

    // Level 4: <gedcom_basename>.toml
    if let Some(ged) = gedcom_path {
        merge_file(&mut base, &ged.with_extension("toml"), tracer);
    }

    // Level 5: --preff explicit file
    if let Some(preff) = preff_path {
        merge_file_required(&mut base, preff, tracer)?;
    }

    // Level 6: --pref overrides
    for pref_str in pref_overrides {
        merge_pref_str(&mut base, pref_str, tracer);
    }

    let prefs: Prefs = base
        .try_into()
        .context("failed to deserialize preferences")?;
    Ok(prefs)
}

// ── Private helpers ──────────────────────────────────────────────────────────

fn home_toml_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| std::path::PathBuf::from(h).join(".genechart.toml"))
}

fn merge_file(base: &mut Value, path: &Path, tracer: &crate::trace::Tracer) {
    match std::fs::read_to_string(path) {
        Ok(content) => match content.parse::<Value>() {
            Ok(overlay) => {
                tracer.emit("prefs", &format!("PREF SOURCE {}", path.display()));
                merge_toml_tracked(base, overlay, "", tracer);
            }
            Err(e) => eprintln!("warning: failed to parse {}: {e}", path.display()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("warning: failed to read {}: {e}", path.display()),
    }
}

/// Like `merge_file`, but returns an error instead of silently skipping a missing file.
/// Used for `--preff` where the user explicitly named the file.
fn merge_file_required(base: &mut Value, path: &Path, tracer: &crate::trace::Tracer) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("--preff: failed to read '{}'", path.display()))?;
    let overlay = content
        .parse::<Value>()
        .with_context(|| format!("--preff: failed to parse '{}'", path.display()))?;
    tracer.emit("prefs", &format!("PREF SOURCE {}", path.display()));
    merge_toml_tracked(base, overlay, "", tracer);
    Ok(())
}

/// Split `s` on commas that are not inside a quoted TOML string.
/// This allows values like `format.marriage = "a, b"` in a single --pref argument.
fn split_pref_assignments(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut in_double = false;
    let mut in_single = false;
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        match bytes[i] {
            b'"' if !in_single => in_double = !in_double,
            b'\'' if !in_double => in_single = !in_single,
            b',' if !in_double && !in_single => {
                let seg = s[start..i].trim();
                if !seg.is_empty() {
                    result.push(seg);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        result.push(last);
    }
    result
}

fn merge_pref_str(base: &mut Value, pref_str: &str, tracer: &crate::trace::Tracer) {
    let toml_doc: String = split_pref_assignments(pref_str).join("\n");
    match toml_doc.parse::<Value>() {
        Ok(overlay) => {
            tracer.emit("prefs", &format!("PREF SOURCE --pref '{pref_str}'"));
            merge_toml_tracked(base, overlay, "", tracer);
        }
        Err(e) => eprintln!("warning: failed to parse --pref '{pref_str}': {e}"),
    }
}

/// Format a TOML leaf value as a string for trace output.
fn toml_value_str(val: &Value) -> String {
    match val {
        Value::String(s) => {
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\"")
        }
        Value::Integer(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Boolean(b) => b.to_string(),
        _ => "[complex]".to_string(),
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

/// Like `merge_toml`, but emits PREF KEY-VALUE / PREF OVERRIDE KEY-VALUE
/// trace lines as each leaf is processed.
///
/// `prefix` is the dotted-key path leading to this call (empty at top level).
fn merge_toml_tracked(
    base: &mut Value,
    overlay: Value,
    prefix: &str,
    tracer: &crate::trace::Tracer,
) {
    let Value::Table(overlay_map) = overlay else {
        // Non-table overlay replaces base entirely (unusual; no per-key trace).
        *base = overlay;
        return;
    };

    let Value::Table(ref mut base_map) = *base else {
        *base = Value::Table(overlay_map);
        return;
    };

    for (key, val) in overlay_map {
        let full_key = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };

        match base_map.get_mut(&key) {
            // Both are tables → recurse, no direct leaf emit here.
            Some(base_val) if base_val.is_table() && val.is_table() => {
                merge_toml_tracked(base_val, val, &full_key, tracer);
            }
            // Existing leaf (or table replaced by leaf) → OVERRIDE.
            Some(base_val) => {
                if !val.is_table() {
                    tracer.emit(
                        "prefs",
                        &format!(
                            "PREF OVERRIDE KEY-VALUE {full_key} = {}",
                            toml_value_str(&val)
                        ),
                    );
                }
                *base_val = val;
            }
            // Key absent → NEW.
            None => {
                if val.is_table() {
                    // New subtable: recurse into an empty base so all leaves
                    // are reported as new.
                    let mut empty = Value::Table(toml::map::Map::new());
                    merge_toml_tracked(&mut empty, val, &full_key, tracer);
                    base_map.insert(key, empty);
                } else {
                    tracer.emit(
                        "prefs",
                        &format!(
                            "PREF          KEY-VALUE {full_key} = {}",
                            toml_value_str(&val)
                        ),
                    );
                    base_map.insert(key, val);
                }
            }
        }
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
        let prefs = load(None, None, &[], &crate::trace::Tracer::disabled()).unwrap();
        assert_eq!(prefs.scope.generations, 4);
        assert_eq!(prefs.scope.direction, "descendants");
        assert_eq!(prefs.layout.layout_type, "simple");
        assert_eq!(prefs.output.output_type, "text");
        assert!(prefs.show.generation_num);
        assert_eq!(prefs.layout.boxed_couples.box_width, 240.0);
        assert_eq!(prefs.output.paper.size, "A4");
    }

    #[test]
    fn new_preferences_load() {
        let prefs = load(None, None, &[], &crate::trace::Tracer::disabled()).unwrap();
        assert_eq!(prefs.show.last_gen_spouses, false);
        assert_eq!(prefs.show.id, false);
        assert_eq!(prefs.output.style.text.names, 0x000);
        assert_eq!(prefs.output.style.text.dates, 0x000);
        assert_eq!(prefs.output.style.text.id, 0xE00);
        assert_eq!(prefs.output.style.fonts.id, "Courier 8");
    }

    #[test]
    fn merge_order() {
        let tmp = std::env::temp_dir();
        let file_a = tmp.join("genechart_test_merge_a.toml");
        let file_b = tmp.join("genechart_test_merge_b.toml");

        fs::write(&file_a, "scope.generations = 10\n").unwrap();
        fs::write(&file_b, "scope.generations = 20\n").unwrap();

        let mut base = DEFAULTS_TOML.parse::<Value>().unwrap();
        let tracer = crate::trace::Tracer::disabled();
        merge_file(&mut base, &file_a, &tracer);
        merge_file(&mut base, &file_b, &tracer);

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
        let tracer = crate::trace::Tracer::disabled();
        merge_pref_str(&mut base, "scope.generations = 99", &tracer);
        let prefs: Prefs = base.try_into().unwrap();
        assert_eq!(prefs.scope.generations, 99);
    }

    #[test]
    fn missing_file_silent() {
        let result = load(
            Some(Path::new("/nonexistent/path/family.ged")),
            None,
            &[],
            &crate::trace::Tracer::disabled(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn toml_round_trip() {
        let prefs = Prefs::default();
        let serialized = toml::to_string(&prefs).unwrap();
        let deserialized: Prefs = toml::from_str(&serialized).unwrap();
        assert_eq!(prefs, deserialized);
    }

    #[test]
    fn preff_overrides_basename_toml() {
        let tmp = std::env::temp_dir().join("genechart_test_preff.toml");
        fs::write(&tmp, "scope.generations = 42\n").unwrap();
        let prefs = load(None, Some(&tmp), &[], &crate::trace::Tracer::disabled()).unwrap();
        assert_eq!(
            prefs.scope.generations, 42,
            "--preff should override defaults"
        );
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn pref_overrides_preff() {
        let tmp = std::env::temp_dir().join("genechart_test_preff2.toml");
        fs::write(&tmp, "scope.generations = 42\n").unwrap();
        let prefs = load(
            None,
            Some(&tmp),
            &["scope.generations = 99".into()],
            &crate::trace::Tracer::disabled(),
        )
        .unwrap();
        assert_eq!(
            prefs.scope.generations, 99,
            "--pref should override --preff"
        );
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn preff_missing_file_is_error() {
        let result = load(
            None,
            Some(Path::new("/nonexistent/preff_xyz.toml")),
            &[],
            &crate::trace::Tracer::disabled(),
        );
        assert!(result.is_err(), "missing --preff file should be an error");
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("preff") || msg.contains("nonexistent"),
            "error message should mention the file: {msg}"
        );
    }

    #[test]
    fn pref_str_with_comma_in_quoted_value() {
        let mut base = DEFAULTS_TOML.parse::<Value>().unwrap();
        // format.marriage value contains a comma; output.type must still be applied.
        let tracer = crate::trace::Tracer::disabled();
        merge_pref_str(
            &mut base,
            r#"format.marriage = "m. {date}, {location}", output.type = "pdf""#,
            &tracer,
        );
        let prefs: Prefs = base.try_into().unwrap();
        assert_eq!(
            prefs.output.output_type, "pdf",
            "output.type should be overridden even when a quoted value contains a comma"
        );
        assert!(
            prefs.format.marriage.contains(", "),
            "marriage format should preserve the comma: {}",
            prefs.format.marriage
        );
    }

    #[test]
    fn pref_str_single_assignment_no_comma() {
        let mut base = DEFAULTS_TOML.parse::<Value>().unwrap();
        let tracer = crate::trace::Tracer::disabled();
        merge_pref_str(&mut base, r#"output.type = "svg""#, &tracer);
        let prefs: Prefs = base.try_into().unwrap();
        assert_eq!(prefs.output.output_type, "svg");
    }
}
