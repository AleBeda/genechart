mod cli;
mod preferences;
mod parser;
mod layout;
mod backend;

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

    // 2. Resolve GEDCOM path
    let gedcom_path = cli::resolve_gedcom_path(&args)?;

    // 3. Detect dump mode (bare --pref with no value)
    let dump_mode = args.prefs.iter().any(|s| s.is_empty());

    // Filter out empty strings (dump-mode sentinels) before passing to preferences::load
    let pref_overrides: Vec<String> = args.prefs.iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect();

    // 4. Load preferences (merging all sources + --preff file + --pref overrides)
    let mut prefs = preferences::load(Some(&gedcom_path), args.preff.as_deref(), &pref_overrides)?;

    // 5. Apply CLI shortcuts (override preference-file values)
    //    Order: dir → type → output path → root → generations → output type

    if let Some(dir) = &args.dir {
        prefs.scope.direction = dir.clone();
    }

    if let Some(lt) = &args.layout_type {
        prefs.layout.layout_type = lt.clone();
    }

    if let Some(out_path) = &args.output {
        prefs.output.path = out_path.display().to_string();
    }

    if let Some(root) = &args.root {
        prefs.scope.root = root.clone();
    }

    if let Some(gens) = args.generations {
        prefs.scope.generations = gens;
    }

    // Output type: explicit flags win, then infer from file extension
    if args.text {
        prefs.output.output_type = "text".to_string();
    } else if args.svg {
        prefs.output.output_type = "svg".to_string();
    } else if args.pdf {
        prefs.output.output_type = "pdf".to_string();
    } else if let Some(out_path) = &args.output {
        // No explicit type flag — infer from extension
        let ext = out_path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());
        match ext.as_deref() {
            Some("txt")  => prefs.output.output_type = "text".to_string(),
            Some("svg")  => prefs.output.output_type = "svg".to_string(),
            Some("pdf")  => prefs.output.output_type = "pdf".to_string(),
            _            => {} // leave as-is
        }
    }

    // Store the resolved GEDCOM path for use in title/copyright templates
    prefs.files.gedcom = gedcom_path.display().to_string();

    // 6. Dump mode: print merged prefs and exit
    if dump_mode {
        let serialized = toml::to_string_pretty(&prefs)
            .unwrap_or_else(|_| toml::to_string(&prefs).unwrap_or_default());
        print!("{serialized}");
        return Ok(());
    }

    // 7. Parse GEDCOM
    parser::set_diagnostics(prefs.diagnostics.clone());
    let mut genrep = parser::parse(&gedcom_path)?;

    // 8. Compute scope
    let root_id = (!prefs.scope.root.is_empty()).then(|| prefs.scope.root.as_str());
    let gens = (prefs.scope.generations > 0).then_some(prefs.scope.generations);
    parser::compute_scope(&mut genrep, root_id, &prefs.scope.direction, gens);

    // 9. Run layout
    let layout_output = layout::run_layout(&genrep, &prefs)?;

    // 10. Open output (file or stdout)
    let mut writer: Box<dyn std::io::Write> = if prefs.output.path.is_empty() {
        Box::new(std::io::stdout())
    } else {
        Box::new(std::fs::File::create(&prefs.output.path)?)
    };

    // 11. Render
    match prefs.output.output_type.to_lowercase().as_str() {
        "svg" => backend::svg::SvgRenderer.render(&layout_output, &prefs, &mut writer)?,
        "pdf" => backend::pdf::PdfRenderer.render(&layout_output, &prefs, &mut writer)?,
        _ => backend::text::TextRenderer.render(&layout_output, &prefs, &mut writer)?,
    }

    Ok(())
}
