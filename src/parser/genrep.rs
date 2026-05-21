use std::collections::HashMap;

#[derive(Clone)]
pub struct GedDate {
    pub raw: String,
}

#[derive(Clone)]
pub struct Event {
    pub date: Option<GedDate>,
    pub place: Option<String>,
}

// NOTE: #[derive(Clone)] above Individual<G> and Family<G> does not eliminate
// the manual field-copying in layout/simple.rs and layout/fan.rs. Rust cannot
// convert Individual<()> to Individual<FanGeo> via clone() + geo assignment
// because the generic parameter G is part of the type. A map_geo() method
// would require the same number of field assignments, offering no savings.
// Clone is retained for future use (e.g. layout algorithms that copy within
// the same G type).
#[derive(Clone)]
pub struct Individual<G = ()> {
    pub id: String,
    pub given: Option<String>,
    pub surname: Option<String>,
    pub sex: Option<char>,
    pub birth: Option<Event>,
    pub death: Option<Event>,
    pub fams: Vec<String>,
    pub famc: Vec<String>,
    pub alt_name: Option<String>, // NAM2: alternate name
    pub name_heb: Option<String>, // NAMH: Hebrew/transliterated name
    pub living: Option<bool>,     // _LIVING: living flag
    pub notes: Vec<String>,
    pub in_scope: bool,
    pub geo: Option<G>,
}

#[derive(Clone)]
pub struct Family<G = ()> {
    pub id: String,
    pub husband_id: Option<String>,
    pub wife_id: Option<String>,
    pub children_ids: Vec<String>,
    pub marriage: Option<Event>,
    pub jmar: Option<String>, // JMAR: Jewish marriage record reference
    pub notes: Vec<String>,
    pub in_scope: bool,
    pub geo: Option<G>,
}

pub struct Genrep<G = ()> {
    pub individuals: HashMap<String, Individual<G>>,
    pub families: HashMap<String, Family<G>>,
    /// ID of the first individual encountered during parsing.
    pub first_individual_id: Option<String>,
}

impl<G> Genrep<G> {
    pub fn get_individual(&self, id: &str) -> Option<&Individual<G>> {
        self.individuals.get(id)
    }

    pub fn get_family(&self, id: &str) -> Option<&Family<G>> {
        self.families.get(id)
    }
}
