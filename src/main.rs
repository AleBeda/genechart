mod backend;
mod cli;
mod layout;
mod parser;
mod preferences;
mod trace;
mod util;

use backend::Renderer as _;

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
            tracer.emit(
                "prefs",
                &format!(
                    "PREF OVERRIDE KEY-VALUE output.path = \"{}\"",
                    out_path.display()
                ),
            );
            prefs.output.path = out_path.display().to_string();
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
    if dump_mode {
        let serialized = toml::to_string_pretty(&prefs)
            .unwrap_or_else(|_| toml::to_string(&prefs).unwrap_or_default());
        print!("{serialized}");
        return Ok(());
    }

    // 8. Parse GEDCOM
    parser::set_diagnostics(prefs.diagnostics.clone());
    let mut genrep = parser::parse(&gedcom_path)?;

    // 9. Compute scope
    let root_id = (!prefs.scope.root.is_empty()).then(|| prefs.scope.root.as_str());
    let gens = (prefs.scope.generations > 0).then_some(prefs.scope.generations);
    parser::compute_scope(&mut genrep, root_id, &prefs.scope.direction, gens);

    // 10. Run layout
    let layout_output = layout::run_layout(&genrep, &prefs)?;

    // 11. Open output (file or stdout)
    let mut writer: Box<dyn std::io::Write> = if prefs.output.path.is_empty() {
        Box::new(std::io::stdout())
    } else {
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
