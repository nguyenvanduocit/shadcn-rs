//! Vector shapes an element draws in its own box: chart lines, areas, bars,
//! dots. Coordinates are logical pixels from the element's top-left.

use crate::color::Color;

/// One step of a path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCommand {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    Close,
}

/// A path with how to paint it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Shape {
    pub commands: Vec<PathCommand>,
    pub fill: Option<Color>,
    /// Colour and width of the outline.
    pub stroke: Option<(Color, f32)>,
}

impl Shape {
    #[must_use]
    pub fn path() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn move_to(mut self, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::MoveTo(x, y));
        self
    }

    #[must_use]
    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::LineTo(x, y));
        self
    }

    #[must_use]
    pub fn quad_to(mut self, cx: f32, cy: f32, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::QuadTo(cx, cy, x, y));
        self
    }

    #[must_use]
    pub fn cubic_to(mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::CubicTo(c1x, c1y, c2x, c2y, x, y));
        self
    }

    #[must_use]
    pub fn close(mut self) -> Self {
        self.commands.push(PathCommand::Close);
        self
    }

    /// A rectangle.
    #[must_use]
    pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self::path()
            .move_to(x, y)
            .line_to(x + w, y)
            .line_to(x + w, y + h)
            .line_to(x, y + h)
            .close()
    }

    /// A circle, as four cubic arcs.
    #[must_use]
    pub fn circle(cx: f32, cy: f32, r: f32) -> Self {
        let k = 0.552_284_8 * r;
        Self::path()
            .move_to(cx + r, cy)
            .cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r)
            .cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy)
            .cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r)
            .cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy)
            .close()
    }

    #[must_use]
    pub fn fill(mut self, color: Color) -> Self {
        self.fill = Some(color);
        self
    }

    #[must_use]
    pub fn stroke(mut self, color: Color, width: f32) -> Self {
        self.stroke = Some((color, width));
        self
    }
}
