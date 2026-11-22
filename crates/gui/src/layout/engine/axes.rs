//! The flex-relative views of Taffy's geometry its flexbox reads through
//! (`main`, `cross`, `main_start`, …), which Taffy keeps to itself, and the
//! sums of its geometry types written out side by side (as Taffy's `Add`
//! does them, in the same order).

use taffy::{AbsoluteAxis, FlexDirection, FlexWrap, Point, Rect, Size};

pub(super) const fn is_row(d: FlexDirection) -> bool {
    matches!(d, FlexDirection::Row | FlexDirection::RowReverse)
}

pub(super) const fn is_column(d: FlexDirection) -> bool {
    matches!(d, FlexDirection::Column | FlexDirection::ColumnReverse)
}

pub(super) const fn is_reverse(d: FlexDirection) -> bool {
    matches!(d, FlexDirection::RowReverse | FlexDirection::ColumnReverse)
}

pub(super) const fn main_axis(d: FlexDirection) -> AbsoluteAxis {
    if is_row(d) {
        AbsoluteAxis::Horizontal
    } else {
        AbsoluteAxis::Vertical
    }
}

pub(super) const fn cross_axis(d: FlexDirection) -> AbsoluteAxis {
    if is_row(d) {
        AbsoluteAxis::Vertical
    } else {
        AbsoluteAxis::Horizontal
    }
}

/// Taffy's `FlexWrap::is_multi_line`.
pub(super) fn is_multi_line(w: FlexWrap) -> bool {
    w != FlexWrap::NoWrap
}

/// Taffy's `FlexWrap::is_reverse`.
pub(super) const fn is_wrap_reverse(w: FlexWrap) -> bool {
    matches!(w, FlexWrap::WrapReverse | FlexWrap::BalanceReverse)
}

/// Taffy's `FlexWrap::is_balance`.
pub(super) const fn is_balance(w: FlexWrap) -> bool {
    matches!(w, FlexWrap::Balance | FlexWrap::BalanceReverse)
}

/// A size read in a flex container's axes.
pub(super) trait Axes<T: Copy> {
    fn main(self, d: FlexDirection) -> T;
    fn cross(self, d: FlexDirection) -> T;
    fn set_main(&mut self, d: FlexDirection, v: T);
    fn set_cross(&mut self, d: FlexDirection, v: T);
    #[must_use]
    fn with_main(self, d: FlexDirection, v: T) -> Self;
    #[must_use]
    fn with_cross(self, d: FlexDirection, v: T) -> Self;
}

impl<T: Copy> Axes<T> for Size<T> {
    fn main(self, d: FlexDirection) -> T {
        if is_row(d) { self.width } else { self.height }
    }
    fn cross(self, d: FlexDirection) -> T {
        if is_row(d) { self.height } else { self.width }
    }
    fn set_main(&mut self, d: FlexDirection, v: T) {
        if is_row(d) {
            self.width = v;
        } else {
            self.height = v;
        }
    }
    fn set_cross(&mut self, d: FlexDirection, v: T) {
        if is_row(d) {
            self.height = v;
        } else {
            self.width = v;
        }
    }
    fn with_main(mut self, d: FlexDirection, v: T) -> Self {
        self.set_main(d, v);
        self
    }
    fn with_cross(mut self, d: FlexDirection, v: T) -> Self {
        self.set_cross(d, v);
        self
    }
}

/// Taffy's `Point::main` and `Point::cross`.
pub(super) fn point_main<T: Copy>(p: Point<T>, d: FlexDirection) -> T {
    if is_row(d) { p.x } else { p.y }
}

pub(super) fn point_cross<T: Copy>(p: Point<T>, d: FlexDirection) -> T {
    if is_row(d) { p.y } else { p.x }
}

/// A rect's sides in a flex container's axes.
pub(super) trait Sides<T: Copy> {
    fn main_start(&self, d: FlexDirection) -> T;
    fn main_end(&self, d: FlexDirection) -> T;
    fn cross_start(&self, d: FlexDirection) -> T;
    fn cross_end(&self, d: FlexDirection) -> T;
}

impl<T: Copy> Sides<T> for Rect<T> {
    fn main_start(&self, d: FlexDirection) -> T {
        if is_row(d) { self.left } else { self.top }
    }
    fn main_end(&self, d: FlexDirection) -> T {
        if is_row(d) { self.right } else { self.bottom }
    }
    fn cross_start(&self, d: FlexDirection) -> T {
        if is_row(d) { self.top } else { self.left }
    }
    fn cross_end(&self, d: FlexDirection) -> T {
        if is_row(d) { self.bottom } else { self.right }
    }
}

/// Taffy's `Rect::main_axis_sum`.
pub(super) fn main_sum(r: Rect<f32>, d: FlexDirection) -> f32 {
    if is_row(d) {
        r.horizontal_axis_sum()
    } else {
        r.vertical_axis_sum()
    }
}

/// Taffy's `Rect::cross_axis_sum`.
pub(super) fn cross_sum(r: Rect<f32>, d: FlexDirection) -> f32 {
    if is_row(d) {
        r.vertical_axis_sum()
    } else {
        r.horizontal_axis_sum()
    }
}

/// Taffy's `Rect<f32> + Rect<f32>`.
pub(super) fn rect_add(a: Rect<f32>, b: Rect<f32>) -> Rect<f32> {
    Rect {
        left: a.left + b.left,
        right: a.right + b.right,
        top: a.top + b.top,
        bottom: a.bottom + b.bottom,
    }
}

/// Taffy's `Size<f32> + Size<f32>`.
pub(super) fn size_add(a: Size<f32>, b: Size<f32>) -> Size<f32> {
    Size {
        width: a.width + b.width,
        height: a.height + b.height,
    }
}

/// Taffy's `Size<f32> - Size<f32>`.
pub(super) fn size_sub(a: Size<f32>, b: Size<f32>) -> Size<f32> {
    Size {
        width: a.width - b.width,
        height: a.height - b.height,
    }
}

/// Taffy's `Rect::zip_size`.
pub(super) fn zip_size<T: Copy, U: Copy, R>(r: Rect<T>, s: Size<U>, f: impl Fn(T, U) -> R) -> Rect<R> {
    Rect {
        left: f(r.left, s.width),
        right: f(r.right, s.width),
        top: f(r.top, s.height),
        bottom: f(r.bottom, s.height),
    }
}
