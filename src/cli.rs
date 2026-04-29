use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "genechart", version, about = "Genealogical chart generator from GEDCOM files")]
pub struct Args {
    /// Path to the .ged file (defaults to first *.ged in current directory)
    pub gedcom: Option<PathBuf>,

    /// GEDCOM individual ID to use as chart root (without @ delimiters)
    #[arg(long)]
    pub root: Option<String>,

    /// Number of generations to include
    #[arg(short = 'g', long)]
    pub generations: Option<u32>,

    /// TOML-style preference overrides, e.g. 'layout.type = "fan"' (repeatable)
    #[arg(long = "pref")]
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
    fn multiple_prefs() {
        let args =
            Args::try_parse_from(["genechart", "--pref", "a=1", "--pref", "b=2"]).unwrap();
        assert_eq!(args.prefs, vec!["a=1", "b=2"]);
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
