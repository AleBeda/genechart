use crate::parser::genrep::Genrep;
use crate::preferences::Prefs;
use crate::scene::Scene;
use anyhow::Result;

pub mod boxed_couples;
pub mod boxes;
pub mod common;
pub mod fan;
pub mod fancy;
pub mod simple;

pub trait Layout {
    type Geo;
    fn compute(&self, genrep: &Genrep, prefs: &Prefs) -> Result<Genrep<Self::Geo>>;
}

pub enum LayoutOutput {
    Simple(Scene),
    BoxedCouples(Scene),
    Boxes(Scene),
    Fan(Scene),
    Fancy(Scene),
}

impl LayoutOutput {
    /// Consume the output and return the contained `Scene`.
    #[allow(dead_code)]
    pub fn into_scene(self) -> Scene {
        match self {
            LayoutOutput::Simple(s)
            | LayoutOutput::BoxedCouples(s)
            | LayoutOutput::Boxes(s)
            | LayoutOutput::Fan(s)
            | LayoutOutput::Fancy(s) => s,
        }
    }

    /// Borrow the contained `Scene`.
    pub fn scene(&self) -> &Scene {
        match self {
            LayoutOutput::Simple(s)
            | LayoutOutput::BoxedCouples(s)
            | LayoutOutput::Boxes(s)
            | LayoutOutput::Fan(s)
            | LayoutOutput::Fancy(s) => s,
        }
    }

    /// Returns `true` if this is a `Fan` layout output (which does not support text rendering).
    pub fn is_fan(&self) -> bool {
        matches!(self, LayoutOutput::Fan(_))
    }

    pub fn is_simple(&self) -> bool {
        matches!(self, LayoutOutput::Simple(_))
    }

    pub fn is_boxed_couples(&self) -> bool {
        matches!(self, LayoutOutput::BoxedCouples(_))
    }

    pub fn is_fancy(&self) -> bool {
        matches!(self, LayoutOutput::Fancy(_))
    }

    #[allow(dead_code)]
    pub fn is_boxes(&self) -> bool {
        matches!(self, LayoutOutput::Boxes(_))
    }
}

pub fn run_layout(genrep: &Genrep, prefs: &Prefs) -> Result<LayoutOutput> {
    match prefs.layout.layout_type.to_lowercase().as_str() {
        "simple" => {
            let result = simple::SimpleLayout.compute(genrep, prefs)?;
            Ok(LayoutOutput::Simple(simple::emit_scene(&result, prefs)))
        }
        "boxed_couples" => {
            let result = boxed_couples::BoxedCouplesLayout.compute(genrep, prefs)?;
            let scene = boxed_couples::emit_scene(&result, prefs);
            Ok(LayoutOutput::BoxedCouples(scene))
        }
        "boxes" => {
            let result = boxes::BoxesLayout.compute(genrep, prefs)?;
            Ok(LayoutOutput::Boxes(boxes::emit_scene(&result, prefs)))
        }
        "fan" => {
            let result = fan::FanLayout.compute(genrep, prefs)?;
            Ok(LayoutOutput::Fan(fan::emit_scene(&result, prefs)))
        }
        "fancy" => {
            let result = fancy::FancyLayout.compute(genrep, prefs)?;
            Ok(LayoutOutput::Fancy(fancy::emit_scene(&result, prefs)))
        }
        other => {
            eprintln!("warning: unknown layout type {other:?}, falling back to 'simple'");
            let result = simple::SimpleLayout.compute(genrep, prefs)?;
            Ok(LayoutOutput::Simple(simple::emit_scene(&result, prefs)))
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
    fn dispatch_boxes() {
        let mut prefs = Prefs::default();
        prefs.layout.layout_type = "boxes".to_string();
        prefs.scope.direction = "ancestors".to_string();
        let output = run_layout(&empty_genrep(), &prefs).unwrap();
        assert!(matches!(output, LayoutOutput::Boxes(_)));
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
    fn dispatch_fancy_ancestors() {
        let mut prefs = Prefs::default();
        prefs.layout.layout_type = "fancy".to_string();
        prefs.scope.direction = "ancestors".to_string();
        let output = run_layout(&empty_genrep(), &prefs).unwrap();
        assert!(matches!(output, LayoutOutput::Fancy(_)));
    }

    #[test]
    fn dispatch_unknown_falls_back_to_simple() {
        let mut prefs = Prefs::default();
        prefs.layout.layout_type = "unknown".to_string();
        let output = run_layout(&empty_genrep(), &prefs).unwrap();
        assert!(matches!(output, LayoutOutput::Simple(_)));
    }
}
