use anyhow::Result;
use crate::parser::genrep::Genrep;
use crate::preferences::Prefs;

pub mod simple;
pub mod boxed_couples;
pub mod fan;

pub trait Layout {
    type Geo;
    fn compute(&self, genrep: &Genrep, prefs: &Prefs) -> Result<Genrep<Self::Geo>>;
}

pub enum LayoutOutput {
    Simple(Genrep<simple::SimpleGeo>),
    BoxedCouples(Genrep<boxed_couples::BoxedCouplesGeo>),
    Fan(Genrep<fan::FanGeo>),
}

pub fn run_layout(genrep: &Genrep, prefs: &Prefs) -> Result<LayoutOutput> {
    match prefs.layout.layout_type.to_lowercase().as_str() {
        "simple" => {
            let result = simple::SimpleLayout.compute(genrep, prefs)?;
            Ok(LayoutOutput::Simple(result))
        }
        "boxed_couples" => {
            let result = boxed_couples::BoxedCouplesLayout.compute(genrep, prefs)?;
            Ok(LayoutOutput::BoxedCouples(result))
        }
        "fan" => {
            let result = fan::FanLayout.compute(genrep, prefs)?;
            Ok(LayoutOutput::Fan(result))
        }
        other => {
            eprintln!("warning: unknown layout type {other:?}, falling back to 'simple'");
            let result = simple::SimpleLayout.compute(genrep, prefs)?;
            Ok(LayoutOutput::Simple(result))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn empty_genrep() -> Genrep {
        Genrep {
            individuals: HashMap::new(),
            families: HashMap::new(),
            first_individual_id: None,
        }
    }

    #[test]
    fn dispatch_simple() {
        let mut prefs = Prefs::default();
        prefs.layout.layout_type = "simple".to_string();
        let output = run_layout(&empty_genrep(), &prefs).unwrap();
        assert!(matches!(output, LayoutOutput::Simple(_)));
    }

    #[test]
    fn dispatch_boxed_couples() {
        let mut prefs = Prefs::default();
        prefs.layout.layout_type = "boxed_couples".to_string();
        let output = run_layout(&empty_genrep(), &prefs).unwrap();
        assert!(matches!(output, LayoutOutput::BoxedCouples(_)));
    }

    #[test]
    fn dispatch_fan() {
        let mut prefs = Prefs::default();
        prefs.layout.layout_type = "fan".to_string();
        prefs.scope.direction = "ancestors".to_string();
        let output = run_layout(&empty_genrep(), &prefs).unwrap();
        assert!(matches!(output, LayoutOutput::Fan(_)));
    }

    #[test]
    fn dispatch_unknown_falls_back_to_simple() {
        let mut prefs = Prefs::default();
        prefs.layout.layout_type = "unknown".to_string();
        let output = run_layout(&empty_genrep(), &prefs).unwrap();
        assert!(matches!(output, LayoutOutput::Simple(_)));
    }
}
