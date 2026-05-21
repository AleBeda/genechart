//! Text-like layout: descendants, ancestors, forest.

use super::Layout;
use super::common::{copy_families, copy_individual, resolve_root_id, sort_families_by_date};
use crate::parser::genrep::{Genrep, Individual};
use crate::preferences::Prefs;
use anyhow::Result;
use std::collections::{HashMap, HashSet};

use crate::util::matches_direction;

#[derive(Debug, Clone, Default)]
pub struct SimpleGeo {
    pub line: usize,
    pub indent: usize,
    pub generation: usize,
    pub is_spouse: bool,
    pub is_ellipsis: bool,
    pub connectors_above: Vec<usize>,
    pub connectors_below: Vec<usize>,
}

fn visit(
    id: &str,
    depth: usize,
    spacing: usize,
    line: &mut usize,
    geo_map: &mut HashMap<String, SimpleGeo>,
    visited: &mut HashSet<String>,
    global_shown: &HashSet<String>,
    ellipsis_count: &mut usize,
    genrep: &Genrep,
) {
    if visited.contains(id) {
        return;
    }
    visited.insert(id.to_string());

    let indi = match genrep.individuals.get(id) {
        Some(i) => i,
        None => return,
    };

    if !indi.in_scope {
        return;
    }

    geo_map.insert(
        id.to_string(),
        SimpleGeo {
            line: *line,
            indent: depth,
            generation: depth + 1,
            is_spouse: false,
            ..Default::default()
        },
    );
    *line += 1 + spacing;

    let fams = sort_families_by_date(indi, genrep);

    for fam_id in &fams {
        let fam = match genrep.families.get(fam_id) {
            Some(f) => f,
            None => continue,
        };

        if !fam.in_scope {
            continue;
        }

        let spouse_id: Option<String> = if fam.husband_id.as_deref() == Some(id) {
            fam.wife_id.clone()
        } else if fam.wife_id.as_deref() == Some(id) {
            fam.husband_id.clone()
        } else {
            None
        };

        if let Some(ref sid) = spouse_id {
            if !visited.contains(sid.as_str()) {
                if let Some(s) = genrep.individuals.get(sid.as_str()) {
                    if s.in_scope {
                        visited.insert(sid.clone());
                        geo_map.insert(
                            sid.clone(),
                            SimpleGeo {
                                line: *line,
                                indent: depth,
                                generation: depth + 1,
                                is_spouse: true,
                                ..Default::default()
                            },
                        );
                        *line += 1 + spacing;
                    }
                }
            }
        }

        let children = fam.children_ids.clone();
        // If every child was already shown as a non-spouse in a previous tree, emit
        // "..." as a placeholder instead of repeating the whole subtree.
        let all_children_shown =
            !children.is_empty() && children.iter().all(|c| global_shown.contains(c.as_str()));
        if all_children_shown {
            let ekey = format!("...{}", *ellipsis_count);
            *ellipsis_count += 1;
            geo_map.insert(
                ekey,
                SimpleGeo {
                    line: *line,
                    indent: depth + 1,
                    generation: depth + 2,
                    is_ellipsis: true,
                    ..Default::default()
                },
            );
            *line += 1 + spacing;
        } else {
            for child_id in &children {
                visit(
                    child_id,
                    depth + 1,
                    spacing,
                    line,
                    geo_map,
                    visited,
                    global_shown,
                    ellipsis_count,
                    genrep,
                );
            }
        }
    }
}

fn layout_descendants(
    genrep: &Genrep,
    root: &str,
    spacing: usize,
    geo_map: &mut HashMap<String, SimpleGeo>,
) {
    let mut visited: HashSet<String> = HashSet::new();
    let mut line: usize = 0;
    let global_shown: HashSet<String> = HashSet::new();
    let mut ellipsis_count: usize = 0;
    visit(
        root,
        0,
        spacing,
        &mut line,
        geo_map,
        &mut visited,
        &global_shown,
        &mut ellipsis_count,
        genrep,
    );
}

fn in_order(
    id: &str,
    depth: usize,
    genrep: &Genrep,
    visited: &mut HashSet<String>,
    ordered: &mut Vec<(String, usize)>,
) {
    if visited.contains(id) {
        return;
    }
    visited.insert(id.to_string());

    let Some(indi) = genrep.individuals.get(id) else {
        return;
    };
    if !indi.in_scope {
        return;
    }

    let parents = indi
        .famc
        .first()
        .and_then(|fam_id| genrep.families.get(fam_id));

    let father_id = parents.and_then(|f| f.husband_id.as_deref());
    let mother_id = parents.and_then(|f| f.wife_id.as_deref());

    if let Some(fid) = father_id {
        in_order(fid, depth + 1, genrep, visited, ordered);
    }

    ordered.push((id.to_string(), depth));

    if let Some(mid) = mother_id {
        in_order(mid, depth + 1, genrep, visited, ordered);
    }
}

fn layout_ancestors(
    genrep: &Genrep,
    root: &str,
    spacing: usize,
    geo_map: &mut HashMap<String, SimpleGeo>,
) {
    let mut visited = HashSet::new();
    let mut ordered: Vec<(String, usize)> = Vec::new();
    in_order(root, 0, genrep, &mut visited, &mut ordered);

    // First pass: assign line numbers, expanding gaps by vert_spacing
    let mut id_to_line: HashMap<String, usize> = HashMap::new();
    for (seq, (id, depth)) in ordered.iter().enumerate() {
        let line_num = seq * (1 + spacing);
        id_to_line.insert(id.clone(), line_num);
        geo_map.insert(
            id.clone(),
            SimpleGeo {
                line: line_num,
                indent: *depth,
                generation: depth + 1,
                connectors_above: Vec::new(),
                connectors_below: Vec::new(),
                ..Default::default()
            },
        );
    }

    // Second pass: compute connectors
    for (id, _depth) in &ordered {
        let Some(indi) = genrep.individuals.get(id.as_str()) else {
            continue;
        };
        let self_line = id_to_line[id.as_str()];

        let parents = indi
            .famc
            .first()
            .and_then(|fam_id| genrep.families.get(fam_id));

        if let Some(fam) = parents {
            if let Some(fid) = &fam.husband_id {
                if let Some(&father_line) = id_to_line.get(fid.as_str()) {
                    let above: Vec<usize> = (father_line + 1..self_line).collect();
                    if let Some(geo) = geo_map.get_mut(id.as_str()) {
                        geo.connectors_above = above;
                    }
                }
            }
            if let Some(mid) = &fam.wife_id {
                if let Some(&mother_line) = id_to_line.get(mid.as_str()) {
                    let below: Vec<usize> = (self_line + 1..mother_line).collect();
                    if let Some(geo) = geo_map.get_mut(id.as_str()) {
                        geo.connectors_below = below;
                    }
                }
            }
        }
    }
}

pub struct SimpleLayout;

impl Layout for SimpleLayout {
    type Geo = SimpleGeo;

    fn compute(&self, genrep: &Genrep, prefs: &Prefs) -> Result<Genrep<SimpleGeo>> {
        let dir = prefs.scope.direction.as_str();
        let mut geo_map: HashMap<String, SimpleGeo> = HashMap::new();

        let spacing = prefs.layout.simple.vert_spacing as usize;
        match dir {
            d if matches_direction(d, "descendants") => {
                if let Some(root) = resolve_root_id(genrep, prefs) {
                    layout_descendants(genrep, &root, spacing, &mut geo_map);
                }
            }
            d if matches_direction(d, "ancestors") || matches_direction(d, "pedigree") => {
                if let Some(root) = resolve_root_id(genrep, prefs) {
                    layout_ancestors(genrep, &root, spacing, &mut geo_map);
                }
            }
            d if matches_direction(d, "forest") => {
                // Roots = in-scope individuals with no in-scope parent family.
                let mut roots: Vec<String> = genrep
                    .individuals
                    .iter()
                    .filter(|(_, i)| i.in_scope)
                    .filter(|(_, i)| {
                        !i.famc
                            .iter()
                            .any(|fam_id| genrep.families.get(fam_id).is_some_and(|f| f.in_scope))
                    })
                    .map(|(id, _)| id.clone())
                    .collect();

                // BFS reachable count from a root (via FAMS → spouse + children).
                let tree_size = |root: &str| -> usize {
                    let mut vis: HashSet<String> = HashSet::new();
                    let mut stack = vec![root.to_string()];
                    while let Some(id) = stack.pop() {
                        if vis.contains(&id) {
                            continue;
                        }
                        let indi = match genrep.individuals.get(&id) {
                            Some(i) if i.in_scope => i,
                            _ => continue,
                        };
                        vis.insert(id.clone());
                        for fam_id in &indi.fams {
                            let fam = match genrep.families.get(fam_id) {
                                Some(f) if f.in_scope => f,
                                _ => continue,
                            };
                            let spouse = if fam.husband_id.as_deref() == Some(id.as_str()) {
                                fam.wife_id.as_deref()
                            } else {
                                fam.husband_id.as_deref()
                            };
                            if let Some(sp) = spouse {
                                if !vis.contains(sp) {
                                    stack.push(sp.to_string());
                                }
                            }
                            for child in &fam.children_ids {
                                if !vis.contains(child) {
                                    stack.push(child.clone());
                                }
                            }
                        }
                    }
                    vis.len()
                };

                // Largest trees first; ID as tiebreaker to ensure determinism.
                roots.sort_by(|a, b| tree_size(b).cmp(&tree_size(a)).then(a.cmp(b)));

                // Remove redundant roots: if two roots are married to each other, keep
                // only the one with the larger tree. The removed spouse will still appear
                // inside the kept tree. This prevents near-duplicate trees for couple-roots.
                {
                    let roots_set: HashSet<String> = roots.iter().cloned().collect();
                    let sizes: HashMap<String, usize> =
                        roots.iter().map(|r| (r.clone(), tree_size(r))).collect();
                    roots.retain(|root| {
                        let indi = match genrep.individuals.get(root.as_str()) {
                            Some(i) => i,
                            None => return false,
                        };
                        for fam_id in &indi.fams {
                            let fam = match genrep.families.get(fam_id) {
                                Some(f) if f.in_scope => f,
                                _ => continue,
                            };
                            let spouse_opt = if fam.husband_id.as_deref() == Some(root.as_str()) {
                                fam.wife_id.as_deref()
                            } else {
                                fam.husband_id.as_deref()
                            };
                            if let Some(sp) = spouse_opt {
                                if roots_set.contains(sp) {
                                    let my_size = sizes[root.as_str()];
                                    let sp_size = sizes[sp];
                                    if sp_size > my_size
                                        || (sp_size == my_size && sp < root.as_str())
                                    {
                                        return false;
                                    }
                                }
                            }
                        }
                        true
                    });
                }

                // Each tree uses a fresh per-tree visited set. `global_shown` tracks
                // canonical IDs of non-spouse individuals placed in all previous trees;
                // when every child of a family is already in `global_shown`, the tree
                // emits "..." instead of repeating the full subtree. `global_shown_spouse`
                // tracks who appeared as a spouse in a previous tree; when
                // last_gen_spouses=true this is used to suppress fully redundant standalone
                // trees (same couple already shown in a prior tree, no new children).
                // Instance keys ("ID##N") are used when the same individual appears in
                // more than one tree.
                let tree_gap = 3;
                let mut next_line: usize = 0;
                let mut instance_count: HashMap<String, usize> = HashMap::new();
                let mut global_shown: HashSet<String> = HashSet::new();
                let mut global_shown_spouse: HashSet<String> = HashSet::new();
                let mut ellipsis_count: usize = 0;

                for root in &roots {
                    // With last_gen_spouses=true, skip a root whose tree would be entirely
                    // redundant: the root was already shown as a spouse in a prior tree AND
                    // every family of theirs has all children already shown (or no children).
                    if prefs.show.last_gen_spouses && global_shown_spouse.contains(root.as_str()) {
                        let all_fam_children_shown =
                            genrep.individuals.get(root.as_str()).map_or(false, |indi| {
                                let fams: Vec<_> = indi
                                    .fams
                                    .iter()
                                    .filter_map(|fid| genrep.families.get(fid))
                                    .filter(|f| f.in_scope)
                                    .collect();
                                !fams.is_empty()
                                    && fams.iter().all(|fam| {
                                        fam.children_ids.is_empty()
                                            || fam
                                                .children_ids
                                                .iter()
                                                .all(|c| global_shown.contains(c.as_str()))
                                    })
                            });
                        if all_fam_children_shown {
                            continue;
                        }
                    }

                    let mut tree_visited: HashSet<String> = HashSet::new();
                    let mut tree_map: HashMap<String, SimpleGeo> = HashMap::new();
                    let mut line: usize = 0;
                    visit(
                        root,
                        0,
                        spacing,
                        &mut line,
                        &mut tree_map,
                        &mut tree_visited,
                        &global_shown,
                        &mut ellipsis_count,
                        genrep,
                    );
                    if tree_map.is_empty() {
                        continue;
                    }
                    // Update tracking sets from this tree before merging into geo_map.
                    for (id, geo) in &tree_map {
                        let canonical = id.split("##").next().unwrap_or(id.as_str());
                        if !geo.is_ellipsis {
                            if geo.is_spouse {
                                global_shown_spouse.insert(canonical.to_string());
                            } else {
                                global_shown.insert(canonical.to_string());
                            }
                        }
                    }
                    let tree_max_line = tree_map.values().map(|g| g.line).max().unwrap_or(0);
                    for (id, mut geo) in tree_map {
                        geo.line += next_line;
                        let n = *instance_count.get(&id).unwrap_or(&0);
                        let key = if n == 0 {
                            id.clone()
                        } else {
                            format!("{}##{}", id, n)
                        };
                        instance_count.insert(id, n + 1);
                        geo_map.insert(key, geo);
                    }
                    next_line += tree_max_line + 1 + tree_gap;
                }
            }
            other => {
                eprintln!("warning: unknown direction {other:?}, falling back to descendants");
                if let Some(root) = resolve_root_id(genrep, prefs) {
                    layout_descendants(genrep, &root, spacing, &mut geo_map);
                }
            }
        }

        if !prefs.show.last_gen_spouses {
            let max_non_spouse_gen = geo_map
                .values()
                .filter(|g| !g.is_spouse && !g.is_ellipsis)
                .map(|g| g.generation)
                .max()
                .unwrap_or(0);
            geo_map.retain(|_, g| !(g.is_spouse && g.generation == max_non_spouse_gen));
        }

        // Build out_individuals from geo_map entries. Instance keys ("ID##N") may
        // be present when the forest layout places the same individual in multiple
        // trees; strip the suffix to find the canonical individual data. Ellipsis
        // keys ("...N") get a minimal placeholder individual so emit_scene can
        // render the "..." marker at the correct position.
        let mut out_individuals = HashMap::new();
        for (key, geo) in &geo_map {
            if geo.is_ellipsis {
                out_individuals.insert(
                    key.clone(),
                    Individual {
                        id: key.clone(),
                        given: None,
                        surname: None,
                        sex: None,
                        birth: None,
                        death: None,
                        fams: vec![],
                        famc: vec![],
                        alt_name: None,
                        name_heb: None,
                        living: None,
                        in_scope: true,
                        geo: Some(geo.clone()),
                    },
                );
                continue;
            }
            let canonical = key.split("##").next().unwrap_or(key.as_str());
            if let Some(indi) = genrep.individuals.get(canonical) {
                out_individuals.insert(key.clone(), copy_individual(indi, Some(geo.clone())));
            }
        }
        let placed_canonical: HashSet<&str> = geo_map
            .keys()
            .map(|k| k.split("##").next().unwrap_or(k.as_str()))
            .collect();
        for (id, indi) in &genrep.individuals {
            if !placed_canonical.contains(id.as_str()) {
                out_individuals.insert(id.clone(), copy_individual(indi, None));
            }
        }

        let out_families = copy_families(genrep, |_| None);

        Ok(Genrep {
            individuals: out_individuals,
            families: out_families,
            first_individual_id: genrep.first_individual_id.clone(),
        })
    }
}

/// Right-align the generation number in a 2-char field: " 1. ", " 9. ", "10. ".
/// Fixed width prevents column shift at the single-digit / double-digit boundary.
pub fn gen_prefix_str(generation: usize) -> String {
    format!("{:>2}. ", generation)
}

/// Convert a `Genrep<SimpleGeo>` into a `Scene` IR in pixel coordinates.
///
/// The coordinate system:
/// - `y_display = (geo.line as f64 + 1.0) * line_height_px` — text baseline (y-down)
/// - `top_y     = geo.line as f64 * line_height_px`
/// - `x_name    = geo.indent as f64 * indent_px + gen_prefix_px`
pub fn emit_scene(genrep: &Genrep<SimpleGeo>, prefs: &Prefs) -> crate::scene::Scene {
    use crate::format::{format_event, format_name};
    use crate::scene::{
        ConnectorPrimitive, Point, Primitive, Rect, Scene, TextAlign, TextAttr, TextPrimitive,
    };
    use crate::text_metrics::{CHAR_WIDTH_RATIO, FONT_SIZE, LINE_HEIGHT, parsed_font};

    // ── Font metrics ──────────────────────────────────────────────────────────
    let (_, font_size) = parsed_font(&prefs.output.style.fonts.names);
    let line_height_px = font_size * (LINE_HEIGHT / FONT_SIZE);
    let char_width_px = font_size * CHAR_WIDTH_RATIO;
    let indent_chars = prefs.layout.simple.indent as f64;
    let indent_px = (indent_chars * char_width_px).max(char_width_px);

    // Width in pixels of the generation-number prefix string.
    let gen_prefix_px = |generation: usize| -> f64 {
        if prefs.show.generation_num {
            gen_prefix_str(generation).chars().count() as f64 * char_width_px
        } else {
            0.0
        }
    };

    // When IDs are shown they occupy a fixed left column: 5 chars for "I9999"
    // plus 1 char gap, so 6 × char_width_px.  All other content is shifted
    // right by this amount; the ID itself sits at x = 0.
    let id_col_px: f64 = if prefs.show.id {
        6.0 * char_width_px
    } else {
        0.0
    };

    let highlighted_ids = crate::layout::common::highlight_set(prefs);
    // ── Collect and sort in-scope individuals ─────────────────────────────────
    let mut entries: Vec<(
        &str,
        &crate::parser::genrep::Individual<SimpleGeo>,
        &SimpleGeo,
    )> = genrep
        .individuals
        .iter()
        .filter(|(_, i)| i.in_scope)
        .filter_map(|(id, i)| i.geo.as_ref().map(|g| (id.as_str(), i, g)))
        .collect();
    entries.sort_by_key(|(_, _, g)| g.line);

    if entries.is_empty() {
        return Scene {
            primitives: Vec::new(),
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
        };
    }

    let max_line = entries.iter().map(|(_, _, g)| g.line).max().unwrap_or(0);

    // ── Compute pixel column positions (mirror render_simple column logic) ────
    let max_name_end_px: f64 = entries
        .iter()
        .map(|(_, indi, geo)| {
            id_col_px
                + geo.indent as f64 * indent_px
                + gen_prefix_px(geo.generation)
                + format_name(*indi, prefs).chars().count() as f64 * char_width_px
        })
        .fold(0.0_f64, f64::max);

    let gap_px = 2.0 * char_width_px;

    let max_birth_w_px: f64 = if prefs.show.birth {
        entries
            .iter()
            .filter_map(|(_, i, _)| {
                i.birth.as_ref().and_then(|e| {
                    format_event(
                        &prefs.format.birth,
                        e.date.as_ref(),
                        e.place.as_deref(),
                        &prefs.format.date_qualifiers,
                    )
                })
            })
            .map(|s| s.chars().count() as f64 * char_width_px)
            .fold(0.0_f64, f64::max)
    } else {
        0.0
    };

    let max_death_w_px: f64 = if prefs.show.death {
        entries
            .iter()
            .filter_map(|(_, i, _)| {
                i.death.as_ref().and_then(|e| {
                    format_event(
                        &prefs.format.death,
                        e.date.as_ref(),
                        e.place.as_deref(),
                        &prefs.format.date_qualifiers,
                    )
                })
            })
            .map(|s| s.chars().count() as f64 * char_width_px)
            .fold(0.0_f64, f64::max)
    } else {
        0.0
    };

    let max_marr_w_px: f64 = if prefs.show.marriage {
        entries
            .iter()
            .filter_map(|(id, _i, g)| {
                if g.is_spouse {
                    find_marriage_in_genrep(id, genrep).and_then(|e| {
                        format_event(
                            &prefs.format.marriage,
                            e.date.as_ref(),
                            e.place.as_deref(),
                            &prefs.format.date_qualifiers,
                        )
                    })
                } else {
                    None
                }
            })
            .map(|s| s.chars().count() as f64 * char_width_px)
            .fold(0.0_f64, f64::max)
    } else {
        0.0
    };

    let x_birth_px = max_name_end_px + gap_px;
    let x_death_px = x_birth_px + max_birth_w_px + gap_px;
    let x_marriage_px = x_death_px + max_death_w_px + gap_px;

    let max_x_px = if max_marr_w_px > 0.0 {
        x_marriage_px + max_marr_w_px
    } else if max_death_w_px > 0.0 {
        x_death_px + max_death_w_px
    } else if max_birth_w_px > 0.0 {
        x_birth_px + max_birth_w_px
    } else {
        max_name_end_px
    };

    // ── Build primitives ──────────────────────────────────────────────────────
    let mut primitives: Vec<Primitive> = Vec::new();

    for (id, indi, geo) in &entries {
        let top_y = geo.line as f64 * line_height_px;

        // Ellipsis placeholder: show "..." aligned with where child names start.
        if geo.is_ellipsis {
            let x = id_col_px + geo.indent as f64 * indent_px + gen_prefix_px(geo.generation);
            primitives.push(Primitive::Text(TextPrimitive {
                content: "...".to_string(),
                bbox: Rect {
                    x,
                    y: top_y,
                    w: 3.0 * char_width_px,
                    h: line_height_px,
                },
                align: TextAlign::Left,
                attrs: vec![TextAttr::IndividualName],
            }));
            continue;
        }

        let x_name = id_col_px + geo.indent as f64 * indent_px + gen_prefix_px(geo.generation);
        let gpx = gen_prefix_px(geo.generation);
        let cid = id.split("##").next().unwrap_or(id);
        let is_highlighted = highlighted_ids.contains(cid);

        // Individual ID — emitted first so the text backend writes it at col 0
        // before any other content fills the line.
        if prefs.show.id {
            let id_str = id.split("##").next().unwrap_or(id).to_string();
            let id_w = id_str.chars().count() as f64 * char_width_px;
            primitives.push(Primitive::Text(TextPrimitive {
                content: id_str,
                bbox: Rect {
                    x: 0.0,
                    y: top_y,
                    w: id_w,
                    h: line_height_px,
                },
                align: TextAlign::Left,
                attrs: vec![TextAttr::IndividualId],
            }));
        }

        // Generation number (non-spouse only)
        if prefs.show.generation_num && !geo.is_spouse {
            let prefix = gen_prefix_str(geo.generation);
            primitives.push(Primitive::Text(TextPrimitive {
                content: prefix,
                bbox: Rect {
                    x: id_col_px + geo.indent as f64 * indent_px,
                    y: top_y,
                    w: gpx,
                    h: line_height_px,
                },
                align: TextAlign::Left,
                attrs: vec![TextAttr::GenerationNum],
            }));
        }

        // Individual / spouse name
        let name = format_name(indi, prefs);
        let name_w = name.chars().count() as f64 * char_width_px;
        let name_attrs = crate::scene::label_attrs(
            if geo.is_spouse {
                TextAttr::SpouseName
            } else {
                TextAttr::IndividualName
            },
            is_highlighted,
        );
        primitives.push(Primitive::Text(TextPrimitive {
            content: name,
            bbox: Rect {
                x: x_name,
                y: top_y,
                w: name_w,
                h: line_height_px,
            },
            align: TextAlign::Left,
            attrs: name_attrs,
        }));

        // Birth data
        if prefs.show.birth {
            if let Some(e) = &indi.birth {
                if let Some(s) = format_event(
                    &prefs.format.birth,
                    e.date.as_ref(),
                    e.place.as_deref(),
                    &prefs.format.date_qualifiers,
                ) {
                    let w = s.chars().count() as f64 * char_width_px;
                    primitives.push(Primitive::Text(TextPrimitive {
                        content: s,
                        bbox: Rect {
                            x: x_birth_px,
                            y: top_y,
                            w,
                            h: line_height_px,
                        },
                        align: TextAlign::Left,
                        attrs: vec![TextAttr::BirthData],
                    }));
                }
            }
        }

        // Death data
        if prefs.show.death {
            if let Some(e) = &indi.death {
                if let Some(s) = format_event(
                    &prefs.format.death,
                    e.date.as_ref(),
                    e.place.as_deref(),
                    &prefs.format.date_qualifiers,
                ) {
                    let w = s.chars().count() as f64 * char_width_px;
                    primitives.push(Primitive::Text(TextPrimitive {
                        content: s,
                        bbox: Rect {
                            x: x_death_px,
                            y: top_y,
                            w,
                            h: line_height_px,
                        },
                        align: TextAlign::Left,
                        attrs: vec![TextAttr::DeathData],
                    }));
                }
            }
        }

        // Marriage data (spouse only)
        if prefs.show.marriage && geo.is_spouse {
            if let Some(e) = find_marriage_in_genrep(id, genrep) {
                if let Some(s) = format_event(
                    &prefs.format.marriage,
                    e.date.as_ref(),
                    e.place.as_deref(),
                    &prefs.format.date_qualifiers,
                ) {
                    let w = s.chars().count() as f64 * char_width_px;
                    primitives.push(Primitive::Text(TextPrimitive {
                        content: s,
                        bbox: Rect {
                            x: x_marriage_px,
                            y: top_y,
                            w,
                            h: line_height_px,
                        },
                        align: TextAlign::Left,
                        attrs: vec![TextAttr::MarriageData],
                    }));
                }
            }
        }

        // ── Connector primitives (ancestors mode) ─────────────────────────────
        // x at the left edge of the parent's indent column (one step right of self's indent).
        // Placing the connector before the gen-prefix keeps the gap to adjacent text equal to
        // `indent` character widths, both in text and SVG output.
        let x_conn = id_col_px + (geo.indent as f64 + 2.0) * indent_px;

        if !geo.connectors_above.is_empty() {
            let first = *geo.connectors_above.iter().min().unwrap();
            if first > 0 {
                let father_line = first - 1;
                let parent_y = (father_line as f64 + 1.2) * line_height_px;
                let child_y = (geo.line as f64 + 0.2) * line_height_px;
                primitives.push(Primitive::Connector(ConnectorPrimitive {
                    parent_points: vec![Point {
                        x: x_conn,
                        y: parent_y,
                    }],
                    child_points: vec![Point {
                        x: x_conn,
                        y: child_y,
                    }],
                }));
            }
        }

        if !geo.connectors_below.is_empty() {
            let last = *geo.connectors_below.iter().max().unwrap();
            let mother_line = last + 1;
            let parent_y = (mother_line as f64 + 0.3) * line_height_px;
            let child_y = (geo.line as f64 + 1.2) * line_height_px;
            primitives.push(Primitive::Connector(ConnectorPrimitive {
                parent_points: vec![Point {
                    x: x_conn,
                    y: parent_y,
                }],
                child_points: vec![Point {
                    x: x_conn,
                    y: child_y,
                }],
            }));
        }
    }

    let max_y_px = (max_line + 1) as f64 * line_height_px;

    Scene {
        primitives,
        canvas_bounds: Rect {
            x: 0.0,
            y: 0.0,
            w: max_x_px,
            h: max_y_px,
        },
    }
}

/// Find the first in-scope family marriage event for an individual in the genrep.
fn find_marriage_in_genrep<'a>(
    id: &str,
    genrep: &'a Genrep<SimpleGeo>,
) -> Option<&'a crate::parser::genrep::Event> {
    let indi = genrep.individuals.get(id)?;
    for fam_id in &indi.fams {
        if let Some(fam) = genrep.families.get(fam_id) {
            if fam.in_scope {
                return fam.marriage.as_ref();
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{compute_scope, parse_str};
    use crate::preferences::Prefs;

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
1 MARR
2 DATE 4 APR 1843
2 PLAC London
0 TRLR
";

    #[test]
    fn test_descendants_two_generations() {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".to_string();
        prefs.scope.direction = "descendants".to_string();

        let result = SimpleLayout.compute(&genrep, &prefs).unwrap();

        let i1_geo = result.individuals["I1"].geo.as_ref().unwrap();
        assert_eq!(i1_geo.line, 0);
        assert_eq!(i1_geo.indent, 0);
        assert_eq!(i1_geo.generation, 1);
        assert!(!i1_geo.is_spouse);

        let i2_geo = result.individuals["I2"].geo.as_ref().unwrap();
        assert_eq!(i2_geo.line, 1);
        assert_eq!(i2_geo.indent, 0);
        assert_eq!(i2_geo.generation, 1);
        assert!(i2_geo.is_spouse);

        let i3_geo = result.individuals["I3"].geo.as_ref().unwrap();
        assert_eq!(i3_geo.line, 2);
        assert_eq!(i3_geo.indent, 1);
        assert_eq!(i3_geo.generation, 2);
        assert!(!i3_geo.is_spouse);
    }

    #[test]
    fn test_forest_direction_no_panic() {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, None, "forest", None);

        let mut prefs = Prefs::default();
        prefs.scope.direction = "forest".to_string();

        let result = SimpleLayout.compute(&genrep, &prefs);
        assert!(result.is_ok());
    }

    /// Two completely disconnected family trees: Tree A (I1→I3) and Tree B (I4→I6).
    const GEDCOM_FOREST: &str = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME Alice /Tree-A/
1 SEX F
1 FAMS @F1@
0 @I2@ INDI
1 NAME Bob /Tree-A/
1 SEX M
1 FAMS @F1@
0 @I3@ INDI
1 NAME Carol /Tree-A/
1 SEX F
1 FAMC @F1@
0 @F1@ FAM
1 HUSB @I2@
1 WIFE @I1@
1 CHIL @I3@
0 @I4@ INDI
1 NAME Dan /Tree-B/
1 SEX M
1 FAMS @F2@
0 @I5@ INDI
1 NAME Eve /Tree-B/
1 SEX F
1 FAMS @F2@
0 @I6@ INDI
1 NAME Frank /Tree-B/
1 SEX M
1 FAMC @F2@
0 @F2@ FAM
1 HUSB @I4@
1 WIFE @I5@
1 CHIL @I6@
0 TRLR
";

    #[test]
    fn test_forest_two_disconnected_trees() {
        let mut genrep = parse_str(GEDCOM_FOREST).unwrap();
        compute_scope(&mut genrep, None, "forest", None);

        let mut prefs = Prefs::default();
        prefs.scope.direction = "forest".to_string();
        prefs.show.last_gen_spouses = true;

        let result = SimpleLayout.compute(&genrep, &prefs).unwrap();

        // Every individual must be placed.
        for id in &["I1", "I2", "I3", "I4", "I5", "I6"] {
            assert!(
                result.individuals[*id].geo.is_some(),
                "{id} must have a geo"
            );
        }

        // Tree A (roots I1 or I2, child I3) must all be on lower lines than tree B roots.
        // Since roots are sorted by ID, tree A (I1/I2) appears before tree B (I4/I5).
        let tree_a_lines: Vec<usize> = ["I1", "I2", "I3"]
            .iter()
            .map(|id| result.individuals[*id].geo.as_ref().unwrap().line)
            .collect();
        let tree_b_lines: Vec<usize> = ["I4", "I5", "I6"]
            .iter()
            .map(|id| result.individuals[*id].geo.as_ref().unwrap().line)
            .collect();

        let tree_a_max = *tree_a_lines.iter().max().unwrap();
        let tree_b_min = *tree_b_lines.iter().min().unwrap();
        assert!(
            tree_a_max < tree_b_min,
            "all of tree A must appear before all of tree B: max_A={tree_a_max} min_B={tree_b_min}"
        );
    }

    const GEDCOM_3GEN: &str = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME John /Ancestor/
1 SEX M
1 FAMS @F1@
1 FAMC @F2@
0 @I2@ INDI
1 NAME Jane /Ancestress/
1 SEX F
1 FAMS @F1@
0 @I3@ INDI
1 NAME Paul /Child/
1 SEX M
1 FAMC @F1@
0 @F1@ FAM
1 HUSB @I1@
1 WIFE @I2@
1 CHIL @I3@
0 @I4@ INDI
1 NAME Grandpa /Ancestor/
1 SEX M
1 FAMS @F2@
0 @I5@ INDI
1 NAME Grandma /Ancestor/
1 SEX F
1 FAMS @F2@
0 @F2@ FAM
1 HUSB @I4@
1 WIFE @I5@
1 CHIL @I1@
0 TRLR
";

    #[test]
    fn test_ancestors_two_generations() {
        let mut genrep = parse_str(GEDCOM_3GEN).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(2));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I3".to_string();
        prefs.scope.direction = "ancestors".to_string();

        let output = SimpleLayout.compute(&genrep, &prefs).unwrap();

        let i1 = output.individuals["I1"].geo.as_ref().unwrap();
        let i3 = output.individuals["I3"].geo.as_ref().unwrap();
        let i2 = output.individuals["I2"].geo.as_ref().unwrap();

        assert!(i1.line < i3.line, "father above root");
        assert!(i3.line < i2.line, "mother below root");
        assert_eq!(i1.indent, 1);
        assert_eq!(i3.indent, 0);
        assert_eq!(i2.indent, 1);
        assert_eq!(i1.generation, 2);
        assert_eq!(i3.generation, 1);
        assert_eq!(i2.generation, 2);
    }

    #[test]
    fn test_descendants_vert_spacing() {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".to_string();
        prefs.scope.direction = "descendants".to_string();
        prefs.layout.simple.vert_spacing = 1;

        let result = SimpleLayout.compute(&genrep, &prefs).unwrap();

        let i1_line = result.individuals["I1"].geo.as_ref().unwrap().line;
        let i2_line = result.individuals["I2"].geo.as_ref().unwrap().line;
        let i3_line = result.individuals["I3"].geo.as_ref().unwrap().line;
        assert_eq!(
            i2_line,
            i1_line + 2,
            "spouse should be 2 lines below root with spacing=1"
        );
        assert_eq!(
            i3_line,
            i2_line + 2,
            "child should be 2 lines below spouse with spacing=1"
        );
    }

    #[test]
    fn test_ancestors_vert_spacing() {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(2));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I3".to_string();
        prefs.scope.direction = "ancestors".to_string();
        prefs.layout.simple.vert_spacing = 1;

        let result = SimpleLayout.compute(&genrep, &prefs).unwrap();

        let father_line = result.individuals["I1"].geo.as_ref().unwrap().line;
        let root_line = result.individuals["I3"].geo.as_ref().unwrap().line;
        let mother_line = result.individuals["I2"].geo.as_ref().unwrap().line;
        assert_eq!(
            root_line,
            father_line + 2,
            "root should be 2 lines below father"
        );
        assert_eq!(
            mother_line,
            root_line + 2,
            "mother should be 2 lines below root"
        );

        let root_geo = result.individuals["I3"].geo.as_ref().unwrap();
        assert!(
            root_geo.connectors_above.contains(&(father_line + 1)),
            "gap line between father and root must carry a connector"
        );
    }

    #[test]
    fn test_ancestors_three_generations() {
        let mut genrep = parse_str(GEDCOM_3GEN).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(3));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I3".to_string();
        prefs.scope.direction = "ancestors".to_string();

        let output = SimpleLayout.compute(&genrep, &prefs).unwrap();

        assert_eq!(output.individuals["I4"].geo.as_ref().unwrap().line, 0);
        assert_eq!(output.individuals["I1"].geo.as_ref().unwrap().line, 1);
        assert_eq!(output.individuals["I5"].geo.as_ref().unwrap().line, 2);
        assert_eq!(output.individuals["I3"].geo.as_ref().unwrap().line, 3);
        assert_eq!(output.individuals["I2"].geo.as_ref().unwrap().line, 4);
    }

    // ── Spouse sort order ──

    /// GEDCOM with one root who has two spouses married in different years:
    /// I2 married in 1900, I3 married in 1850. I3 (earlier) must appear first.
    const GEDCOM_TWO_SPOUSES: &str = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME Root /Person/
1 SEX M
1 FAMS @F1@
1 FAMS @F2@
0 @I2@ INDI
1 NAME Later /Spouse/
1 SEX F
1 FAMS @F1@
0 @I3@ INDI
1 NAME Earlier /Spouse/
1 SEX F
1 FAMS @F2@
0 @F1@ FAM
1 HUSB @I1@
1 WIFE @I2@
1 MARR
2 DATE 5 JUN 1900
0 @F2@ FAM
1 HUSB @I1@
1 WIFE @I3@
1 MARR
2 DATE 10 MAR 1850
0 TRLR
";

    #[test]
    fn test_spouses_sorted_by_marriage_date() {
        let mut genrep = parse_str(GEDCOM_TWO_SPOUSES).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".to_string();
        prefs.scope.direction = "descendants".to_string();
        prefs.show.last_gen_spouses = true;

        let result = SimpleLayout.compute(&genrep, &prefs).unwrap();

        let i1_line = result.individuals["I1"].geo.as_ref().unwrap().line;
        let i2_line = result.individuals["I2"].geo.as_ref().unwrap().line; // 1900
        let i3_line = result.individuals["I3"].geo.as_ref().unwrap().line; // 1850

        // I3 (married 1850) must appear on an earlier line than I2 (married 1900).
        assert!(
            i3_line < i2_line,
            "Earlier spouse (I3, 1850) should appear before later spouse (I2, 1900): \
             I1={i1_line}, I2={i2_line}, I3={i3_line}"
        );
        // Root must be on line 0
        assert_eq!(i1_line, 0, "root must be on line 0");
    }
}
