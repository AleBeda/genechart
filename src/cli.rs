use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "genechart", version, about = "Genealogical chart generator from GEDCOM files")]
pub struct Args {
    /// Path to the .ged file (defaults to first *.ged in current directory)
    pub gedcom: Option<PathBuf>,

    /// GEDCOM individual ID to use as chart root (without @ delimiters)
    #[arg(short = 'r', long)]
    pub root: Option<String>,

    /// Number of generations to include
    #[arg(short = 'g', long, alias = "gen")]
    pub generations: Option<u32>,

    /// Set scope.direction (e.g. descendants, ancestors)
    #[arg(long, value_name = "DIRECTION")]
    pub dir: Option<String>,

    /// Set layout algorithm (simple, fan, boxed_couples)
    #[arg(long = "type", value_name = "TYPE")]
    pub layout_type: Option<String>,

    /// Output as plain text
    #[arg(long)]
    pub text: bool,

    /// Output as SVG
    #[arg(long)]
    pub svg: bool,

    /// Output as PDF
    #[arg(long)]
    pub pdf: bool,

    /// Output file path
    #[arg(short = 'o', long, value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// TOML-style preference overrides, e.g. 'layout.type = "fan"' (repeatable).
    /// Bare --pref (no value) dumps the merged preferences and exits.
    #[arg(
        long = "pref",
        value_name = "TOML-ASSIGNMENT",
        num_args = 0..=1,
        default_missing_value = "",
    )]
    pub prefs: Vec<String>,

    /// TOML preferences file to load after the gedcom-basename file and before --pref overrides
    #[arg(long = "preff", value_name = "FILE")]
    pub preff: Option<PathBuf>,

    #[arg(long, hide = true)]
    pub strict: bool,
}

pub fn parse() -> Args {
    let args = Args::parse();
    if args.strict {
        todo!("--strict is not yet implemented");
    }
    args
}

pub fn resolve_gedcom_path(args: &Args) -> anyhow::Result<PathBuf> {
    if let Some(path) = &args.gedcom {
        return Ok(path.clone());
    }

    let mut entries: Vec<PathBuf> = std::fs::read_dir(".")?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ged"))
        .collect();

    entries.sort();

    entries
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No .ged file found in the current directory"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_args() {
        let args = Args::try_parse_from(["genechart"]).unwrap();
        assert!(args.gedcom.is_none());
        assert!(args.root.is_none());
        assert!(args.generations.is_none());
        assert!(args.prefs.is_empty());
    }

    #[test]
    fn root_and_generations() {
        let args = Args::try_parse_from(["genechart", "--root", "I42", "-g", "3"]).unwrap();
        assert_eq!(args.root.as_deref(), Some("I42"));
        assert_eq!(args.generations, Some(3));
    }

    #[test]
    fn root_short_flag() {
        let args = Args::try_parse_from(["genechart", "-r", "I10"]).unwrap();
        assert_eq!(args.root.as_deref(), Some("I10"));
    }

    #[test]
    fn gen_alias() {
        let args = Args::try_parse_from(["genechart", "--gen", "5"]).unwrap();
        assert_eq!(args.generations, Some(5));
    }

    #[test]
    fn dir_flag() {
        let args = Args::try_parse_from(["genechart", "--dir", "ancestors"]).unwrap();
        assert_eq!(args.dir.as_deref(), Some("ancestors"));
    }

    #[test]
    fn type_flag() {
        let args = Args::try_parse_from(["genechart", "--type", "fan"]).unwrap();
        assert_eq!(args.layout_type.as_deref(), Some("fan"));
    }

    #[test]
    fn output_type_flags() {
        let args = Args::try_parse_from(["genechart", "--svg"]).unwrap();
        assert!(args.svg);
        assert!(!args.pdf);
        assert!(!args.text);

        let args = Args::try_parse_from(["genechart", "--pdf"]).unwrap();
        assert!(args.pdf);

        let args = Args::try_parse_from(["genechart", "--text"]).unwrap();
        assert!(args.text);
    }

    #[test]
    fn output_path_short() {
        let args = Args::try_parse_from(["genechart", "-o", "/tmp/chart.svg"]).unwrap();
        assert_eq!(args.output.as_deref(), Some(std::path::Path::new("/tmp/chart.svg")));
    }

    #[test]
    fn output_path_long() {
        let args = Args::try_parse_from(["genechart", "--output", "/tmp/chart.pdf"]).unwrap();
        assert_eq!(args.output.as_deref(), Some(std::path::Path::new("/tmp/chart.pdf")));
    }

    #[test]
    fn multiple_prefs() {
        let args =
            Args::try_parse_from(["genechart", "--pref", "a=1", "--pref", "b=2"]).unwrap();
        assert_eq!(args.prefs, vec!["a=1", "b=2"]);
    }

    #[test]
    fn bare_pref_dump_mode() {
        let args = Args::try_parse_from(["genechart", "--pref"]).unwrap();
        assert_eq!(args.prefs, vec![""]);
        assert!(args.prefs.iter().any(|s| s.is_empty()),
            "bare --pref should set dump mode");
    }

    #[test]
    fn preff_arg() {
        let args = Args::try_parse_from(["genechart", "--preff", "/tmp/my.toml"]).unwrap();
        assert_eq!(args.preff.as_deref(), Some(std::path::Path::new("/tmp/my.toml")));
    }

    #[test]
    fn help_does_not_panic() {
        let result = Args::try_parse_from(["genechart", "--help"]);
        assert!(result.is_err());
    }

    #[test]
    fn version_does_not_panic() {
        let result = Args::try_parse_from(["genechart", "--version"]);
        assert!(result.is_err());
    }
}
