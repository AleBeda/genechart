pub mod genrep;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::OnceLock;

use crate::preferences::DiagnosticsPrefs;
use crate::util::matches_direction;
use genrep::{Event, Family, GedDate, Genrep, Individual};

static DIAG: OnceLock<DiagnosticsPrefs> = OnceLock::new();

/// Call once from `main` after loading preferences, before parsing.
pub fn set_diagnostics(diag: DiagnosticsPrefs) {
    let _ = DIAG.set(diag);
}

macro_rules! diag_warn {
    ($($arg:tt)*) => {
        if DIAG.get().map_or(false, |d| d.warnings) {
            eprintln!("Warning: {}", format_args!($($arg)*));
        }
    }
}

// ── internal parser state ────────────────────────────────────────────────────

enum RecordCtx {
    None,
    Indi {
        indi: Individual<()>,
        raw_name: Option<String>,
    },
    Fam(Family<()>),
    Other,
}

#[derive(Clone, Copy, PartialEq)]
enum EventCtx {
    None,
    Birth,
    Death,
    Marriage,
}

#[derive(Clone, Copy)]
enum TextSlot {
    None,
    RawName,
    BirthDate,
    BirthPlace,
    DeathDate,
    DeathPlace,
    MarrDate,
    MarrPlace,
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn strip_at(s: &str) -> String {
    s.trim_matches('@').to_string()
}

/// Parse "Given /Surname/" into (given, surname).
fn parse_name(raw: &str) -> (Option<String>, Option<String>) {
    let given = raw
        .split('/')
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let surname = raw
        .split('/')
        .nth(1)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    (given, surname)
}

/// Split one GEDCOM line into (level, xref_id, tag, value).
fn parse_line(line: &str) -> Option<(u32, Option<String>, String, String)> {
    let mut iter = line.splitn(2, ' ');
    let level: u32 = iter.next()?.trim().parse().ok()?;
    let rest = iter.next().unwrap_or("").trim_start();

    let (xref, rest) = if rest.starts_with('@') {
        if let Some(pos) = rest[1..].find('@') {
            let xref = rest[..pos + 2].to_string();
            let rest = rest[pos + 2..].trim_start();
            (Some(xref), rest)
        } else {
            (Option::<String>::None, rest)
        }
    } else {
        (Option::<String>::None, rest)
    };

    let mut iter = rest.splitn(2, ' ');
    let tag = iter.next()?.to_string();
    let value = iter.next().unwrap_or("").to_string();

    Some((level, xref, tag, value))
}

/// Commit the current record-in-progress to the output maps.
fn commit_record(
    ctx: &mut RecordCtx,
    individuals: &mut HashMap<String, Individual<()>>,
    families: &mut HashMap<String, Family<()>>,
) {
    let old = std::mem::replace(ctx, RecordCtx::None);
    match old {
        RecordCtx::Indi { mut indi, raw_name } => {
            if let Some(name) = raw_name {
                let (given, surname) = parse_name(&name);
                indi.given = given;
                indi.surname = surname;
            }
            individuals.insert(indi.id.clone(), indi);
        }
        RecordCtx::Fam(fam) => {
            families.insert(fam.id.clone(), fam);
        }
        _ => {}
    }
}

/// Append a CONC/CONT continuation to the last string field that was set.
fn apply_continuation(ctx: &mut RecordCtx, slot: TextSlot, prefix: &str, value: &str) {
    match slot {
        TextSlot::None => {}
        TextSlot::RawName => {
            if let RecordCtx::Indi { raw_name, .. } = ctx {
                if let Some(s) = raw_name {
                    s.push_str(prefix);
                    s.push_str(value);
                }
            }
        }
        TextSlot::BirthDate => {
            if let RecordCtx::Indi { indi, .. } = ctx {
                if let Some(e) = &mut indi.birth {
                    if let Some(d) = &mut e.date {
                        d.raw.push_str(prefix);
                        d.raw.push_str(value);
                    }
                }
            }
        }
        TextSlot::BirthPlace => {
            if let RecordCtx::Indi { indi, .. } = ctx {
                if let Some(e) = &mut indi.birth {
                    if let Some(p) = &mut e.place {
                        p.push_str(prefix);
                        p.push_str(value);
                    }
                }
            }
        }
        TextSlot::DeathDate => {
            if let RecordCtx::Indi { indi, .. } = ctx {
                if let Some(e) = &mut indi.death {
                    if let Some(d) = &mut e.date {
                        d.raw.push_str(prefix);
                        d.raw.push_str(value);
                    }
                }
            }
        }
        TextSlot::DeathPlace => {
            if let RecordCtx::Indi { indi, .. } = ctx {
                if let Some(e) = &mut indi.death {
                    if let Some(p) = &mut e.place {
                        p.push_str(prefix);
                        p.push_str(value);
                    }
                }
            }
        }
        TextSlot::MarrDate => {
            if let RecordCtx::Fam(fam) = ctx {
                if let Some(e) = &mut fam.marriage {
                    if let Some(d) = &mut e.date {
                        d.raw.push_str(prefix);
                        d.raw.push_str(value);
                    }
                }
            }
        }
        TextSlot::MarrPlace => {
            if let RecordCtx::Fam(fam) = ctx {
                if let Some(e) = &mut fam.marriage {
                    if let Some(p) = &mut e.place {
                        p.push_str(prefix);
                        p.push_str(value);
                    }
                }
            }
        }
    }
}

// ── public API ───────────────────────────────────────────────────────────────

/// Read a UTF-8 GEDCOM 5.5.1 file and return the internal representation.
pub fn parse(path: &Path) -> anyhow::Result<Genrep> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {}", path.display(), e))?;
    parse_str(&content)
}

pub(crate) fn parse_str(content: &str) -> anyhow::Result<Genrep> {
    let mut individuals: HashMap<String, Individual<()>> = HashMap::new();
    let mut families: HashMap<String, Family<()>> = HashMap::new();
    let mut first_indi_id: Option<String> = Option::None;

    let mut ctx = RecordCtx::None;
    let mut event_ctx = EventCtx::None;
    let mut text_slot = TextSlot::None;
    let mut warned_tags: HashSet<String> = HashSet::new();

    for (lineno, line) in content.lines().enumerate() {
        let n = lineno + 1;
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }

        let (level, xref, tag, value) = match parse_line(line) {
            Some(t) => t,
            Option::None => {
                diag_warn!("cannot parse line {n}: {line:?}");
                continue;
            }
        };

        if level == 0 {
            commit_record(&mut ctx, &mut individuals, &mut families);
            event_ctx = EventCtx::None;
            text_slot = TextSlot::None;

            match tag.as_str() {
                "INDI" => {
                    let id = strip_at(xref.as_deref().unwrap_or(""));
                    if first_indi_id.is_none() {
                        first_indi_id = Some(id.clone());
                    }
                    ctx = RecordCtx::Indi {
                        indi: Individual {
                            id,
                            given: Option::None,
                            surname: Option::None,
                            sex: Option::None,
                            birth: Option::None,
                            death: Option::None,
                            fams: Vec::new(),
                            famc: Vec::new(),
                            alt_name: Option::None,
                            name_heb: Option::None,
                            living: Option::None,
                            in_scope: false,
                            geo: Option::None,
                        },
                        raw_name: Option::None,
                    };
                }
                "FAM" => {
                    let id = strip_at(xref.as_deref().unwrap_or(""));
                    ctx = RecordCtx::Fam(Family {
                        id,
                        husband_id: Option::None,
                        wife_id: Option::None,
                        children_ids: Vec::new(),
                        marriage: Option::None,
                        jmar: Option::None,
                        in_scope: false,
                        geo: Option::None,
                    });
                }
                _ => {
                    ctx = RecordCtx::Other;
                }
            }
        } else {
            // Handle CONC/CONT before any level-specific reset.
            if tag == "CONC" || tag == "CONT" {
                let prefix = if tag == "CONT" { "\n" } else { "" };
                apply_continuation(&mut ctx, text_slot, prefix, &value);
                continue;
            }

            if level == 1 {
                event_ctx = EventCtx::None;
                text_slot = TextSlot::None;
            }

            match level {
                1 => {
                    // Silently skip sub-tags of HEAD / TRLR / unknown level-0 records.
                    if matches!(ctx, RecordCtx::Other | RecordCtx::None) {
                        continue;
                    }
                    match tag.as_str() {
                        "NAME" => {
                            if let RecordCtx::Indi { raw_name, .. } = &mut ctx {
                                *raw_name = Some(value.clone());
                                text_slot = TextSlot::RawName;
                            }
                        }
                        "SEX" => {
                            if let RecordCtx::Indi { indi, .. } = &mut ctx {
                                indi.sex = value.chars().next();
                            }
                        }
                        "BIRT" => {
                            if let RecordCtx::Indi { indi, .. } = &mut ctx {
                                indi.birth = Some(Event {
                                    date: Option::None,
                                    place: Option::None,
                                });
                                event_ctx = EventCtx::Birth;
                            }
                        }
                        "DEAT" => {
                            if let RecordCtx::Indi { indi, .. } = &mut ctx {
                                indi.death = Some(Event {
                                    date: Option::None,
                                    place: Option::None,
                                });
                                event_ctx = EventCtx::Death;
                            }
                        }
                        "FAMS" => {
                            if let RecordCtx::Indi { indi, .. } = &mut ctx {
                                indi.fams.push(strip_at(&value));
                            }
                        }
                        "FAMC" => {
                            if let RecordCtx::Indi { indi, .. } = &mut ctx {
                                indi.famc.push(strip_at(&value));
                            }
                        }
                        "HUSB" => {
                            if let RecordCtx::Fam(fam) = &mut ctx {
                                fam.husband_id = Some(strip_at(&value));
                            }
                        }
                        "WIFE" => {
                            if let RecordCtx::Fam(fam) = &mut ctx {
                                fam.wife_id = Some(strip_at(&value));
                            }
                        }
                        "CHIL" => {
                            if let RecordCtx::Fam(fam) = &mut ctx {
                                fam.children_ids.push(strip_at(&value));
                            }
                        }
                        "MARR" => {
                            if let RecordCtx::Fam(fam) = &mut ctx {
                                fam.marriage = Some(Event {
                                    date: Option::None,
                                    place: Option::None,
                                });
                                event_ctx = EventCtx::Marriage;
                            }
                        }
                        "NOTE" | "CHAN" | "TEXT" => {} // silently skip
                        "NAM2" => {
                            if let RecordCtx::Indi { indi, .. } = &mut ctx {
                                indi.alt_name = Some(value.clone());
                            }
                        }
                        "NAMH" => {
                            if let RecordCtx::Indi { indi, .. } = &mut ctx {
                                indi.name_heb = Some(value.clone());
                            }
                        }
                        "JMAR" => {
                            if let RecordCtx::Fam(fam) = &mut ctx {
                                fam.jmar = Some(value.clone());
                            }
                        }
                        "_LIVING" => {
                            if let RecordCtx::Indi { indi, .. } = &mut ctx {
                                indi.living = match value.trim() {
                                    "Y" => Some(true),
                                    "N" => Some(false),
                                    _ => Option::None,
                                };
                            }
                        }
                        _ => {
                            if warned_tags.insert(tag.clone()) {
                                diag_warn!("unknown tag {tag} at line {n}");
                            }
                        }
                    }
                }
                2 => {
                    // Silently skip level-2 tags outside INDI/FAM records.
                    if matches!(ctx, RecordCtx::Other | RecordCtx::None) {
                        continue;
                    }
                    match tag.as_str() {
                        "DATE" => match event_ctx {
                            EventCtx::Birth => {
                                if let RecordCtx::Indi { indi, .. } = &mut ctx {
                                    if let Some(e) = &mut indi.birth {
                                        e.date = Some(GedDate { raw: value.clone() });
                                        text_slot = TextSlot::BirthDate;
                                    }
                                }
                            }
                            EventCtx::Death => {
                                if let RecordCtx::Indi { indi, .. } = &mut ctx {
                                    if let Some(e) = &mut indi.death {
                                        e.date = Some(GedDate { raw: value.clone() });
                                        text_slot = TextSlot::DeathDate;
                                    }
                                }
                            }
                            EventCtx::Marriage => {
                                if let RecordCtx::Fam(fam) = &mut ctx {
                                    if let Some(e) = &mut fam.marriage {
                                        e.date = Some(GedDate { raw: value.clone() });
                                        text_slot = TextSlot::MarrDate;
                                    }
                                }
                            }
                            EventCtx::None => {}
                        },
                        "PLAC" => match event_ctx {
                            EventCtx::Birth => {
                                if let RecordCtx::Indi { indi, .. } = &mut ctx {
                                    if let Some(e) = &mut indi.birth {
                                        e.place = Some(value.clone());
                                        text_slot = TextSlot::BirthPlace;
                                    }
                                }
                            }
                            EventCtx::Death => {
                                if let RecordCtx::Indi { indi, .. } = &mut ctx {
                                    if let Some(e) = &mut indi.death {
                                        e.place = Some(value.clone());
                                        text_slot = TextSlot::DeathPlace;
                                    }
                                }
                            }
                            EventCtx::Marriage => {
                                if let RecordCtx::Fam(fam) = &mut ctx {
                                    if let Some(e) = &mut fam.marriage {
                                        e.place = Some(value.clone());
                                        text_slot = TextSlot::MarrPlace;
                                    }
                                }
                            }
                            EventCtx::None => {}
                        },
                        _ => {
                            text_slot = TextSlot::None;
                        } // reset on unknown sub-fields so CONT can't bleed
                    }
                }
                _ => {} // level 3+: silently skip
            }
        }
    }

    commit_record(&mut ctx, &mut individuals, &mut families);

    // Repair incomplete FAMS cross-references: if a FAM record names a HUSB or WIFE
    // that does not already list that family in their fams, add it. This makes parsing
    // robust to GEDCOM files where the INDI/FAMS back-pointer was omitted.
    for (fam_id, fam) in &families {
        for individual_id in [fam.husband_id.as_deref(), fam.wife_id.as_deref()]
            .into_iter()
            .flatten()
        {
            if let Some(indi) = individuals.get_mut(individual_id) {
                if !indi.fams.contains(fam_id) {
                    indi.fams.push(fam_id.clone());
                }
            }
        }
    }

    Ok(Genrep {
        individuals,
        families,
        first_individual_id: first_indi_id,
    })
}

// ── scope computation ────────────────────────────────────────────────────────

/// Set `in_scope = true` on individuals and families based on direction and depth.
///
/// Equivalent to `compute_scope_opts(genrep, root_id, direction, generations, false)`.
#[allow(dead_code)]
pub fn compute_scope(
    genrep: &mut Genrep,
    root_id: Option<&str>,
    direction: &str,
    generations: Option<u32>,
) {
    compute_scope_opts(genrep, root_id, direction, generations, false);
}

/// Like [`compute_scope`] but with an explicit `last_gen_spouses` flag.
///
/// When `last_gen_spouses` is `true` and a generation limit is set, spouses of
/// individuals in the deepest visible generation are included in scope even
/// though their children are not.
pub fn compute_scope_opts(
    genrep: &mut Genrep,
    root_id: Option<&str>,
    direction: &str,
    generations: Option<u32>,
    last_gen_spouses: bool,
) {
    let dir = direction.trim().to_ascii_lowercase();
    if matches_direction(&dir, "forest") {
        for indi in genrep.individuals.values_mut() {
            indi.in_scope = true;
        }
        for fam in genrep.families.values_mut() {
            fam.in_scope = true;
        }
        return;
    }

    let root = match resolve_root(genrep, root_id) {
        Some(r) => r,
        Option::None => return,
    };

    if matches_direction(&dir, "descendants") {
        scope_descendants(genrep, &root, generations, last_gen_spouses);
    } else if matches_direction(&dir, "ancestors") || matches_direction(&dir, "pedigree") {
        scope_ancestors(genrep, &root, generations);
    } else {
        diag_warn!("unknown direction '{direction}', defaulting to forest");
        for indi in genrep.individuals.values_mut() {
            indi.in_scope = true;
        }
        for fam in genrep.families.values_mut() {
            fam.in_scope = true;
        }
    }
}

fn resolve_root(genrep: &Genrep, root_id: Option<&str>) -> Option<String> {
    match root_id {
        Some(id) => {
            if genrep.individuals.contains_key(id) {
                Some(id.to_string())
            } else {
                diag_warn!("root individual '{id}' not found");
                Option::None
            }
        }
        Option::None => match &genrep.first_individual_id {
            Some(id) => Some(id.clone()),
            Option::None => {
                diag_warn!("no individuals in genrep");
                Option::None
            }
        },
    }
}

fn scope_descendants(
    genrep: &mut Genrep,
    root: &str,
    generations: Option<u32>,
    last_gen_spouses: bool,
) {
    let mut indi_scope: HashSet<String> = HashSet::new();
    let mut fam_scope: HashSet<String> = HashSet::new();

    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    queue.push_back((root.to_string(), 0));

    while let Some((id, depth)) = queue.pop_front() {
        if indi_scope.contains(&id) {
            continue;
        }
        indi_scope.insert(id.clone());

        // Whether this individual is in the last visible generation.
        let at_last_gen = generations.is_some_and(|g| depth >= g.saturating_sub(1));
        // Add children only when not at the last generation.
        let add_children = !at_last_gen;
        // Add spouses when not at the last generation, or when the caller opts
        // in to showing last-generation spouses.
        let add_spouses = !at_last_gen || last_gen_spouses;

        if add_children || add_spouses {
            let fams: Vec<String> = genrep
                .individuals
                .get(&id)
                .map(|i| i.fams.clone())
                .unwrap_or_default();

            for fam_id in fams {
                let (spouse, children) = match genrep.families.get(&fam_id) {
                    Some(fam) => {
                        let spouse = if fam.husband_id.as_deref() == Some(id.as_str()) {
                            fam.wife_id.clone()
                        } else {
                            fam.husband_id.clone()
                        };
                        (spouse, fam.children_ids.clone())
                    }
                    Option::None => continue,
                };

                fam_scope.insert(fam_id);
                if add_spouses {
                    if let Some(sp) = spouse {
                        indi_scope.insert(sp);
                    }
                }
                if add_children {
                    for child_id in children {
                        if !indi_scope.contains(&child_id) {
                            queue.push_back((child_id, depth + 1));
                        }
                    }
                }
            }
        }
    }

    apply_scope(genrep, &indi_scope, &fam_scope);
}

fn scope_ancestors(genrep: &mut Genrep, root: &str, generations: Option<u32>) {
    let mut indi_scope: HashSet<String> = HashSet::new();
    let mut fam_scope: HashSet<String> = HashSet::new();

    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    queue.push_back((root.to_string(), 0));

    while let Some((id, depth)) = queue.pop_front() {
        if indi_scope.contains(&id) {
            continue;
        }
        indi_scope.insert(id.clone());

        if generations.is_none_or(|g| depth < g.saturating_sub(1)) {
            let famcs: Vec<String> = genrep
                .individuals
                .get(&id)
                .map(|i| i.famc.clone())
                .unwrap_or_default();

            for fam_id in famcs {
                let (husb, wife) = match genrep.families.get(&fam_id) {
                    Some(fam) => (fam.husband_id.clone(), fam.wife_id.clone()),
                    Option::None => continue,
                };

                fam_scope.insert(fam_id);
                for parent_id in [husb, wife].into_iter().flatten() {
                    if !indi_scope.contains(&parent_id) {
                        queue.push_back((parent_id, depth + 1));
                    }
                }
            }
        }
    }

    apply_scope(genrep, &indi_scope, &fam_scope);
}

fn apply_scope(genrep: &mut Genrep, indi_scope: &HashSet<String>, fam_scope: &HashSet<String>) {
    for id in indi_scope {
        if let Some(indi) = genrep.individuals.get_mut(id) {
            indi.in_scope = true;
        }
    }
    for id in fam_scope {
        if let Some(fam) = genrep.families.get_mut(id) {
            fam.in_scope = true;
        }
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME John /Ancestor/
1 SEX M
1 BIRT
2 DATE 1 JAN 1812
2 PLAC London
1 FAMS @F1@
0 @I2@ INDI
1 NAME Jane /Ancestress/
1 SEX F
1 FAMS @F1@
0 @I3@ INDI
1 NAME Paul /Ancestor/
1 SEX M
1 FAMC @F1@
0 @F1@ FAM
1 HUSB @I1@
1 WIFE @I2@
1 CHIL @I3@
1 MARR
2 DATE 4 APR 1843
2 PLAC London
0 TRLR
";

    #[test]
    fn test_parse_individual() {
        let gr = parse_str(SAMPLE).unwrap();
        let i1 = gr.get_individual("I1").expect("I1 missing");
        assert_eq!(i1.given.as_deref(), Some("John"));
        assert_eq!(i1.surname.as_deref(), Some("Ancestor"));
        assert_eq!(i1.sex, Some('M'));
        let birth = i1.birth.as_ref().expect("birth missing");
        assert_eq!(
            birth.date.as_ref().map(|d| d.raw.as_str()),
            Some("1 JAN 1812")
        );
        assert_eq!(birth.place.as_deref(), Some("London"));
    }

    #[test]
    fn test_family_links() {
        let gr = parse_str(SAMPLE).unwrap();
        let f1 = gr.get_family("F1").expect("F1 missing");
        assert_eq!(f1.husband_id.as_deref(), Some("I1"));
        assert_eq!(f1.wife_id.as_deref(), Some("I2"));
        assert_eq!(f1.children_ids, vec!["I3"]);
        let marr = f1.marriage.as_ref().expect("marriage missing");
        assert_eq!(
            marr.date.as_ref().map(|d| d.raw.as_str()),
            Some("4 APR 1843")
        );
        assert_eq!(marr.place.as_deref(), Some("London"));
    }

    #[test]
    fn test_scope_descendants() {
        let mut gr = parse_str(SAMPLE).unwrap();
        compute_scope(&mut gr, Some("I1"), "descendants", Some(2));
        assert!(gr.get_individual("I1").unwrap().in_scope);
        assert!(gr.get_individual("I2").unwrap().in_scope);
        assert!(gr.get_individual("I3").unwrap().in_scope);
        assert!(gr.get_family("F1").unwrap().in_scope);
    }

    const SAMPLE_4GEN: &str = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME Root /Person/
1 SEX M
1 FAMS @F1@
0 @I2@ INDI
1 NAME Root /Spouse/
1 SEX F
1 FAMS @F1@
0 @I3@ INDI
1 NAME Child /Person/
1 SEX M
1 FAMC @F1@
1 FAMS @F2@
0 @I4@ INDI
1 NAME Grandchild /Person/
1 SEX M
1 FAMC @F2@
1 FAMS @F3@
0 @I5@ INDI
1 NAME GreatGrandchild /Person/
1 SEX M
1 FAMC @F3@
0 @F1@ FAM
1 HUSB @I1@
1 WIFE @I2@
1 CHIL @I3@
0 @F2@ FAM
1 HUSB @I3@
1 CHIL @I4@
0 @F3@ FAM
1 HUSB @I4@
1 CHIL @I5@
0 TRLR
";

    #[test]
    fn test_scope_descendants_g3_stops_at_grandchildren() {
        let mut gr = parse_str(SAMPLE_4GEN).unwrap();
        compute_scope(&mut gr, Some("I1"), "descendants", Some(3));
        // gen 1: root
        assert!(
            gr.get_individual("I1").unwrap().in_scope,
            "root must be in scope"
        );
        // gen 1: root's spouse (same generation as root)
        assert!(
            gr.get_individual("I2").unwrap().in_scope,
            "spouse must be in scope"
        );
        // gen 2: children
        assert!(
            gr.get_individual("I3").unwrap().in_scope,
            "child must be in scope"
        );
        // gen 3: grandchildren
        assert!(
            gr.get_individual("I4").unwrap().in_scope,
            "grandchild must be in scope"
        );
        // gen 4: great-grandchildren — must NOT be shown with g=3
        assert!(
            !gr.get_individual("I5").unwrap().in_scope,
            "great-grandchild must NOT be in scope with g=3"
        );
    }

    #[test]
    fn test_scope_ancestors_g3_stops_at_grandparents() {
        let mut gr = parse_str(SAMPLE_4GEN).unwrap();
        compute_scope(&mut gr, Some("I5"), "ancestors", Some(3));
        // gen 1: root
        assert!(
            gr.get_individual("I5").unwrap().in_scope,
            "root must be in scope"
        );
        // gen 2: parent
        assert!(
            gr.get_individual("I4").unwrap().in_scope,
            "parent must be in scope"
        );
        // gen 3: grandparent
        assert!(
            gr.get_individual("I3").unwrap().in_scope,
            "grandparent must be in scope"
        );
        // gen 4: great-grandparent — must NOT be shown with g=3
        assert!(
            !gr.get_individual("I1").unwrap().in_scope,
            "great-grandparent must NOT be in scope with g=3"
        );
    }

    #[test]
    fn test_scope_forest() {
        let mut gr = parse_str(SAMPLE).unwrap();
        compute_scope(&mut gr, Option::None, "forest", Option::None);
        for indi in gr.individuals.values() {
            assert!(indi.in_scope, "individual {} not in scope", indi.id);
        }
        for fam in gr.families.values() {
            assert!(fam.in_scope, "family {} not in scope", fam.id);
        }
    }

    #[test]
    fn test_direction_prefix_only_valid_prefixes_accepted() {
        // "desc" is a valid prefix of "descendants" — I3's ancestors should not be in scope.
        let mut gr = parse_str(SAMPLE).unwrap();
        compute_scope(&mut gr, Some("I3"), "desc", Some(2));
        assert!(
            gr.get_individual("I3").unwrap().in_scope,
            "I3 should be in scope"
        );
        assert!(
            !gr.get_individual("I1").unwrap().in_scope,
            "I1 is not a descendant of I3"
        );
        assert!(
            !gr.get_individual("I2").unwrap().in_scope,
            "I2 is not a descendant of I3"
        );

        // "descABC" is NOT a valid prefix — must not silently match descendants.
        // The fallback is forest scope, so I1 ends up in scope.
        let mut gr2 = parse_str(SAMPLE).unwrap();
        compute_scope(&mut gr2, Some("I3"), "descABC", Some(2));
        assert!(
            gr2.get_individual("I1").unwrap().in_scope,
            "descABC must not match 'descendants'; forest fallback puts I1 in scope"
        );
    }

    #[test]
    fn test_unknown_tag_no_error() {
        let ged = "0 @I1@ INDI\n1 NAME Test /Person/\n1 UNKN something\n0 TRLR\n";
        let result = parse_str(ged);
        assert!(result.is_ok());
        let gr = result.unwrap();
        assert!(gr.get_individual("I1").is_some());
    }

    #[test]
    fn test_cont_after_unknown_level2_does_not_bleed() {
        // A CONT at level 3 after an unrecognised level-2 tag (e.g. SOUR, NOTE)
        // must not append to the previously set field (e.g. birth place).
        let ged = "\
0 @I1@ INDI
1 NAME Test /Person/
1 BIRT
2 DATE 1 JAN 1812
2 PLAC London
2 SOUR some source
3 CONT Image Group Number: 005790649
0 TRLR
";
        let gr = parse_str(ged).unwrap();
        let i1 = gr.get_individual("I1").unwrap();
        let place = i1
            .birth
            .as_ref()
            .and_then(|e| e.place.as_deref())
            .unwrap_or("");
        assert_eq!(
            place, "London",
            "CONT after unknown SOUR must not append to birth place"
        );
    }

    #[test]
    fn test_new_tags_parsed() {
        let ged = "\
0 @I1@ INDI
1 NAME Test /Person/
1 NAM2 Test Person (alternate)
1 NAMH טסט פרסון
1 _LIVING Y
0 @I2@ INDI
1 NAME Dead /Person/
1 _LIVING N
0 @F1@ FAM
1 HUSB @I1@
1 WIFE @I2@
1 JMAR ref-12345
0 TRLR
";
        let gr = parse_str(ged).unwrap();
        let i1 = gr.get_individual("I1").unwrap();
        assert_eq!(i1.alt_name.as_deref(), Some("Test Person (alternate)"));
        assert_eq!(i1.name_heb.as_deref(), Some("טסט פרסון"));
        assert_eq!(i1.living, Some(true));

        let i2 = gr.get_individual("I2").unwrap();
        assert_eq!(i2.living, Some(false));

        let f1 = gr.get_family("F1").unwrap();
        assert_eq!(f1.jmar.as_deref(), Some("ref-12345"));
    }

    #[test]
    fn test_fams_cross_reference_repair() {
        // @I1@ INDI has only FAMS @F1@, but also appears as HUSB in @F2@.
        // After parsing, @I1@.fams should contain both F1 and F2.
        let gedcom = "0 @I1@ INDI\n\
                      1 NAME John /Doe/\n\
                      1 FAMS @F1@\n\
                      0 @F1@ FAM\n\
                      1 HUSB @I1@\n\
                      1 CHIL @I3@\n\
                      0 @F2@ FAM\n\
                      1 HUSB @I1@\n\
                      1 CHIL @I4@\n\
                      0 TRLR\n";
        let gr = parse_str(gedcom).unwrap();
        let i1 = gr.get_individual("I1").unwrap();
        assert!(
            i1.fams.contains(&"F1".to_string()),
            "F1 should be in I1.fams"
        );
        assert!(
            i1.fams.contains(&"F2".to_string()),
            "F2 should be in I1.fams (repaired)"
        );
        let f2 = gr.get_family("F2").unwrap();
        assert_eq!(f2.children_ids, vec!["I4"], "F2 children must be parsed");
    }
}
