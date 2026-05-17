//! Plain-text output backend.

use crate::backend::Renderer;
use crate::layout::LayoutOutput;
use crate::preferences::Prefs;
use crate::scene::{Primitive, Scene, TextAttr};
use crate::text_metrics::{CHAR_WIDTH_RATIO, FONT_SIZE, LINE_HEIGHT, parsed_font};
use std::collections::{BTreeMap, HashSet};

// ── String helpers ────────────────────────────────────────────────────────────

fn display_len(s: &str) -> usize {
    s.chars().count()
}

fn write_at_col(s: &mut String, col: usize, text: &str, dot_leaders: bool) {
    let cur = display_len(s);
    if cur <= col {
        let gap = col - cur;
        if dot_leaders && gap >= 4 {
            s.push(' ');
            s.extend(std::iter::repeat_n('.', gap - 2));
            s.push(' ');
        } else {
            s.extend(std::iter::repeat_n(' ', gap));
        }
    } else {
        s.push_str("  ");
    }
    s.push_str(text);
}

fn pad_line_to(s: &mut String, min_len: usize) {
    if s.len() < min_len {
        s.extend(std::iter::repeat_n(' ', min_len - s.len()));
    }
}

fn set_char_at(s: &mut String, byte_pos: usize, ch: char) {
    if byte_pos < s.len() {
        s.replace_range(byte_pos..byte_pos + 1, &ch.to_string());
    }
}

// ── Text grid for boxed_couples rendering ─────────────────────────────────────

const PRIORITY_CONNECTOR: u8 = 1;
const PRIORITY_INTERSECTION: u8 = 2;
const PRIORITY_BOX: u8 = 3;
const PRIORITY_TEXT: u8 = 4;
const PRIORITY_T_JUNCTION: u8 = 5; // overwrites box borders at connector endpoints

#[derive(Clone)]
struct Cell {
    ch: char,
    priority: u8,
}

struct TextGrid {
    rows: usize,
    cols: usize,
    data: Vec<Vec<Option<Cell>>>,
}

impl TextGrid {
    fn new(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            data: vec![vec![None; cols]; rows],
        }
    }

    fn set(&mut self, row: usize, col: usize, ch: char, priority: u8) {
        if row >= self.rows || col >= self.cols {
            return;
        }
        match &self.data[row][col] {
            Some(cell) if cell.priority >= priority => {}
            _ => {
                self.data[row][col] = Some(Cell { ch, priority });
            }
        }
    }

    fn to_rendered_string(self) -> String {
        let mut lines: Vec<String> = self
            .data
            .iter()
            .map(|row| {
                let s: String = row
                    .iter()
                    .map(|c| c.as_ref().map(|cell| cell.ch).unwrap_or(' '))
                    .collect();
                s.trim_end_matches(' ').to_string()
            })
            .collect();
        while lines.last().map_or(false, |l| l.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }
}

fn display_to_row(display_y: f64, line_height_px: f64) -> usize {
    (display_y / line_height_px).round() as usize
}

fn display_to_col(display_x: f64, char_width_px: f64) -> usize {
    (display_x / char_width_px).round() as usize
}

fn draw_box(
    grid: &mut TextGrid,
    bbox: &crate::scene::Rect,
    line_height_px: f64,
    char_width_px: f64,
) {
    let row0 = display_to_row(bbox.y, line_height_px);
    let col0 = display_to_col(bbox.x, char_width_px);
    let row1 = display_to_row(bbox.y + bbox.h, line_height_px);
    let col1 = display_to_col(bbox.x + bbox.w, char_width_px);

    let w = col1.saturating_sub(col0);
    let h = row1.saturating_sub(row0);
    if w < 2 || h < 2 {
        return;
    }

    // Top edge
    grid.set(row0, col0, '\u{250C}', PRIORITY_BOX); // ┌
    grid.set(row0, col0 + w - 1, '\u{2510}', PRIORITY_BOX); // ┐
    for c in col0 + 1..col0 + w - 1 {
        grid.set(row0, c, '\u{2500}', PRIORITY_BOX); // ─
    }

    // Middle rows
    for r in row0 + 1..row0 + h - 1 {
        grid.set(r, col0, '\u{2502}', PRIORITY_BOX); // │
        grid.set(r, col0 + w - 1, '\u{2502}', PRIORITY_BOX); // │
    }

    // Bottom edge
    grid.set(row0 + h - 1, col0, '\u{2514}', PRIORITY_BOX); // └
    grid.set(row0 + h - 1, col0 + w - 1, '\u{2518}', PRIORITY_BOX); // ┘
    for c in col0 + 1..col0 + w - 1 {
        grid.set(row0 + h - 1, c, '\u{2500}', PRIORITY_BOX); // ─
    }
}

fn draw_text_on_grid(
    grid: &mut TextGrid,
    text: &crate::scene::TextPrimitive,
    _line_height_px: f64,
    char_width_px: f64,
    content: &str,
    row: usize,
) {
    if row >= grid.rows {
        return;
    }

    let text_len = content.chars().count();
    let max_cols = grid.cols.max(1);

    let start_col = match text.align {
        crate::scene::TextAlign::Left => display_to_col(text.bbox.x, char_width_px),
        crate::scene::TextAlign::Center => {
            let center = display_to_col(text.bbox.x + text.bbox.w / 2.0, char_width_px);
            center.saturating_sub(text_len / 2)
        }
        crate::scene::TextAlign::Right => {
            let right = display_to_col(text.bbox.x + text.bbox.w, char_width_px);
            right.saturating_sub(text_len)
        }
    };

    let chars: Vec<char> = content.chars().collect();
    let available = if start_col < max_cols {
        max_cols.saturating_sub(start_col)
    } else {
        0
    };

    let mut end = chars.len();
    if available < chars.len() {
        let suffix = "...";
        if available < suffix.len() {
            end = available;
        } else {
            end = available.saturating_sub(suffix.len());
        }
    }

    for (i, &ch) in chars[..end].iter().enumerate() {
        let col = start_col + i;
        if col < grid.cols {
            grid.set(row, col, ch, PRIORITY_TEXT);
        }
    }
    if end < chars.len() {
        for (i, ch) in "...".chars().enumerate() {
            let col = start_col + end + i;
            if col < grid.cols {
                grid.set(row, col, ch, PRIORITY_TEXT);
            }
        }
    }
}

fn clear_row_segment(grid: &mut TextGrid, row: usize, start_col: usize, end_col: usize) {
    if row >= grid.rows {
        return;
    }
    for c in start_col..end_col {
        if c < grid.cols {
            grid.data[row][c] = None;
        }
    }
}

fn endpoint_char(is_parent: bool, downward: bool) -> char {
    match (is_parent, downward) {
        (true, true) => '\u{252C}',   // parent exits bottom of box → ┬
        (true, false) => '\u{2534}',  // parent exits top of box    → ┴
        (false, true) => '\u{2534}',  // child enters top of box    → ┴
        (false, false) => '\u{252C}', // child enters bottom of box → ┬
    }
}

fn bar_char(
    is_parent: bool,
    is_child: bool,
    is_left: bool,
    is_right: bool,
    downward: bool,
) -> char {
    if is_parent && is_child {
        if is_left {
            '\u{251C}' // ├
        } else if is_right {
            '\u{2524}' // ┤
        } else {
            '\u{253C}' // ┼
        }
    } else if is_parent {
        if is_left {
            if downward { '\u{2514}' } else { '\u{250C}' } // └ or ┌
        } else if is_right {
            if downward { '\u{2518}' } else { '\u{2510}' } // ┘ or ┐
        } else {
            if downward { '\u{2534}' } else { '\u{252C}' } // ┴ or ┬
        }
    } else {
        // is_child only
        if is_left {
            if downward { '\u{250C}' } else { '\u{2514}' } // ┌ or └
        } else if is_right {
            if downward { '\u{2510}' } else { '\u{2518}' } // ┐ or ┘
        } else {
            if downward { '\u{252C}' } else { '\u{2534}' } // ┬ or ┴
        }
    }
}

fn draw_connector_on_grid(
    grid: &mut TextGrid,
    conn: &crate::scene::ConnectorPrimitive,
    line_height_px: f64,
    char_width_px: f64,
    downward: bool,
) {
    if conn.parent_points.is_empty() || conn.child_points.is_empty() {
        return;
    }

    let parent_col = display_to_col(conn.parent_points[0].x, char_width_px);
    // "bottom border" connector endpoints carry coordinate bbox.y + bbox.h, which
    // display_to_row maps to row1 (one past the bottom border drawn by draw_box).
    // Subtract 1 so the T-junction lands on the actual border row.
    let parent_row = {
        let raw = display_to_row(conn.parent_points[0].y, line_height_px);
        if downward { raw.saturating_sub(1) } else { raw }
    };

    let child_cols: Vec<usize> = conn
        .child_points
        .iter()
        .map(|c| display_to_col(c.x, char_width_px))
        .collect();
    let child_rows: Vec<usize> = conn
        .child_points
        .iter()
        .map(|c| {
            let raw = display_to_row(c.y, line_height_px);
            if downward { raw } else { raw.saturating_sub(1) }
        })
        .collect();

    // Single child: straight vertical line only, no horizontal bar
    if child_cols.len() == 1 {
        let child_col = child_cols[0];
        let child_row = child_rows[0];

        // T-junctions at the box borders (overwrite with PRIORITY_T_JUNCTION)
        if parent_row < grid.rows {
            grid.set(
                parent_row,
                parent_col,
                endpoint_char(true, downward),
                PRIORITY_T_JUNCTION,
            );
        }
        if child_row < grid.rows {
            grid.set(
                child_row,
                child_col,
                endpoint_char(false, downward),
                PRIORITY_T_JUNCTION,
            );
        }

        // Vertical between the two endpoint rows (exclusive at both ends)
        let (r_start, r_end) = if downward {
            (parent_row.saturating_add(1), child_row)
        } else {
            (child_row.saturating_add(1), parent_row)
        };
        for r in r_start..r_end {
            if r < grid.rows {
                grid.set(r, parent_col, '\u{2502}', PRIORITY_CONNECTOR); // │
            }
        }
        return;
    }

    // Multiple children: bar at midpoint
    let mut all_cols = vec![parent_col];
    all_cols.extend(&child_cols);
    let bar_left = *all_cols.iter().min().unwrap();
    let bar_right = *all_cols.iter().max().unwrap();

    let extreme_child_row = if downward {
        child_rows.iter().copied().min().unwrap_or(0)
    } else {
        child_rows.iter().copied().max().unwrap_or(0)
    };
    let bar_row = (parent_row + extreme_child_row) / 2;
    if bar_row >= grid.rows {
        return;
    }

    // T-junction at parent box border
    if parent_row < grid.rows {
        grid.set(
            parent_row,
            parent_col,
            endpoint_char(true, downward),
            PRIORITY_T_JUNCTION,
        );
    }

    // Parent vertical (between parent endpoint and bar, exclusive at both ends)
    if downward {
        for r in (parent_row + 1)..bar_row {
            if r < grid.rows {
                grid.set(r, parent_col, '\u{2502}', PRIORITY_CONNECTOR); // │
            }
        }
    } else {
        for r in (bar_row + 1)..parent_row {
            if r < grid.rows {
                grid.set(r, parent_col, '\u{2502}', PRIORITY_CONNECTOR); // │
            }
        }
    }

    // Horizontal bar
    for c in bar_left..=bar_right {
        if c < grid.cols {
            grid.set(bar_row, c, '\u{2500}', PRIORITY_CONNECTOR); // ─
        }
    }

    // Child verticals + T-junctions at child box borders
    for (i, &cc) in child_cols.iter().enumerate() {
        let cr = child_rows[i];

        // T-junction at child box border
        if cr < grid.rows {
            grid.set(cr, cc, endpoint_char(false, downward), PRIORITY_T_JUNCTION);
        }

        if downward {
            for r in (bar_row + 1)..cr {
                if r < grid.rows {
                    grid.set(r, cc, '\u{2502}', PRIORITY_CONNECTOR); // │
                }
            }
        } else {
            for r in (cr + 1)..bar_row {
                if r < grid.rows {
                    grid.set(r, cc, '\u{2502}', PRIORITY_CONNECTOR); // │
                }
            }
        }
    }

    // Intersection characters at bar row
    let child_set: HashSet<usize> = child_cols.iter().copied().collect();
    for c in bar_left..=bar_right {
        if c >= grid.cols {
            continue;
        }
        let is_parent = c == parent_col;
        let is_child = child_set.contains(&c);
        if !is_parent && !is_child {
            continue;
        }
        let is_left = c == bar_left;
        let is_right = c == bar_right;
        let ch = bar_char(is_parent, is_child, is_left, is_right, downward);
        grid.set(bar_row, c, ch, PRIORITY_INTERSECTION);
    }
}
// ── Scene → text-grid rendering ───────────────────────────────────────────────

fn render_boxed_couples_text(scene: &Scene, prefs: &Prefs) -> String {
    if scene.primitives.is_empty() {
        return String::new();
    }

    let (_, font_size) = parsed_font(&prefs.output.style.fonts.names);
    let line_height_px = font_size * (LINE_HEIGHT / FONT_SIZE);
    let char_width_px = font_size * CHAR_WIDTH_RATIO;

    let total_rows = ((scene.canvas_bounds.h / line_height_px).ceil() as usize).max(1);
    let total_cols = ((scene.canvas_bounds.w / char_width_px).ceil() as usize).max(1);
    let mut grid = TextGrid::new(total_rows, total_cols);

    // Flatten Group primitives so the three render passes work on leaf primitives
    let flat: Vec<&Primitive> = flatten_primitives(&scene.primitives);

    // Collect boxes in order
    let boxes: Vec<&crate::scene::BoxPrimitive> = flat
        .iter()
        .filter_map(|p| {
            if let Primitive::Box(b) = p {
                Some(b)
            } else {
                None
            }
        })
        .collect();

    // Pass 1: draw box outlines
    for box_prim in &boxes {
        draw_box(&mut grid, &box_prim.bbox, line_height_px, char_width_px);
    }

    // Pass 2: place text per box, grouping by Y band so same-height texts share a row
    for box_prim in &boxes {
        let bbox = &box_prim.bbox;
        let box_row0 = display_to_row(bbox.y, line_height_px);
        let box_row1 = display_to_row(bbox.y + bbox.h, line_height_px);
        let box_col0 = display_to_col(bbox.x, char_width_px);
        let box_col1 = display_to_col(bbox.x + bbox.w, char_width_px);

        // Find text primitives contained within this box
        let texts: Vec<&crate::scene::TextPrimitive> = flat
            .iter()
            .filter_map(|p| {
                if let Primitive::Text(t) = p {
                    if t.bbox.x >= bbox.x
                        && t.bbox.y >= bbox.y
                        && (t.bbox.x + t.bbox.w) <= (bbox.x + bbox.w)
                        && (t.bbox.y + t.bbox.h) <= (bbox.y + bbox.h)
                    {
                        Some(t)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        // Group texts by Y band: texts at the same Y (within line_height_px) share a row.
        // This handles wide boxes where two spouses have the same Y for each data slot.
        // Group by pixel-exact Y position. Texts at the same Y (within 0.5 px) share a row.
        // This handles wide boxes where two spouses have the same Y for each data slot,
        // while keeping same-column texts (which differ by ≥ date_font_size ≈ 10 px) separate.
        let mut y_groups: BTreeMap<i64, Vec<&crate::scene::TextPrimitive>> = BTreeMap::new();
        for text in &texts {
            let key = text.bbox.y.round() as i64;
            y_groups.entry(key).or_default().push(text);
        }

        // Assign text rows sequentially from box_row0 + 1.
        // MarriageData gets a blank row before and after it.
        let mut current_row = box_row0 + 1;
        for (_key, group) in &y_groups {
            if current_row >= grid.rows || current_row >= box_row1 {
                break;
            }

            let has_marriage = group
                .iter()
                .any(|t| t.attrs.contains(&TextAttr::MarriageData));

            if has_marriage {
                current_row += 1; // blank row before marriage
                if current_row >= grid.rows {
                    break;
                }
            }

            // Clear interior of this row once, preserving left/right border columns
            clear_row_segment(
                &mut grid,
                current_row,
                box_col0 + 1,
                box_col1.saturating_sub(1),
            );

            for text in group {
                let mut content = text.content.clone();
                if text.attrs.contains(&TextAttr::MarriageData) && content.starts_with("⚭ ") {
                    content = content[3..].to_string();
                }
                if text.attrs.contains(&TextAttr::Highlighted)
                    && prefs.output.style.text.highlights.fallback == "uppercase"
                {
                    content = content.to_uppercase();
                }
                draw_text_on_grid(
                    &mut grid,
                    text,
                    line_height_px,
                    char_width_px,
                    &content,
                    current_row,
                );
            }

            current_row += 1;

            if has_marriage {
                current_row += 1; // blank row after marriage
            }
        }
    }

    // Pass 3: draw connectors
    // Direction is derived from root_pos (the single source of truth), not from row positions.
    let root_pos_bottom = prefs.layout.root_pos.is_empty()
        || prefs
            .layout
            .root_pos
            .to_ascii_lowercase()
            .starts_with("bot");
    // root at bottom → children appear above root in display space → upward connectors
    let downward = !root_pos_bottom;
    for prim in &flat {
        if let Primitive::Connector(c) = prim {
            draw_connector_on_grid(&mut grid, c, line_height_px, char_width_px, downward);
        }
    }

    grid.to_rendered_string()
}
/// Recursively flatten `Primitive::Group` containers into a flat list of leaf primitives.
fn flatten_primitives(prims: &[Primitive]) -> Vec<&Primitive> {
    let mut result = Vec::new();
    for p in prims {
        if let Primitive::Group(g) = p {
            result.extend(flatten_primitives(&g.children));
        } else {
            result.push(p);
        }
    }
    result
}

fn render_scene_text(scene: &Scene, prefs: &Prefs, fallback_shift: usize) -> String {
    let (_, font_size) = parsed_font(&prefs.output.style.fonts.names);
    let line_height_px = font_size * (LINE_HEIGHT / FONT_SIZE);
    let char_width_px = font_size * CHAR_WIDTH_RATIO;
    let total_lines = ((scene.canvas_bounds.h / line_height_px).ceil() as usize).max(1);
    let mut lines: Vec<String> = vec![String::new(); total_lines];

    let dot_leaders = prefs.output.style.dot_leaders;

    // First pass: identify lines that contain highlighted text.
    let highlighted_lines: std::collections::HashSet<usize> = {
        let mut set = std::collections::HashSet::new();
        for prim in &scene.primitives {
            if let Primitive::Text(t) = prim {
                if t.attrs.contains(&TextAttr::Highlighted) {
                    let li = ((t.bbox.y / line_height_px).round() as usize).min(total_lines - 1);
                    set.insert(li);
                }
            }
        }
        set
    };

    // Pre-pass: write IndividualId primitives first so connectors from ancestor rows
    // cannot displace them — IDs must always appear at column 0 (or fallback_shift).
    for prim in &scene.primitives {
        if let Primitive::Text(t) = prim {
            if !t.attrs.contains(&TextAttr::IndividualId) {
                continue;
            }
            let line_idx = ((t.bbox.y / line_height_px).round() as usize).min(total_lines - 1);
            let col = (t.bbox.x / char_width_px).round() as usize + fallback_shift;
            let is_highlighted = t.attrs.contains(&TextAttr::Highlighted);
            let content =
                if is_highlighted && prefs.output.style.text.highlights.fallback == "uppercase" {
                    t.content.to_uppercase()
                } else {
                    t.content.clone()
                };
            write_at_col(&mut lines[line_idx], col, &content, false);
        }
    }

    // Main pass: all remaining primitives in emission order (preserving the
    // interleaved connector / non-ID text ordering that the simple layout relies on).
    for prim in &scene.primitives {
        match prim {
            Primitive::Text(t) => {
                if t.attrs.contains(&TextAttr::IndividualId) {
                    continue; // already written in pre-pass
                }
                let line_idx = ((t.bbox.y / line_height_px).round() as usize).min(total_lines - 1);
                let col = (t.bbox.x / char_width_px).round() as usize + fallback_shift;
                let use_dot_leaders = dot_leaders
                    && matches!(
                        t.attrs
                            .iter()
                            .find(|a| !matches!(a, TextAttr::Highlighted))
                            .unwrap_or(&TextAttr::IndividualName),
                        TextAttr::BirthData | TextAttr::DeathData | TextAttr::MarriageData
                    );
                let is_highlighted = t.attrs.contains(&TextAttr::Highlighted);
                let content = if is_highlighted
                    && prefs.output.style.text.highlights.fallback == "uppercase"
                {
                    t.content.to_uppercase()
                } else {
                    t.content.clone()
                };
                write_at_col(&mut lines[line_idx], col, &content, use_dot_leaders);
            }
            Primitive::Connector(c) => {
                if c.parent_points.is_empty() || c.child_points.is_empty() {
                    continue;
                }
                let x_col =
                    (c.parent_points[0].x / char_width_px - 1.0).round() as usize + fallback_shift;
                let y_parent = (c.parent_points[0].y / line_height_px).round() as usize;
                let y_child = (c.child_points[0].y / line_height_px).round() as usize;
                let (y_start, y_end) = if y_parent > y_child {
                    (y_child, y_parent)
                } else {
                    (y_parent, y_child)
                };
                for row in y_start..y_end {
                    if row < total_lines {
                        pad_line_to(&mut lines[row], x_col + 1);
                        set_char_at(&mut lines[row], x_col, '│');
                    }
                }
            }
            Primitive::Box(_)
            | Primitive::Wedge(_)
            | Primitive::FancyText(_)
            | Primitive::FancyConn(_)
            | Primitive::Group(_) => {}
        }
    }

    // Replace leading spaces with fallback marker on highlighted lines.
    if fallback_shift > 0 {
        let fallback_str = &prefs.output.style.text.highlights.fallback;
        let marker_len = fallback_str.len();
        for &line_idx in &highlighted_lines {
            if line_idx < lines.len() {
                let current = &lines[line_idx];
                if current.len() >= marker_len {
                    let rest = &current.as_str()[marker_len..];
                    lines[line_idx] = format!("{fallback_str}{rest}");
                } else {
                    lines[line_idx] = fallback_str.to_string();
                }
            }
        }
    }

    // Trim trailing empty lines
    while lines.last().map_or(false, |l| l.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
}

// ── Public API ────────────────────────────────────────────────────────────────

pub struct TextRenderer;

impl Renderer for TextRenderer {
    fn render(
        &self,
        output: &LayoutOutput,
        prefs: &Prefs,
        writer: &mut dyn std::io::Write,
    ) -> anyhow::Result<()> {
        if output.is_fan() {
            anyhow::bail!("fan layout does not support text output; use --svg or --pdf");
        }
        if output.is_fancy() {
            anyhow::bail!("fancy layout does not support text output; use --svg or --pdf");
        }

        // Title
        if !prefs.output.text.title.is_empty() {
            let title = crate::backend::expand_title_template(&prefs.output.text.title, prefs);
            writeln!(writer, "{title}")?;
            writeln!(writer)?;
        }

        // Body
        if output.is_boxed_couples() {
            let body = render_boxed_couples_text(output.scene(), prefs);
            write!(writer, "{}", body)?;
        } else {
            let scene = output.scene();
            let fallback_shift = if !prefs.files.highlights.is_empty()
                && prefs.output.style.text.highlights.fallback != "uppercase"
            {
                prefs.output.style.text.highlights.fallback.len() + 1
            } else {
                0
            };
            let body = render_scene_text(scene, prefs, fallback_shift);
            writeln!(writer, "{body}")?;
        }

        // Copyright
        if !prefs.output.text.copyright.is_empty() {
            writeln!(writer)?;
            writeln!(writer, "{}", prefs.output.text.copyright)?;
        }

        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::run_layout;
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
1 MARR
2 DATE 4 APR 1843
2 PLAC London
0 TRLR
";

    fn make_prefs() -> Prefs {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.layout.layout_type = "simple".into();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.generation_num = true;
        prefs.show.birth = true;
        prefs.format.birth = "* {date}, {location}".into();
        prefs.show.death = true;
        prefs.format.death = "× {date}, {location}".into();
        prefs.show.marriage = true;
        prefs.format.marriage = "⚭ {date}, {location}".into();
        prefs.output.text.title = "".into();
        prefs.output.text.copyright = "".into();
        prefs
    }

    fn render_text(prefs: &Prefs) -> Vec<String> {
        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        let layout_out = run_layout(&genrep, prefs).unwrap();
        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&layout_out, prefs, &mut buf).unwrap();
        String::from_utf8(buf)
            .unwrap()
            .lines()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn test_correct_names_and_order() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        assert!(
            lines[0].contains("John") && lines[0].contains("Ancestor"),
            "line 0 should be John: {:?}",
            lines[0]
        );
        assert!(
            lines[1].contains("Jane"),
            "line 1 should be Jane (spouse): {:?}",
            lines[1]
        );
        assert!(
            lines[2].contains("Paul"),
            "line 2 should be Paul (child): {:?}",
            lines[2]
        );
    }

    #[test]
    fn test_birth_data_on_root_line() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        assert!(
            lines[0].contains("1 JAN 1812"),
            "birth date missing: {:?}",
            lines[0]
        );
        assert!(
            lines[0].contains("London"),
            "birth place missing: {:?}",
            lines[0]
        );
    }

    #[test]
    fn test_marriage_on_spouse_line() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        assert!(
            lines[1].contains("4 APR 1843"),
            "marriage date missing: {:?}",
            lines[1]
        );
    }

    #[test]
    fn test_no_birth_prefix_when_absent() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        assert!(
            !lines[1].contains("* "),
            "unexpected birth prefix on spouse line: {:?}",
            lines[1]
        );
    }

    #[test]
    fn test_spouse_name_aligned_with_non_spouse() {
        // With generation numbers on, spouse names must start at the same column
        // as the non-spouse name (i.e. after the "N. " prefix width).
        let prefs = make_prefs(); // generation_num = true
        let lines = render_text(&prefs);
        // John (non-spouse) line starts with "1. John…"
        let root_name_col = lines[0].find("John").expect("John not on line 0");
        // Jane (spouse) line should start "   Jane…" — same column as John
        let spouse_name_col = lines[1].find("Jane").expect("Jane not on line 1");
        assert_eq!(
            root_name_col, spouse_name_col,
            "spouse name column ({spouse_name_col}) must equal non-spouse name column ({root_name_col});\n  line0: {:?}\n  line1: {:?}",
            lines[0], lines[1]
        );
    }

    #[test]
    fn test_column_alignment() {
        let prefs = make_prefs();
        let lines = render_text(&prefs);
        let birth_pos = lines[0].find("* ").expect("birth not found on line 0");
        assert!(
            birth_pos > "1. John Ancestor".len(),
            "birth should be after name: {:?}",
            lines[0]
        );
    }

    #[test]
    fn test_sex_unknown_column_aligned() {
        // Regression: unknown sex previously left a trailing space in the formatted
        // name, inflating the birth column for everyone by 1.
        const GED: &str = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME Big /Nameperson/
1 BIRT
2 DATE 1 JAN 1900
1 FAMS @F1@
0 @I2@ INDI
1 NAME Al /Bo/
1 SEX M
1 BIRT
2 DATE 2 FEB 1901
1 FAMS @F1@
0 @F1@ FAM
1 HUSB @I2@
1 WIFE @I1@
0 TRLR
";
        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I2"), "descendants", Some(2));
        let mut prefs = Prefs::default();
        prefs.scope.root = "I2".into();
        prefs.scope.direction = "descendants".into();
        prefs.layout.layout_type = "simple".into();
        prefs.format.individual = "{firstname} {lastname} {sex}".into();
        prefs.show.generation_num = false;
        prefs.show.birth = true;
        prefs.format.birth = "* {date}".into();
        prefs.show.death = false;
        prefs.show.marriage = false;
        prefs.show.last_gen_spouses = true;

        let layout_out = run_layout(&genrep, &prefs).unwrap();
        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&layout_out, &prefs, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = text.lines().collect();

        // Use character counts (display columns), not byte offsets.
        // ♂ is 3 bytes but 1 display column — the test must measure visual alignment.
        let char_positions: Vec<usize> = lines
            .iter()
            .filter_map(|l| l.find("* ").map(|b| l[..b].chars().count()))
            .collect();
        assert_eq!(
            char_positions.len(),
            2,
            "expected birth on both lines: {:?}",
            lines
        );
        assert_eq!(
            char_positions[0], char_positions[1],
            "birth columns must align visually; lines:\n{:?}",
            lines
        );
        assert_eq!(
            char_positions[0],
            "Big Nameperson".chars().count() + 2,
            "birth column should equal display width of longest name + 2"
        );
    }

    #[test]
    fn test_gen_prefix_str_fixed_width() {
        // Single-digit and double-digit generation numbers must produce the same width
        // so that name columns stay aligned across the gen-9 / gen-10 boundary.
        use crate::layout::simple::gen_prefix_str;
        assert_eq!(
            gen_prefix_str(1),
            " 1. ",
            "gen 1 should be right-aligned in 2 chars"
        );
        assert_eq!(
            gen_prefix_str(9),
            " 9. ",
            "gen 9 should be right-aligned in 2 chars"
        );
        assert_eq!(gen_prefix_str(10), "10. ", "gen 10 should be 4 chars total");
        assert_eq!(
            gen_prefix_str(1).len(),
            gen_prefix_str(10).len(),
            "gen-1 and gen-10 prefix must be the same byte length"
        );
    }

    #[test]
    fn test_gen_prefix_present_in_output() {
        // With generation numbers on, the root line must start with " 1. ".
        let prefs = make_prefs(); // show.generation_num = true
        let lines = render_text(&prefs);
        assert!(
            lines[0].starts_with(" 1. "),
            "root line should start with \" 1. \" (right-aligned); got: {:?}",
            lines[0]
        );
    }

    #[test]
    fn test_title_and_copyright() {
        let mut prefs = make_prefs();
        prefs.output.text.title = "My Chart".into();
        prefs.output.text.copyright = "© 2026".into();

        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        let layout_out = run_layout(&genrep, &prefs).unwrap();

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&layout_out, &prefs, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();

        assert!(text.starts_with("My Chart\n"), "title should be first line");
        assert!(
            text.trim_end().ends_with("© 2026"),
            "copyright should be last line"
        );
    }
    #[test]
    fn test_fallback_literal_shifts_content() {
        use crate::scene::{Primitive, Rect, Scene, TextAlign, TextAttr, TextPrimitive};

        let scene = Scene {
            primitives: vec![
                Primitive::Text(TextPrimitive {
                    content: "John Doe".to_string(),
                    bbox: Rect {
                        x: 6.0,
                        y: 0.0,
                        w: 12.0,
                        h: 16.0,
                    },
                    align: TextAlign::Left,
                    attrs: vec![TextAttr::IndividualName, TextAttr::Highlighted],
                }),
                Primitive::Text(TextPrimitive {
                    content: "Jane Doe".to_string(),
                    bbox: Rect {
                        x: 6.0,
                        y: 16.0,
                        w: 12.0,
                        h: 16.0,
                    },
                    align: TextAlign::Left,
                    attrs: vec![TextAttr::IndividualName],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 200.0,
                h: 32.0,
            },
        };
        let mut prefs = Prefs::default();
        prefs.files.highlights = "/path/to/highlights.txt".into();
        prefs.output.style.text.highlights.fallback = "->".into();
        prefs.output.style.text.highlights.fallback = "->".into();
        prefs.output.text.title = "".into();
        prefs.output.text.copyright = "".into();
        // shift = len("->") + 1 = 3; both names at col 6 → shifted to col 9
        // fallback "->" prepended only on highlighted line
        let mut buf = Vec::<u8>::new();
        TextRenderer
            .render(&LayoutOutput::Simple(scene), &prefs, &mut buf)
            .unwrap();
        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        let hl_line = lines[0];
        let normal_line = lines[1];
        // Highlighted line: fallback prepended, content shifted
        assert!(hl_line.starts_with("->"));
        assert!(
            hl_line.find("John").unwrap() >= 3,
            "John should be shifted right; line: {:?}",
            hl_line
        );
        // Non-highlighted line: content shifted, no fallback marker
        assert!(!normal_line.starts_with("->"));
        assert!(
            normal_line.find("Jane").unwrap() >= 3,
            "Jane should be shifted right too; line: {:?}",
            normal_line
        );
        // Both names must be at the same column (aligned)
        assert_eq!(
            hl_line.find("John").unwrap(),
            normal_line.find("Jane").unwrap(),
            "highlighted and normal lines must be aligned;\n  hl: {:?}\n  normal: {:?}",
            hl_line,
            normal_line
        );
    }
    #[test]
    fn test_bc_text_structure() {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.layout.layout_type = "boxed_couples".into();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.generation_num = false;
        prefs.show.birth = false;
        prefs.show.death = false;
        prefs.show.marriage = false;

        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        let layout_out = run_layout(&genrep, &prefs).unwrap();

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&layout_out, &prefs, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();

        // Should contain box-drawing characters

        // Should contain box-drawing characters
        assert!(
            output.contains('\u{250C}'),
            "missing top-left corner: {:?}",
            output
        );
        assert!(
            output.contains('\u{2510}'),
            "missing top-right corner: {:?}",
            output
        );
        assert!(
            output.contains('\u{2514}'),
            "missing bottom-left corner: {:?}",
            output
        );
        assert!(
            output.contains('\u{2518}'),
            "missing bottom-right corner: {:?}",
            output
        );
        assert!(
            output.contains('\u{2500}'),
            "missing horizontal line: {:?}",
            output
        );
        assert!(
            output.contains('\u{2502}'),
            "missing vertical line: {:?}",
            output
        );
    }

    #[test]
    fn test_bc_text_contains_names() {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.layout.layout_type = "boxed_couples".into();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.generation_num = false;
        prefs.show.birth = false;
        prefs.show.death = false;
        prefs.show.marriage = false;

        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        let layout_out = run_layout(&genrep, &prefs).unwrap();

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&layout_out, &prefs, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();

        assert!(output.contains("John"), "root name missing: {:?}", output);
        assert!(output.contains("Jane"), "spouse name missing: {:?}", output);
        assert!(output.contains("Paul"), "child name missing: {:?}", output);
    }

    #[test]
    fn test_bc_text_connectors() {
        let mut prefs = Prefs::default();
        prefs.scope.root = "I1".into();
        prefs.scope.direction = "descendants".into();
        prefs.layout.layout_type = "boxed_couples".into();
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.generation_num = false;
        prefs.show.birth = false;
        prefs.show.death = false;
        prefs.show.marriage = false;

        let mut genrep = parse_str(GEDCOM).unwrap();
        compute_scope(&mut genrep, Some("I1"), "descendants", Some(2));
        let layout_out = run_layout(&genrep, &prefs).unwrap();

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&layout_out, &prefs, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();

        // Connectors between generations should be visible
        assert!(
            output.contains('\u{252C}') || output.contains('\u{2500}'),
            "connector bar missing: {:?}",
            output
        );
    }

    #[test]
    fn test_bc_text_empty_scene() {
        use crate::scene::{Rect, Scene};

        let scene = Scene {
            primitives: vec![],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            },
        };
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();

        // Empty scene should not produce box characters
        assert!(
            !result.contains('\u{250C}'),
            "empty scene should not have box characters: {:?}",
            result
        );
    }

    #[test]
    fn test_bc_text_truncation() {
        use crate::scene::{
            BoxPrimitive, Primitive, Rect, Scene, TextAlign, TextAttr, TextPrimitive,
        };

        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 40.0,
                        h: 30.0,
                    },
                }),
                Primitive::Text(TextPrimitive {
                    content: "Very Long Name That Should Be Truncated".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 40.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 40.0,
                h: 30.0,
            },
        };
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();

        // The long text should be truncated with "..."
        assert!(
            result.contains("..."),
            "long text should be truncated: {:?}",
            result
        );
    }

    #[test]
    fn test_bc_text_highlight_uppercase() {
        use crate::scene::{
            BoxPrimitive, Primitive, Rect, Scene, TextAlign, TextAttr, TextPrimitive,
        };

        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 40.0,
                        h: 30.0,
                    },
                }),
                Primitive::Text(TextPrimitive {
                    content: "john".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 40.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName, TextAttr::Highlighted],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 40.0,
                h: 30.0,
            },
        };
        // default fallback is "uppercase"
        let mut prefs = Prefs::default();
        prefs.output.style.text.highlights.fallback = "uppercase".into();
        prefs.output.text.title = "".into();
        prefs.output.text.copyright = "".into();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();

        // Highlighted text should be uppercased when fallback == "uppercase"
        assert!(
            result.contains("JOHN"),
            "highlighted text should be uppercased: {:?}",
            result
        );
    }
    #[test]
    fn test_bc_text_no_border_overlap() {
        // Regression: text should not overlap box top border.
        // With sequential row assignment, first text row is box_row0 + 1.
        use crate::scene::{
            BoxPrimitive, Primitive, Rect, Scene, TextAlign, TextAttr, TextPrimitive,
        };

        // Box wide enough for "Name" (80px ≈ 10 cols), tall enough for 2+ rows
        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 80.0,
                        h: 36.0,
                    },
                }),
                Primitive::Text(TextPrimitive {
                    content: "Name".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 80.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 80.0,
                h: 36.0,
            },
        };
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = result.lines().collect();

        // Row 0 is the top border (┌ ─ ┐).
        // "Name" should be on row 1 or later, not on the top border row.
        let name_row = lines.iter().position(|l| l.contains("Name")).unwrap_or(0);
        assert!(
            name_row > 0,
            "Name should be below top border row 0, found on row {name_row}; lines: {:?}",
            lines
        );
    }

    #[test]
    fn test_bc_text_sequential_rows() {
        // Birth and death data must appear on separate rows, not overlapping.
        use crate::scene::{
            BoxPrimitive, Primitive, Rect, Scene, TextAlign, TextAttr, TextPrimitive,
        };

        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 100.0, // wide enough for full dates
                        h: 72.0,
                    },
                }),
                Primitive::Text(TextPrimitive {
                    content: "Name".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 100.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName],
                }),
                Primitive::Text(TextPrimitive {
                    content: "* 1 JAN 1800".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 2.0, // pixel-level spacing, rounds to same row as Name
                        w: 100.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::BirthData],
                }),
                Primitive::Text(TextPrimitive {
                    content: "x 1 JAN 1850".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 4.0, // also rounds to same row
                        w: 100.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::DeathData],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 72.0,
            },
        };
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = result.lines().collect();

        // Each piece of text should be on its own row
        let name_row = lines.iter().position(|l| l.contains("Name")).unwrap_or(0);
        let birth_row = lines
            .iter()
            .position(|l| l.contains("1 JAN 1800"))
            .unwrap_or(0);
        let death_row = lines
            .iter()
            .position(|l| l.contains("1 JAN 1850"))
            .unwrap_or(0);
        assert!(
            name_row != birth_row,
            "Name and birth must be on different rows; name_row={name_row}, birth_row={birth_row}; lines: {:?}",
            lines
        );
        assert!(
            birth_row != death_row,
            "Birth and death must be on different rows; birth_row={birth_row}, death_row={death_row}; lines: {:?}",
            lines
        );
    }

    #[test]
    fn test_bc_text_no_marriage_symbol() {
        // ⚭ should be stripped from marriage data in text backend.
        // Box must be tall enough to accommodate a blank row before and after marriage data.
        use crate::scene::{
            BoxPrimitive, Primitive, Rect, Scene, TextAlign, TextAttr, TextPrimitive,
        };

        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 200.0, // wide enough for full date without truncation
                        h: 90.0,  // tall enough for blank-before + marriage + blank-after
                    },
                }),
                Primitive::Text(TextPrimitive {
                    content: "⚭ 4 APR 1843, London".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 200.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::MarriageData],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 200.0,
                h: 90.0,
            },
        };
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();

        assert!(
            !result.contains("⚭"),
            "marriage symbol should be stripped from text backend: {:?}",
            result
        );
        assert!(
            result.contains("4 APR 1843"),
            "marriage date should still be present: {:?}",
            result
        );
    }
    #[test]
    fn test_bc_text_no_trailing_garbage() {
        // Longer text on a row should not leave trailing chars for shorter text.
        use crate::scene::{
            BoxPrimitive, Primitive, Rect, Scene, TextAlign, TextAttr, TextPrimitive,
        };

        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 100.0,
                        h: 36.0,
                    },
                }),
                Primitive::Text(TextPrimitive {
                    content: "Very Long Name".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 100.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName],
                }),
                Primitive::Text(TextPrimitive {
                    content: "Short".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 2.0, // would round to same row as the long name
                        w: 100.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::BirthData],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 36.0,
            },
        };
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = result.lines().collect();

        // "Short" should be on its own row (sequential), not overlapping "Very Long Name"
        let short_row = lines.iter().position(|l| l.contains("Short")).unwrap_or(0);
        // The row with "Short" should not contain characters from "Very Long Name"
        // beyond what "Short" writes (excluding box borders and spaces)
        let short_line = lines[short_row];
        // After trimming, the line should not have "Name" trailing
        assert!(
            !short_line.trim().contains("Name"),
            "short text row should not have trailing garbage from previous longer text: {:?}",
            short_line
        );
    }

    #[test]
    fn test_bc_text_connectors_downward() {
        // Connectors should render with T-junctions for children below parent.
        use crate::scene::{
            BoxPrimitive, ConnectorPrimitive, Point, Primitive, Rect, Scene, TextAlign, TextAttr,
            TextPrimitive,
        };

        // Parent at y=0..18, child at y=72..90 — enough vertical distance
        // for bar_row to be between parent_row and child_row.
        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 20.0,
                        h: 18.0,
                    },
                }),
                Primitive::Text(TextPrimitive {
                    content: "Parent".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 20.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName],
                }),
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 72.0,
                        w: 20.0,
                        h: 18.0,
                    },
                }),
                Primitive::Text(TextPrimitive {
                    content: "Child".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 72.0,
                        w: 20.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName],
                }),
                Primitive::Connector(ConnectorPrimitive {
                    parent_points: vec![Point { x: 10.0, y: 18.0 }],
                    child_points: vec![Point { x: 10.0, y: 72.0 }],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 20.0,
                h: 90.0,
            },
        };
        // root_pos="top" → root at top → children below → downward connectors
        let mut prefs = Prefs::default();
        prefs.layout.root_pos = "top".to_string();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();

        // Single child → straight vertical │, no horizontal bar (task #52 requirement).
        assert!(
            result.contains('\u{2502}'),
            "vertical connector line missing: {:?}",
            result
        );
        assert!(
            !result.contains('\u{2500}'),
            "single-child connector should not have horizontal bar: {:?}",
            result
        );
    }

    #[test]
    fn test_bc_text_connectors_upward() {
        // Connectors should render for children above parent (root_pos="bottom").
        use crate::scene::{
            BoxPrimitive, ConnectorPrimitive, Point, Primitive, Rect, Scene, TextAlign, TextAttr,
            TextPrimitive,
        };

        // Child at y=0..18, parent at y=72..90 — enough vertical distance.
        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 72.0,
                        w: 20.0,
                        h: 18.0,
                    },
                }),
                Primitive::Text(TextPrimitive {
                    content: "Parent".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 72.0,
                        w: 20.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName],
                }),
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 20.0,
                        h: 18.0,
                    },
                }),
                Primitive::Text(TextPrimitive {
                    content: "Child".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 20.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName],
                }),
                Primitive::Connector(ConnectorPrimitive {
                    parent_points: vec![Point { x: 10.0, y: 90.0 }],
                    child_points: vec![Point { x: 10.0, y: 18.0 }],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 20.0,
                h: 108.0,
            },
        };
        // Default root_pos="" → root at bottom → children above → upward connectors
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();

        // Single child → straight vertical │, no horizontal bar.
        assert!(
            result.contains('\u{2502}'),
            "vertical connector line missing for upward connector: {:?}",
            result
        );
        assert!(
            !result.contains('\u{2500}'),
            "single-child upward connector should not have horizontal bar: {:?}",
            result
        );
    }

    #[test]
    fn test_bc_text_intersection_chars() {
        // Connector intersections should use proper box-drawing characters.
        use crate::scene::{
            BoxPrimitive, ConnectorPrimitive, Point, Primitive, Rect, Scene, TextAlign, TextAttr,
            TextPrimitive,
        };

        // Parent centered between two children; enough vertical distance.
        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 20.0,
                        y: 0.0,
                        w: 20.0,
                        h: 18.0,
                    },
                }),
                Primitive::Text(TextPrimitive {
                    content: "Parent".to_string(),
                    bbox: Rect {
                        x: 20.0,
                        y: 0.0,
                        w: 20.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName],
                }),
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 72.0,
                        w: 20.0,
                        h: 18.0,
                    },
                }),
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 40.0,
                        y: 72.0,
                        w: 20.0,
                        h: 18.0,
                    },
                }),
                Primitive::Connector(ConnectorPrimitive {
                    parent_points: vec![Point { x: 30.0, y: 18.0 }],
                    child_points: vec![Point { x: 10.0, y: 72.0 }, Point { x: 50.0, y: 72.0 }],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 60.0,
                h: 90.0,
            },
        };
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();

        // With parent between two children, should have:
        // ┬ at child columns, ┴ or ╪ at parent column
        let has_t_junction_down = result.contains('\u{252C}'); // ┬
        let has_t_junction_up = result.contains('\u{2534}'); // ┴
        let has_cross = result.contains('\u{252A}'); // ╪
        assert!(
            has_t_junction_down || has_t_junction_up || has_cross,
            "connector intersections should use box-drawing T-junctions or crosses; got: {:?}",
            result
        );
    }

    #[test]
    fn test_bc_text_sides_on_text_rows() {
        // Box side borders (│) must remain visible on rows that also contain text.
        use crate::scene::{
            BoxPrimitive, Primitive, Rect, Scene, TextAlign, TextAttr, TextPrimitive,
        };

        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 80.0,
                        h: 54.0, // 3 rows: top border, text row, bottom border
                    },
                }),
                Primitive::Text(TextPrimitive {
                    content: "Name".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 80.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 80.0,
                h: 54.0,
            },
        };
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = result.lines().collect();

        let name_row = lines.iter().position(|l| l.contains("Name")).unwrap_or(0);
        assert!(
            name_row > 0,
            "Name should be below top border; name_row={name_row}; lines: {:?}",
            lines
        );
        // The row containing "Name" must also have the │ side border character.
        assert!(
            lines[name_row].contains('\u{2502}'),
            "Box side border │ should be present on the text row; got: {:?}",
            lines[name_row]
        );
    }

    #[test]
    fn test_bc_text_wide_box_same_rows() {
        // In a wide (two-spouse) box, texts at the same Y position but different X
        // must appear on the same output line.
        use crate::scene::{
            BoxPrimitive, Primitive, Rect, Scene, TextAlign, TextAttr, TextPrimitive,
        };

        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 120.0,
                        h: 54.0,
                    },
                }),
                // Left spouse name — same Y as right spouse name
                Primitive::Text(TextPrimitive {
                    content: "Alice".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 60.0,
                        h: 13.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::SpouseName],
                }),
                // Right spouse name — same Y, different X column
                Primitive::Text(TextPrimitive {
                    content: "Bob".to_string(),
                    bbox: Rect {
                        x: 60.0,
                        y: 0.0,
                        w: 60.0,
                        h: 13.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::SpouseName],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 120.0,
                h: 54.0,
            },
        };
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = result.lines().collect();

        let alice_row = lines
            .iter()
            .position(|l| l.contains("Alice"))
            .expect("Alice not found");
        let bob_row = lines
            .iter()
            .position(|l| l.contains("Bob"))
            .expect("Bob not found");
        assert_eq!(
            alice_row, bob_row,
            "Alice and Bob (same-Y texts in a wide box) must be on the same output row; lines: {:?}",
            lines
        );
    }

    #[test]
    fn test_bc_text_marriage_blank_lines() {
        // MarriageData must be preceded and followed by a blank row.
        use crate::scene::{
            BoxPrimitive, Primitive, Rect, Scene, TextAlign, TextAttr, TextPrimitive,
        };

        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 120.0,
                        h: 108.0,
                    },
                }),
                // Individual name (different Y from marriage)
                Primitive::Text(TextPrimitive {
                    content: "John".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 120.0,
                        h: 13.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName],
                }),
                // Marriage data at a different Y
                Primitive::Text(TextPrimitive {
                    content: "⚭ 1843".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 36.0,
                        w: 120.0,
                        h: 13.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::MarriageData],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 120.0,
                h: 108.0,
            },
        };
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = result.lines().collect();

        let marr_row = lines
            .iter()
            .position(|l| l.contains("1843"))
            .expect("marriage data not found");

        // The row immediately before marriage data must be blank (only box borders or spaces)
        let row_before = lines[marr_row - 1];
        let interior_before = row_before.trim_matches(|c| c == '│' || c == ' ');
        assert!(
            interior_before.is_empty(),
            "row before marriage data should be blank, got: {:?}",
            row_before
        );

        // The row immediately after marriage data must also be blank (if it exists inside the box)
        if marr_row + 1 < lines.len() {
            let row_after = lines[marr_row + 1];
            let interior_after = row_after
                .trim_matches(|c| c == '│' || c == ' ' || c == '└' || c == '┘' || c == '─');
            assert!(
                interior_after.is_empty(),
                "row after marriage data should be blank (or bottom border), got: {:?}",
                row_after
            );
        }
    }

    #[test]
    fn test_bc_text_single_child_vertical() {
        // A single-child connector must draw a straight │ with no horizontal bar.
        use crate::scene::{ConnectorPrimitive, Point, Primitive, Rect, Scene};

        let scene = Scene {
            primitives: vec![Primitive::Connector(ConnectorPrimitive {
                parent_points: vec![Point { x: 40.0, y: 18.0 }],
                child_points: vec![Point { x: 40.0, y: 72.0 }],
            })],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 80.0,
                h: 90.0,
            },
        };
        let mut prefs = Prefs::default();
        prefs.layout.root_pos = "top".to_string();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();

        assert!(
            result.contains('\u{2502}'),
            "single-child connector must draw │: {:?}",
            result
        );
        assert!(
            !result.contains('\u{2500}'),
            "single-child connector must NOT draw horizontal bar ─: {:?}",
            result
        );
    }

    #[test]
    fn test_bc_text_connector_corners() {
        // Two-children downward connector must use corner characters at bar ends.
        use crate::scene::{ConnectorPrimitive, Point, Primitive, Rect, Scene};

        // Parent at col corresponding to x=40 (center), children at x=10 and x=70.
        let scene = Scene {
            primitives: vec![Primitive::Connector(ConnectorPrimitive {
                parent_points: vec![Point { x: 40.0, y: 18.0 }],
                child_points: vec![Point { x: 10.0, y: 72.0 }, Point { x: 70.0, y: 72.0 }],
            })],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 80.0,
                h: 90.0,
            },
        };
        // root_pos="top" → root at top → children below → downward connectors
        let mut prefs = Prefs::default();
        prefs.layout.root_pos = "top".to_string();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();

        // Bar endpoints must be ┌ (left child) and ┐ (right child); parent gets ┴ in interior.
        assert!(
            result.contains('\u{250C}'), // ┌
            "left-end child corner ┌ missing: {:?}",
            result
        );
        assert!(
            result.contains('\u{2510}'), // ┐
            "right-end child corner ┐ missing: {:?}",
            result
        );
        assert!(
            result.contains('\u{2534}'), // ┴
            "parent interior ┴ missing: {:?}",
            result
        );
    }

    #[test]
    fn test_bc_text_connector_joins_box_downward() {
        // Connector must draw ┬ at parent bottom border and ┴ at child top border (downward tree).
        // line_height_px=18: y=36→row1 (parent bottom), y=54→row3 (child top), y=45→row2 (gap).
        // Boxes must be h=36 (2 rows) so draw_box actually renders borders.
        use crate::scene::{BoxPrimitive, ConnectorPrimitive, Point, Primitive, Rect, Scene};

        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 20.0,
                        y: 0.0,
                        w: 40.0,
                        h: 36.0,
                    },
                }),
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 20.0,
                        y: 54.0,
                        w: 40.0,
                        h: 18.0,
                    },
                }),
                Primitive::Connector(ConnectorPrimitive {
                    parent_points: vec![Point { x: 40.0, y: 36.0 }],
                    child_points: vec![Point { x: 40.0, y: 54.0 }],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 80.0,
                h: 90.0,
            },
        };
        let mut prefs = Prefs::default();
        prefs.layout.root_pos = "top".to_string(); // downward=true
        prefs.layout.root_pos = "top".to_string(); // downward=true
        prefs.output.text.title = "".into();
        prefs.output.text.copyright = "".into();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = result.lines().collect();

        // Row 1 = parent bottom border: must contain ┬ (lower side T-junction)
        assert!(
            lines[1].contains('\u{252C}'),
            "parent bottom border should have ┬; got: {:?}",
            lines[1]
        );
        // Row 3 = child top border: must contain ┴ (upper side T-junction)
        assert!(
            lines[3].contains('\u{2534}'),
            "child top border should have ┴; got: {:?}",
            lines[3]
        );
        // Row 2 = gap row: must contain │ (no blank gap between borders)
        assert!(
            lines[2].contains('\u{2502}'),
            "connector row between boxes should have │ (no gap); got: {:?}",
            lines[2]
        );
    }

    #[test]
    fn test_ancestors_id_alignment() {
        const GED: &str = "\
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
        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(3));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I3".into();
        prefs.scope.direction = "ancestors".into();
        prefs.layout.layout_type = "simple".into();
        prefs.show.id = true;
        prefs.show.generation_num = true;
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.birth = false;
        prefs.show.death = false;
        prefs.show.marriage = false;
        prefs.output.text.title = "".into();
        prefs.output.text.copyright = "".into();

        let layout_out = run_layout(&genrep, &prefs).unwrap();
        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&layout_out, &prefs, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();

        // Every non-empty line must start with the ID at column 0 — not indented.
        for line in output.lines() {
            if line.is_empty() {
                continue;
            }
            assert!(
                line.starts_with('I'),
                "ID line must start with 'I' at column 0 (not indented after │); got: {:?}",
                line
            );
        }
    }

    #[test]
    fn test_bc_text_connector_joins_box_upward() {
        // Connector must draw ┬ at child bottom border and ┴ at parent top border (upward tree).
        // line_height_px=18: y=36→row1 (child bottom), y=54→row3 (parent top), y=45→row2 (gap).
        // Child box must be h=36 (2 rows) so draw_box actually renders the bottom border.
        use crate::scene::{BoxPrimitive, ConnectorPrimitive, Point, Primitive, Rect, Scene};

        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 20.0,
                        y: 54.0,
                        w: 40.0,
                        h: 18.0,
                    },
                }),
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 20.0,
                        y: 0.0,
                        w: 40.0,
                        h: 36.0,
                    },
                }),
                Primitive::Connector(ConnectorPrimitive {
                    parent_points: vec![Point { x: 40.0, y: 54.0 }],
                    child_points: vec![Point { x: 40.0, y: 36.0 }],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 80.0,
                h: 90.0,
            },
        };
        let mut prefs = Prefs::default(); // root_pos="bottom" → downward=false (upward)
        prefs.output.text.title = "".into();
        prefs.output.text.copyright = "".into();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = result.lines().collect();

        // Row 1 = child bottom border: must contain ┬ (lower side T-junction)
        assert!(
            lines[1].contains('\u{252C}'),
            "child bottom border should have ┬; got: {:?}",
            lines[1]
        );
        // Row 3 = parent top border: must contain ┴ (upper side T-junction)
        assert!(
            lines[3].contains('\u{2534}'),
            "parent top border should have ┴; got: {:?}",
            lines[3]
        );
        // Row 2 = gap row: must contain │
        assert!(
            lines[2].contains('\u{2502}'),
            "connector row between boxes should have │ (no gap); got: {:?}",
            lines[2]
        );
    }

    #[test]
    fn test_ancestors_connector_not_in_mother_row() {
        // Regression for queue item #66: the connector below the root must NOT
        // extend into the mother's own text row.  With vert_spacing=1 every pair
        // of individuals has a gap row, so connector_below would draw │ in the
        // mother's row with the old (mother_line+0.5)*lh formula.
        const GED: &str = "\
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
        let mut genrep = parse_str(GED).unwrap();
        compute_scope(&mut genrep, Some("I3"), "ancestors", Some(3));

        let mut prefs = Prefs::default();
        prefs.scope.root = "I3".into();
        prefs.scope.direction = "ancestors".into();
        prefs.layout.layout_type = "simple".into();
        prefs.layout.simple.vert_spacing = 1; // creates gap rows between all individuals
        prefs.format.individual = "{firstname} {lastname}".into();
        prefs.show.generation_num = false;
        prefs.show.birth = false;
        prefs.show.death = false;
        prefs.show.marriage = false;
        prefs.output.text.title = "".into();
        prefs.output.text.copyright = "".into();

        let layout_out = run_layout(&genrep, &prefs).unwrap();
        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&layout_out, &prefs, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.lines().collect();

        assert!(
            lines.iter().any(|l| l.contains('│')),
            "expected at least one │ connector in ancestors output"
        );

        for line in &lines {
            if line.contains("Jane") || line.contains("Ancestress") {
                assert!(
                    !line.contains('│'),
                    "connector must not extend into mother's own row: {:?}",
                    line
                );
            }
        }
    }

    #[test]
    fn test_bc_text_no_spouse_blank_row() {
        // A blank SpouseName primitive must advance the row counter so individual name
        // appears at row 2 (not row 1), matching the layout of a box with a full spouse.
        use crate::scene::{
            BoxPrimitive, Primitive, Rect, Scene, TextAlign, TextAttr, TextPrimitive,
        };

        let scene = Scene {
            primitives: vec![
                Primitive::Box(BoxPrimitive {
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 80.0,
                        h: 54.0,
                    },
                }),
                // Blank spouse placeholder (emitted by emit_blank_spouse_section)
                Primitive::Text(TextPrimitive {
                    content: String::new(),
                    bbox: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 80.0,
                        h: 9.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::SpouseName],
                }),
                // Individual name at a larger Y (bottom section)
                Primitive::Text(TextPrimitive {
                    content: "Individual".to_string(),
                    bbox: Rect {
                        x: 0.0,
                        y: 27.0,
                        w: 80.0,
                        h: 14.0,
                    },
                    align: TextAlign::Center,
                    attrs: vec![TextAttr::IndividualName],
                }),
            ],
            canvas_bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 80.0,
                h: 54.0,
            },
        };
        let mut prefs = Prefs::default();
        prefs.output.text.title = "".into();
        prefs.output.text.copyright = "".into();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = result.lines().collect();

        let ind_row = lines
            .iter()
            .position(|l| l.contains("Individual"))
            .expect("Individual not found");
        assert_eq!(
            ind_row, 2,
            "Individual should be at row 2 (after blank spouse row); got row {}: {:?}",
            ind_row, lines
        );
    }
}
