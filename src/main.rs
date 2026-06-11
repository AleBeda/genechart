mod backend;
mod cli;
mod format;
mod layout;
mod parser;
mod photos;
mod preferences;
mod scene;
mod text_metrics;
mod trace;
mod util;

use backend::Renderer as _;

/// Return the current UTC time as "YYYY-MM-DD HH:MM:SS.mmm UTC".
fn current_utc_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_ms = dur.as_millis() as i64;
    let secs = total_ms / 1000;
    let ms = (total_ms % 1000) as u32;
    let day_secs = secs % 86400;
    let days = secs / 86400;
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}.{ms:03} UTC")
}

/// Convert days since Unix epoch (1970-01-01) to (year, month, day).
/// Uses Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 {
        z / 146097
    } else {
        (z - 146096) / 146097
    };
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let yr = yoe as i32 + (era as i32) * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let yr = if mo <= 2 { yr + 1 } else { yr };
    (yr, mo, d)
}

/// Re-format known color fields in a serialized TOML string from decimal to hex literals.
fn hexify_color_fields(toml: &str) -> String {
    const COLOR_FIELDS: &[&str] = &[
        "alignment_lines_color",
        "background_color",
        "background",
        "border",
        "color",
        "dates",
        "id",
        "names",
    ];
    let mut out = String::with_capacity(toml.len());
    for line in toml.lines() {
        let trimmed = line.trim_start();
        let mut converted = false;
        for field in COLOR_FIELDS {
            let prefix = format!("{field} = ");
            if let Some(rest) = trimmed.strip_prefix(prefix.as_str()) {
                if let Ok(n) = rest.parse::<i64>() {
                    let indent = &line[..line.len() - trimmed.len()];
                    out.push_str(&format!("{indent}{field} = 0x{n:X}\n"));
                    converted = true;
                    break;
                }
            }
        }
        if !converted {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn resolve_rel_path(s: &str, base: &std::path::Path) -> std::path::PathBuf {
    let p = std::path::PathBuf::from(s);
    if p.is_absolute() { p } else { base.join(p) }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    // 1. Parse CLI
    let args = cli::parse();

    // 2. Create tracer
    let tracer = trace::Tracer::new(&args.trace);

    // 3. Resolve GEDCOM path
    let gedcom_path = cli::resolve_gedcom_path(&args)?;

    // 4. Detect dump mode (bare --pref with no value)
    let dump_mode = args.prefs.iter().any(|s| s.is_empty());

    // Filter out empty strings (dump-mode sentinels) before passing to preferences::load
    let pref_overrides: Vec<String> = args
        .prefs
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect();

    // 5. Load preferences (merging all sources + --preff file + --pref overrides)
    let mut prefs = preferences::load(
        Some(&gedcom_path),
        args.preff.as_deref(),
        &pref_overrides,
        &tracer,
    )?;

    // 6. Apply CLI shortcuts (override preference-file values).
    //    Each shortcut that is set emits a trace line so --trace prefs shows
    //    the full resolution chain including command-line flags.
    //    Order: dir → type → output path → root → generations → output type
    {
        let any = args.dir.is_some()
            || args.layout_type.is_some()
            || args.output.is_some()
            || args.root.is_some()
            || args.generations.is_some()
            || args.text
            || args.svg
            || args.pdf;
        if any {
            tracer.emit("prefs", "PREF SOURCE <command-line flags>");
        }

        if let Some(dir) = &args.dir {
            tracer.emit(
                "prefs",
                &format!("PREF OVERRIDE KEY-VALUE scope.direction = \"{dir}\""),
            );
            prefs.scope.direction = dir.clone();
        }

        if let Some(lt) = &args.layout_type {
            tracer.emit(
                "prefs",
                &format!("PREF OVERRIDE KEY-VALUE layout.type = \"{lt}\""),
            );
            prefs.layout.layout_type = lt.clone();
        }

        if let Some(out_path) = &args.output {
            // With --prpref, -o is used only to infer output.type; don't set the path.
            if !args.prpref {
                tracer.emit(
                    "prefs",
                    &format!(
                        "PREF OVERRIDE KEY-VALUE output.path = \"{}\"",
                        out_path.display()
                    ),
                );
                prefs.output.path = out_path.display().to_string();
            }
        }

        if let Some(root) = &args.root {
            tracer.emit(
                "prefs",
                &format!("PREF OVERRIDE KEY-VALUE scope.root = \"{root}\""),
            );
            prefs.scope.root = root.clone();
        }

        if let Some(gens) = args.generations {
            tracer.emit(
                "prefs",
                &format!("PREF OVERRIDE KEY-VALUE scope.generations = {gens}"),
            );
            prefs.scope.generations = gens;
        }

        // Output type: explicit flags win, then infer from file extension.
        if args.text {
            tracer.emit("prefs", "PREF OVERRIDE KEY-VALUE output.type = \"text\"");
            prefs.output.output_type = "text".to_string();
        } else if args.svg {
            tracer.emit("prefs", "PREF OVERRIDE KEY-VALUE output.type = \"svg\"");
            prefs.output.output_type = "svg".to_string();
        } else if args.pdf {
            tracer.emit("prefs", "PREF OVERRIDE KEY-VALUE output.type = \"pdf\"");
            prefs.output.output_type = "pdf".to_string();
        } else if let Some(out_path) = &args.output {
            let ext = out_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase());
            let inferred = match ext.as_deref() {
                Some("txt") => Some("text"),
                Some("svg") => Some("svg"),
                Some("pdf") => Some("pdf"),
                _ => None,
            };
            if let Some(t) = inferred {
                tracer.emit(
                    "prefs",
                    &format!("PREF OVERRIDE KEY-VALUE output.type = \"{t}\""),
                );
                prefs.output.output_type = t.to_string();
            }
        }
    }

    // Store the resolved GEDCOM path for use in title/copyright templates
    prefs.files.gedcom = gedcom_path.display().to_string();

    // 7. Dump mode: print merged prefs and exit
    if args.prpref || dump_mode {
        prefs.title = Some(format!(
            "Resolved preferences — {}",
            current_utc_timestamp()
        ));
        let serialized = toml::to_string_pretty(&prefs)
            .unwrap_or_else(|_| toml::to_string(&prefs).unwrap_or_default());
        print!("{}", hexify_color_fields(&serialized));
        return Ok(());
    }

    // 8. Parse GEDCOM (with optional merge from further files)
    parser::set_diagnostics(prefs.diagnostics.clone());
    parser::set_parser_tags(prefs.custom.gedcom.tags.clone());

    let gedcom_dir = gedcom_path.parent().unwrap_or(std::path::Path::new("."));
    let mut further_pairs: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();

    if !args.merge.is_empty() {
        // CLI --merge takes precedence over preferences; collect pairs
        for chunk in args.merge.chunks(2) {
            if chunk.len() == 2 {
                further_pairs.push((
                    resolve_rel_path(&chunk[0], gedcom_dir),
                    resolve_rel_path(&chunk[1], gedcom_dir),
                ));
            }
        }
    } else {
        // Fall back to preference-based merge pairs
        let mfiles = &prefs.files.merge;
        let maliases = &prefs.files.merge_aliases;
        if mfiles.len() > maliases.len() {
            anyhow::bail!(
                "files.merge_aliases must have at least as many entries as files.merge \
                 ({} merge files, {} alias files)",
                mfiles.len(),
                maliases.len()
            );
        }
        for (ged_str, alias_str) in mfiles.iter().zip(maliases.iter()) {
            further_pairs.push((
                resolve_rel_path(ged_str, gedcom_dir),
                resolve_rel_path(alias_str, gedcom_dir),
            ));
        }
    }

    let mut genrep = if further_pairs.is_empty() {
        parser::parse(&gedcom_path)?
    } else {
        parser::parse_and_merge(&gedcom_path, &further_pairs)?
    };

    // 9. Compute scope
    let root_id = (!prefs.scope.root.is_empty()).then_some(prefs.scope.root.as_str());
    let gens = (prefs.scope.generations > 0).then_some(prefs.scope.generations);
    parser::compute_scope_opts(
        &mut genrep,
        root_id,
        &prefs.scope.direction,
        gens,
        prefs.show.last_gen_spouses,
    );

    // 10. Run layout
    #[cfg(feature = "bc_debug")]
    layout::boxed_couples::bc_debug_init();
    let layout_output = layout::run_layout(&genrep, &prefs)?;
    #[cfg(feature = "bc_debug")]
    layout::boxed_couples::bc_debug_flush();

    // 11. Open output (file or stdout)
    let mut writer: Box<dyn std::io::Write> = if prefs.output.path.is_empty() {
        Box::new(std::io::stdout())
    } else {
        let path = std::path::Path::new(&prefs.output.path);
        if prefs.output.noclobber && path.exists() {
            anyhow::bail!("output file already exists: {}", prefs.output.path);
        }
        Box::new(std::fs::File::create(&prefs.output.path)?)
    };

    // 12. Render
    match prefs.output.output_type.to_lowercase().as_str() {
        "svg" => backend::svg::SvgRenderer.render(&layout_output, &prefs, &mut writer)?,
        "pdf" => backend::pdf::PdfRenderer.render(&layout_output, &prefs, &mut writer)?,
        _ => backend::text::TextRenderer.render(&layout_output, &prefs, &mut writer)?,
    }

    Ok(())
}
