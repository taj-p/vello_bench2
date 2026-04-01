//! Benchmark scene definitions.

mod clip;
mod filter_layers;
mod polyline;
mod rect;
mod strokes;
mod svg;
mod text;

use crate::backend::Renderer;
pub use clip::ClipScene;
pub use filter_layers::FilterLayersScene;
pub use polyline::PolylineScene;
pub use rect::RectScene;
pub use strokes::StrokesScene;
pub use svg::SvgScene;
pub use text::TextScene;
use vello_common::kurbo::Affine;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneId {
    Rect,
    Strokes,
    Polyline,
    Svg,
    Clip,
    Text,
    FilterLayers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamId {
    NumRects,
    PaintMode,
    RectSize,
    Rotated,
    GradientShape,
    DynamicGradient,
    ImageFilter,
    ImageOpaque,
    UseDrawImage,
    NumStrokes,
    CurveType,
    Segments,
    StrokeWidth,
    Cap,
    NumVertices,
    SvgAsset,
    ClipMode,
    ClipMethod,
    NumRuns,
    FontSize,
    FilterKind,
    Speed,
    BlurStdDeviation,
    ShadowDx,
    ShadowDy,
    ShadowAlpha,
}

impl ParamId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NumRects => "num_rects",
            Self::PaintMode => "paint_mode",
            Self::RectSize => "rect_size",
            Self::Rotated => "rotated",
            Self::GradientShape => "gradient_shape",
            Self::DynamicGradient => "dynamic_gradient",
            Self::ImageFilter => "image_filter",
            Self::ImageOpaque => "image_opaque",
            Self::UseDrawImage => "use_draw_image",
            Self::NumStrokes => "num_strokes",
            Self::CurveType => "curve_type",
            Self::Segments => "segments",
            Self::StrokeWidth => "stroke_width",
            Self::Cap => "cap",
            Self::NumVertices => "num_vertices",
            Self::SvgAsset => "svg_asset",
            Self::ClipMode => "clip_mode",
            Self::ClipMethod => "clip_method",
            Self::NumRuns => "num_runs",
            Self::FontSize => "font_size",
            Self::FilterKind => "filter_kind",
            Self::Speed => "speed",
            Self::BlurStdDeviation => "blur_std_deviation",
            Self::ShadowDx => "shadow_dx",
            Self::ShadowDy => "shadow_dy",
            Self::ShadowAlpha => "shadow_alpha",
        }
    }
}

/// A tweakable parameter for a benchmark scene.
#[derive(Debug, Clone)]
pub struct Param {
    /// Internal name used as key.
    pub id: ParamId,
    /// Human-readable label for UI.
    pub label: &'static str,
    /// The kind of control: numeric range or dropdown select.
    pub kind: ParamKind,
    /// Current value.
    pub value: f64,
}

/// Whether a parameter is a numeric range control or a dropdown select.
#[derive(Debug, Clone)]
pub enum ParamKind {
    /// A range slider with min, max, and step.
    Slider {
        /// Minimum value.
        min: f64,
        /// Maximum value.
        max: f64,
        /// Step increment.
        step: f64,
    },
    /// A dropdown select with `(label, value)` options.
    Select(Vec<(&'static str, f64)>),
}

/// Trait for benchmark scenes with tweakable parameters.
pub trait BenchScene {
    /// Stable identifier used for backend capability checks.
    fn scene_id(&self) -> SceneId;
    /// Display name of this scene.
    fn name(&self) -> &str;
    /// Return the list of tweakable parameters.
    fn params(&self) -> Vec<Param>;
    /// Update a parameter by name.
    fn set_param(&mut self, param: ParamId, value: f64);
    /// Render one frame into the scene.
    ///
    /// `view` is a view transform (e.g. pan/zoom) applied by the interactive mode.
    /// Scenes should compose it with their own transforms.
    fn render(
        &mut self,
        backend: &mut dyn Renderer,
        width: u32,
        height: u32,
        time: f64,
        view: Affine,
    );
}

// ── Shared animation helpers ─────────────────────────────────────────────────

/// Bounce a position off a boundary, reversing velocity on contact.
pub(crate) fn bounce(pos: &mut f64, vel: &mut f64, max: f64) {
    if *pos < 0.0 {
        *pos = 0.0;
        *vel = vel.abs();
    } else if *pos > max {
        *pos = max;
        *vel = -vel.abs();
    }
}

/// Compute a speed-scaled delta time from a millisecond timestamp.
///
/// On the first call (`last_time == 0`), returns a synthetic kick of 0.5s
/// to spread elements on screen. Updates `last_time` in place.
pub(crate) fn delta_time(last_time: &mut f64, time: f64, speed: f64) -> f64 {
    // Cap raw dt to 100ms to avoid huge jumps after tab switches / pauses.
    let dt = ((time - *last_time) / 1000.0).clamp(0.0, 0.1) * speed;
    *last_time = time;
    dt
}

/// Return all available benchmark scenes.
pub fn all_scenes() -> Vec<Box<dyn BenchScene>> {
    vec![
        Box::new(RectScene::new()),
        Box::new(StrokesScene::new()),
        Box::new(PolylineScene::new()),
        Box::new(SvgScene::new()),
        Box::new(ClipScene::new()),
        Box::new(TextScene::new()),
        Box::new(FilterLayersScene::new()),
    ]
}

pub fn scene_index(scene_id: SceneId) -> usize {
    match scene_id {
        SceneId::Rect => 0,
        SceneId::Strokes => 1,
        SceneId::Polyline => 2,
        SceneId::Svg => 3,
        SceneId::Clip => 4,
        SceneId::Text => 5,
        SceneId::FilterLayers => 6,
    }
}
