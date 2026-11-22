//! A property's value: fixed, or bound to signals (ADR-0013, ADR-0022).
//!
//! Every [`crate::Styled`] setter takes either a value (`.padding(16.0)`)
//! or a closure that gives one (`.padding(move || if dense.get() { 8.0 } else { 16.0 })`).
//! A closure is kept on the node and run where the node's style resolves,
//! under that node's own tracker: a signal it read changing resolves that
//! node again, and the style diff (ADR-0014) then dirties only the phase the
//! property belongs to (a colour repaints, a padding lays out). The
//! instance that built the node does not render again.
//!
//! A property has one owner: the last setter that wrote it, fixed or
//! bound. A write is known by the [`Field`]s of the [`Style`] it writes,
//! not by the setter's name: a later write of every field an earlier
//! binding wrote drops that binding (`.padding_x(move || …)` then
//! `.padding(8.0)`, or `.width(move || …)` then `.size(40.0)`); writes that
//! only overlap apply in call order, as the setters were written.
//!
//! Conversions a setter accepts beyond its own type (`f32` pixels for a
//! [`crate::style::Length`], a [`crate::style::Shadow`] for the node's
//! [`crate::style::Shadows`]) are [`Convert`] impls; a closure may return
//! any of them too.

use std::any::Any;
use std::rc::Rc;

use smallvec::SmallVec;

use crate::style::Style;

/// What a setter was given: a value, or a closure that gives one.
pub enum Value<T> {
    Fixed(T),
    Bound(Rc<dyn Fn() -> T>),
}

/// Anything a setter of `T` takes. `M` tells the impls apart (a value of
/// `T`, a value that converts to `T`, or a closure giving either); the
/// compiler infers it, and no caller names it.
pub trait IntoValue<T, M> {
    fn into_value(self) -> Value<T>;
}

/// A source of `T` that a setter of `T` takes as well: pixels for a length,
/// one shadow for a list of them.
pub trait Convert<T> {
    fn convert(self) -> T;
}

/// The markers of [`IntoValue`]'s impls.
pub mod marker {
    use std::marker::PhantomData;

    /// A value of the property's own type.
    pub struct Fixed;
    /// A closure giving the property's own type.
    pub struct Bound;
    /// A value that [`super::Convert`]s to the property's type.
    pub struct Via<S>(PhantomData<S>);
    /// A closure giving a value that [`super::Convert`]s.
    pub struct BoundVia<S>(PhantomData<S>);
}

impl<T> IntoValue<T, marker::Fixed> for T {
    fn into_value(self) -> Value<T> {
        Value::Fixed(self)
    }
}

impl<T, F: Fn() -> T + 'static> IntoValue<T, marker::Bound> for F {
    fn into_value(self) -> Value<T> {
        Value::Bound(Rc::new(self))
    }
}

impl<T, S: Convert<T>> IntoValue<T, marker::Via<S>> for S {
    fn into_value(self) -> Value<T> {
        Value::Fixed(self.convert())
    }
}

impl<T, S: Convert<T>, F: Fn() -> S + 'static> IntoValue<T, marker::BoundVia<S>> for F {
    fn into_value(self) -> Value<T> {
        Value::Bound(Rc::new(move || self().convert()))
    }
}

impl<T> Convert<Option<T>> for T {
    fn convert(self) -> Option<T> {
        Some(self)
    }
}

/// A short list kept inline, so that writing it (a binding that sets a
/// node's shadows on hover) allocates nothing; a longer one spills to the
/// heap. Shadows, filters and grid tracks are kept so.
#[derive(Clone, PartialEq)]
pub struct Inline<T, const N: usize>(SmallVec<[T; N]>);

impl<T, const N: usize> Inline<T, N> {
    /// The empty list.
    #[must_use]
    pub const fn new() -> Self {
        Self(SmallVec::new_const())
    }

    /// `n` copies of `item` (`repeat(3, Track::Fr(1.0))` is CSS's `repeat(3, 1fr)`).
    #[must_use]
    pub fn repeat(n: usize, item: T) -> Self
    where
        T: Clone,
    {
        Self(SmallVec::from_elem(item, n))
    }

    /// Whether it holds anything on the heap: it never does at `N` items or fewer.
    #[must_use]
    pub fn spilled(&self) -> bool {
        self.0.spilled()
    }

    pub fn push(&mut self, item: T) {
        self.0.push(item);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

impl<T, const N: usize> Default for Inline<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: std::fmt::Debug, const N: usize> std::fmt::Debug for Inline<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.0.iter()).finish()
    }
}

impl<T, const N: usize> std::ops::Deref for Inline<T, N> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T, const N: usize> std::ops::DerefMut for Inline<T, N> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}

impl<T, const N: usize> FromIterator<T> for Inline<T, N> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<'a, T, const N: usize> IntoIterator for &'a Inline<T, N> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T, const N: usize, const K: usize> From<[T; K]> for Inline<T, N> {
    fn from(items: [T; K]) -> Self {
        items.into_iter().collect()
    }
}

impl<T, const N: usize> From<Vec<T>> for Inline<T, N> {
    fn from(items: Vec<T>) -> Self {
        items.into_iter().collect()
    }
}

impl<T, const N: usize> Convert<Inline<T, N>> for T {
    fn convert(self) -> Inline<T, N> {
        let mut v = Inline::new();
        v.push(self);
        v
    }
}

impl<T, const N: usize, const K: usize> Convert<Inline<T, N>> for [T; K] {
    fn convert(self) -> Inline<T, N> {
        self.into_iter().collect()
    }
}

impl<T: Clone, const N: usize> Convert<Inline<T, N>> for &[T] {
    fn convert(self) -> Inline<T, N> {
        self.iter().cloned().collect()
    }
}

impl<T, const N: usize> Convert<Inline<T, N>> for Vec<T> {
    fn convert(self) -> Inline<T, N> {
        self.into_iter().collect()
    }
}

/// One field of a [`Style`] a setter writes: a side of the padding, a
/// corner's radius, the text colour. A setter writes one or more.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Display,
    Inline,
    Invisible,
    Position,
    InsetTop,
    InsetRight,
    InsetBottom,
    InsetLeft,
    FlexDirection,
    FlexWrap,
    FlexGrow,
    FlexShrink,
    FlexBasis,
    AlignItems,
    AlignSelf,
    JustifyItems,
    JustifySelf,
    JustifyContent,
    AlignContent,
    RowGap,
    ColumnGap,
    GridColumns,
    GridRows,
    GridAutoRows,
    GridAutoFlow,
    GridColumnStart,
    GridColumnSpan,
    GridRowStart,
    GridRowSpan,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    MarginStart,
    MarginEnd,
    Width,
    Height,
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,
    AspectRatio,
    IconSize,
    OverflowX,
    OverflowY,
    ScrollbarHidden,
    Background,
    BackgroundImage,
    BorderTop,
    BorderRight,
    BorderBottom,
    BorderLeft,
    BorderColor,
    RadiusTopLeft,
    RadiusTopRight,
    RadiusBottomRight,
    RadiusBottomLeft,
    Shadows,
    Ring,
    Opacity,
    Rotate,
    Scale,
    TranslateX,
    TranslateY,
    Origin,
    Mask,
    Filters,
    Cursor,
    Transition,
    Duration,
    Delay,
    Easing,
    Animation,
    ExitAnimation,
    TextColor,
    TextSize,
    LineHeight,
    FontWeight,
    FontFamily,
    TextAlign,
    Direction,
    Nowrap,
    Truncate,
    LineClamp,
    TextWrap,
    Italic,
    Underline,
    UnderlineOffset,
    LineThrough,
    Tracking,
    TabularNums,
    StrokeWidth,
    PlaceholderColor,
    SelectionColor,
    EdgeBar,
    InsetLayer,
    OverlayBorder,
    Backdrop,
    HitSlop,
}

impl Field {
    /// Its bit in [`Fields`].
    const fn bit(self) -> u128 {
        match self {
            Self::Display => 1 << 0,
            Self::Inline => 1 << 1,
            Self::Invisible => 1 << 2,
            Self::Position => 1 << 3,
            Self::InsetTop => 1 << 4,
            Self::InsetRight => 1 << 5,
            Self::InsetBottom => 1 << 6,
            Self::InsetLeft => 1 << 7,
            Self::FlexDirection => 1 << 8,
            Self::FlexWrap => 1 << 9,
            Self::FlexGrow => 1 << 10,
            Self::FlexShrink => 1 << 11,
            Self::FlexBasis => 1 << 12,
            Self::AlignItems => 1 << 13,
            Self::AlignSelf => 1 << 14,
            Self::JustifyItems => 1 << 15,
            Self::JustifySelf => 1 << 16,
            Self::JustifyContent => 1 << 17,
            Self::AlignContent => 1 << 18,
            Self::RowGap => 1 << 19,
            Self::ColumnGap => 1 << 20,
            Self::GridColumns => 1 << 21,
            Self::GridRows => 1 << 22,
            Self::GridAutoRows => 1 << 23,
            Self::GridAutoFlow => 1 << 24,
            Self::GridColumnStart => 1 << 25,
            Self::GridColumnSpan => 1 << 26,
            Self::GridRowStart => 1 << 27,
            Self::GridRowSpan => 1 << 28,
            Self::PaddingTop => 1 << 29,
            Self::PaddingRight => 1 << 30,
            Self::PaddingBottom => 1 << 31,
            Self::PaddingLeft => 1 << 32,
            Self::MarginTop => 1 << 33,
            Self::MarginRight => 1 << 34,
            Self::MarginBottom => 1 << 35,
            Self::MarginLeft => 1 << 36,
            Self::MarginStart => 1 << 37,
            Self::MarginEnd => 1 << 38,
            Self::Width => 1 << 39,
            Self::Height => 1 << 40,
            Self::MinWidth => 1 << 41,
            Self::MinHeight => 1 << 42,
            Self::MaxWidth => 1 << 43,
            Self::MaxHeight => 1 << 44,
            Self::AspectRatio => 1 << 45,
            Self::IconSize => 1 << 46,
            Self::OverflowX => 1 << 47,
            Self::OverflowY => 1 << 48,
            Self::ScrollbarHidden => 1 << 49,
            Self::Background => 1 << 50,
            Self::BackgroundImage => 1 << 51,
            Self::BorderTop => 1 << 52,
            Self::BorderRight => 1 << 53,
            Self::BorderBottom => 1 << 54,
            Self::BorderLeft => 1 << 55,
            Self::BorderColor => 1 << 56,
            Self::RadiusTopLeft => 1 << 57,
            Self::RadiusTopRight => 1 << 58,
            Self::RadiusBottomRight => 1 << 59,
            Self::RadiusBottomLeft => 1 << 60,
            Self::Shadows => 1 << 61,
            Self::Ring => 1 << 62,
            Self::Opacity => 1 << 63,
            Self::Rotate => 1 << 64,
            Self::Scale => 1 << 65,
            Self::TranslateX => 1 << 66,
            Self::TranslateY => 1 << 67,
            Self::Origin => 1 << 68,
            Self::Mask => 1 << 69,
            Self::Filters => 1 << 70,
            Self::Cursor => 1 << 71,
            Self::Transition => 1 << 72,
            Self::Duration => 1 << 73,
            Self::Delay => 1 << 74,
            Self::Easing => 1 << 75,
            Self::Animation => 1 << 76,
            Self::ExitAnimation => 1 << 77,
            Self::TextColor => 1 << 78,
            Self::TextSize => 1 << 79,
            Self::LineHeight => 1 << 80,
            Self::FontWeight => 1 << 81,
            Self::FontFamily => 1 << 82,
            Self::TextAlign => 1 << 83,
            Self::Direction => 1 << 84,
            Self::Nowrap => 1 << 85,
            Self::Truncate => 1 << 86,
            Self::LineClamp => 1 << 87,
            Self::TextWrap => 1 << 88,
            Self::Italic => 1 << 89,
            Self::Underline => 1 << 90,
            Self::UnderlineOffset => 1 << 91,
            Self::LineThrough => 1 << 92,
            Self::Tracking => 1 << 93,
            Self::TabularNums => 1 << 94,
            Self::StrokeWidth => 1 << 95,
            Self::PlaceholderColor => 1 << 96,
            Self::SelectionColor => 1 << 97,
            Self::EdgeBar => 1 << 98,
            Self::InsetLayer => 1 << 99,
            Self::OverlayBorder => 1 << 100,
            Self::Backdrop => 1 << 101,
            Self::HitSlop => 1 << 102,
        }
    }
}

/// The [`Field`]s one write sets, a bit each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fields(u128);

impl Fields {
    /// The fields in `fields`.
    #[must_use]
    pub const fn of(fields: &[Field]) -> Self {
        let (mut bits, mut rest) = (0_u128, fields);
        while let [field, tail @ ..] = rest {
            bits |= field.bit();
            rest = tail;
        }
        Self(bits)
    }

    /// Whether every field of `other` is one of these.
    #[must_use]
    pub const fn covers(self, other: Self) -> bool {
        other.0 & !self.0 == 0
    }

    /// Whether any field of `other` is one of these.
    #[must_use]
    pub const fn meets(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

/// What one replayed write is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A fixed value written after a binding (kept so the order of writes
    /// holds; the fixed style has it too).
    Fixed,
    /// A bound setter.
    Bound,
    /// A parent's restyle of a node that has bindings
    /// ([`crate::Element::restyle`]): it runs after the node's own writes,
    /// so the properties it writes are the parent's. The fixed style does
    /// not have it.
    Restyle,
}

/// One write a node's resolve replays.
#[derive(Clone)]
pub(crate) struct Op {
    fields: Fields,
    kind: Kind,
    run: Run,
}

/// What a write runs.
#[derive(Clone)]
enum Run {
    /// A fixed value, or a parent's restyle: it writes as it runs.
    Write(Rc<dyn Fn(&mut Style)>),
    /// A closure: run on its own ([`Bound::eval`]), what it gave written
    /// where the writes replay ([`Bound::write`]).
    Bound(Rc<dyn Bound>),
}

/// A bound setter's closure and the write of what it gives, kept apart so
/// a node's resolve runs each closure under a tracker of its own and
/// replays the others from what they last gave (ADR-0022: a binding that
/// reads `focus_visible` does not run again on hover).
pub(crate) trait Bound {
    /// Run the closure, tracked by whatever is running, keeping what it
    /// gave in `slot`: in the value already there when it is of its type,
    /// so a run allocates nothing once the slot is made.
    fn eval(&self, slot: &mut Option<Box<dyn Any>>);
    /// Write what `slot` holds ([`Self::eval`] filled it).
    fn write(&self, slot: &dyn Any, style: &mut Style);
    /// Run the closure and write what it gives.
    fn apply(&self, style: &mut Style);
}

struct BoundWrite<T> {
    f: Rc<dyn Fn() -> T>,
    write: fn(&mut Style, T),
}

impl<T: Clone + 'static> Bound for BoundWrite<T> {
    fn eval(&self, slot: &mut Option<Box<dyn Any>>) {
        let v = (self.f)();
        match slot.as_mut().and_then(|b| b.downcast_mut::<T>()) {
            Some(held) => *held = v,
            None => *slot = Some(Box::new(v)),
        }
    }

    fn write(&self, slot: &dyn Any, style: &mut Style) {
        if let Some(v) = slot.downcast_ref::<T>() {
            (self.write)(style, v.clone());
        }
    }

    fn apply(&self, style: &mut Style) {
        (self.write)(style, (self.f)());
    }
}

impl Op {
    /// Its closure, for a bound setter.
    pub(crate) fn bound(&self) -> Option<&dyn Bound> {
        match &self.run {
            Run::Bound(b) => Some(&**b),
            Run::Write(_) => None,
        }
    }

    /// Run it over `style`: a bound one runs its closure.
    pub(crate) fn apply(&self, style: &mut Style) {
        match &self.run {
            Run::Write(w) => w(style),
            Run::Bound(b) => b.apply(style),
        }
    }
}

/// The bound properties of a node: the writes its style resolve replays
/// over the node's fixed style, in the order they were set. Empty, it
/// holds nothing on the heap.
#[derive(Clone, Default)]
pub struct Bindings(Option<Rc<Vec<Op>>>);

impl Bindings {
    /// The writes to replay, shared, for the resolve's memo to know them by
    /// pointer; `None` when nothing is bound.
    pub(crate) fn ops(&self) -> Option<&Rc<Vec<Op>>> {
        self.0.as_ref()
    }

    /// Whether any property is bound.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// Replay the bound properties over `style`, as the node's resolve
    /// does over its fixed style: each closure runs, tracked by whatever is
    /// running, and writes its property.
    pub fn apply_to(&self, style: &mut Style) {
        if let Some(ops) = &self.0 {
            apply(ops, style);
        }
    }

    fn write(&mut self, fields: Fields, kind: Kind, run: Run) {
        let ops = Rc::make_mut(self.0.get_or_insert_default());
        // A later write of every field an earlier one wrote owns them: the
        // earlier one goes, a restyle's (which writes what it writes)
        // stays before it.
        ops.retain(|o| o.kind == Kind::Restyle || !fields.covers(o.fields));
        // Only fixed writes left: the fixed style holds them all.
        if kind == Kind::Fixed && ops.iter().all(|o| o.kind == Kind::Fixed) {
            self.0 = None;
            return;
        }
        ops.push(Op { fields, kind, run });
    }

    /// `fields` were written with a fixed value: it drops a binding of
    /// no more than them, and, after another binding, replays in its place.
    fn fixed<T: Clone + 'static>(&mut self, fields: Fields, value: T, write: fn(&mut Style, T)) {
        // No earlier write touches these fields: the fixed style's value
        // stands under the replay as it is, and nothing need be kept for it.
        if let Some(ops) = &self.0
            && ops.iter().all(|o| o.kind != Kind::Restyle && !o.fields.meets(fields))
        {
            return;
        }
        self.write(
            fields,
            Kind::Fixed,
            Run::Write(Rc::new(move |s| write(s, value.clone()))),
        );
    }

    fn bound<T: Clone + 'static>(&mut self, fields: Fields, f: Rc<dyn Fn() -> T>, write: fn(&mut Style, T)) {
        self.write(fields, Kind::Bound, Run::Bound(Rc::new(BoundWrite { f, write })));
    }

    /// Replay `f` after every write the node made, as a parent's restyle
    /// of it: what `f` writes is the parent's, over a binding of the same
    /// property. `false`, and nothing kept, when nothing is bound: the
    /// caller then applies `f` to the fixed style itself.
    pub(crate) fn restyle(&mut self, f: Rc<dyn Fn(Style) -> Style>) -> bool {
        let Some(ops) = &mut self.0 else {
            return false;
        };
        Rc::make_mut(ops).push(Op {
            fields: Fields::of(&[]),
            kind: Kind::Restyle,
            run: Run::Write(Rc::new(move |s| *s = f(std::mem::take(s)))),
        });
        true
    }
}

/// Replay `ops` over `style`.
pub(crate) fn apply(ops: &[Op], style: &mut Style) {
    for op in ops {
        op.apply(style);
    }
}

/// Write `fields` of `style` with `value`. A closure is kept in
/// `bindings` to run where the node resolves; without `bindings` (a
/// bare [`Style`], a span) it runs now, tracked by whatever is
/// running.
pub(crate) fn set<T: Clone + 'static, M>(
    style: &mut Style,
    bindings: Option<&mut Bindings>,
    fields: Fields,
    value: impl IntoValue<T, M>,
    write: fn(&mut Style, T),
) {
    match value.into_value() {
        Value::Fixed(v) => {
            // Without a binding the fixed style holds it alone: no clone
            // (a spilled list would allocate one for nothing).
            if let Some(b) = bindings
                && !b.is_empty()
            {
                b.fixed(fields, v.clone(), write);
            }
            write(style, v);
        }
        Value::Bound(f) => match bindings {
            Some(b) => b.bound(fields, f, write),
            None => write(style, f()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Bindings, Field, Fields, set};
    use crate::style::Style;

    /// Every field, checked to be every one: a field added to [`Field`]
    /// and not here does not compile.
    macro_rules! every_field {
        ($($f:ident),* $(,)?) => {
            const EVERY: &[Field] = &[$(Field::$f),*];
            #[expect(dead_code, reason = "it holds the list to every variant")]
            const fn listed(f: Field) {
                match f {
                    $(Field::$f)|* => {}
                }
            }
        };
    }

    every_field!(
        Display,
        Inline,
        Invisible,
        Position,
        InsetTop,
        InsetRight,
        InsetBottom,
        InsetLeft,
        FlexDirection,
        FlexWrap,
        FlexGrow,
        FlexShrink,
        FlexBasis,
        AlignItems,
        AlignSelf,
        JustifyItems,
        JustifySelf,
        JustifyContent,
        AlignContent,
        RowGap,
        ColumnGap,
        GridColumns,
        GridRows,
        GridAutoRows,
        GridAutoFlow,
        GridColumnStart,
        GridColumnSpan,
        GridRowStart,
        GridRowSpan,
        PaddingTop,
        PaddingRight,
        PaddingBottom,
        PaddingLeft,
        MarginTop,
        MarginRight,
        MarginBottom,
        MarginLeft,
        MarginStart,
        MarginEnd,
        Width,
        Height,
        MinWidth,
        MinHeight,
        MaxWidth,
        MaxHeight,
        AspectRatio,
        IconSize,
        OverflowX,
        OverflowY,
        ScrollbarHidden,
        Background,
        BackgroundImage,
        BorderTop,
        BorderRight,
        BorderBottom,
        BorderLeft,
        BorderColor,
        RadiusTopLeft,
        RadiusTopRight,
        RadiusBottomRight,
        RadiusBottomLeft,
        Shadows,
        Ring,
        Opacity,
        Rotate,
        Scale,
        TranslateX,
        TranslateY,
        Origin,
        Mask,
        Filters,
        Cursor,
        Transition,
        Duration,
        Delay,
        Easing,
        Animation,
        ExitAnimation,
        TextColor,
        TextSize,
        LineHeight,
        FontWeight,
        FontFamily,
        TextAlign,
        Direction,
        Nowrap,
        Truncate,
        LineClamp,
        TextWrap,
        Italic,
        Underline,
        UnderlineOffset,
        LineThrough,
        Tracking,
        TabularNums,
        StrokeWidth,
        PlaceholderColor,
        SelectionColor,
        EdgeBar,
        InsetLayer,
        OverlayBorder,
        Backdrop,
        HitSlop,
    );

    #[test]
    fn each_field_has_a_bit_of_its_own() {
        let mut seen = 0_u128;
        for &f in EVERY {
            let bit = f.bit();
            assert_eq!(bit.count_ones(), 1, "{f:?} has one bit");
            assert_eq!(seen & bit, 0, "{f:?} shares its bit");
            seen |= bit;
        }
    }

    #[test]
    fn a_write_covers_the_fields_it_sets() {
        let padding = Fields::of(&[
            Field::PaddingTop,
            Field::PaddingRight,
            Field::PaddingBottom,
            Field::PaddingLeft,
        ]);
        let x = Fields::of(&[Field::PaddingRight, Field::PaddingLeft]);
        assert!(padding.covers(x));
        assert!(!x.covers(padding));
        assert!(x.covers(Fields::of(&[])));
        assert!(padding.meets(x));
        assert!(!x.meets(Fields::of(&[Field::PaddingTop])));
    }

    fn opacity(s: &mut Style, v: f32) {
        s.opacity = v;
    }

    fn rotate(s: &mut Style, v: f32) {
        s.transform.rotate = v;
    }

    /// The style the node resolves to: its fixed style, the writes replayed.
    fn resolved(style: &Style, b: &Bindings) -> Style {
        let mut out = style.clone();
        b.apply_to(&mut out);
        out
    }

    #[test]
    fn a_fixed_write_of_other_fields_after_a_binding_keeps_nothing() {
        let (mut style, mut b) = (Style::default(), Bindings::default());
        set(
            &mut style,
            Some(&mut b),
            Fields::of(&[Field::Opacity]),
            || 0.5_f32,
            opacity,
        );
        set(&mut style, Some(&mut b), Fields::of(&[Field::Rotate]), 90.0_f32, rotate);
        assert_eq!(b.ops().map(|o| o.len()), Some(1), "only the binding is replayed");
        let s = resolved(&style, &b);
        assert!((s.opacity - 0.5).abs() < f32::EPSILON);
        assert!((s.transform.rotate - 90.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_fixed_write_after_a_binding_of_its_field_still_wins() {
        let (mut style, mut b) = (Style::default(), Bindings::default());
        set(
            &mut style,
            Some(&mut b),
            Fields::of(&[Field::Opacity]),
            || 0.5_f32,
            opacity,
        );
        set(
            &mut style,
            Some(&mut b),
            Fields::of(&[Field::Rotate]),
            || 45.0_f32,
            rotate,
        );
        set(&mut style, Some(&mut b), Fields::of(&[Field::Rotate]), 90.0_f32, rotate);
        let s = resolved(&style, &b);
        assert!((s.opacity - 0.5).abs() < f32::EPSILON);
        assert!(
            (s.transform.rotate - 90.0).abs() < f32::EPSILON,
            "the later fixed write owns rotate"
        );
    }
}
