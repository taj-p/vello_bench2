//! Vector graphics (SVG) benchmark scene using usvg for proper parsing.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use super::{BenchScene, Param, ParamId, ParamKind, SceneId};
use crate::backend::Renderer;
use usvg::tiny_skia_path::PathSegment;
use usvg::{Group, Node};
use vello_common::kurbo::{Affine, BezPath, Stroke};
use vello_common::peniko::Color;

/// A single draw command in document order.
enum DrawCmd {
    Fill {
        path: BezPath,
        transform: Affine,
        color: Color,
    },
    Stroke {
        path: BezPath,
        transform: Affine,
        color: Color,
        width: f64,
    },
    PushClip {
        path: BezPath,
        transform: Affine,
    },
    PopClip,
}

/// A pre-parsed SVG asset ready for rendering.
struct SvgAsset {
    name: &'static str,
    commands: Vec<DrawCmd>,
    width: f64,
    height: f64,
}

/// Benchmark scene that renders one of several SVG assets.
pub struct SvgScene {
    assets: Vec<SvgAsset>,
    selected: usize,
}

impl std::fmt::Debug for SvgScene {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SvgScene")
            .field("selected", &self.selected)
            .finish()
    }
}

impl SvgScene {
    /// Create a new SVG scene with all bundled assets.
    pub fn new() -> Self {
        let load = |name: &'static str, data: &[u8]| {
            let tree = usvg::Tree::from_data(data, &usvg::Options::default())
                .unwrap_or_else(|e| panic!("Failed to parse {name}: {e}"));
            let mut commands = Vec::new();
            convert_group(&mut commands, tree.root(), Affine::IDENTITY);
            SvgAsset {
                name,
                commands,
                width: tree.size().width() as f64,
                height: tree.size().height() as f64,
            }
        };
        let assets = vec![
            load(
                "Ghostscript Tiger",
                include_bytes!("../../assets/Ghostscript_Tiger.svg"),
            ),
            load(
                "Coat of Arms",
                include_bytes!("../../assets/coat_of_arms.svg"),
            ),
            load("Heraldry", include_bytes!("../../assets/heraldry.svg")),
        ];
        Self {
            assets,
            selected: 0,
        }
    }
}

// ── usvg → draw command conversion ──────────────────────────────────────────

fn convert_group(commands: &mut Vec<DrawCmd>, g: &Group, parent_transform: Affine) {
    let transform = parent_transform * convert_transform(&g.transform());

    // Handle clip path on this group.
    let has_clip = if let Some(clip) = g.clip_path() {
        let clip_transform = transform * convert_transform(&clip.transform());
        let clip_path = flatten_group_to_path(clip.root());
        if !clip_path.elements().is_empty() {
            commands.push(DrawCmd::PushClip {
                path: clip_path,
                transform: clip_transform,
            });
        }
        true
    } else {
        false
    };

    for child in g.children() {
        match child {
            Node::Group(group) => convert_group(commands, group, transform),
            Node::Path(p) => {
                let bez = convert_path(p);

                if let Some(fill) = p.fill() {
                    let color = usvg_paint_to_color(&fill.paint(), fill.opacity());
                    commands.push(DrawCmd::Fill {
                        path: bez.clone(),
                        transform,
                        color,
                    });
                }

                if let Some(stroke) = p.stroke() {
                    let color = usvg_paint_to_color(&stroke.paint(), stroke.opacity());
                    commands.push(DrawCmd::Stroke {
                        path: bez,
                        transform,
                        color,
                        width: stroke.width().get() as f64,
                    });
                }
            }
            Node::Image(_) | Node::Text(_) => {}
        }
    }

    if has_clip {
        commands.push(DrawCmd::PopClip);
    }
}

/// Flatten all paths in a group into a single BezPath (for clip paths).
fn flatten_group_to_path(g: &Group) -> BezPath {
    let mut bp = BezPath::new();
    for child in g.children() {
        match child {
            Node::Path(p) => {
                for seg in p.data().segments() {
                    match seg {
                        PathSegment::MoveTo(pt) => bp.move_to((pt.x as f64, pt.y as f64)),
                        PathSegment::LineTo(pt) => bp.line_to((pt.x as f64, pt.y as f64)),
                        PathSegment::QuadTo(p1, p2) => {
                            bp.quad_to((p1.x as f64, p1.y as f64), (p2.x as f64, p2.y as f64));
                        }
                        PathSegment::CubicTo(p1, p2, p3) => {
                            bp.curve_to(
                                (p1.x as f64, p1.y as f64),
                                (p2.x as f64, p2.y as f64),
                                (p3.x as f64, p3.y as f64),
                            );
                        }
                        PathSegment::Close => bp.close_path(),
                    }
                }
            }
            Node::Group(group) => {
                let sub = flatten_group_to_path(group);
                bp.extend(sub.iter());
            }
            Node::Image(_) | Node::Text(_) => {}
        }
    }
    bp
}

fn convert_transform(t: &usvg::Transform) -> Affine {
    Affine::new([
        t.sx as f64,
        t.ky as f64,
        t.kx as f64,
        t.sy as f64,
        t.tx as f64,
        t.ty as f64,
    ])
}

fn convert_path(p: &usvg::Path) -> BezPath {
    let mut bp = BezPath::new();
    for seg in p.data().segments() {
        match seg {
            PathSegment::MoveTo(pt) => bp.move_to((pt.x as f64, pt.y as f64)),
            PathSegment::LineTo(pt) => bp.line_to((pt.x as f64, pt.y as f64)),
            PathSegment::QuadTo(p1, p2) => {
                bp.quad_to((p1.x as f64, p1.y as f64), (p2.x as f64, p2.y as f64));
            }
            PathSegment::CubicTo(p1, p2, p3) => {
                bp.curve_to(
                    (p1.x as f64, p1.y as f64),
                    (p2.x as f64, p2.y as f64),
                    (p3.x as f64, p3.y as f64),
                );
            }
            PathSegment::Close => bp.close_path(),
        }
    }
    bp
}

fn usvg_paint_to_color(paint: &usvg::Paint, opacity: usvg::Opacity) -> Color {
    match paint {
        usvg::Paint::Color(c) => {
            Color::from_rgba8(c.red, c.green, c.blue, (opacity.get() * 255.0) as u8)
        }
        // For gradients/patterns, fall back to a visible grey.
        _ => Color::from_rgba8(128, 128, 128, (opacity.get() * 255.0) as u8),
    }
}

// ── BenchScene impl ──────────────────────────────────────────────────────────

impl BenchScene for SvgScene {
    fn scene_id(&self) -> SceneId {
        SceneId::Svg
    }

    fn name(&self) -> &str {
        "Vector Graphics"
    }

    fn params(&self) -> Vec<Param> {
        vec![Param {
            id: ParamId::SvgAsset,
            label: "SVG Asset",
            kind: ParamKind::Select(
                self.assets
                    .iter()
                    .enumerate()
                    .map(|(i, a)| (a.name, i as f64))
                    .collect(),
            ),
            value: self.selected as f64,
        }]
    }

    fn set_param(&mut self, param: ParamId, value: f64) {
        if param == ParamId::SvgAsset {
            let idx = value as usize;
            if idx < self.assets.len() {
                self.selected = idx;
            }
        }
    }

    fn render(
        &mut self,
        backend: &mut dyn Renderer,
        width: u32,
        height: u32,
        _time: f64,
        view: Affine,
    ) {
        let asset = &self.assets[self.selected];

        // Scale to fit viewport, center.
        let s = (width as f64 / asset.width).min(height as f64 / asset.height);
        let tx = (width as f64 - asset.width * s) / 2.0;
        let ty = (height as f64 - asset.height * s) / 2.0;
        let base = view * Affine::translate((tx, ty)) * Affine::scale(s);

        for cmd in &asset.commands {
            match cmd {
                DrawCmd::Fill {
                    path,
                    transform,
                    color,
                } => {
                    backend.set_transform(base * *transform);
                    backend.set_paint((*color).into());
                    backend.fill_path(path);
                }
                DrawCmd::Stroke {
                    path,
                    transform,
                    color,
                    width,
                } => {
                    backend.set_transform(base * *transform);
                    backend.set_paint((*color).into());
                    backend.set_stroke(Stroke::new(*width));
                    backend.stroke_path(path);
                }
                DrawCmd::PushClip { path, transform } => {
                    backend.set_transform(base * *transform);
                    backend.push_clip_path(path);
                }
                DrawCmd::PopClip => {
                    backend.pop_clip_path();
                }
            }
        }

        backend.reset_transform();
    }
}
