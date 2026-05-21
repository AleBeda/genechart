//! Text-like layout: descendants, ancestors, forest (stub).

use super::Layout;
use super::common::{copy_families, copy_individual, resolve_root_id, sort_families_by_date};
use crate::parser::genrep::Genrep;
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
    pub connectors_above: Vec<usize>,
    pub connectors_below: Vec<usize>,
}

fn word_wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 || text.chars().count() <= width {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_chars = word.chars().count();
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word_chars <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn count_note_lines(notes: &[String], available_chars: usize) -> usize {
    notes
        .iter()
        .filter(|n| !n.trim().is_empty())
        .map(|n| {
            n.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| word_wrap(l, available_chars).len())
                .sum::<usize>()
                .max(1)
        })
        .sum()
}

/// Conservative lower bound on chart width in chars (ignores indentation).
/// Used to pre-allocate note lines before the actual max_x_px is known.
fn estimate_max_x_chars(genrep: &Genrep, prefs: &Prefs) -> usize {
    use crate::format::{format_event, format_name};

    let id_col: usize = if prefs.show.id { 6 } else { 0 };
    let gen_pfx: usize = if prefs.show.generation_num { 4 } else { 0 };
    let gap = 2usize;

    let max_name: usize = genrep
        .individuals
        .values()
        .filter(|i| i.in_scope)
        .map(|i| format_name(i, prefs).chars().count())
        .max()
        .unwrap_or(0);

    let max_birth: usize = if prefs.show.birth {
        genrep
            .individuals
            .values()
            .filter(|i| i.in_scope)
            .filter_map(|i| {
                i.birth.as_ref().and_then(|e| {
                    format_event(
                        &prefs.format.birth,
                        e.date.as_ref(),
                        e.place.as_deref(),
                        &prefs.format.date_qualifiers,
                    )
                })
            })
            .map(|s| s.chars().count())
            .max()
            .unwrap_or(0)
    } else {
        0
    };

    let max_death: usize = if prefs.show.death {
        genrep
            .individuals
            .values()
            .filter(|i| i.in_scope)
            .filter_map(|i| {
                i.death.as_ref().and_then(|e| {
                    format_event(
                        &prefs.format.death,
                        e.date.as_ref(),
                        e.place.as_deref(),
                        &prefs.format.date_qualifiers,
                    )
                })
            })
            .map(|s| s.chars().count())
            .max()
            .unwrap_or(0)
    } else {
        0
    };

    let max_marr: usize = if prefs.show.marriage {
        genrep
            .individuals
            .values()
            .filter(|i| i.in_scope)
            .filter_map(|i| {
                i.fams.iter().find_map(|fid| {
                    genrep.families.get(fid.as_str()).and_then(|fam| {
                        fam.marriage.as_ref().and_then(|e| {
                            format_event(
                                &prefs.format.marriage,
                                e.date.as_ref(),
                                e.place.as_deref(),
                                &prefs.format.date_qualifiers,
                            )
                        })
                    })
                })
            })
            .map(|s| s.chars().count())
            .max()
            .unwrap_or(0)
    } else {
        0
    };

    let name_end = id_col + gen_pfx + max_name;
    let mut total = name_end;
    if max_birth > 0 {
        total += gap + max_birth;
    }
    if max_death > 0 {
        total += gap + max_death;
    }
    if max_marr > 0 {
        total += gap + max_marr;
    }
    total
}

/// Configuration for note line allocation in the layout phase.
#[derive(Copy, Clone)]
struct NoteWrap {
    show: bool,
    max_x_chars: usize,
    id_col_chars: usize,
    indent_chars: usize,
}

impl NoteWrap {
    /// Available chars for note text content at the given depth
    /// (after subtracting indentation and the `| ` prefix).
    fn avail(&self, depth: usize) -> usize {
        self.max_x_chars
            .saturating_sub(self.id_col_chars + depth * self.indent_chars + 6)
    }
}

fn visit(
    id: &str,
    depth: usize,
    spacing: usize,
    note_wrap: NoteWrap,
    line: &mut usize,
    geo_map: &mut HashMap<String, SimpleGeo>,
    visited: &mut HashSet<String>,
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
    let note_lines = if note_wrap.show {
        count_note_lines(&indi.notes, note_wrap.avail(depth))
    } else {
        0
    };
    *line += 1 + note_lines + spacing;

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
                        let spouse_note_lines = if note_wrap.show {
                            count_note_lines(&s.notes, note_wrap.avail(depth))
                        } else {
                            0
                        };
                        *line += 1 + spouse_note_lines + spacing;
                    }
                }
            }
        }

        let children = fam.children_ids.clone();
        for child_id in &children {
            visit(
                child_id,
                depth + 1,
                spacing,
                note_wrap,
                line,
                geo_map,
                visited,
                genrep,
            );
        }
    }
}

fn layout_descendants(
    genrep: &Genrep,
    root: &str,
    spacing: usize,
    note_wrap: NoteWrap,
    geo_map: &mut HashMap<String, SimpleGeo>,
) {
    let mut visited: HashSet<String> = HashSet::new();
    let mut line: usize = 0;
    visit(
        root,
        0,
        spacing,
        note_wrap,
        &mut line,
        geo_map,
        &mut visited,
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
    note_wrap: NoteWrap,
    geo_map: &mut HashMap<String, SimpleGeo>,
) {
    let mut visited = HashSet::new();
    let mut ordered: Vec<(String, usize)> = Vec::new();
    in_order(root, 0, genrep, &mut visited, &mut ordered);

    // First pass: assign line numbers, expanding gaps by vert_spacing and note lines
    let mut id_to_line: HashMap<String, usize> = HashMap::new();
    let mut running_line = 0usize;
    for (id, depth) in &ordered {
        let line_num = running_line;
        id_to_line.insert(id.clone(), line_num);
        geo_map.insert(
            id.clone(),
            SimpleGeo {
                line: line_num,
                indent: *depth,
                generation: depth + 1,
                is_spouse: false,
                connectors_above: Vec::new(),
                connectors_below: Vec::new(),
            },
        );
        let note_lines = if note_wrap.show {
            genrep
                .individuals
                .get(id.as_str())
                .map_or(0, |i| count_note_lines(&i.notes, note_wrap.avail(*depth)))
        } else {
            0
        };
        running_line += 1 + note_lines + spacing;
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
        let note_wrap = NoteWrap {
            show: prefs.show.notes,
            max_x_chars: estimate_max_x_chars(genrep, prefs),
            id_col_chars: if prefs.show.id { 6 } else { 0 },
            indent_chars: prefs.layout.simple.indent as usize,
        };
        match dir {
            d if matches_direction(d, "descendants") => {
                if let Some(root) = resolve_root_id(genrep, prefs) {
                    layout_descendants(genrep, &root, spacing, note_wrap, &mut geo_map);
                }
            }
            d if matches_direction(d, "ancestors") || matches_direction(d, "pedigree") => {
                if let Some(root) = resolve_root_id(genrep, prefs) {
                    layout_ancestors(genrep, &root, spacing, note_wrap, &mut geo_map);
                }
            }
            d if matches_direction(d, "forest") => {
                eprintln!("warning: forest direction is not yet implemented; output will be empty");
            }
            other => {
                eprintln!("warning: unknown direction {other:?}, falling back to descendants");
                if let Some(root) = resolve_root_id(genrep, prefs) {
                    layout_descendants(genrep, &root, spacing, note_wrap, &mut geo_map);
                }
            }
        }

        if !prefs.show.last_gen_spouses {
            let max_non_spouse_gen = geo_map
                .values()
                .filter(|g| !g.is_spouse)
                .map(|g| g.generation)
                .max()
                .unwrap_or(0);
            geo_map.retain(|_, g| !(g.is_spouse && g.generation == max_non_spouse_gen));
        }

        let mut out_individuals = HashMap::new();
        for (id, indi) in &genrep.individuals {
            let geo = geo_map.get(id).cloned();
            out_individuals.insert(id.clone(), copy_individual(indi, geo));
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

    let max_x_chars = (max_x_px / char_width_px).floor() as usize;

    let max_line = entries
        .iter()
        .map(|(_, indi, g)| {
            let nl = if prefs.show.notes && !indi.notes.is_empty() {
                let x_note = id_col_px + g.indent as f64 * indent_px + 4.0 * char_width_px;
                let note_x_chars = (x_note / char_width_px) as usize;
                let avail = max_x_chars.saturating_sub(note_x_chars + 2);
                count_note_lines(&indi.notes, avail)
            } else {
                0
            };
            g.line + nl
        })
        .max()
        .unwrap_or(0);

    // ── Build primitives ──────────────────────────────────────────────────────
    let mut primitives: Vec<Primitive> = Vec::new();

    for (id, indi, geo) in &entries {
        let top_y = geo.line as f64 * line_height_px;
        let x_name = id_col_px + geo.indent as f64 * indent_px + gen_prefix_px(geo.generation);
        let gpx = gen_prefix_px(geo.generation);
        let is_highlighted = highlighted_ids.contains(*id);

        // Individual ID — emitted first so the text backend writes it at col 0
        // before any other content fills the line.
        if prefs.show.id {
            let id_str = format!("{id}");
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

        // ── Note lines ────────────────────────────────────────────────────────
        if prefs.show.notes && !indi.notes.is_empty() {
            let x_note = id_col_px + geo.indent as f64 * indent_px + 4.0 * char_width_px;
            // Vertical bar centered on the first character of the name (SVG backend only).
            let bar_x = id_col_px
                + geo.indent as f64 * indent_px
                + gen_prefix_px(geo.generation)
                + 0.5 * char_width_px;
            let note_x_chars = (x_note / char_width_px) as usize;
            let avail = max_x_chars.saturating_sub(note_x_chars + 2);
            let mut note_line_offset = 1usize;
            for note in &indi.notes {
                if note.trim().is_empty() {
                    continue;
                }
                let bar_start_offset = note_line_offset;
                for raw_line in note.lines() {
                    if raw_line.trim().is_empty() {
                        note_line_offset += 1;
                        continue;
                    }
                    for wrapped in word_wrap(raw_line, avail) {
                        let note_y = (geo.line + note_line_offset) as f64 * line_height_px;
                        let content = format!("| {wrapped}");
                        let w = content.chars().count() as f64 * char_width_px;
                        primitives.push(Primitive::Text(TextPrimitive {
                            content,
                            bbox: Rect {
                                x: x_note,
                                y: note_y,
                                w,
                                h: line_height_px,
                            },
                            align: TextAlign::Left,
                            attrs: vec![TextAttr::NoteText],
                        }));
                        note_line_offset += 1;
                    }
                }
                if note_line_offset > bar_start_offset {
                    let underline_extra = line_height_px * 0.16 + 1.0;
                    primitives.push(Primitive::NoteBar(crate::scene::NoteBarPrimitive {
                        x: bar_x,
                        top_y: (geo.line + bar_start_offset) as f64 * line_height_px
                            + underline_extra,
                        bottom_y: (geo.line + note_line_offset) as f64 * line_height_px
                            + underline_extra,
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

    /// GEDCOM with one root who has a 2-line note, and a child.
    /// With show.notes = true the child must be placed 2 lines lower than without notes.
    /// Note lines are intentionally short so they never wrap regardless of chart width.
    const GEDCOM_WITH_NOTE: &str = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME Root /Person/
1 SEX M
1 NOTE Line one
2 CONT Line two
1 FAMS @F1@
0 @I2@ INDI
1 NAME Child /Person/
1 SEX M
1 FAMC @F1@
0 @F1@ FAM
1 HUSB @I1@
1 CHIL @I2@
0 TRLR
";

    #[test]
    fn test_notes_allocate_lines() {
        let mut genrep_notes = parse_str(GEDCOM_WITH_NOTE).unwrap();
        compute_scope(&mut genrep_notes, Some("I1"), "descendants", None);
        let mut genrep_no_notes = parse_str(GEDCOM_WITH_NOTE).unwrap();
        compute_scope(&mut genrep_no_notes, Some("I1"), "descendants", None);

        let mut prefs_notes = Prefs::default();
        prefs_notes.scope.root = "I1".to_string();
        prefs_notes.scope.direction = "descendants".to_string();
        prefs_notes.show.notes = true;

        let mut prefs_no_notes = Prefs::default();
        prefs_no_notes.scope.root = "I1".to_string();
        prefs_no_notes.scope.direction = "descendants".to_string();
        prefs_no_notes.show.notes = false;

        let result_notes = SimpleLayout.compute(&genrep_notes, &prefs_notes).unwrap();
        let result_no_notes = SimpleLayout
            .compute(&genrep_no_notes, &prefs_no_notes)
            .unwrap();

        let child_line_notes = result_notes.individuals["I2"].geo.as_ref().unwrap().line;
        let child_line_no_notes = result_no_notes.individuals["I2"].geo.as_ref().unwrap().line;

        assert_eq!(
            child_line_notes,
            child_line_no_notes + 2,
            "child should be 2 lines lower when root has a 2-line note"
        );
    }

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
