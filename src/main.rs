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

    // 3. Load preferences (merging all sources + --pref overrides)
    let mut prefs = preferences::load(Some(&gedcom_path), &args.prefs)?;

    // 4. Apply CLI shortcuts (override preference-file values)
    if let Some(root) = &args.root {
        prefs.scope.root = root.clone();
    }
    if let Some(gens) = args.generations {
        prefs.scope.generations = gens;
    }

    // Store the resolved GEDCOM path for use in title/copyright templates
    prefs.files.gedcom = gedcom_path.display().to_string();

    // 5. Parse GEDCOM
    let mut genrep = parser::parse(&gedcom_path)?;

    // 6. Compute scope
    let root_id = (!prefs.scope.root.is_empty()).then(|| prefs.scope.root.as_str());
    let gens = (prefs.scope.generations > 0).then_some(prefs.scope.generations);
    parser::compute_scope(&mut genrep, root_id, &prefs.scope.direction, gens);

    // 7. Run layout
    let layout_output = layout::run_layout(&genrep, &prefs)?;

    // 8. Open output (file or stdout)
    let mut writer: Box<dyn std::io::Write> = if prefs.output.path.is_empty() {
        Box::new(std::io::stdout())
    } else {
        Box::new(std::fs::File::create(&prefs.output.path)?)
    };

    // 9. Render
    match prefs.output.output_type.to_lowercase().as_str() {
        "svg" => backend::svg::SvgRenderer.render(&layout_output, &prefs, &mut writer)?,
        "pdf" => backend::pdf::PdfRenderer.render(&layout_output, &prefs, &mut writer)?,
        _ => backend::text::TextRenderer.render(&layout_output, &prefs, &mut writer)?,
    }

    Ok(())
}
