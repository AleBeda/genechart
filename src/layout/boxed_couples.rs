//! Recursive box-placement layout for couples (stub).

use anyhow::Result;
use crate::parser::genrep::Genrep;
use crate::preferences::Prefs;
use super::Layout;
use std::collections::HashMap;

pub struct BoxedCouplesGeo;

pub struct BoxedCouplesLayout;

impl Layout for BoxedCouplesLayout {
    type Geo = BoxedCouplesGeo;

    fn compute(&self, _genrep: &Genrep, _prefs: &Prefs) -> Result<Genrep<BoxedCouplesGeo>> {
        Ok(Genrep {
            individuals: HashMap::new(),
            families: HashMap::new(),
            first_individual_id: None,
        })
    }
}
