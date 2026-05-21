//! Intermediate representation (IR) for the layout→backend pipeline.
//! Forward-looking variants and fields (Wedge, GenerationNum, Right, etc.) are
//! defined here for completeness but not yet wired up.
#![allow(dead_code)]
//!
//! The layout layer emits a `Scene` containing a list of `Primitive`s.
//! The backend layer renders those primitives to SVG (or other formats)
//! without needing any knowledge of genealogical relationships.
//!
//! ## Coordinate system
//! All coordinates in `Scene` are in *display space*:
//! - (0, 0) is the top-left of the chart content area.
//! - x increases rightward.
//! - y increases downward.
//!
//! The SVG backend adds `MARGIN` (and `chart_top_offset`) to convert display
//! coordinates to final SVG coordinates.

/// A 2D point in display space.
#[derive(Debug, Clone)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// An axis-aligned rectangle in display space.
#[derive(Debug, Clone)]
pub struct Rect {
    /// Left edge x.
    pub x: f64,
    /// Top edge y.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

/// Semantic attribute of a `TextPrimitive`, used by the backend to choose font,
/// color, and weight from preferences without knowing genealogical context.
#[derive(Debug, Clone, PartialEq)]
pub enum TextAttr {
    /// The name of the chart's root / descendant individual.
    IndividualName,
    /// The name of a spouse.
    SpouseName,
    /// A birth event line.
    BirthData,
    /// A death event line.
    DeathData,
    /// A marriage event line.
    MarriageData,
    /// An ID string (individual or family).
    IndividualId,
    /// A generation-number prefix.
    GenerationNum,
    /// Highlighted individual or family
    Highlighted,
    /// A GEDCOM NOTE line.
    NoteText,
}

/// Build a `Vec<TextAttr>` with `base` plus `Highlighted` when `highlighted` is true.
pub fn label_attrs(base: TextAttr, highlighted: bool) -> Vec<TextAttr> {
    if highlighted {
        vec![base, TextAttr::Highlighted]
    } else {
        vec![base]
    }
}

/// Horizontal alignment of text within its bounding box.
#[derive(Debug, Clone)]
pub enum TextAlign {
    /// Text is left-aligned; the baseline starts at `bbox.x`.
    Left,
    /// Text is centered; the baseline is at `bbox.x + bbox.w / 2`.
    Center,
    /// Text is right-aligned; the baseline ends at `bbox.x + bbox.w`.
    Right,
}

/// A text primitive.
///
/// `bbox.y + bbox.h` is the *baseline* of the text (SVG convention).
/// `bbox.h` is the font size.
/// The `attrs` list carries semantic attributes (IndividualName, BirthData, …)
/// and optionally `Highlighted`. The backend applies all attributes; order is
/// irrelevant. The layout layer guarantees non-conflicting attributes.
#[derive(Debug, Clone)]
pub struct TextPrimitive {
    pub content: String,
    pub bbox: Rect,
    pub align: TextAlign,
    pub attrs: Vec<TextAttr>,
}

/// A box (rectangle) primitive.
#[derive(Debug, Clone)]
pub struct BoxPrimitive {
    pub bbox: Rect,
}

/// An image primitive (boxes layout photos).
///
/// `href` is a base64 data URI (`data:image/jpeg;base64,...`) for embedded photos,
/// or a relative/absolute path string for linked photos.
#[derive(Debug, Clone)]
pub struct ImagePrimitive {
    pub bbox: Rect,
    pub href: String,
}

/// A filled rectangle primitive (e.g. photo placeholder).
#[derive(Debug, Clone)]
pub struct FilledRectPrimitive {
    pub bbox: Rect,
    /// CSS color string, e.g. `"#e8e8e8"`.
    pub fill: String,
}

/// A connector primitive: vertical lines from parents, a horizontal bar, and
/// vertical drops to each child.
///
/// `parent_points` contains one or two points (one per spouse column).
/// `child_points` contains one point per child.
/// All points are in display space.
#[derive(Debug, Clone)]
pub struct ConnectorPrimitive {
    pub parent_points: Vec<Point>,
    pub child_points: Vec<Point>,
}

/// A single text line for the fancy layout (absolute canvas coordinates).
#[derive(Debug, Clone)]
pub struct FancyLine {
    pub x: f64,
    pub y: f64,
    pub text: String,
    pub attrs: Vec<TextAttr>,
}

/// All text lines for one individual or spouse in the fancy layout.
#[derive(Debug, Clone)]
pub struct FancyTextItem {
    pub lines: Vec<FancyLine>,
    pub individual_id: String,
    pub highlighted: bool,
}

/// Connector kind for the fancy layout (used for SVG grouping).
#[derive(Debug, Clone)]
pub enum FancyConnKind {
    IndivToSpouse,
    SpouseToChildren,
}

/// A connector in the fancy layout (SVG path, pre-computed in emit_scene).
#[derive(Debug, Clone)]
pub struct FancyConnector {
    pub d: String,
    pub stroke: String,
    pub stroke_width: f64,
    pub kind: FancyConnKind,
    /// SVG id for the wrapper `<g>` element; empty string → no wrapper emitted.
    pub id: String,
    /// SVG stroke-dasharray value; empty string → solid line.
    pub stroke_dasharray: String,
}
/// A group of primitives rendered as a single SVG `<g>` element.
/// `id` is the SVG id attribute; empty string → no id attribute emitted.
/// Children may themselves be `Primitive::Group` (enables double-wrapping).
#[derive(Debug, Clone)]
pub struct GroupPrimitive {
    pub id: String,
    pub children: Vec<Primitive>,
}

/// Connector from an individual's right edge to one or more spouse boxes.
/// Used by the `boxes` layout (descendants direction).
///
/// Geometry (display space):
///   `bar_y = (individual_exit.y + spouse_entries[0].y) / 2`
///   - vertical segment: `individual_exit` → `(individual_exit.x, bar_y)`
///   - horizontal bar: `(individual_exit.x, bar_y)` → `(last_spouse.x, bar_y)`
///   - vertical drops: `(spouse_i.x, bar_y)` → `spouse_i` for each entry
#[derive(Debug, Clone)]
pub struct BoxesSpouseConnector {
    /// Attach point: right edge of the individual box, at the box's top edge (display coords).
    pub individual_exit: Point,
    /// Entry points: top-center of each spouse box (display coords), left to right.
    pub spouse_entries: Vec<Point>,
}

/// A wedge primitive (fan layout).
#[derive(Debug, Clone)]
pub struct WedgePrimitive {
    pub cx: f64,
    pub cy: f64,
    pub angle_center: f64,
    pub angle_span: f64,
    pub radius_inner: f64,
    pub radius_outer: f64,
    pub label: Option<String>,
    pub label_attrs: Vec<TextAttr>,
    pub radial_text: bool,
    pub individual_id: String,
    pub birth_line: Option<String>,
    pub death_line: Option<String>,
}

/// A single renderable element.
#[derive(Debug, Clone)]
pub enum Primitive {
    Box(BoxPrimitive),
    Text(TextPrimitive),
    Connector(ConnectorPrimitive),
    Wedge(WedgePrimitive),
    FancyText(FancyTextItem),
    FancyConn(FancyConnector),
    Group(GroupPrimitive),
    BoxesSpouseConnector(BoxesSpouseConnector),
    Image(ImagePrimitive),
    FilledRect(FilledRectPrimitive),
}

/// The complete IR emitted by a layout algorithm.
///
/// `canvas_bounds` describes the content area in display space (origin is 0,0).
/// The backend adds margins around it.
#[derive(Debug, Clone)]
pub struct Scene {
    pub primitives: Vec<Primitive>,
    /// Bounding box of the chart content (excluding margins).
    pub canvas_bounds: Rect,
}
