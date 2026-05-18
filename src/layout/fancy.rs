//! Fancy (cascading descendants) layout.

use anyhow::{Result, bail};
use std::collections::{HashMap, HashSet};

use crate::format::{format_event, format_name};
use crate::layout::Layout;
use crate::layout::common::{
    copy_families, copy_individual, highlight_set, resolve_root_id, sort_families_by_date,
};
use crate::parser::genrep::{Genrep, Individual};
use crate::preferences::Prefs;
use crate::scene::{
    FancyConnKind, FancyConnector, FancyLine, FancyTextItem, GroupPrimitive, Primitive, Rect,
    Scene, TextAttr, label_attrs,
};
use crate::text_metrics::{CHAR_WIDTH_RATIO, FONT_SIZE, parsed_font};
use crate::util::matches_direction;

// Non-configurable geometry constants (canvas units).
const V_OFFSET: f64 = 8.0; // x from ind name-left to vertical connector
const DATA_INDENT: f64 = 12.0; // x from connector to data / spouse-name start
const IND_DATA_OFFSET: f64 = V_OFFSET + DATA_INDENT; // = 20.0
const NAME_TO_CONN_GAP: f64 = 3.0; // gap: end of spouse name → horiz connector
const ARC_R: f64 = 6.0; // quarter-circle radius
const CHILD_SHORT_H: f64 = 10.0; // short horizontal from spine to child gen x
const IND_SPOUSE_GAP: f64 = 8.0; // vertical gap: ind block bottom → spouse name
const CHILD_SIBLING_GAP: f64 = 6.0; // vertical gap between successive children
const CHILD_TEXT_GAP: f64 = 6.0; // horizontal gap: connector end → child name
const SPOUSE_TEXT_GAP: f64 = 4.0; // horizontal gap: connector end → spouse name
pub struct FancyLayout;

#[derive(Debug, Clone)]
pub struct FancyGeo {
    pub x: f64,          // left edge of name
    pub y: f64,          // top of name line
    pub generation: u32, // 1 = root
    #[allow(dead_code)]
    pub is_main: bool, // true = direct descendant; false = spouse
}

impl Layout for FancyLayout {
    type Geo = FancyGeo;

    fn compute(&self, genrep: &Genrep, prefs: &Prefs) -> Result<Genrep<FancyGeo>> {
        let dir = prefs.scope.direction.to_lowercase();
        let is_desc = matches_direction(&dir, "descendants");
        let is_anc = matches_direction(&dir, "ancestors") || matches_direction(&dir, "pedigree");
        if !is_desc && !is_anc {
            eprintln!(
                "warning: fancy layout requires direction=descendants or direction=ancestors"
            );
            bail!("fancy layout: unsupported direction {dir:?}");
        }

        let root_opt = resolve_root_id(genrep, prefs);
        let root_id = match root_opt.as_deref() {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => {
                return Ok(Genrep {
                    individuals: HashMap::new(),
                    families: copy_families(genrep, |_| None),
                    first_individual_id: genrep.first_individual_id.clone(),
                });
            }
        };

        let mut out: HashMap<String, Individual<FancyGeo>> = HashMap::new();
        if is_desc {
            place_subtree(&root_id, 0.0, 0.0, 1, true, prefs, genrep, &mut out);
        } else {
            let mut visit_count: HashMap<String, usize> = HashMap::new();
            place_anc_subtree(
                &root_id,
                0.0,
                0.0,
                1,
                prefs,
                genrep,
                &mut out,
                &mut visit_count,
            );
        }

        Ok(Genrep {
            individuals: out,
            families: copy_families(genrep, |_| None),
            first_individual_id: genrep.first_individual_id.clone(),
        })
    }
}

fn name_font_size(prefs: &Prefs) -> f64 {
    let (_, sz) = parsed_font(&prefs.output.style.fonts.names);
    if sz <= 0.0 { FONT_SIZE } else { sz }
}

fn data_font_size(prefs: &Prefs) -> f64 {
    let (_, sz_names) = parsed_font(&prefs.output.style.fonts.names);
    let sz_names = if sz_names <= 0.0 { FONT_SIZE } else { sz_names };
    if prefs.output.style.fonts.dates.trim().is_empty() {
        sz_names
    } else {
        let (_, sz) = parsed_font(&prefs.output.style.fonts.dates);
        if sz <= 0.0 { sz_names } else { sz }
    }
}

fn name_lh(prefs: &Prefs) -> f64 {
    name_font_size(prefs) * 1.2
}
fn data_lh(prefs: &Prefs) -> f64 {
    data_font_size(prefs) * 1.2
}

fn ind_height(prefs: &Prefs) -> f64 {
    name_lh(prefs)
        + if prefs.show.birth {
            data_lh(prefs)
        } else {
            0.0
        }
        + if prefs.show.death {
            data_lh(prefs)
        } else {
            0.0
        }
}

fn spouse_height(prefs: &Prefs) -> f64 {
    name_lh(prefs)
        + if prefs.show.birth {
            data_lh(prefs)
        } else {
            0.0
        }
        + if prefs.show.death {
            data_lh(prefs)
        } else {
            0.0
        }
        + if prefs.show.marriage {
            data_lh(prefs)
        } else {
            0.0
        }
}

#[allow(clippy::too_many_arguments)]
fn place_subtree(
    id: &str,
    x_gen: f64,
    y_start: f64,
    generation: u32,
    is_main: bool,
    prefs: &Prefs,
    genrep: &Genrep,
    out: &mut HashMap<String, Individual<FancyGeo>>,
) -> f64 {
    let ind = match genrep.get_individual(id) {
        Some(i) if i.in_scope => i,
        _ => return 0.0,
    };

    let geo = FancyGeo {
        x: x_gen,
        y: y_start,
        generation,
        is_main,
    };
    out.insert(id.to_string(), copy_individual(ind, Some(geo)));

    let max_gen = prefs.scope.generations;
    let gen_width = prefs.layout.fancy.gen_width;
    let child_gap = prefs.layout.fancy.child_gap;
    let mut y_cursor = y_start + ind_height(prefs);

    let fam_ids = sort_families_by_date(ind, genrep);
    for fam_id in &fam_ids {
        let fam = match genrep.get_family(fam_id) {
            Some(f) => f,
            None => continue,
        };

        // Spouse id = the other side of the family.
        let spouse_id: Option<&String> = if fam.husband_id.as_deref() == Some(id) {
            fam.wife_id.as_ref()
        } else {
            fam.husband_id.as_ref()
        };

        // Show spouse unless at last gen with show.last_gen_spouses = false.
        let skip_spouse = generation == max_gen && !prefs.show.last_gen_spouses;
        if !skip_spouse {
            if let Some(sid) = spouse_id {
                if let Some(sp) = genrep.get_individual(sid) {
                    if sp.in_scope {
                        y_cursor += IND_SPOUSE_GAP;
                        let sp_geo = FancyGeo {
                            x: x_gen,
                            y: y_cursor,
                            generation,
                            is_main: false,
                        };
                        out.insert(sid.clone(), copy_individual(sp, Some(sp_geo)));
                        y_cursor += spouse_height(prefs) + child_gap;
                    }
                }
            }
        }

        // Recurse into children if within generation limit.
        if generation < max_gen {
            for child_id in &fam.children_ids {
                let h = place_subtree(
                    child_id,
                    x_gen + gen_width + CHILD_TEXT_GAP,
                    y_cursor,
                    generation + 1,
                    true,
                    prefs,
                    genrep,
                    out,
                );
                y_cursor += h + CHILD_SIBLING_GAP;
            }
        }
    }

    y_cursor - y_start
}

fn anc_instance_key(id: &str, count: usize) -> String {
    if count == 0 {
        id.to_string()
    } else {
        format!("{}##{}", id, count)
    }
}

fn place_anc_subtree(
    id: &str,
    x_gen: f64,
    y_start: f64,
    generation: u32,
    prefs: &Prefs,
    genrep: &Genrep,
    out: &mut HashMap<String, Individual<FancyGeo>>,
    visit_count: &mut HashMap<String, usize>,
) -> (f64, f64) {
    let count = *visit_count.get(id).unwrap_or(&0);
    *visit_count.entry(id.to_string()).or_insert(0) += 1;
    let instance_key = anc_instance_key(id, count);

    let ind = match genrep.get_individual(id) {
        Some(i) if i.in_scope => i,
        _ => return (y_start, 0.0),
    };

    let max_gen = prefs.scope.generations;
    let gen_width = prefs.layout.fancy.gen_width;
    let anc_gap = prefs.layout.fancy.anc_gap;
    let inh = ind_height(prefs);
    let next_x = x_gen + gen_width + CHILD_TEXT_GAP;

    let parent_fam = if generation < max_gen {
        ind.famc.first().and_then(|fid| genrep.get_family(fid))
    } else {
        None
    };

    let self_y;
    let total_h;

    if let Some(fam) = parent_fam {
        let father_id = fam
            .husband_id
            .as_ref()
            .filter(|fid| genrep.get_individual(fid).map_or(false, |i| i.in_scope));
        let mother_id = fam
            .wife_id
            .as_ref()
            .filter(|mid| genrep.get_individual(mid).map_or(false, |i| i.in_scope));

        let mut y_cursor = y_start;
        let mut father_y: Option<f64> = None;
        let mut mother_y: Option<f64> = None;

        if let Some(fid) = father_id {
            let (fy, fh) = place_anc_subtree(
                fid,
                next_x,
                y_cursor,
                generation + 1,
                prefs,
                genrep,
                out,
                visit_count,
            );
            father_y = Some(fy);
            y_cursor += fh;
        }

        if father_id.is_some() && mother_id.is_some() {
            y_cursor += inh + anc_gap;
        }

        if let Some(mid) = mother_id {
            let (my, mh) = place_anc_subtree(
                mid,
                next_x,
                y_cursor,
                generation + 1,
                prefs,
                genrep,
                out,
                visit_count,
            );
            mother_y = Some(my);
            y_cursor += mh;
        }

        self_y = match (father_y, mother_y) {
            (Some(fy), Some(my)) => (fy + my) / 2.0,
            (Some(fy), None) => fy,
            (None, Some(my)) => my,
            (None, None) => y_start,
        };
        total_h = (y_cursor - y_start).max(inh);
    } else {
        self_y = y_start;
        total_h = inh;
    }

    out.insert(
        instance_key,
        copy_individual(
            ind,
            Some(FancyGeo {
                x: x_gen,
                y: self_y,
                generation,
                is_main: true,
            }),
        ),
    );
    (self_y, total_h)
}

pub fn emit_scene(genrep: &Genrep<FancyGeo>, prefs: &Prefs) -> Scene {
    let dir = prefs.scope.direction.to_lowercase();
    if matches_direction(&dir, "ancestors") || matches_direction(&dir, "pedigree") {
        emit_anc_scene(genrep, prefs)
    } else {
        emit_desc_scene(genrep, prefs)
    }
}

fn emit_desc_scene(genrep: &Genrep<FancyGeo>, prefs: &Prefs) -> Scene {
    let highlighted_ids = highlight_set(prefs);
    let conn_color = hex_color_fancy(prefs.output.style.connectors.border);
    let conn_width = if prefs.output.style.connectors.width > 0.0 {
        prefs.output.style.connectors.width
    } else {
        1.0
    };

    let n_lh = name_lh(prefs);
    let d_lh = data_lh(prefs);
    let nfs = name_font_size(prefs);

    let root_id = {
        let r = prefs.scope.root.trim();
        if !r.is_empty() && genrep.individuals.contains_key(r) {
            r.to_string()
        } else {
            match genrep.first_individual_id.as_deref() {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => {
                    return Scene {
                        primitives: vec![],
                        canvas_bounds: Rect {
                            x: 0.0,
                            y: 0.0,
                            w: 0.0,
                            h: 0.0,
                        },
                    };
                }
            }
        }
    };

    let mut primitives: Vec<Primitive> = Vec::new();
    let mut indiv_conns: Vec<FancyConnector> = Vec::new();
    let mut spouse_conns: Vec<FancyConnector> = Vec::new();
    let mut max_x: f64 = 0.0;
    let mut max_y: f64 = 0.0;

    emit_subtree(
        &root_id,
        genrep,
        prefs,
        &highlighted_ids,
        &conn_color,
        conn_width,
        n_lh,
        d_lh,
        nfs,
        &mut primitives,
        &mut indiv_conns,
        &mut spouse_conns,
        &mut max_x,
        &mut max_y,
    );

    for c in indiv_conns {
        primitives.push(Primitive::FancyConn(c));
    }
    for c in spouse_conns {
        primitives.push(Primitive::FancyConn(c));
    }

    Scene {
        primitives,
        canvas_bounds: Rect {
            x: 0.0,
            y: 0.0,
            w: max_x,
            h: max_y,
        },
    }
}

fn emit_anc_scene(genrep: &Genrep<FancyGeo>, prefs: &Prefs) -> Scene {
    let highlighted_ids = highlight_set(prefs);
    let conn_color = hex_color_fancy(prefs.output.style.connectors.border);
    let conn_width = prefs.output.style.connectors.width.max(0.1);

    let n_lh = name_lh(prefs);
    let d_lh = data_lh(prefs);
    let nfs = name_font_size(prefs);

    let root_id = {
        let r = prefs.scope.root.trim();
        if !r.is_empty() && genrep.individuals.contains_key(r) {
            r.to_string()
        } else {
            match genrep.first_individual_id.as_deref() {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => {
                    return Scene {
                        primitives: vec![],
                        canvas_bounds: Rect {
                            x: 0.0,
                            y: 0.0,
                            w: 0.0,
                            h: 0.0,
                        },
                    };
                }
            }
        }
    };

    let mut primitives: Vec<Primitive> = Vec::new();
    let mut anc_conns: Vec<FancyConnector> = Vec::new();
    let mut max_x: f64 = 0.0;
    let mut max_y: f64 = 0.0;
    let mut visit_count: HashMap<String, usize> = HashMap::new();

    emit_anc_subtree(
        &root_id,
        genrep,
        prefs,
        &highlighted_ids,
        &conn_color,
        conn_width,
        n_lh,
        d_lh,
        nfs,
        &mut primitives,
        &mut anc_conns,
        &mut max_x,
        &mut max_y,
        &mut visit_count,
    );

    if prefs.show.duplicated_individual {
        emit_anc_dup_links(genrep, &conn_color, conn_width, n_lh, &mut anc_conns);
    }

    for c in anc_conns {
        primitives.push(Primitive::FancyConn(c));
    }

    Scene {
        primitives,
        canvas_bounds: Rect {
            x: 0.0,
            y: 0.0,
            w: max_x,
            h: max_y,
        },
    }
}

fn hex_color_fancy(val: i64) -> String {
    let r = (val >> 8) & 0xF;
    let g = (val >> 4) & 0xF;
    let b = val & 0xF;
    format!("#{r:X}{r:X}{g:X}{g:X}{b:X}{b:X}")
}

#[allow(clippy::too_many_arguments)]
fn emit_anc_subtree(
    id: &str,
    genrep: &Genrep<FancyGeo>,
    prefs: &Prefs,
    highlighted_ids: &HashSet<String>,
    conn_color: &str,
    conn_width: f64,
    n_lh: f64,
    d_lh: f64,
    nfs: f64,
    primitives: &mut Vec<Primitive>,
    anc_conns: &mut Vec<FancyConnector>,
    max_x: &mut f64,
    max_y: &mut f64,
    visit_count: &mut HashMap<String, usize>,
) {
    let count = *visit_count.get(id).unwrap_or(&0);
    *visit_count.entry(id.to_string()).or_insert(0) += 1;
    let my_key = anc_instance_key(id, count);

    let ind = match genrep.individuals.get(&my_key) {
        Some(i) if i.in_scope => i,
        _ => return,
    };
    let geo = match ind.geo.as_ref() {
        Some(g) => g,
        None => return,
    };

    let highlighted = highlighted_ids.contains(&ind.id);
    let base_name = format_name(ind, prefs);
    let name_text = if prefs.show.generation_num {
        format!("{}. {}", geo.generation, base_name)
    } else {
        base_name.clone()
    };

    let (name_family, _) = parsed_font(&prefs.output.style.fonts.names);
    let is_desc_bold = matches!(
        prefs.output.style.fonts.descendant.trim(),
        "bold" | "bolder"
    );
    let (data_family, dfs) = {
        let (fam, sz) = parsed_font(&prefs.output.style.fonts.dates);
        let fam = if fam.is_empty() {
            name_family.clone()
        } else {
            fam
        };
        let sz = if sz <= 0.0 { nfs } else { sz };
        (fam, sz)
    };
    let (id_family, id_sz) = {
        let (fam, sz) = parsed_font(&prefs.output.style.fonts.id);
        let fam = if fam.is_empty() {
            "Courier New, monospace".to_string()
        } else {
            fam
        };
        let sz = if sz <= 0.0 { 8.0 } else { sz };
        (fam, sz)
    };
    let name_text_w =
        crate::backend::font_metrics::measure_text_w(&name_text, &name_family, nfs, is_desc_bold)
            .unwrap_or_else(|| name_text.chars().count() as f64 * nfs * CHAR_WIDTH_RATIO);
    let gen_prefix_w = if prefs.show.generation_num {
        let prefix = format!("{}. ", geo.generation);
        crate::backend::font_metrics::measure_text_w(&prefix, &name_family, nfs, is_desc_bold)
            .unwrap_or(0.0)
    } else {
        0.0
    };
    let ind_data_x = geo.x + gen_prefix_w + IND_DATA_OFFSET;

    // ── Build text lines ─────────────────────────────────────────────────────
    let mut lines: Vec<FancyLine> = Vec::new();
    lines.push(FancyLine {
        x: geo.x,
        y: geo.y,
        text: name_text.clone(),
        attrs: label_attrs(TextAttr::IndividualName, highlighted),
    });
    if prefs.show.id {
        let ind_id_str = ind
            .id
            .trim_start_matches('@')
            .trim_end_matches('@')
            .to_string();
        lines.push(FancyLine {
            x: geo.x + name_text_w + 4.0,
            y: geo.y,
            text: ind_id_str,
            attrs: vec![TextAttr::IndividualId],
        });
    }
    let mut y_off = n_lh;
    if prefs.show.birth {
        if let Some(ev) = &ind.birth {
            if let Some(s) = format_event(
                &prefs.format.birth,
                ev.date.as_ref(),
                ev.place.as_deref(),
                &prefs.format.date_qualifiers,
            ) {
                lines.push(FancyLine {
                    x: ind_data_x,
                    y: geo.y + y_off,
                    text: s,
                    attrs: vec![TextAttr::BirthData],
                });
            }
        }
        y_off += d_lh;
    }
    if prefs.show.death {
        if let Some(ev) = &ind.death {
            if let Some(s) = format_event(
                &prefs.format.death,
                ev.date.as_ref(),
                ev.place.as_deref(),
                &prefs.format.date_qualifiers,
            ) {
                lines.push(FancyLine {
                    x: ind_data_x,
                    y: geo.y + y_off,
                    text: s,
                    attrs: vec![TextAttr::DeathData],
                });
            }
        }
    }

    // Update canvas bounds.
    for line in &lines {
        let is_name = line.attrs.contains(&TextAttr::IndividualName);
        let is_id = line.attrs.contains(&TextAttr::IndividualId);
        let (mfam, msz, mbold) = if is_id {
            (id_family.as_str(), id_sz, false)
        } else if is_name {
            (name_family.as_str(), nfs, is_desc_bold)
        } else {
            (data_family.as_str(), dfs, false)
        };
        let w = crate::backend::font_metrics::measure_text_w(&line.text, mfam, msz, mbold)
            .unwrap_or_else(|| line.text.chars().count() as f64 * msz * CHAR_WIDTH_RATIO);
        *max_x = f64::max(*max_x, line.x + w);
    }
    *max_y = f64::max(*max_y, geo.y + ind_height(prefs));

    // ── Emit text group ──────────────────────────────────────────────────────
    // For duplicate instances, the group id uses "-dup-N" suffix (e.g. "anc-text-I5-dup-1").
    let group_id = format!(
        "anc-text-{}",
        my_key
            .trim_start_matches('@')
            .trim_end_matches('@')
            .replace("##", "-dup-")
    );
    primitives.push(Primitive::Group(GroupPrimitive {
        id: group_id,
        children: vec![Primitive::Group(GroupPrimitive {
            id: String::new(),
            children: vec![Primitive::FancyText(FancyTextItem {
                lines,
                individual_id: ind.id.clone(),
                highlighted,
            })],
        })],
    }));

    // ── Build connector to parents ───────────────────────────────────────────
    let gen_width = prefs.layout.fancy.gen_width;
    let parent_fam = ind.famc.first().and_then(|fid| genrep.get_family(fid));

    if let Some(fam) = parent_fam {
        // Peek at each parent's NEXT instance key (before recursing into them).
        let father_id = fam
            .husband_id
            .as_deref()
            .filter(|fid| genrep.get_individual(fid).map_or(false, |i| i.in_scope));
        let father_key =
            father_id.map(|fid| anc_instance_key(fid, *visit_count.get(fid).unwrap_or(&0)));
        let father_geo = father_key
            .as_ref()
            .and_then(|k| genrep.individuals.get(k).and_then(|i| i.geo.as_ref()));

        let mother_id = fam
            .wife_id
            .as_deref()
            .filter(|mid| genrep.get_individual(mid).map_or(false, |i| i.in_scope));
        let mother_key =
            mother_id.map(|mid| anc_instance_key(mid, *visit_count.get(mid).unwrap_or(&0)));
        let mother_geo = mother_key
            .as_ref()
            .and_then(|k| genrep.individuals.get(k).and_then(|i| i.geo.as_ref()));

        if father_geo.is_some() || mother_geo.is_some() {
            let name_end_x = geo.x + name_text_w;
            let parent_conn_end_x = geo.x + gen_width;
            let x_spine = parent_conn_end_x - CHILD_SHORT_H - ARC_R;
            let child_mid_y = geo.y + n_lh / 2.0;

            let mut d = String::new();

            match (father_geo, mother_geo) {
                (Some(fg), Some(mg)) => {
                    let father_mid_y = fg.y + n_lh / 2.0;
                    let mother_mid_y = mg.y + n_lh / 2.0;

                    // Horizontal from name end to spine junction.
                    d.push_str(&format!(
                        "M {:.1} {:.1} L {:.1} {:.1}",
                        name_end_x, child_mid_y, x_spine, child_mid_y
                    ));

                    // Upper branch to father (child_mid_y > father_mid_y).
                    if child_mid_y - father_mid_y > ARC_R {
                        d.push_str(&format!(
                            " M {:.1} {:.1} L {:.1} {:.1} A {ARC_R} {ARC_R} 0 0 1 {:.1} {:.1} L {:.1} {:.1}",
                            x_spine, child_mid_y,
                            x_spine, father_mid_y + ARC_R,
                            x_spine + ARC_R, father_mid_y,
                            parent_conn_end_x, father_mid_y,
                        ));
                    } else {
                        d.push_str(&format!(
                            " M {:.1} {:.1} L {:.1} {:.1}",
                            x_spine, child_mid_y, parent_conn_end_x, father_mid_y,
                        ));
                    }

                    // Lower branch to mother (child_mid_y < mother_mid_y).
                    if mother_mid_y - child_mid_y > ARC_R {
                        d.push_str(&format!(
                            " M {:.1} {:.1} L {:.1} {:.1} A {ARC_R} {ARC_R} 0 0 0 {:.1} {:.1} L {:.1} {:.1}",
                            x_spine, child_mid_y,
                            x_spine, mother_mid_y - ARC_R,
                            x_spine + ARC_R, mother_mid_y,
                            parent_conn_end_x, mother_mid_y,
                        ));
                    } else {
                        d.push_str(&format!(
                            " M {:.1} {:.1} L {:.1} {:.1}",
                            x_spine, child_mid_y, parent_conn_end_x, mother_mid_y,
                        ));
                    }
                }
                (Some(fg), None) => {
                    let parent_mid_y = fg.y + n_lh / 2.0;
                    d.push_str(&format!(
                        "M {:.1} {:.1} L {:.1} {:.1}",
                        name_end_x, child_mid_y, parent_conn_end_x, parent_mid_y,
                    ));
                }
                (None, Some(mg)) => {
                    let parent_mid_y = mg.y + n_lh / 2.0;
                    d.push_str(&format!(
                        "M {:.1} {:.1} L {:.1} {:.1}",
                        name_end_x, child_mid_y, parent_conn_end_x, parent_mid_y,
                    ));
                }
                (None, None) => {}
            }

            if !d.is_empty() {
                *max_x = f64::max(*max_x, parent_conn_end_x);
                let conn_id = format!(
                    "anc-conn-{}",
                    my_key
                        .trim_start_matches('@')
                        .trim_end_matches('@')
                        .replace("##", "-dup-")
                );
                anc_conns.push(FancyConnector {
                    d,
                    stroke: conn_color.to_string(),
                    stroke_width: conn_width,
                    kind: FancyConnKind::IndivToSpouse,
                    id: conn_id,
                    stroke_dasharray: String::new(),
                });
            }
        }

        // ── Recurse into parents ─────────────────────────────────────────────
        if let Some(fid) = father_id {
            emit_anc_subtree(
                fid,
                genrep,
                prefs,
                highlighted_ids,
                conn_color,
                conn_width,
                n_lh,
                d_lh,
                nfs,
                primitives,
                anc_conns,
                max_x,
                max_y,
                visit_count,
            );
        }
        if let Some(mid) = mother_id {
            emit_anc_subtree(
                mid,
                genrep,
                prefs,
                highlighted_ids,
                conn_color,
                conn_width,
                n_lh,
                d_lh,
                nfs,
                primitives,
                anc_conns,
                max_x,
                max_y,
                visit_count,
            );
        }
    }
}

fn emit_anc_dup_links(
    genrep: &Genrep<FancyGeo>,
    conn_color: &str,
    conn_width: f64,
    n_lh: f64,
    anc_conns: &mut Vec<FancyConnector>,
) {
    // Collect all "##"-keyed instances grouped by base ID, as (x, y) pairs.
    let mut dup_groups: HashMap<String, Vec<(f64, f64)>> = HashMap::new();

    for (key, ind) in &genrep.individuals {
        if let Some(pos) = key.find("##") {
            let geo = match ind.geo.as_ref() {
                Some(g) => g,
                None => continue,
            };
            dup_groups
                .entry(key[..pos].to_string())
                .or_default()
                .push((geo.x, geo.y));
        }
    }

    for (base_id, mut points) in dup_groups {
        let base_geo = match genrep
            .individuals
            .get(&base_id)
            .and_then(|i| i.geo.as_ref())
        {
            Some(g) => g,
            None => continue,
        };
        // Include the base (first) instance.
        points.push((base_geo.x, base_geo.y));
        // Sort by (x, y) so the polyline goes from the instance closest to the
        // root (smallest x) to the furthest. For same-generation instances (same
        // x) this becomes a vertical sort by y.
        points.sort_by(|a, b| a.partial_cmp(b).unwrap());

        // Build a polyline connecting all instances at their mid-y positions.
        let mut d = String::new();
        for (i, (x, y)) in points.iter().enumerate() {
            let mid_y = y + n_lh / 2.0;
            if i == 0 {
                d.push_str(&format!("M {:.1} {:.1}", x, mid_y));
            } else {
                d.push_str(&format!(" L {:.1} {:.1}", x, mid_y));
            }
        }

        let dup_id = format!(
            "anc-dup-{}",
            base_id.trim_start_matches('@').trim_end_matches('@')
        );
        anc_conns.push(FancyConnector {
            d,
            stroke: conn_color.to_string(),
            stroke_width: conn_width,
            kind: FancyConnKind::IndivToSpouse,
            id: dup_id,
            stroke_dasharray: "4 3".to_string(),
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_subtree(
    id: &str,
    genrep: &Genrep<FancyGeo>,
    prefs: &Prefs,
    highlighted_ids: &HashSet<String>,
    conn_color: &str,
    conn_width: f64,
    n_lh: f64,
    d_lh: f64,
    nfs: f64,
    primitives: &mut Vec<Primitive>,
    indiv_conns: &mut Vec<FancyConnector>,
    spouse_conns: &mut Vec<FancyConnector>,
    max_x: &mut f64,
    max_y: &mut f64,
) {
    let ind = match genrep.get_individual(id) {
        Some(i) if i.in_scope => i,
        _ => return,
    };
    let geo = match ind.geo.as_ref() {
        Some(g) => g,
        None => return,
    };

    // ── Text for main individual ──────────────────────────────────────────────
    let highlighted = highlighted_ids.contains(&ind.id);
    let base_name = format_name(ind, prefs);
    let name_text = if prefs.show.generation_num {
        format!("{}. {}", geo.generation, base_name)
    } else {
        base_name.clone()
    };

    // Compute x positions anchored to actual name start (after gen-num prefix).
    let (name_family, _) = parsed_font(&prefs.output.style.fonts.names);
    let is_desc_bold = matches!(
        prefs.output.style.fonts.descendant.trim(),
        "bold" | "bolder"
    );
    let (data_family, dfs) = {
        let (fam, sz) = parsed_font(&prefs.output.style.fonts.dates);
        let fam = if fam.is_empty() {
            name_family.clone()
        } else {
            fam
        };
        let sz = if sz <= 0.0 { nfs } else { sz };
        (fam, sz)
    };
    let (id_family, id_sz) = {
        let (fam, sz) = parsed_font(&prefs.output.style.fonts.id);
        let fam = if fam.is_empty() {
            "Courier New, monospace".to_string()
        } else {
            fam
        };
        let sz = if sz <= 0.0 { 8.0 } else { sz };
        (fam, sz)
    };
    let name_text_w =
        crate::backend::font_metrics::measure_text_w(&name_text, &name_family, nfs, is_desc_bold)
            .unwrap_or_else(|| name_text.chars().count() as f64 * nfs * CHAR_WIDTH_RATIO);
    let gen_prefix_w = if prefs.show.generation_num {
        let prefix = format!("{}. ", geo.generation);
        crate::backend::font_metrics::measure_text_w(&prefix, &name_family, nfs, is_desc_bold)
            .unwrap_or(0.0)
    } else {
        0.0
    };
    let name_start_x = geo.x + gen_prefix_w;
    let first_char_half_w = base_name
        .chars()
        .next()
        .and_then(|c| {
            crate::backend::font_metrics::measure_text_w(
                &c.to_string(),
                &name_family,
                nfs,
                is_desc_bold,
            )
        })
        .map(|w| w / 2.0)
        .unwrap_or(nfs * CHAR_WIDTH_RATIO / 2.0);
    let xv = name_start_x + first_char_half_w;
    let ind_data_x = name_start_x + IND_DATA_OFFSET;
    let mut lines: Vec<FancyLine> = Vec::new();
    lines.push(FancyLine {
        x: geo.x,
        y: geo.y,
        text: name_text.clone(),
        attrs: label_attrs(TextAttr::IndividualName, highlighted),
    });
    if prefs.show.id {
        let ind_id_str = ind
            .id
            .trim_start_matches('@')
            .trim_end_matches('@')
            .to_string();
        lines.push(FancyLine {
            x: geo.x + name_text_w + 4.0,
            y: geo.y,
            text: ind_id_str,
            attrs: vec![TextAttr::IndividualId],
        });
    }
    let mut y_off = n_lh;

    if prefs.show.birth {
        if let Some(ev) = &ind.birth {
            if let Some(s) = format_event(
                &prefs.format.birth,
                ev.date.as_ref(),
                ev.place.as_deref(),
                &prefs.format.date_qualifiers,
            ) {
                lines.push(FancyLine {
                    x: ind_data_x,
                    y: geo.y + y_off,
                    text: s,
                    attrs: vec![TextAttr::BirthData],
                });
            }
        }
        y_off += d_lh;
    }
    if prefs.show.death {
        if let Some(ev) = &ind.death {
            if let Some(s) = format_event(
                &prefs.format.death,
                ev.date.as_ref(),
                ev.place.as_deref(),
                &prefs.format.date_qualifiers,
            ) {
                lines.push(FancyLine {
                    x: ind_data_x,
                    y: geo.y + y_off,
                    text: s,
                    attrs: vec![TextAttr::DeathData],
                });
            }
        }
    }

    for line in &lines {
        let is_name = line.attrs.contains(&TextAttr::IndividualName);
        let is_id = line.attrs.contains(&TextAttr::IndividualId);
        let (mfam, msz, mbold) = if is_id {
            (id_family.as_str(), id_sz, false)
        } else if is_name {
            (name_family.as_str(), nfs, is_desc_bold)
        } else {
            (data_family.as_str(), dfs, false)
        };
        let w = crate::backend::font_metrics::measure_text_w(&line.text, mfam, msz, mbold)
            .unwrap_or_else(|| line.text.chars().count() as f64 * msz * CHAR_WIDTH_RATIO);
        *max_x = f64::max(*max_x, line.x + w);
    }
    *max_y = f64::max(*max_y, geo.y + ind_height(prefs));

    primitives.push(Primitive::FancyText(FancyTextItem {
        lines,
        individual_id: ind.id.clone(),
        highlighted,
    }));

    // ── Iterate families ──────────────────────────────────────────────────────
    let max_gen = prefs.scope.generations;
    let gen_width = prefs.layout.fancy.gen_width;

    // Collect spouse y-positions for IndivToSpouse connector.
    let mut spouse_ys: Vec<f64> = Vec::new();

    let fam_ids = sort_families_by_date(ind, genrep);
    for fam_id in &fam_ids {
        let fam = match genrep.get_family(fam_id) {
            Some(f) => f,
            None => continue,
        };

        let spouse_id: Option<&String> = if fam.husband_id.as_deref() == Some(id) {
            fam.wife_id.as_ref()
        } else {
            fam.husband_id.as_ref()
        };

        // ── Spouse text ───────────────────────────────────────────────────────
        let skip_spouse = geo.generation == max_gen && !prefs.show.last_gen_spouses;
        let mut spouse_name_w: f64 = 0.0;
        let mut spouse_y: Option<f64> = None;

        if !skip_spouse {
            if let Some(sid) = spouse_id {
                if let Some(sp) = genrep.get_individual(sid) {
                    if sp.in_scope {
                        if let Some(sg) = sp.geo.as_ref() {
                            let sp_highlighted = highlighted_ids.contains(&sp.id);
                            let sp_name = format_name(sp, prefs);
                            let sp_bold =
                                matches!(prefs.output.style.fonts.spouse.trim(), "bold" | "bolder");
                            spouse_name_w = crate::backend::font_metrics::measure_text_w(
                                &sp_name,
                                &name_family,
                                nfs,
                                sp_bold,
                            )
                            .unwrap_or_else(|| {
                                sp_name.chars().count() as f64 * nfs * CHAR_WIDTH_RATIO
                            });
                            let sp_name_x = name_start_x + IND_DATA_OFFSET + SPOUSE_TEXT_GAP;
                            let sp_data_x = name_start_x + 2.0 * IND_DATA_OFFSET + SPOUSE_TEXT_GAP;

                            let mut sp_lines: Vec<FancyLine> = Vec::new();
                            sp_lines.push(FancyLine {
                                x: sp_name_x,
                                y: sg.y,
                                text: sp_name.clone(),
                                attrs: label_attrs(TextAttr::SpouseName, sp_highlighted),
                            });
                            if prefs.show.id {
                                let sp_id_str = sp
                                    .id
                                    .trim_start_matches('@')
                                    .trim_end_matches('@')
                                    .to_string();
                                sp_lines.push(FancyLine {
                                    x: sp_name_x + spouse_name_w + 4.0,
                                    y: sg.y,
                                    text: sp_id_str,
                                    attrs: vec![TextAttr::IndividualId],
                                });
                            }

                            let mut sy_off = n_lh;
                            if prefs.show.birth {
                                if let Some(ev) = &sp.birth {
                                    if let Some(s) = format_event(
                                        &prefs.format.birth,
                                        ev.date.as_ref(),
                                        ev.place.as_deref(),
                                        &prefs.format.date_qualifiers,
                                    ) {
                                        sp_lines.push(FancyLine {
                                            x: sp_data_x,
                                            y: sg.y + sy_off,
                                            text: s,
                                            attrs: vec![TextAttr::BirthData],
                                        });
                                    }
                                }
                                sy_off += d_lh;
                            }
                            if prefs.show.death {
                                if let Some(ev) = &sp.death {
                                    if let Some(s) = format_event(
                                        &prefs.format.death,
                                        ev.date.as_ref(),
                                        ev.place.as_deref(),
                                        &prefs.format.date_qualifiers,
                                    ) {
                                        sp_lines.push(FancyLine {
                                            x: sp_data_x,
                                            y: sg.y + sy_off,
                                            text: s,
                                            attrs: vec![TextAttr::DeathData],
                                        });
                                    }
                                }
                                sy_off += d_lh;
                            }
                            let marriage_text: Option<String> = if prefs.show.marriage {
                                fam.marriage.as_ref().and_then(|ev| {
                                    format_event(
                                        &prefs.format.marriage,
                                        ev.date.as_ref(),
                                        ev.place.as_deref(),
                                        &prefs.format.date_qualifiers,
                                    )
                                })
                            } else {
                                None
                            };
                            if let Some(ref s) = marriage_text {
                                sp_lines.push(FancyLine {
                                    x: sp_data_x,
                                    y: sg.y + sy_off,
                                    text: s.clone(),
                                    attrs: vec![TextAttr::MarriageData],
                                });
                            }
                            if prefs.show.id && prefs.show.marriage {
                                let fam_id_str = fam_id
                                    .trim_start_matches('@')
                                    .trim_end_matches('@')
                                    .to_string();
                                let marr_w = marriage_text
                                    .as_deref()
                                    .and_then(|s| {
                                        crate::backend::font_metrics::measure_text_w(
                                            s,
                                            &data_family,
                                            dfs,
                                            false,
                                        )
                                    })
                                    .unwrap_or_else(|| {
                                        marriage_text
                                            .as_deref()
                                            .map(|s| {
                                                s.chars().count() as f64 * dfs * CHAR_WIDTH_RATIO
                                            })
                                            .unwrap_or(0.0)
                                    });
                                let id_x =
                                    sp_data_x + if marr_w > 0.0 { marr_w + 4.0 } else { 0.0 };
                                sp_lines.push(FancyLine {
                                    x: id_x,
                                    y: sg.y + sy_off,
                                    text: fam_id_str,
                                    attrs: vec![TextAttr::IndividualId],
                                });
                            }

                            *max_x = f64::max(*max_x, sp_name_x + spouse_name_w);
                            *max_y = f64::max(*max_y, sg.y + spouse_height(prefs));
                            for sp_line in &sp_lines {
                                if sp_line.attrs.contains(&TextAttr::SpouseName) {
                                    continue;
                                }
                                let is_id = sp_line.attrs.contains(&TextAttr::IndividualId);
                                let (mfam, msz) = if is_id {
                                    (id_family.as_str(), id_sz)
                                } else {
                                    (data_family.as_str(), dfs)
                                };
                                let w = crate::backend::font_metrics::measure_text_w(
                                    &sp_line.text,
                                    mfam,
                                    msz,
                                    false,
                                )
                                .unwrap_or_else(|| {
                                    sp_line.text.chars().count() as f64 * msz * CHAR_WIDTH_RATIO
                                });
                                *max_x = f64::max(*max_x, sp_line.x + w);
                            }

                            primitives.push(Primitive::FancyText(FancyTextItem {
                                lines: sp_lines,
                                individual_id: sp.id.clone(),
                                highlighted: sp_highlighted,
                            }));

                            spouse_ys.push(sg.y);
                            spouse_y = Some(sg.y);
                        }
                    }
                }
            }
        }

        // ── Children (recursive) ──────────────────────────────────────────────
        if geo.generation < max_gen {
            for child_id in &fam.children_ids {
                emit_subtree(
                    child_id,
                    genrep,
                    prefs,
                    highlighted_ids,
                    conn_color,
                    conn_width,
                    n_lh,
                    d_lh,
                    nfs,
                    primitives,
                    indiv_conns,
                    spouse_conns,
                    max_x,
                    max_y,
                );
            }
        }

        // ── SpouseToChildren connector ────────────────────────────────────────
        if let Some(y_sp) = spouse_y {
            if geo.generation < max_gen {
                let children: Vec<f64> = fam
                    .children_ids
                    .iter()
                    .filter_map(|cid| genrep.get_individual(cid))
                    .filter(|ci| ci.in_scope)
                    .filter_map(|ci| ci.geo.as_ref())
                    .map(|cg| cg.y)
                    .collect();

                if !children.is_empty() {
                    let child_gen_x = geo.x + gen_width;
                    let x_spine = child_gen_x - CHILD_SHORT_H - ARC_R;
                    let sp_text_x = name_start_x + IND_DATA_OFFSET + SPOUSE_TEXT_GAP;
                    let x_conn_start = sp_text_x + spouse_name_w + NAME_TO_CONN_GAP;
                    let y_sp_mid = y_sp + n_lh / 2.0;
                    let y_last_mid = children.last().unwrap() + n_lh / 2.0;

                    let mut d = format!(
                        "M {:.1} {:.1} L {:.1} {:.1} A {ARC_R} {ARC_R} 0 0 1 {:.1} {:.1} L {:.1} {:.1}",
                        x_conn_start,
                        y_sp_mid,
                        x_spine - ARC_R,
                        y_sp_mid,
                        x_spine,
                        y_sp_mid + ARC_R,
                        x_spine,
                        y_last_mid - ARC_R,
                    );
                    for y_c in &children {
                        let y_c_mid = y_c + n_lh / 2.0;
                        d.push_str(&format!(
                            " M {:.1} {:.1} A {ARC_R} {ARC_R} 0 0 0 {:.1} {:.1} L {:.1} {:.1}",
                            x_spine,
                            y_c_mid - ARC_R,
                            x_spine + ARC_R,
                            y_c_mid,
                            child_gen_x,
                            y_c_mid,
                        ));
                    }
                    *max_x = f64::max(*max_x, child_gen_x);
                    spouse_conns.push(FancyConnector {
                        d,
                        stroke: conn_color.to_string(),
                        stroke_width: conn_width,
                        kind: FancyConnKind::SpouseToChildren,
                        id: String::new(),
                        stroke_dasharray: String::new(),
                    });
                }
            }
        }
    }

    // ── IndivToSpouse connector ───────────────────────────────────────────────
    if !spouse_ys.is_empty() {
        let y_trunk_top = geo.y + n_lh;
        let y_trunk_bot = spouse_ys.last().unwrap() + n_lh / 2.0 - ARC_R;
        let sp_conn_x = ind_data_x;

        let mut d = format!("M {xv:.1} {y_trunk_top:.1} L {xv:.1} {y_trunk_bot:.1}");
        for y_sp in &spouse_ys {
            let y_mid = y_sp + n_lh / 2.0;
            d.push_str(&format!(
                " M {xv:.1} {:.1} A {ARC_R} {ARC_R} 0 0 0 {:.1} {y_mid:.1} L {sp_conn_x:.1} {y_mid:.1}",
                y_mid - ARC_R,
                xv + ARC_R,
            ));
        }
        indiv_conns.push(FancyConnector {
            d,
            stroke: conn_color.to_string(),
            stroke_width: conn_width,
            kind: FancyConnKind::IndivToSpouse,
            id: String::new(),
            stroke_dasharray: String::new(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;
    use crate::parser::{compute_scope, parse_str};

    const GEDCOM: &str = "\
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
0 TRLR
";

    fn anc_prefs() -> crate::preferences::Prefs {
        let mut prefs = crate::preferences::Prefs::default();
        prefs.scope.root = "I3".into();
        prefs.scope.direction = "ancestors".into();
        prefs.scope.generations = 2;
        prefs.layout.layout_type = "fancy".into();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.birth = false;
        prefs.show.death = false;
        prefs.show.marriage = false;
        prefs.show.generation_num = false;
        prefs
    }

    #[test]
    fn anc_place_basic() {
        let prefs = anc_prefs();
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(2));
        let result = FancyLayout.compute(&genrep, &prefs).unwrap();

        let paul = result.get_individual("I3").unwrap();
        let john = result.get_individual("I1").unwrap();
        let jane = result.get_individual("I2").unwrap();

        let paul_y = paul.geo.as_ref().unwrap().y;
        let john_y = john.geo.as_ref().unwrap().y;
        let jane_y = jane.geo.as_ref().unwrap().y;

        // John (father) is above Paul; Jane (mother) is below.
        assert!(
            john_y <= paul_y,
            "father should be at or above root: john_y={john_y} paul_y={paul_y}"
        );
        assert!(
            jane_y >= paul_y,
            "mother should be at or below root: jane_y={jane_y} paul_y={paul_y}"
        );

        // Paul's y is the midpoint of John's and Jane's y.
        let expected = (john_y + jane_y) / 2.0;
        assert!(
            (paul_y - expected).abs() < 0.1,
            "paul_y={paul_y} expected={expected}"
        );
    }

    #[test]
    fn anc_emit_scene_contains_names() {
        let prefs = anc_prefs();
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(2));
        let result = FancyLayout.compute(&genrep, &prefs).unwrap();
        let scene = emit_scene(&result, &prefs);

        let all_text: Vec<String> = scene
            .primitives
            .iter()
            .flat_map(|p| {
                if let Primitive::Group(outer) = p {
                    outer
                        .children
                        .iter()
                        .flat_map(|c| {
                            if let Primitive::Group(inner) = c {
                                inner
                                    .children
                                    .iter()
                                    .flat_map(|ic| {
                                        if let Primitive::FancyText(item) = ic {
                                            item.lines
                                                .iter()
                                                .map(|l| l.text.clone())
                                                .collect::<Vec<_>>()
                                        } else {
                                            vec![]
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            } else {
                                vec![]
                            }
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                }
            })
            .collect();

        let joined = all_text.join(" ");
        assert!(joined.contains("Paul"), "missing Paul: {joined}");
        assert!(joined.contains("John"), "missing John: {joined}");
        assert!(joined.contains("Jane"), "missing Jane: {joined}");
    }

    // Consanguineous GEDCOM: I1 is both paternal and maternal grandfather of I4.
    const CONSANG_GEDCOM: &str = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME Common /Ancestor/
1 SEX M
0 @I2@ INDI
1 NAME Father /Individual/
1 SEX M
1 FAMS @F1@
1 FAMC @F2@
0 @I3@ INDI
1 NAME Mother /Individual/
1 SEX F
1 FAMS @F1@
1 FAMC @F3@
0 @I4@ INDI
1 NAME Root /Individual/
1 SEX M
1 FAMC @F1@
0 @F1@ FAM
1 HUSB @I2@
1 WIFE @I3@
1 CHIL @I4@
0 @F2@ FAM
1 HUSB @I1@
1 CHIL @I2@
0 @F3@ FAM
1 HUSB @I1@
1 CHIL @I3@
0 TRLR
";

    fn consang_prefs(dup: bool) -> crate::preferences::Prefs {
        let mut prefs = crate::preferences::Prefs::default();
        prefs.scope.root = "I4".into();
        prefs.scope.direction = "ancestors".into();
        prefs.scope.generations = 3;
        prefs.layout.layout_type = "fancy".into();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.birth = false;
        prefs.show.death = false;
        prefs.show.marriage = false;
        prefs.show.generation_num = false;
        prefs.show.duplicated_individual = dup;
        prefs
    }

    #[test]
    fn anc_place_dup_false_still_shows_both_instances() {
        // With show.duplicated_individual=false, both instances should still be
        // placed (the preference only controls the dashed link, not visibility).
        let prefs = consang_prefs(false);
        let mut genrep = parse_str(CONSANG_GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I4"), "ancestors", Some(3));
        let result = FancyLayout.compute(&genrep, &prefs).unwrap();

        assert!(
            result.individuals.contains_key("I1"),
            "I1 (first instance) should exist"
        );
        assert!(
            result.individuals.contains_key("I1##1"),
            "I1##1 (second instance) should also exist even when show.duplicated_individual=false"
        );
    }

    #[test]
    fn anc_place_dup_true_two_entries() {
        let prefs = consang_prefs(true);
        let mut genrep = parse_str(CONSANG_GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I4"), "ancestors", Some(3));
        let result = FancyLayout.compute(&genrep, &prefs).unwrap();

        assert!(
            result.individuals.contains_key("I1"),
            "I1 (first instance) should exist"
        );
        assert!(
            result.individuals.contains_key("I1##1"),
            "I1##1 (second instance) should exist when show.duplicated_individual=true"
        );

        let y1 = result.individuals["I1"].geo.as_ref().unwrap().y;
        let y2 = result.individuals["I1##1"].geo.as_ref().unwrap().y;
        assert!(
            (y1 - y2).abs() > 1.0,
            "two instances of I1 should be at different y positions: y1={y1} y2={y2}"
        );
    }

    #[test]
    fn anc_emit_dup_false_shows_both_no_dashed_link() {
        // With false: both instances appear in the scene, but no dashed connector.
        let prefs = consang_prefs(false);
        let mut genrep = parse_str(CONSANG_GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I4"), "ancestors", Some(3));
        let result = FancyLayout.compute(&genrep, &prefs).unwrap();
        let scene = emit_scene(&result, &prefs);

        let mut common_count = 0usize;
        for p in &scene.primitives {
            if let Primitive::Group(outer) = p {
                for c in &outer.children {
                    if let Primitive::Group(inner) = c {
                        for ic in &inner.children {
                            if let Primitive::FancyText(item) = ic {
                                if item.lines.iter().any(|l| l.text.contains("Common")) {
                                    common_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(
            common_count, 2,
            "both instances should appear when duplicated_individual=false"
        );

        // No dashed connector when false.
        let has_dashed = scene.primitives.iter().any(|p| {
            if let Primitive::FancyConn(c) = p {
                !c.stroke_dasharray.is_empty()
            } else {
                false
            }
        });
        assert!(
            !has_dashed,
            "no dashed connector expected when duplicated_individual=false"
        );
    }

    #[test]
    fn anc_emit_dup_true_dashed_link() {
        let prefs = consang_prefs(true);
        let mut genrep = parse_str(CONSANG_GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I4"), "ancestors", Some(3));
        let result = FancyLayout.compute(&genrep, &prefs).unwrap();
        let scene = emit_scene(&result, &prefs);

        // Common Ancestor should appear twice.
        let mut common_count = 0usize;
        for p in &scene.primitives {
            if let Primitive::Group(outer) = p {
                for c in &outer.children {
                    if let Primitive::Group(inner) = c {
                        for ic in &inner.children {
                            if let Primitive::FancyText(item) = ic {
                                if item.lines.iter().any(|l| l.text.contains("Common")) {
                                    common_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(
            common_count, 2,
            "Common Ancestor should appear twice when duplicated_individual=true"
        );

        // A dashed FancyConn connector should exist.
        let has_dashed = scene.primitives.iter().any(|p| {
            if let Primitive::FancyConn(c) = p {
                !c.stroke_dasharray.is_empty()
            } else {
                false
            }
        });
        assert!(
            has_dashed,
            "expected a dashed connector for duplicated individual"
        );
    }
}
