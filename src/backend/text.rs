//! Plain-text output backend.

use crate::backend::Renderer;
use crate::layout::LayoutOutput;
use crate::preferences::Prefs;
use crate::scene::{Primitive, Scene, TextAttr};
use crate::text_metrics::{CHAR_WIDTH_RATIO, FONT_SIZE, LINE_HEIGHT, parsed_font};
use std::collections::HashMap;

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

fn draw_connector_on_grid(
    grid: &mut TextGrid,
    conn: &crate::scene::ConnectorPrimitive,
    line_height_px: f64,
    char_width_px: f64,
) {
    if conn.parent_points.is_empty() || conn.child_points.is_empty() {
        return;
    }

    let parent_pt = &conn.parent_points[0];
    let parent_col = display_to_col(parent_pt.x, char_width_px);
    let parent_row = display_to_row(parent_pt.y, line_height_px);

    let child_cols: Vec<usize> = conn
        .child_points
        .iter()
        .map(|c| display_to_col(c.x, char_width_px))
        .collect();
    let child_rows: Vec<usize> = conn
        .child_points
        .iter()
        .map(|c| display_to_row(c.y, line_height_px))
        .collect();

    let first_child_row = child_rows[0];

    // Determine direction
    let downward = first_child_row > parent_row;

    // Horizontal bar at midpoint
    let bar_row = if downward {
        (parent_row + first_child_row) / 2
    } else {
        (first_child_row + parent_row) / 2
    };
    if bar_row >= grid.rows {
        return;
    }

    // Collect all columns for bar span
    let mut all_cols = vec![parent_col];
    all_cols.extend(&child_cols);
    let bar_left = *all_cols.iter().min().unwrap();
    let bar_right = *all_cols.iter().max().unwrap();

    if downward {
        // Children below parent
        // Vertical drop from parent to bar
        for r in (parent_row + 1)..bar_row {
            if r < grid.rows {
                grid.set(r, parent_col, '\u{2502}', PRIORITY_CONNECTOR); // │
            }
        }

        // Horizontal bar
        for c in bar_left..=bar_right {
            if c < grid.cols {
                grid.set(bar_row, c, '\u{2500}', PRIORITY_CONNECTOR); // ─
            }
        }

        // Vertical drops from bar to each child
        for (i, &child_col) in child_cols.iter().enumerate() {
            let child_row = child_rows[i];
            for r in (bar_row + 1)..child_row {
                if r < grid.rows {
                    grid.set(r, child_col, '\u{2502}', PRIORITY_CONNECTOR); // │
                }
            }
        }

        // Corner/intersection characters
        if bar_left < bar_right {
            // Left end of bar
            let left_is_child = child_cols.contains(&bar_left);
            let right_is_child = child_cols.contains(&bar_right);
            let left_is_parent = parent_col == bar_left;
            let right_is_parent = parent_col == bar_right;

            if left_is_child && left_is_parent {
                grid.set(bar_row, bar_left, '\u{252B}', PRIORITY_INTERSECTION); // ┫
            } else if left_is_child {
                grid.set(bar_row, bar_left, '\u{252C}', PRIORITY_INTERSECTION); // ┬
            }

            if right_is_child && right_is_parent {
                grid.set(bar_row, bar_right, '\u{2534}', PRIORITY_INTERSECTION); // ┴
            } else if right_is_child {
                grid.set(bar_row, bar_right, '\u{252C}', PRIORITY_INTERSECTION); // ┬
            }

            // Parent column intersection
            if parent_col > bar_left && parent_col < bar_right {
                let parent_between_children = child_cols.iter().any(|&c| c < parent_col)
                    && child_cols.iter().any(|&c| c > parent_col);
                if parent_between_children {
                    grid.set(bar_row, parent_col, '\u{252A}', PRIORITY_INTERSECTION); // ╪
                } else {
                    if parent_col == bar_left {
                        grid.set(bar_row, parent_col, '\u{2524}', PRIORITY_INTERSECTION); // ├
                    } else if parent_col == bar_right {
                        grid.set(bar_row, parent_col, '\u{2525}', PRIORITY_INTERSECTION); // ┤
                    } else {
                        grid.set(bar_row, parent_col, '\u{2524}', PRIORITY_INTERSECTION); // ├
                    }
                }
            }
        } else if bar_left == bar_right {
            // Single column: just a vertical line
            for r in (parent_row + 1)..first_child_row {
                if r < grid.rows {
                    grid.set(r, bar_left, '\u{2502}', PRIORITY_CONNECTOR); // │
                }
            }
        }
    } else {
        // Children above parent (upward connectors)
        // Vertical line from parent up to bar
        for r in (bar_row + 1)..=parent_row {
            if r < grid.rows {
                grid.set(r, parent_col, '\u{2502}', PRIORITY_CONNECTOR); // │
            }
        }

        // Horizontal bar
        for c in bar_left..=bar_right {
            if c < grid.cols {
                grid.set(bar_row, c, '\u{2500}', PRIORITY_CONNECTOR); // ─
            }
        }

        // Vertical lines from each child down to bar
        for (i, &child_col) in child_cols.iter().enumerate() {
            let child_row = child_rows[i];
            for r in child_row..bar_row {
                if r < grid.rows {
                    grid.set(r, child_col, '\u{2502}', PRIORITY_CONNECTOR); // │
                }
            }
        }

        // Corner/intersection characters
        if bar_left < bar_right {
            // Left end of bar
            let left_is_child = child_cols.contains(&bar_left);
            let right_is_child = child_cols.contains(&bar_right);
            let left_is_parent = parent_col == bar_left;
            let right_is_parent = parent_col == bar_right;

            if left_is_child && left_is_parent {
                grid.set(bar_row, bar_left, '\u{252B}', PRIORITY_INTERSECTION); // ┫
            } else if left_is_child {
                grid.set(bar_row, bar_left, '\u{252C}', PRIORITY_INTERSECTION); // ┬
            }

            if right_is_child && right_is_parent {
                grid.set(bar_row, bar_right, '\u{2534}', PRIORITY_INTERSECTION); // ┴
            } else if right_is_child {
                grid.set(bar_row, bar_right, '\u{252C}', PRIORITY_INTERSECTION); // ┬
            }

            // Parent column intersection
            if parent_col > bar_left && parent_col < bar_right {
                let parent_between_children = child_cols.iter().any(|&c| c < parent_col)
                    && child_cols.iter().any(|&c| c > parent_col);
                if parent_between_children {
                    grid.set(bar_row, parent_col, '\u{252A}', PRIORITY_INTERSECTION); // ╪
                } else {
                    if parent_col == bar_left {
                        grid.set(bar_row, parent_col, '\u{2524}', PRIORITY_INTERSECTION); // ├
                    } else if parent_col == bar_right {
                        grid.set(bar_row, parent_col, '\u{2525}', PRIORITY_INTERSECTION); // ┤
                    } else {
                        grid.set(bar_row, parent_col, '\u{2524}', PRIORITY_INTERSECTION); // ├
                    }
                }
            }
        } else if bar_left == bar_right {
            // Single column: just a vertical line
            for r in first_child_row..=parent_row {
                if r < grid.rows {
                    grid.set(r, bar_left, '\u{2502}', PRIORITY_CONNECTOR); // │
                }
            }
        }
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

    // Collect boxes in order
    let boxes: Vec<&crate::scene::BoxPrimitive> = scene
        .primitives
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

    // Pass 2: place text sequentially per box
    for box_prim in &boxes {
        let bbox = &box_prim.bbox;
        let box_row0 = display_to_row(bbox.y, line_height_px);
        let box_row1 = display_to_row(bbox.y + bbox.h, line_height_px);
        let box_col0 = display_to_col(bbox.x, char_width_px);
        let box_col1 = display_to_col(bbox.x + bbox.w, char_width_px);

        // Find text primitives contained within this box
        let mut texts: Vec<&crate::scene::TextPrimitive> = scene
            .primitives
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

        // Sort by display Y to preserve visual top-to-bottom order
        texts.sort_by_key(|t| (t.bbox.y / line_height_px).round() as usize);

        // Assign rows sequentially from box_row0 + 1
        let mut current_row = box_row0 + 1;
        for text in texts {
            if current_row >= box_row1 && box_row1 > box_row0 {
                break; // No more room in this box
            }
            if current_row >= grid.rows {
                break;
            }

            // Build content string, applying transforms
            let mut content = text.content.clone();

            // Strip leading "⚭ " from marriage data text
            if text.attrs.contains(&crate::scene::TextAttr::MarriageData) {
                if content.starts_with("⚭ ") {
                    content = content[3..].to_string();
                }
            }

            // Apply highlight uppercase
            if text.attrs.contains(&crate::scene::TextAttr::Highlighted)
                && prefs.output.style.text.highlights.fallback == "uppercase"
            {
                content = content.to_uppercase();
            }

            // Clear any trailing characters from previous writes on this row
            clear_row_segment(&mut grid, current_row, box_col0, box_col1);

            // Write text at this row
            draw_text_on_grid(
                &mut grid,
                text,
                line_height_px,
                char_width_px,
                &content,
                current_row,
            );

            current_row += 1;
        }
    }

    // Pass 3: draw connectors
    for prim in &scene.primitives {
        if let Primitive::Connector(c) = prim {
            draw_connector_on_grid(&mut grid, c, line_height_px, char_width_px);
        }
    }

    grid.to_rendered_string()
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

    for prim in &scene.primitives {
        match prim {
            Primitive::Text(t) => {
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
                let x_col = (c.parent_points[0].x / char_width_px).round() as usize;
                let y_start = (c.parent_points[0].y / line_height_px).round() as usize;
                let y_end = (c.child_points[0].y / line_height_px).round() as usize;
                for row in y_start..y_end {
                    if row < total_lines {
                        pad_line_to(&mut lines[row], x_col + 1);
                        set_char_at(&mut lines[row], x_col, '│');
                    }
                }
            }
            Primitive::Box(_) | Primitive::Wedge(_) => {
                // Not used in simple layout text output
            }
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

        // Title
        if !prefs.output.text.title.is_empty() {
            let gedcom_name = std::path::Path::new(&prefs.files.gedcom)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let mut vars = HashMap::new();
            vars.insert("gedcom".to_string(), gedcom_name.to_string());
            let title = strfmt::strfmt(&prefs.output.text.title, &vars)
                .unwrap_or_else(|_| prefs.output.text.title.clone());
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
        let prefs = Prefs::default();
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
                        h: 36.0,
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
                h: 36.0,
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
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();

        // Should contain vertical lines and a horizontal bar
        assert!(
            result.contains('\u{2502}'),
            "vertical connector line missing: {:?}",
            result
        );
        assert!(
            result.contains('\u{2500}'),
            "horizontal connector bar missing: {:?}",
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
        let prefs = Prefs::default();
        let output = LayoutOutput::BoxedCouples(scene);

        let mut buf = Vec::<u8>::new();
        TextRenderer.render(&output, &prefs, &mut buf).unwrap();
        let result = String::from_utf8(buf).unwrap();

        // Should contain vertical lines connecting parent and child
        assert!(
            result.contains('\u{2502}'),
            "vertical connector line missing for upward connector: {:?}",
            result
        );
        assert!(
            result.contains('\u{2500}'),
            "horizontal connector bar missing for upward connector: {:?}",
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
}
