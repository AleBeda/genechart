//! Text-like layout: descendants, ancestors, forest (stub).

use anyhow::Result;
use crate::parser::genrep::Genrep;
use crate::preferences::Prefs;
use super::Layout;
use std::collections::HashMap;

pub struct SimpleGeo;

pub struct SimpleLayout;

impl Layout for SimpleLayout {
    type Geo = SimpleGeo;

    fn compute(&self, _genrep: &Genrep, _prefs: &Prefs) -> Result<Genrep<SimpleGeo>> {
        Ok(Genrep {
            individuals: HashMap::new(),
            families: HashMap::new(),
            first_individual_id: None,
        })
    }
}
