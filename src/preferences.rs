//! Multi-level TOML preference system.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use toml::Value;

const DEFAULTS_TOML: &str = include_str!("defaults.toml");

// ── Structs ──────────────────────────────────────────────────────────────────

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Prefs {
    pub title: Option<String>,
    #[serde(default)]
    pub files: FilesPrefs,
    #[serde(default)]
    pub scope: ScopePrefs,
    #[serde(default)]
    pub show: ShowPrefs,
    #[serde(default)]
    pub photos: PhotosPrefs,
    #[serde(default)]
    pub format: FormatPrefs,
    #[serde(default)]
    pub layout: LayoutPrefs,
    #[serde(default)]
    pub output: OutputPrefs,
    #[serde(default)]
    pub diagnostics: DiagnosticsPrefs,
    #[serde(default)]
    pub custom: CustomPrefs,
    #[serde(default)]
    pub plugins: PluginsPrefs,
}

impl Default for Prefs {
    fn default() -> Self {
        load(None, &[], &[], &crate::trace::Tracer::disabled())
            .expect("embedded defaults.toml must be valid")
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FilesPrefs {
    #[serde(default)]
    pub gedcom: String,
    #[serde(default)]
    pub highlights: String,
    #[serde(default)]
    pub merge: Vec<String>,
    #[serde(default)]
    pub merge_aliases: Vec<String>,
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
    pub nickname: bool,
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
    pub notes_html: bool,
    #[serde(default)]
    pub last_gen_spouses: bool,
    #[serde(default)]
    pub id: bool,
    #[serde(default)]
    pub duplicated_individual: bool,
    #[serde(default)]
    pub photo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhotosPrefs {
    #[serde(default)]
    pub directory: String,
    #[serde(default)]
    pub index: String,
    #[serde(default)]
    pub embedded: bool,
    #[serde(default, deserialize_with = "de_f64")]
    pub width: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub height: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub margin: f64,
    #[serde(default)]
    pub scale: String,
    #[serde(default = "default_true")]
    pub box_resize: bool,
    #[serde(default, deserialize_with = "de_f64")]
    pub downsample: f64,
}

impl Default for PhotosPrefs {
    fn default() -> Self {
        PhotosPrefs {
            directory: "photos".to_string(),
            index: String::new(),
            embedded: false,
            width: 100.0,
            height: 100.0,
            margin: 2.0,
            scale: "crop".to_string(),
            box_resize: true,
            downsample: 72.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FormatPrefs {
    #[serde(default)]
    pub individual: String,
    #[serde(default)]
    pub individual_nickname: String, // used in lieu of `individual` when show.nickname && nickname set
    #[serde(default)]
    pub birth: String,
    #[serde(default)]
    pub death: String,
    #[serde(default)]
    pub marriage: String,
    #[serde(default)]
    pub date_qualifiers: String, // "none" | "gedcom" | "compact"
    #[serde(default)]
    pub no_name: String,
    #[serde(default)]
    pub living: String, // text for {living} in format.individual when individual.living = true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CustomGedcomTagsPrefs {
    #[serde(default)]
    pub alt_name: String,
    #[serde(default)]
    pub relig_name: String,
    #[serde(default)]
    pub living: String,
    #[serde(default)]
    pub relig_marr: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CustomGedcomPrefs {
    #[serde(default)]
    pub tags: CustomGedcomTagsPrefs,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CustomPrefs {
    #[serde(default)]
    pub gedcom: CustomGedcomPrefs,
}

/// Experimental plugin configuration. Each field is a Lua script path (empty =
/// disabled). Only effective in builds compiled with `--features lua`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PluginsPrefs {
    #[serde(default)]
    pub parse: ParsePluginsPrefs,
}

/// Parse-time Lua hooks. `all` runs before the type-specific `indi`/`fam`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ParsePluginsPrefs {
    /// Script defining `on_individual(ind)`, run for every individual.
    #[serde(default)]
    pub indi: String,
    /// Script defining `on_family(fam)`, run for every family.
    #[serde(default)]
    pub fam: String,
    /// Script defining both `on_individual` and `on_family`, run before `indi`/`fam`.
    #[serde(default)]
    pub all: String,
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
    #[serde(default)]
    pub fancy: FancyPrefs,
    #[serde(default)]
    pub boxes: BoxesPrefs,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BoxesPrefs {
    #[serde(default, deserialize_with = "de_f64")]
    pub box_width: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub box_height: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub gap_width: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub gap_height: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub couple_y_offset: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FanPrefs {
    #[serde(default, deserialize_with = "de_f64")]
    pub ring_height: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub ring_gap: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub outer_ring_height: f64,
    #[serde(default)]
    pub radial_gen: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FancyPrefs {
    #[serde(default, deserialize_with = "de_f64")]
    pub gen_width: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub child_gap: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub anc_gap: f64,
    #[serde(default = "default_true")]
    pub compact: bool,
}

impl Default for FancyPrefs {
    fn default() -> Self {
        Self {
            gen_width: 0.0,
            child_gap: 0.0,
            anc_gap: 0.0,
            compact: true,
        }
    }
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
    #[serde(default, deserialize_with = "de_f64")]
    pub box_width_3_spouses: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputPrefs {
    #[serde(default, rename = "type")]
    pub output_type: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub noclobber: bool,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
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
    pub gen_numbers: i64,
    #[serde(default)]
    pub notes: i64,
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: i64,
    #[serde(default)]
    pub copyright: i64,
    #[serde(default)]
    pub row_rule: i64,
    #[serde(default)]
    pub note_bar: i64,
    #[serde(default)]
    pub note_link: i64,
    #[serde(default)]
    pub highlights: HighlightsPrefs,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HighlightsPrefs {
    #[serde(default)]
    pub color: i64,
    #[serde(default)]
    pub background_color: i64,
    #[serde(default)]
    pub fallback: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct StylePrefs {
    #[serde(default = "default_true")]
    pub dot_leaders: bool,
    #[serde(default)]
    pub boxes: BoxStylePrefs,
    #[serde(default)]
    pub wedges: WedgeStylePrefs,
    #[serde(default)]
    pub connectors: ConnectorStylePrefs,
    #[serde(default)]
    pub fonts: FontPrefs,
    #[serde(default)]
    pub spacing: SpacingPrefs,
    #[serde(default)]
    pub text: TextStylePrefs,
    #[serde(default)]
    pub realistic_tree: RealisticTreePrefs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RealisticTreePrefs {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub trunk_color: i64,
    #[serde(default)]
    pub leaf_color: i64,
    #[serde(default)]
    pub leaf_density: String,
}

impl Default for RealisticTreePrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            style: "tapered".to_string(),
            trunk_color: 0x3d2b1f,
            leaf_color: 0x4a7c3f,
            leaf_density: "medium".to_string(),
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
pub struct WedgeStylePrefs {
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
    #[serde(default)]
    pub id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SpacingPrefs {
    #[serde(default)]
    pub boxed_couples: BoxedCouplesSpacingPrefs,
    #[serde(default, deserialize_with = "de_f64")]
    pub title: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub copyright: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub names_autocompress: f64,
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
/// 5. `preff_paths` (the `--preff` files, applied in order; each errors if the path does not exist)
/// 6. `pref_overrides` (each `--pref` assignment string)
pub fn load(
    gedcom_path: Option<&Path>,
    preff_paths: &[PathBuf],
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

    // Level 5: --preff explicit files, applied in command-line order so a later
    // file overrides conflicting preferences from an earlier one.
    for preff in preff_paths {
        merge_file_required(&mut base, preff, tracer)?;
    }

    // Level 6: --pref overrides
    for pref_str in pref_overrides {
        merge_pref_str(&mut base, pref_str, tracer)?;
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
                let unknown = find_unknown_keys(base, &overlay, "");
                for key in &unknown {
                    eprintln!(
                        "warning: {}: unknown preference key '{key}'",
                        path.display()
                    );
                }
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
    let unknown = find_unknown_keys(base, &overlay, "");
    for key in &unknown {
        eprintln!(
            "warning: {}: unknown preference key '{key}'",
            path.display()
        );
    }
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

fn merge_pref_str(base: &mut Value, pref_str: &str, tracer: &crate::trace::Tracer) -> Result<()> {
    let toml_doc: String = split_pref_assignments(pref_str).join("\n");
    let overlay = toml_doc
        .parse::<Value>()
        .with_context(|| format!("failed to parse --pref '{pref_str}'"))?;
    let unknown = find_unknown_keys(base, &overlay, "");
    if !unknown.is_empty() {
        anyhow::bail!("--pref: unknown preference key(s): {}", unknown.join(", "));
    }
    tracer.emit("prefs", &format!("PREF SOURCE --pref '{pref_str}'"));
    merge_toml_tracked(base, overlay, "", tracer);
    Ok(())
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

/// Returns all dotted key paths present in `overlay` that are absent from `base`.
fn find_unknown_keys(base: &Value, overlay: &Value, prefix: &str) -> Vec<String> {
    let mut unknown = Vec::new();
    collect_unknown_keys(base, overlay, prefix, &mut unknown);
    unknown
}

fn collect_unknown_keys(base: &Value, overlay: &Value, prefix: &str, unknown: &mut Vec<String>) {
    let Value::Table(overlay_map) = overlay else {
        return;
    };
    let base_map = if let Value::Table(m) = base {
        Some(m)
    } else {
        None
    };
    for (key, val) in overlay_map {
        let full_key = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match base_map.and_then(|m| m.get(key)) {
            Some(base_val) if val.is_table() && base_val.is_table() => {
                collect_unknown_keys(base_val, val, &full_key, unknown);
            }
            Some(_) => {}
            None => unknown.push(full_key),
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

// ── Highlights file reader ──────────────────────────────────────────────────

use std::collections::HashSet;

/// Read a highlights file and return the set of IDs to highlight.
///
/// Parses each line: skips blanks and lines starting with `#`, takes the first
/// whitespace-delimited token as the ID.  Returns an empty set on error,
/// emitting a warning to stderr.
pub fn load_highlights(path: &Path) -> HashSet<String> {
    if path.display().to_string().is_empty() {
        return HashSet::new();
    }
    match std::fs::read_to_string(path) {
        Ok(content) => content
            .lines()
            .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
            .filter_map(|l| l.split_whitespace().next().map(|s| s.to_string()))
            .collect(),
        Err(e) => {
            eprintln!("warning: cannot read highlights file {:?}: {e}", path);
            HashSet::new()
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn noclobber_defaults_false() {
        let prefs = load(None, &[], &[], &crate::trace::Tracer::disabled()).unwrap();
        assert!(!prefs.output.noclobber);
    }

    #[test]
    fn duplicated_individual_defaults_false() {
        let prefs = Prefs::default();
        assert!(!prefs.show.duplicated_individual);
    }

    #[test]
    fn noclobber_not_flagged_as_unknown() {
        // Verify that output.noclobber is present in the TOML base after
        // merging defaults, so that find_unknown_keys does not warn about it.
        let mut base = Value::Table(toml::map::Map::new());
        let defaults: Value = DEFAULTS_TOML.parse().unwrap();
        merge_toml_tracked(&mut base, defaults, "", &crate::trace::Tracer::disabled());

        let user_overlay: Value = "output.noclobber = true\n".parse().unwrap();
        let unknown = find_unknown_keys(&base, &user_overlay, "");
        assert!(
            unknown.is_empty(),
            "output.noclobber should be a known key, flagged as unknown: {:?}",
            unknown
        );
    }

    #[test]
    fn defaults_load() {
        let prefs = load(None, &[], &[], &crate::trace::Tracer::disabled()).unwrap();
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
        let prefs = load(None, &[], &[], &crate::trace::Tracer::disabled()).unwrap();
        assert!(!prefs.show.last_gen_spouses);
        assert!(!prefs.show.id);
        assert_eq!(prefs.output.style.text.names, 0x000);
        assert_eq!(prefs.output.style.text.dates, 0x000);
        assert_eq!(prefs.output.style.text.id, 0xE00);
        assert_eq!(prefs.output.style.text.gen_numbers, 0x000);
        assert_eq!(prefs.output.style.text.notes, 0x000);
        assert_eq!(prefs.output.style.text.title, 0x000);
        assert_eq!(prefs.output.style.text.copyright, 0x000);
        assert_eq!(prefs.output.style.text.row_rule, 0xCCC);
        assert_eq!(prefs.output.style.text.note_bar, 0xCCC);
        assert_eq!(prefs.output.style.text.note_link, 0x06C);
        assert_eq!(prefs.output.style.spacing.names_autocompress, 0.85);
        assert!(prefs.diagnostics.errors);
        assert_eq!(prefs.output.style.fonts.id, "Courier 8");
        assert_eq!(prefs.output.style.wedges.width, 0.5);
        assert_eq!(prefs.output.style.wedges.border, 0x222);
        assert_eq!(prefs.output.style.wedges.background, 0xFFF);
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
        merge_toml_tracked(
            &mut base,
            "scope.generations = 7".parse::<Value>().unwrap(),
            "",
            &crate::trace::Tracer::disabled(),
        );
        // CLI override
        let tracer = crate::trace::Tracer::disabled();
        merge_pref_str(&mut base, "scope.generations = 99", &tracer).unwrap();
        let prefs: Prefs = base.try_into().unwrap();
        assert_eq!(prefs.scope.generations, 99);
    }

    #[test]
    fn missing_file_silent() {
        let result = load(
            Some(Path::new("/nonexistent/path/family.ged")),
            &[],
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
        let prefs = load(
            None,
            std::slice::from_ref(&tmp),
            &[],
            &crate::trace::Tracer::disabled(),
        )
        .unwrap();
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
            std::slice::from_ref(&tmp),
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
    fn later_preff_overrides_earlier_preff() {
        // Two --preff files setting the same key; the second must win, and a key
        // set only by the first must survive.
        let a = std::env::temp_dir().join("genechart_test_preff_a.toml");
        let b = std::env::temp_dir().join("genechart_test_preff_b.toml");
        fs::write(
            &a,
            "scope.generations = 42\nscope.direction = \"ancestors\"\n",
        )
        .unwrap();
        fs::write(&b, "scope.generations = 7\n").unwrap();
        let prefs = load(
            None,
            &[a.clone(), b.clone()],
            &[],
            &crate::trace::Tracer::disabled(),
        )
        .unwrap();
        assert_eq!(
            prefs.scope.generations, 7,
            "later --preff should override an earlier one"
        );
        assert_eq!(
            prefs.scope.direction, "ancestors",
            "a key set only in the earlier --preff should survive"
        );
        let _ = fs::remove_file(&a);
        let _ = fs::remove_file(&b);
    }

    #[test]
    fn preff_missing_file_is_error() {
        let result = load(
            None,
            &[PathBuf::from("/nonexistent/preff_xyz.toml")],
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
        )
        .unwrap();
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
        merge_pref_str(&mut base, r#"output.type = "svg""#, &tracer).unwrap();
        let prefs: Prefs = base.try_into().unwrap();
        assert_eq!(prefs.output.output_type, "svg");
    }

    #[test]
    fn pref_unknown_key_is_error() {
        let mut base = DEFAULTS_TOML.parse::<Value>().unwrap();
        let tracer = crate::trace::Tracer::disabled();
        let result = merge_pref_str(&mut base, "scope.generatins = 5", &tracer);
        assert!(result.is_err(), "unknown key should be an error");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("generatins"),
            "error should name the bad key: {msg}"
        );
    }

    #[test]
    fn pref_valid_key_is_ok() {
        let mut base = DEFAULTS_TOML.parse::<Value>().unwrap();
        let tracer = crate::trace::Tracer::disabled();
        assert!(merge_pref_str(&mut base, "scope.generations = 5", &tracer).is_ok());
    }

    #[test]
    fn load_with_bad_pref_is_error() {
        let result = load(
            None,
            &[],
            &["scope.generatins = 5".into()],
            &crate::trace::Tracer::disabled(),
        );
        assert!(result.is_err(), "bad --pref key should abort load");
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("generatins"),
            "error should mention the bad key: {msg}"
        );
    }
}
