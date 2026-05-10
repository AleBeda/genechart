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
#[derive(Debug, Clone)]
pub enum TextAttr {
    /// The name of the chart's root / descendant individual.
    IndividualName,
    /// The name of a spouse.
    SpouseName,
    /// A birth or baptism event line.
    BirthData,
    /// A death or burial event line.
    DeathData,
    /// A marriage event line.
    MarriageData,
    /// An ID string (individual or family).
    IndividualId,
    /// A generation-number prefix.
    GenerationNum,
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
#[derive(Debug, Clone)]
pub struct TextPrimitive {
    pub content: String,
    pub bbox: Rect,
    pub align: TextAlign,
    pub attr: TextAttr,
}

/// A box (rectangle) primitive.
#[derive(Debug, Clone)]
pub struct BoxPrimitive {
    pub bbox: Rect,
    pub is_highlighted: bool,
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

/// A wedge primitive (fan layout — not yet migrated).
#[derive(Debug, Clone)]
pub struct WedgePrimitive {
    pub cx: f64,
    pub cy: f64,
    pub angle_center: f64,
    pub angle_span: f64,
    pub radius_inner: f64,
    pub radius_outer: f64,
}

/// A single renderable element.
#[derive(Debug, Clone)]
pub enum Primitive {
    Box(BoxPrimitive),
    Text(TextPrimitive),
    Connector(ConnectorPrimitive),
    Wedge(WedgePrimitive),
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
