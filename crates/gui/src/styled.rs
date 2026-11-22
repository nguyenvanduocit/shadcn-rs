//! The style builder: one method per Tailwind utility shadcn uses, so a class
//! list reads across almost word for word (`px-4 py-2 rounded-md` →
//! `.px(4.0).py(2.0).rounded_md()`). Spacing and sizes take Tailwind's scale,
//! where 1 is 4px; `_px` variants take pixels.

use crate::color::Color;
use crate::motion::{Animation, Easing, Properties};
use crate::style::{
    Align, Axis, BackgroundImage, Calc, Corners, Cursor, Direction, Display, Edges, Filter, FlexDirection, FontFamily,
    GridSpan, Ink, Justify, Length, LineHeight, LinearGradient, MaskImage, Origin, Overflow, Position, Radius, Ring,
    ScrollFade, Shadow, Stop, Style, TextAlign, Track, WrapStyle,
};

/// Pixels in one step of Tailwind's spacing scale.
pub const UNIT: f32 = 4.0;

/// shadcn's base radius (`--radius: 0.625rem`), from which `rounded-sm` to
/// `rounded-3xl` derive ([`Radius`]); a subtree sets its own with `radius_var`.
pub const RADIUS: f32 = 10.0;

/// A length on Tailwind's scale.
#[must_use]
pub fn units(n: f32) -> Length {
    Length::Px(n * UNIT)
}

/// A length in pixels.
#[must_use]
pub fn px(v: f32) -> Length {
    Length::Px(v)
}

/// A fraction of the containing block.
#[must_use]
pub fn relative(fraction: f32) -> Length {
    Length::Fraction(fraction)
}

fn shadow(y: f32, blur: f32, spread: f32, alpha: f32) -> Shadow {
    Shadow {
        x: 0.0,
        y,
        blur,
        spread,
        color: Color::BLACK.opacity(alpha),
    }
}

/// The background gradient's stops, made empty (`bg-linear-to-b`) if there is none:
/// Tailwind's `from-*`, `via-*` and `to-*` edit them in any order.
fn gradient(s: &mut Style) -> &mut LinearGradient {
    &mut s
        .effects
        .background_image
        .get_or_insert_with(BackgroundImage::default)
        .gradient
}

/// Paint the first stop (`from-*`) or the last (`to-*`) with `color`.
fn stop_ink(s: &mut Style, last: bool, color: Color) {
    let stops = &mut gradient(s).stops;
    let stop = if last { stops.last_mut() } else { stops.first_mut() };
    if let Some(stop) = stop {
        stop.ink = Ink::Color(color);
    }
}

/// `via-*`: a middle stop at 50%, between `from` and `to`.
fn via(s: &mut Style, color: Color) {
    let stops = &mut gradient(s).stops;
    let mid = Stop {
        ink: Ink::Color(color),
        at: Calc::fraction(0.5),
    };
    if stops.len() > 2 {
        if let Some(stop) = stops.get_mut(1) {
            *stop = mid;
        }
    } else {
        stops.insert(1.min(stops.len()), mid);
    }
}

/// Fade these edges of a scroll container, keeping a size set before.
fn scroll_fade(s: &mut Style, axis: Axis, start: bool, end: bool) {
    let size = match s.effects.mask {
        Some(MaskImage::ScrollFade(f)) => f.size,
        _ => None,
    };
    s.effects.mask = Some(MaskImage::ScrollFade(ScrollFade { axis, start, end, size }));
}

/// `scroll-fade-n`: the fade's full size on Tailwind's scale.
fn scroll_fade_size(s: &mut Style, n: f32) {
    let size = Some(n * UNIT);
    if let Some(MaskImage::ScrollFade(f)) = &mut s.effects.mask {
        f.size = size;
    } else {
        s.effects.mask = Some(MaskImage::ScrollFade(ScrollFade {
            axis: Axis::Vertical,
            start: false,
            end: false,
            size,
        }));
    }
}

/// Conditional building, for elements and styles.
pub trait Fluent: Sized {
    /// Apply `f` when `condition` holds.
    #[must_use]
    fn when(self, condition: bool, f: impl FnOnce(Self) -> Self) -> Self {
        if condition { f(self) } else { self }
    }

    /// Apply `f` with the value when there is one.
    #[must_use]
    fn when_some<T>(self, value: Option<T>, f: impl FnOnce(Self, T) -> Self) -> Self {
        match value {
            Some(v) => f(self, v),
            None => self,
        }
    }
}

macro_rules! setters {
    ($($(#[$doc:meta])* $name:ident($($arg:ident: $ty:ty),*) => |$s:ident| $body:expr;)*) => {
        $(
            $(#[$doc])*
            #[must_use]
            fn $name(mut self, $($arg: $ty),*) -> Self {
                let $s = self.style_mut();
                $body;
                self
            }
        )*
    };
}

/// Anything with a [`Style`]: elements, and the style a `hover`/`focus`
/// refinement receives.
pub trait Styled: Sized {
    fn style_mut(&mut self) -> &mut Style;

    setters! {
        // ── display and position ──
        block() => |s| s.display = Display::Block;
        flex() => |s| s.display = Display::Flex;
        /// `inline-flex`: a flex box that shrinks to fit in block flow.
        inline_flex() => |s| { s.display = Display::Flex; s.inline = true };
        inline_block() => |s| { s.display = Display::Block; s.inline = true };
        grid() => |s| s.display = Display::Grid;
        /// `hidden`: not laid out, not painted.
        hidden() => |s| s.display = Display::None;
        invisible() => |s| s.invisible = true;
        relative() => |s| s.position = Position::Relative;
        absolute() => |s| s.position = Position::Absolute;
        inset(n: f32) => |s| s.inset = Edges::all(units(n));
        top(n: f32) => |s| s.inset.top = units(n);
        right(n: f32) => |s| s.inset.right = units(n);
        bottom(n: f32) => |s| s.inset.bottom = units(n);
        left(n: f32) => |s| s.inset.left = units(n);
        top_len(l: Length) => |s| s.inset.top = l;
        right_len(l: Length) => |s| s.inset.right = l;
        bottom_len(l: Length) => |s| s.inset.bottom = l;
        left_len(l: Length) => |s| s.inset.left = l;

        // ── flex ──
        flex_row() => |s| { s.display = Display::Flex; s.flex_direction = FlexDirection::Row };
        flex_col() => |s| { s.display = Display::Flex; s.flex_direction = FlexDirection::Column };
        flex_row_reverse() => |s| { s.display = Display::Flex; s.flex_direction = FlexDirection::RowReverse };
        flex_col_reverse() => |s| { s.display = Display::Flex; s.flex_direction = FlexDirection::ColumnReverse };
        flex_wrap() => |s| s.flex_wrap = true;
        /// `flex-1`: grow and shrink from a zero basis.
        flex_1() => |s| { s.flex_grow = 1.0; s.flex_shrink = 1.0; s.flex_basis = Length::Fraction(0.0) };
        flex_auto() => |s| { s.flex_grow = 1.0; s.flex_shrink = 1.0; s.flex_basis = Length::Auto };
        flex_none() => |s| { s.flex_grow = 0.0; s.flex_shrink = 0.0; s.flex_basis = Length::Auto };
        grow() => |s| s.flex_grow = 1.0;
        shrink_0() => |s| s.flex_shrink = 0.0;
        basis(n: f32) => |s| s.flex_basis = units(n);
        basis_len(l: Length) => |s| s.flex_basis = l;
        items_start() => |s| s.align_items = Some(Align::Start);
        items_center() => |s| s.align_items = Some(Align::Center);
        items_end() => |s| s.align_items = Some(Align::End);
        items_stretch() => |s| s.align_items = Some(Align::Stretch);
        items_baseline() => |s| s.align_items = Some(Align::Baseline);
        self_start() => |s| s.align_self = Some(Align::Start);
        self_center() => |s| s.align_self = Some(Align::Center);
        self_end() => |s| s.align_self = Some(Align::End);
        self_stretch() => |s| s.align_self = Some(Align::Stretch);
        justify_self_end() => |s| s.justify_self = Some(Align::End);
        justify_items_center() => |s| s.justify_items = Some(Align::Center);
        justify_start() => |s| s.justify_content = Some(Justify::Start);
        justify_center() => |s| s.justify_content = Some(Justify::Center);
        justify_end() => |s| s.justify_content = Some(Justify::End);
        justify_between() => |s| s.justify_content = Some(Justify::Between);
        justify_around() => |s| s.justify_content = Some(Justify::Around);
        justify_evenly() => |s| s.justify_content = Some(Justify::Evenly);
        content_start() => |s| s.align_content = Some(Justify::Start);
        content_center() => |s| s.align_content = Some(Justify::Center);
        gap(n: f32) => |s| { s.row_gap = n * UNIT; s.column_gap = n * UNIT };
        gap_x(n: f32) => |s| s.column_gap = n * UNIT;
        gap_y(n: f32) => |s| s.row_gap = n * UNIT;

        // ── grid ──
        /// `grid-cols-n`: n equal columns.
        grid_cols(n: u16) => |s| s.grid_columns = vec![Track::Fr(1.0); usize::from(n)];
        grid_cols_tracks(tracks: Vec<Track>) => |s| s.grid_columns = tracks;
        grid_rows(n: u16) => |s| s.grid_rows = vec![Track::Fr(1.0); usize::from(n)];
        grid_rows_tracks(tracks: Vec<Track>) => |s| s.grid_rows = tracks;
        auto_rows_min() => |s| s.grid_auto_rows = vec![Track::MinContent];
        grid_flow_col() => |s| s.grid_flow_column = true;
        col_span(n: u16) => |s| s.grid_column.span = Some(n);
        col_start(n: i16) => |s| s.grid_column.start = Some(n);
        row_span(n: u16) => |s| s.grid_row.span = Some(n);
        row_start(n: i16) => |s| s.grid_row.start = Some(n);
        col_span_full() => |s| s.grid_column = GridSpan { start: Some(1), span: None };

        // ── spacing ──
        p(n: f32) => |s| s.padding = Edges::all(n * UNIT);
        px(n: f32) => |s| { s.padding.left = n * UNIT; s.padding.right = n * UNIT };
        py(n: f32) => |s| { s.padding.top = n * UNIT; s.padding.bottom = n * UNIT };
        pt(n: f32) => |s| s.padding.top = n * UNIT;
        pr(n: f32) => |s| s.padding.right = n * UNIT;
        pb(n: f32) => |s| s.padding.bottom = n * UNIT;
        pl(n: f32) => |s| s.padding.left = n * UNIT;
        m(n: f32) => |s| s.margin = Edges::all(units(n));
        mx(n: f32) => |s| { s.margin.left = units(n); s.margin.right = units(n) };
        my(n: f32) => |s| { s.margin.top = units(n); s.margin.bottom = units(n) };
        mt(n: f32) => |s| s.margin.top = units(n);
        mr(n: f32) => |s| s.margin.right = units(n);
        mb(n: f32) => |s| s.margin.bottom = units(n);
        ml(n: f32) => |s| s.margin.left = units(n);
        mx_auto() => |s| { s.margin.left = Length::Auto; s.margin.right = Length::Auto };
        ml_auto() => |s| s.margin.left = Length::Auto;
        mr_auto() => |s| s.margin.right = Length::Auto;
        mt_auto() => |s| s.margin.top = Length::Auto;
        /// `ms-*`: margin on the inline start (left, right under `dir_rtl`).
        ms(n: f32) => |s| s.margin_start = Some(units(n));
        /// `me-*`: margin on the inline end.
        me(n: f32) => |s| s.margin_end = Some(units(n));
        ms_auto() => |s| s.margin_start = Some(Length::Auto);
        me_auto() => |s| s.margin_end = Some(Length::Auto);

        // ── sizing ──
        w(n: f32) => |s| s.width = units(n);
        w_px(v: f32) => |s| s.width = px(v);
        w_len(l: Length) => |s| s.width = l;
        w_full() => |s| s.width = relative(1.0);
        w_auto() => |s| s.width = Length::Auto;
        /// `w-fit`: as wide as the content, at most the space there is.
        w_fit() => |s| s.width = Length::FitContent;
        h(n: f32) => |s| s.height = units(n);
        h_px(v: f32) => |s| s.height = px(v);
        h_len(l: Length) => |s| s.height = l;
        h_full() => |s| s.height = relative(1.0);
        h_auto() => |s| s.height = Length::Auto;
        /// `size-n`: width and height.
        size(n: f32) => |s| { s.width = units(n); s.height = units(n) };
        /// Size of the icons below that set none (`[&_svg]:size-n`).
        icon_size(n: f32) => |s| s.text.icon_size = Some(n * UNIT);
        size_px(v: f32) => |s| { s.width = px(v); s.height = px(v) };
        size_full() => |s| { s.width = relative(1.0); s.height = relative(1.0) };
        min_w(n: f32) => |s| s.min_width = units(n);
        min_w_px(v: f32) => |s| s.min_width = px(v);
        min_w_full() => |s| s.min_width = relative(1.0);
        min_h(n: f32) => |s| s.min_height = units(n);
        min_h_px(v: f32) => |s| s.min_height = px(v);
        max_w(n: f32) => |s| s.max_width = units(n);
        max_w_px(v: f32) => |s| s.max_width = px(v);
        max_w_full() => |s| s.max_width = relative(1.0);
        /// `max-w-[calc(fraction × 100% + px)]`, as `max-w-[calc(100%-2rem)]` is `max_w_calc(1.0, -32.0)`.
        max_w_calc(fraction: f32, px: f32) => |s| s.max_width = Length::Calc { fraction, px };
        max_h(n: f32) => |s| s.max_height = units(n);
        max_h_px(v: f32) => |s| s.max_height = px(v);
        aspect_square() => |s| s.aspect_ratio = Some(1.0);
        aspect(ratio: f32) => |s| s.aspect_ratio = Some(ratio);

        // ── overflow ──
        overflow_hidden() => |s| { s.overflow_x = Overflow::Hidden; s.overflow_y = Overflow::Hidden };
        overflow_x_hidden() => |s| s.overflow_x = Overflow::Hidden;
        overflow_y_scroll() => |s| { s.overflow_y = Overflow::Scroll; if s.overflow_x == Overflow::Visible { s.overflow_x = Overflow::Hidden } };
        overflow_x_scroll() => |s| { s.overflow_x = Overflow::Scroll; if s.overflow_y == Overflow::Visible { s.overflow_y = Overflow::Hidden } };
        overflow_scroll() => |s| { s.overflow_x = Overflow::Scroll; s.overflow_y = Overflow::Scroll };

        // ── background, border, radius ──
        bg(c: Color) => |s| s.background = Some(c);
        /// A gradient background tile; the `bg-linear-*` setters build the common ones.
        background_image(image: BackgroundImage) => |s| s.effects.background_image = Some(image);
        bg_linear_to_t() => |s| gradient(s).angle = 0.0;
        bg_linear_to_r() => |s| gradient(s).angle = 90.0;
        bg_linear_to_b() => |s| gradient(s).angle = 180.0;
        bg_linear_to_l() => |s| gradient(s).angle = 270.0;
        /// `from-*`: the gradient's first colour.
        from(c: Color) => |s| stop_ink(s, false, c);
        /// `from-n%`: where the first colour ends, a fraction of the gradient line.
        gradient_from_at(fraction: f32) => |s| if let Some(stop) = gradient(s).stops.first_mut() { stop.at = Calc::fraction(fraction) };
        via(c: Color) => |s| via(s, c);
        /// `to-*`: the gradient's last colour.
        to(c: Color) => |s| stop_ink(s, true, c);
        /// `border`: 1px on every side.
        border() => |s| s.border_widths = Edges::all(1.0);
        border_w(v: f32) => |s| s.border_widths = Edges::all(v);
        border_t() => |s| s.border_widths.top = 1.0;
        border_r() => |s| s.border_widths.right = 1.0;
        border_b() => |s| s.border_widths.bottom = 1.0;
        border_l() => |s| s.border_widths.left = 1.0;
        /// `border-l-n`: an `n`-pixel left edge.
        border_l_w(v: f32) => |s| s.border_widths.left = v;
        border_0() => |s| s.border_widths = Edges::all(0.0);
        border_color(c: Color) => |s| s.border_color = Some(c);
        /// `rounded-[…]`: pixels, or a [`Radius`] derived from `--radius`.
        rounded(r: impl Into<Radius>) => |s| s.radius = Corners::all(r.into());
        rounded_none() => |s| s.radius = Corners::all(Radius::px(0.0));
        rounded_xs() => |s| s.radius = Corners::all(Radius::px(2.0));
        rounded_sm() => |s| s.radius = Corners::all(Radius::SM);
        rounded_md() => |s| s.radius = Corners::all(Radius::MD);
        rounded_lg() => |s| s.radius = Corners::all(Radius::LG);
        rounded_xl() => |s| s.radius = Corners::all(Radius::XL);
        rounded_2xl() => |s| s.radius = Corners::all(Radius::XL2);
        rounded_3xl() => |s| s.radius = Corners::all(Radius::XL3);
        rounded_full() => |s| s.radius = Corners::all(Radius::px(9999.0));
        rounded_t(r: impl Into<Radius>) => |s| { let r = r.into(); s.radius.top_left = r; s.radius.top_right = r };
        rounded_b(r: impl Into<Radius>) => |s| { let r = r.into(); s.radius.bottom_left = r; s.radius.bottom_right = r };
        rounded_l(r: impl Into<Radius>) => |s| { let r = r.into(); s.radius.top_left = r; s.radius.bottom_left = r };
        rounded_r(r: impl Into<Radius>) => |s| { let r = r.into(); s.radius.top_right = r; s.radius.bottom_right = r };
        /// `[--radius:…]`: the base every `rounded-sm` to `rounded-xl` (any
        /// [`Radius`]) below this node derives from, in pixels.
        radius_var(px: f32) => |s| s.text.radius = Some(px);

        // ── effects ──
        shadow_none() => |s| s.shadows.clear();
        shadow_xs() => |s| s.shadows = vec![shadow(1.0, 2.0, 0.0, 0.05)];
        shadow_sm() => |s| s.shadows = vec![shadow(1.0, 3.0, 0.0, 0.1), shadow(1.0, 2.0, -1.0, 0.1)];
        shadow_md() => |s| s.shadows = vec![shadow(4.0, 6.0, -1.0, 0.1), shadow(2.0, 4.0, -2.0, 0.1)];
        shadow_lg() => |s| s.shadows = vec![shadow(10.0, 15.0, -3.0, 0.1), shadow(4.0, 6.0, -4.0, 0.1)];
        shadow_xl() => |s| s.shadows = vec![shadow(20.0, 25.0, -5.0, 0.1), shadow(8.0, 10.0, -6.0, 0.1)];
        shadow(sh: Shadow) => |s| s.shadows.push(sh);
        /// `ring-[w] ring-color`: a solid outline outside the border box.
        ring(width: f32, color: Color) => |s| s.ring = Some(Ring { width, color });
        ring_none() => |s| s.ring = None;
        opacity(v: f32) => |s| s.opacity = v;
        /// `rotate-n`: turn the node's painting `degrees` clockwise about its origin.
        rotate(degrees: f32) => |s| s.transform.rotate = degrees;
        /// `scale-n`: scale the node's painting by `n / 100` about its origin.
        scale(factor: f32) => |s| s.transform.scale = factor;
        /// `translate-x-*`: move the node, a fraction of its own width plus pixels.
        translate_x(x: Calc) => |s| s.transform.translate[0] = x;
        translate_y(y: Calc) => |s| s.transform.translate[1] = y;
        /// `origin-*`: the point it scales and turns about.
        origin(o: Origin) => |s| s.transform.origin = o;

        // ── motion ──
        /// `transition`: colours, opacity, shadow and transform ease to new values.
        transition() => |s| s.motion.get_or_insert_default().transition = Properties::DEFAULT;
        transition_all() => |s| s.motion.get_or_insert_default().transition = Properties::ALL;
        transition_colors() => |s| s.motion.get_or_insert_default().transition = Properties::COLORS;
        transition_opacity() => |s| s.motion.get_or_insert_default().transition = Properties::OPACITY;
        transition_shadow() => |s| s.motion.get_or_insert_default().transition = Properties::SHADOW;
        transition_transform() => |s| s.motion.get_or_insert_default().transition = Properties::TRANSFORM;
        /// `transition-[…]`: these properties ease (`transition-[color,box-shadow]`
        /// is `Properties::COLOR | Properties::SHADOW`).
        transition_props(p: Properties) => |s| s.motion.get_or_insert_default().transition = p;
        transition_none() => |s| s.motion.get_or_insert_default().transition = Properties::NONE;
        /// `duration-n`: transitions and tw-animate animations take `ms` milliseconds.
        duration_ms(ms: f32) => |s| s.motion.get_or_insert_default().duration = Some(ms / 1000.0);
        delay_ms(ms: f32) => |s| s.motion.get_or_insert_default().delay = Some(ms / 1000.0);
        ease_linear() => |s| s.motion.get_or_insert_default().easing = Some(Easing::Linear);
        ease_in() => |s| s.motion.get_or_insert_default().easing = Some(Easing::TW_IN);
        ease_out() => |s| s.motion.get_or_insert_default().easing = Some(Easing::TW_OUT);
        ease_in_out() => |s| s.motion.get_or_insert_default().easing = Some(Easing::TW_IN_OUT);
        /// `ease-[cubic-bezier(…)]`.
        ease(e: Easing) => |s| s.motion.get_or_insert_default().easing = Some(e);
        /// `animate-*`: play `a` from the frame the node first has it.
        animate(a: Animation) => |s| s.motion.get_or_insert_default().animation = Some(std::rc::Rc::new(a));
        animate_none() => |s| s.motion.get_or_insert_default().animation = None;
        /// `data-[state=closed]:animate-out`: play `a` when the node leaves
        /// the tree, which keeps it painted, inert, until `a` ends.
        animate_out(a: Animation) => |s| s.motion.get_or_insert_default().exit = Some(std::rc::Rc::new(a));
        /// `mask-image: linear-gradient(…)` over this node and its children.
        mask_image(g: LinearGradient) => |s| s.effects.mask = Some(MaskImage::Linear(g));
        /// `scroll-fade`, `scroll-fade-y`: fade the top and bottom where more content lies beyond.
        scroll_fade() => |s| scroll_fade(s, Axis::Vertical, true, true);
        scroll_fade_x() => |s| scroll_fade(s, Axis::Horizontal, true, true);
        scroll_fade_t() => |s| scroll_fade(s, Axis::Vertical, true, false);
        scroll_fade_b() => |s| scroll_fade(s, Axis::Vertical, false, true);
        /// `scroll-fade-s`, `scroll-fade-l`: the inline start (left; `gui` lays out left to right).
        scroll_fade_s() => |s| scroll_fade(s, Axis::Horizontal, true, false);
        /// `scroll-fade-e`, `scroll-fade-r`.
        scroll_fade_e() => |s| scroll_fade(s, Axis::Horizontal, false, true);
        /// `scroll-fade-n`: the fade's full size, `n` on Tailwind's scale.
        scroll_fade_size(n: f32) => |s| scroll_fade_size(s, n);
        scroll_fade_none() => |s| s.effects.mask = None;
        scrollbar_none() => |s| s.scrollbar_none = true;
        /// `brightness-*`: every colour channel times `k`.
        brightness(k: f32) => |s| s.effects.filters.push(Filter::Brightness(k));
        grayscale() => |s| s.effects.filters.push(Filter::Grayscale(1.0));
        cursor(c: Cursor) => |s| s.cursor = Some(c);
        cursor_pointer() => |s| s.cursor = Some(Cursor::Pointer);
        cursor_default() => |s| s.cursor = Some(Cursor::Default);
        cursor_text() => |s| s.cursor = Some(Cursor::Text);
        cursor_not_allowed() => |s| s.cursor = Some(Cursor::NotAllowed);

        // ── text ──
        text_color(c: Color) => |s| s.text.color = Some(c);
        /// Font size in pixels with Tailwind's paired line height.
        text_size(size: f32, line_height: f32) => |s| { s.text.size = Some(size); s.text.line_height = Some(LineHeight::Px(line_height)) };
        text_xs() => |s| { s.text.size = Some(12.0); s.text.line_height = Some(LineHeight::Px(16.0)) };
        text_sm() => |s| { s.text.size = Some(14.0); s.text.line_height = Some(LineHeight::Px(20.0)) };
        text_base() => |s| { s.text.size = Some(16.0); s.text.line_height = Some(LineHeight::Px(24.0)) };
        text_lg() => |s| { s.text.size = Some(18.0); s.text.line_height = Some(LineHeight::Px(28.0)) };
        text_xl() => |s| { s.text.size = Some(20.0); s.text.line_height = Some(LineHeight::Px(28.0)) };
        text_2xl() => |s| { s.text.size = Some(24.0); s.text.line_height = Some(LineHeight::Px(32.0)) };
        text_3xl() => |s| { s.text.size = Some(30.0); s.text.line_height = Some(LineHeight::Px(36.0)) };
        text_4xl() => |s| { s.text.size = Some(36.0); s.text.line_height = Some(LineHeight::Px(40.0)) };
        leading_none() => |s| s.text.line_height = Some(LineHeight::Relative(1.0));
        leading_tight() => |s| s.text.line_height = Some(LineHeight::Relative(1.25));
        leading_snug() => |s| s.text.line_height = Some(LineHeight::Relative(1.375));
        leading_normal() => |s| s.text.line_height = Some(LineHeight::Relative(1.5));
        leading_relaxed() => |s| s.text.line_height = Some(LineHeight::Relative(1.625));
        leading_px(v: f32) => |s| s.text.line_height = Some(LineHeight::Px(v));
        font_normal() => |s| s.text.weight = Some(400);
        font_medium() => |s| s.text.weight = Some(500);
        font_semibold() => |s| s.text.weight = Some(600);
        font_bold() => |s| s.text.weight = Some(700);
        font_extrabold() => |s| s.text.weight = Some(800);
        font_sans() => |s| s.text.family = Some(FontFamily::Sans);
        font_mono() => |s| s.text.family = Some(FontFamily::Mono);
        text_left() => |s| s.text.align = Some(TextAlign::Left);
        text_start() => |s| s.text.align = Some(TextAlign::Start);
        text_end() => |s| s.text.align = Some(TextAlign::End);
        /// `dir="rtl"`: this node and what it holds run right to left.
        dir_rtl() => |s| s.text.direction = Some(Direction::Rtl);
        /// `dir="ltr"`.
        dir_ltr() => |s| s.text.direction = Some(Direction::Ltr);
        text_center() => |s| s.text.align = Some(TextAlign::Center);
        text_right() => |s| s.text.align = Some(TextAlign::Right);
        whitespace_nowrap() => |s| s.text.nowrap = Some(true);
        /// `truncate`: one line, cut with an ellipsis, and the box may shrink below its text.
        truncate() => |s| { s.text.truncate = Some(true); s.text.nowrap = Some(true); s.overflow_x = Overflow::Hidden; s.min_width = Length::Px(0.0) };
        line_clamp(lines: u16) => |s| s.text.line_clamp = Some(lines);
        /// `text-balance`: wrapped lines of even width, as narrow as their count allows.
        text_balance() => |s| s.text.wrap_style = Some(WrapStyle::Balance);
        /// `text-pretty`: the last line is never one word alone.
        text_pretty() => |s| s.text.wrap_style = Some(WrapStyle::Pretty);
        italic() => |s| s.text.italic = Some(true);
        underline() => |s| s.text.underline = Some(true);
        no_underline() => |s| s.text.underline = Some(false);
        /// `underline-offset-n`: the underline `n` pixels below the baseline.
        underline_offset(px: f32) => |s| s.text.underline_offset = Some(px);
        line_through() => |s| s.text.line_through = Some(true);
        tracking_tight() => |s| s.text.tracking = Some(-0.025);
        tracking_wide() => |s| s.text.tracking = Some(0.025);
        tracking_widest() => |s| s.text.tracking = Some(0.1);
        /// `tabular-nums`: every digit as wide as the others.
        tabular_nums() => |s| s.text.tabular_nums = Some(true);
    }
}

impl Fluent for Style {}

impl Styled for Style {
    fn style_mut(&mut self) -> &mut Style {
        self
    }
}
