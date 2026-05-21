//! Half-circle pedigree fan layout.

use anyhow::{Result, bail};

use crate::layout::common::{copy_families, copy_individual, resolve_root_id};
use crate::util::matches_direction;
use std::collections::HashMap;
use std::f64::consts::PI;

use super::Layout;
use crate::parser::genrep::{Genrep, Individual};
use crate::preferences::{FanPrefs, Prefs};

#[derive(Debug, Clone)]
pub struct FanGeo {
    pub angle_center: f64,
    pub angle_span: f64,
    pub radius_inner: f64,
    pub radius_outer: f64,
    // x, y are text midpoint offsets; retained for tests but no longer used in emit_scene
    // (emit_scene recomputes from geometry to avoid dependency on the stored values).
    #[allow(dead_code)]
    pub x: f64,
    #[allow(dead_code)]
    pub y: f64,
}

pub struct FanLayout;

impl Layout for FanLayout {
    type Geo = FanGeo;

    fn compute(&self, genrep: &Genrep, prefs: &Prefs) -> Result<Genrep<FanGeo>> {
        let dir = prefs.scope.direction.to_lowercase();
        if !matches_direction(&dir, "ancestors") && !matches_direction(&dir, "pedigree") {
            eprintln!("warning: fan layout requires direction=ancestors");
            bail!("fan layout requires direction=ancestors");
        }

        let root_opt = resolve_root_id(genrep, prefs);
        let root_id = root_opt.as_deref().unwrap_or("");

        if root_id.is_empty() {
            return Ok(Genrep {
                individuals: HashMap::new(),
                families: copy_families(genrep, |_| None),
                first_individual_id: genrep.first_individual_id.clone(),
            });
        }

        let ring_height = prefs.layout.fan.ring_height;
        let max_gen = prefs.scope.generations;

        let mut individuals: HashMap<String, Individual<FanGeo>> = HashMap::new();

        if let Some(root) = genrep.get_individual(root_id) {
            let root_geo = FanGeo {
                angle_center: 90.0,
                angle_span: 180.0,
                radius_inner: 0.0,
                radius_outer: ring_height,
                x: 0.0,
                y: 0.0,
            };
            individuals.insert(root_id.to_string(), copy_individual(root, Some(root_geo)));

            place_ancestors(
                genrep,
                root_id,
                90.0,
                180.0,
                0u32,
                &prefs.layout.fan,
                max_gen,
                &mut individuals,
            );
        }

        Ok(Genrep {
            individuals,
            families: copy_families(genrep, |_| None),
            first_individual_id: genrep.first_individual_id.clone(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn place_ancestors(
    genrep: &Genrep,
    id: &str,
    angle_center: f64,
    angle_span: f64,
    depth: u32,
    fan_prefs: &FanPrefs,
    max_gen: u32,
    out: &mut HashMap<String, Individual<FanGeo>>,
) {
    if max_gen == 0 || depth + 1 >= max_gen {
        return;
    }

    let ind = match genrep.get_individual(id) {
        Some(i) => i,
        None => return,
    };

    let famc_id = match ind.famc.first() {
        Some(fid) => fid.clone(),
        None => return,
    };

    let fam = match genrep.get_family(&famc_id) {
        Some(f) => f,
        None => return,
    };

    let next_depth = depth + 1;
    let child_span = angle_span / 2.0;
    let rh = if next_depth >= fan_prefs.radial_gen {
        fan_prefs.outer_ring_height
    } else {
        fan_prefs.ring_height
    };
    let radius_inner = if next_depth < fan_prefs.radial_gen {
        next_depth as f64 * (fan_prefs.ring_height + fan_prefs.ring_gap)
    } else {
        fan_prefs.radial_gen as f64 * (fan_prefs.ring_height + fan_prefs.ring_gap)
            + (next_depth - fan_prefs.radial_gen) as f64
                * (fan_prefs.outer_ring_height + fan_prefs.ring_gap)
    };
    let radius_outer = radius_inner + rh;
    let radius_mid = (radius_inner + radius_outer) / 2.0;

    // Father: left side of chart = higher angle range → center at angle_center + angle_span/4
    let father_angle = angle_center + angle_span / 4.0;
    if let Some(father_id) = &fam.husband_id {
        if let Some(father) = genrep.get_individual(father_id) {
            if father.in_scope {
                let (x, y) = to_xy(radius_mid, father_angle);
                let geo = FanGeo {
                    angle_center: father_angle,
                    angle_span: child_span,
                    radius_inner,
                    radius_outer,
                    x,
                    y,
                };
                out.insert(father_id.clone(), copy_individual(father, Some(geo)));
                place_ancestors(
                    genrep,
                    father_id,
                    father_angle,
                    child_span,
                    next_depth,
                    fan_prefs,
                    max_gen,
                    out,
                );
            }
        }
    }

    // Mother: right side of chart = lower angle range → center at angle_center - angle_span/4
    let mother_angle = angle_center - angle_span / 4.0;
    if let Some(mother_id) = &fam.wife_id {
        if let Some(mother) = genrep.get_individual(mother_id) {
            if mother.in_scope {
                let (x, y) = to_xy(radius_mid, mother_angle);
                let geo = FanGeo {
                    angle_center: mother_angle,
                    angle_span: child_span,
                    radius_inner,
                    radius_outer,
                    x,
                    y,
                };
                out.insert(mother_id.clone(), copy_individual(mother, Some(geo)));
                place_ancestors(
                    genrep,
                    mother_id,
                    mother_angle,
                    child_span,
                    next_depth,
                    fan_prefs,
                    max_gen,
                    out,
                );
            }
        }
    }
}

fn to_xy(radius: f64, angle_deg: f64) -> (f64, f64) {
    let rad = angle_deg * PI / 180.0;
    (radius * rad.cos(), radius * rad.sin())
}

pub fn emit_scene(genrep: &Genrep<FanGeo>, prefs: &Prefs) -> crate::scene::Scene {
    use crate::format::{format_event, format_name};
    use crate::scene::{Primitive, Rect, Scene, TextAttr, WedgePrimitive};
    // C2a — compute max_radius
    let max_radius = genrep
        .individuals
        .values()
        .filter_map(|i| i.geo.as_ref())
        .map(|g| g.radius_outer)
        .fold(0.0_f64, f64::max);

    // Fan center in display space (render_scene adds MARGIN and chart_top_offset)
    let cx = max_radius;
    let cy = max_radius;

    // Threshold angle_span below which a wedge gets radial text.
    // At radial_gen=3: threshold = 180/8 = 22.5° (depth-3 wedges get radial).
    // At radial_gen=0: threshold = 180°, all wedges get radial.
    let inner_span_threshold = 180.0 / 2.0f64.powi(prefs.layout.fan.radial_gen as i32);

    // C2b — highlights
    let highlighted_ids = crate::layout::common::highlight_set(prefs);
    let mut indis: Vec<_> = genrep
        .individuals
        .values()
        .filter(|i| i.in_scope)
        .filter_map(|i| i.geo.as_ref().map(|g| (i, g)))
        .collect();
    indis.sort_by(|(_, a), (_, b)| {
        a.radius_inner
            .partial_cmp(&b.radius_inner)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut primitives: Vec<Primitive> = Vec::with_capacity(indis.len());
    for (indi, geo) in &indis {
        let radial_text = geo.angle_span <= inner_span_threshold + 1e-9;
        let birth_line = if prefs.show.birth {
            indi.birth.as_ref().and_then(|e| {
                format_event(
                    &prefs.format.birth,
                    e.date.as_ref(),
                    e.place.as_deref(),
                    &prefs.format.date_qualifiers,
                )
            })
        } else {
            None
        };
        let death_line = if prefs.show.death {
            indi.death.as_ref().and_then(|e| {
                format_event(
                    &prefs.format.death,
                    e.date.as_ref(),
                    e.place.as_deref(),
                    &prefs.format.date_qualifiers,
                )
            })
        } else {
            None
        };
        primitives.push(Primitive::Wedge(WedgePrimitive {
            cx,
            cy,
            angle_center: geo.angle_center,
            angle_span: geo.angle_span,
            radius_inner: geo.radius_inner,
            radius_outer: geo.radius_outer,
            label: Some(format_name(indi, prefs)),
            label_attrs: crate::scene::label_attrs(
                TextAttr::IndividualName,
                highlighted_ids.contains(&indi.id),
            ),
            radial_text,
            individual_id: indi.id.clone(),
            birth_line,
            death_line,
        }));
    }

    // C2d — canvas bounds: full diameter wide, half-circle height
    let canvas_bounds = Rect {
        x: 0.0,
        y: 0.0,
        w: 2.0 * max_radius,
        h: max_radius,
    };

    Scene {
        primitives,
        canvas_bounds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::genrep::{Family, Genrep, Individual};
    use crate::preferences::Prefs;

    fn make_individual(id: &str, famc: Vec<String>) -> Individual<()> {
        Individual {
            id: id.to_string(),
            given: None,
            surname: None,
            sex: None,
            birth: None,
            death: None,
            fams: vec![],
            famc,
            alt_name: None,
            name_heb: None,
            living: None,
            notes: vec![],
            in_scope: true,
            geo: None,
        }
    }

    fn make_family(id: &str, husband: Option<&str>, wife: Option<&str>, child: &str) -> Family<()> {
        Family {
            id: id.to_string(),
            husband_id: husband.map(str::to_string),
            wife_id: wife.map(str::to_string),
            children_ids: vec![child.to_string()],
            marriage: None,
            jmar: None,
            notes: vec![],
            in_scope: true,
            geo: None,
        }
    }

    fn test_genrep() -> Genrep {
        let mut individuals = HashMap::new();
        let mut families = HashMap::new();

        // I1 = root, I2 = father, I3 = mother, I4 = paternal grandfather, I5 = paternal grandmother
        individuals.insert(
            "I1".to_string(),
            make_individual("I1", vec!["F1".to_string()]),
        );
        individuals.insert(
            "I2".to_string(),
            make_individual("I2", vec!["F2".to_string()]),
        );
        individuals.insert("I3".to_string(), make_individual("I3", vec![]));
        individuals.insert("I4".to_string(), make_individual("I4", vec![]));
        individuals.insert("I5".to_string(), make_individual("I5", vec![]));

        families.insert(
            "F1".to_string(),
            make_family("F1", Some("I2"), Some("I3"), "I1"),
        );
        families.insert(
            "F2".to_string(),
            make_family("F2", Some("I4"), Some("I5"), "I2"),
        );

        Genrep {
            individuals,
            families,
            first_individual_id: Some("I1".to_string()),
        }
    }

    fn ancestors_prefs() -> Prefs {
        let mut prefs = Prefs::default();
        prefs.scope.direction = "ancestors".to_string();
        prefs.scope.root = "I1".to_string();
        prefs.scope.generations = 4;
        prefs.layout.fan.ring_height = 80.0;
        prefs.layout.fan.ring_gap = 20.0;
        prefs
    }

    #[test]
    fn root_placement() {
        let result = FanLayout
            .compute(&test_genrep(), &ancestors_prefs())
            .unwrap();
        let geo = result.individuals["I1"].geo.as_ref().unwrap();
        assert_eq!(geo.angle_center, 90.0);
        assert_eq!(geo.angle_span, 180.0);
        assert!(geo.x.abs() < 1e-10);
        assert!(geo.y.abs() < 1e-10);
    }

    #[test]
    fn father_arc() {
        let result = FanLayout
            .compute(&test_genrep(), &ancestors_prefs())
            .unwrap();
        let geo = result.individuals["I2"].geo.as_ref().unwrap();
        assert!(
            (geo.angle_center - 135.0).abs() < 1e-10,
            "father angle_center={}",
            geo.angle_center
        );
        assert!((geo.angle_span - 90.0).abs() < 1e-10);
    }

    #[test]
    fn mother_arc() {
        let result = FanLayout
            .compute(&test_genrep(), &ancestors_prefs())
            .unwrap();
        let geo = result.individuals["I3"].geo.as_ref().unwrap();
        assert!(
            (geo.angle_center - 45.0).abs() < 1e-10,
            "mother angle_center={}",
            geo.angle_center
        );
        assert!((geo.angle_span - 90.0).abs() < 1e-10);
    }

    #[test]
    fn paternal_grandfather_arc() {
        let result = FanLayout
            .compute(&test_genrep(), &ancestors_prefs())
            .unwrap();
        let geo = result.individuals["I4"].geo.as_ref().unwrap();
        assert!(
            (geo.angle_center - 157.5).abs() < 1e-10,
            "paternal grandfather angle_center={}",
            geo.angle_center
        );
        assert!((geo.angle_span - 45.0).abs() < 1e-10);
    }

    #[test]
    fn no_overlap() {
        let result = FanLayout
            .compute(&test_genrep(), &ancestors_prefs())
            .unwrap();

        let fg = result.individuals["I2"].geo.as_ref().unwrap();
        let mg = result.individuals["I3"].geo.as_ref().unwrap();

        let father_min = fg.angle_center - fg.angle_span / 2.0; // 90
        let father_max = fg.angle_center + fg.angle_span / 2.0; // 180
        let mother_min = mg.angle_center - mg.angle_span / 2.0; // 0
        let mother_max = mg.angle_center + mg.angle_span / 2.0; // 90

        assert!(father_max <= 180.0 + 1e-10);
        assert!(mother_min >= -1e-10);
        // contiguous: father's lower edge meets mother's upper edge
        assert!((father_min - mother_max).abs() < 1e-10);
        // total span covers the full half-circle
        assert!((fg.angle_span + mg.angle_span - 180.0).abs() < 1e-10);
    }

    #[test]
    fn non_pedigree_direction_errors() {
        let mut prefs = ancestors_prefs();
        prefs.scope.direction = "descendants".to_string();
        assert!(FanLayout.compute(&test_genrep(), &prefs).is_err());
    }

    #[test]
    fn outer_ring_height_at_radial_gen() {
        let mut prefs = ancestors_prefs();
        prefs.layout.fan.outer_ring_height = 180.0;
        prefs.layout.fan.radial_gen = 2; // switch at grandparent depth
        prefs.scope.generations = 6;
        let result = FanLayout.compute(&test_genrep(), &prefs).unwrap();
        // I4 = paternal grandfather, depth 2 = first outer ring
        // radius_inner = 2 * (80 + 20) = 200; radius_outer = 200 + 180 = 380
        let geo = result.individuals["I4"].geo.as_ref().unwrap();
        assert!(
            (geo.radius_inner - 200.0).abs() < 1e-6,
            "radius_inner={}",
            geo.radius_inner
        );
        assert!(
            (geo.radius_outer - 380.0).abs() < 1e-6,
            "radius_outer={}",
            geo.radius_outer
        );
    }
}
