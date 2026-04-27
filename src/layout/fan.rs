//! Half-circle pedigree fan layout (stub).

use anyhow::Result;
use crate::parser::genrep::Genrep;
use crate::preferences::Prefs;
use super::Layout;
use std::collections::HashMap;

pub struct FanGeo;

pub struct FanLayout;

impl Layout for FanLayout {
    type Geo = FanGeo;

    fn compute(&self, _genrep: &Genrep, _prefs: &Prefs) -> Result<Genrep<FanGeo>> {
        Ok(Genrep {
            individuals: HashMap::new(),
            families: HashMap::new(),
            first_individual_id: None,
        })
    }
}
