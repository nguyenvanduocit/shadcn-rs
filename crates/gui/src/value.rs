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
//! The common reads are sources rather than closures ([`source`]): a flag
//! of an [`crate::Interaction`] (`.background(i.hover(rest, lit))`), a
//! signal (`.opacity(open.map(|o| if *o { 1.0 } else { 0.0 }))`), a
//! [`Group`] of signals (shadcn's theme). A source's mount needs no
//! tracker: it registers with what it reads, which wakes it alone.
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
use std::sync::Arc;

use smallvec::SmallVec;

use crate::reactive::Signal;
use crate::style::Style;

pub mod source;

use source::{Closure, Read, Via};
pub use source::{Group, Grouped, Mapped, Toggle};

/// What a setter was given: a value, or a closure that gives one.
#[derive(Clone)]
pub enum Value<T> {
    Fixed(T),
    Bound(Rc<dyn Fn() -> T>),
}

/// Anything a setter of `T` takes. `M` tells the impls apart (a value of
/// `T`, a value that converts to `T`, or a closure giving either); the
/// compiler infers it, and no caller names it.
pub trait IntoValue<T, M> {
    fn into_value(self) -> Value<T>;

    /// What a style setter keeps of it: the value, or the closure with the
    /// setter's `write` in one allocation, the closure's type kept until
    /// then (where it is erased, and where each node keeps what it gave:
    /// `Bound` in this module). Without `keep` (no node keeps it: a bare
    /// [`Style`], a span) a closure runs now, tracked by whatever is
    /// running, and gives its value.
    ///
    /// The impls here keep the closure's type; this default, for any
    /// other, keeps it as [`Self::into_value`] gives it, one allocation
    /// more. One trait for every setter and every reactive prop, so a prop
    /// given as `impl IntoValue` passes on to a style setter as it is.
    #[doc(hidden)]
    #[track_caller]
    fn into_setting<W>(self, write: W, keep: bool) -> Setting<T>
    where
        Self: Sized,
        T: Clone + PartialEq + 'static,
        W: Fn(&mut Style, T) + Copy + 'static,
    {
        match self.into_value() {
            Value::Fixed(v) => Setting(Given::Fixed(v)),
            Value::Bound(f) => setting(Closure(move || f()), write, keep),
        }
    }
}

/// What a setter keeps of `read`: with `keep`, a definition of it and
/// `write`; without (no node keeps it), what it gives now, tracked by
/// whatever runs.
fn setting<R, W>(read: R, write: W, keep: bool) -> Setting<R::Out>
where
    R: Read,
    R::Out: Clone + PartialEq + 'static,
    W: Fn(&mut Style, R::Out) + Copy + 'static,
{
    if keep {
        Setting(Given::Bound(Rc::new(BoundWrite::new(read, write))))
    } else {
        Setting(Given::Fixed(read.get()))
    }
}

/// What a style setter was given, as [`IntoValue::into_setting`] keeps it.
#[doc(hidden)]
pub struct Setting<T>(Given<T>);

enum Given<T> {
    Fixed(T),
    Bound(Rc<dyn Bound>),
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
    /// A source ([`super::source`]) giving the property's own type.
    pub struct Source;
    /// A source giving a value that [`super::Convert`]s.
    pub struct SourceVia<S>(PhantomData<S>);
}

impl<T> IntoValue<T, marker::Fixed> for T {
    fn into_value(self) -> Value<T> {
        Value::Fixed(self)
    }

    #[track_caller]
    fn into_setting<W>(self, _: W, _: bool) -> Setting<T> {
        Setting(Given::Fixed(self))
    }
}

impl<T, F: Fn() -> T + 'static> IntoValue<T, marker::Bound> for F {
    fn into_value(self) -> Value<T> {
        Value::Bound(Rc::new(self))
    }

    #[track_caller]
    fn into_setting<W>(self, write: W, keep: bool) -> Setting<T>
    where
        T: Clone + PartialEq + 'static,
        W: Fn(&mut Style, T) + Copy + 'static,
    {
        setting(Closure(self), write, keep)
    }
}

impl<T, S: Convert<T>> IntoValue<T, marker::Via<S>> for S {
    fn into_value(self) -> Value<T> {
        Value::Fixed(self.convert())
    }

    #[track_caller]
    fn into_setting<W>(self, _: W, _: bool) -> Setting<T> {
        Setting(Given::Fixed(self.convert()))
    }
}

impl<T, S: Convert<T>, F: Fn() -> S + 'static> IntoValue<T, marker::BoundVia<S>> for F {
    fn into_value(self) -> Value<T> {
        Value::Bound(Rc::new(move || self().convert()))
    }

    #[track_caller]
    fn into_setting<W>(self, write: W, keep: bool) -> Setting<T>
    where
        T: Clone + PartialEq + 'static,
        W: Fn(&mut Style, T) + Copy + 'static,
    {
        setting(Closure(move || self().convert()), write, keep)
    }
}

/// The [`IntoValue`] impls of a source, of the property's own type and of
/// one that converts to it: a setter keeps it as a source; a prop that
/// takes a value or a closure ([`IntoValue::into_value`]) reads it as a
/// closure does, tracked.
macro_rules! source_values {
    ($([$($g:tt)*] $ty:ty => $gives:ty;)*) => {
        $(
            impl<$($g)*> IntoValue<$gives, marker::Source> for $ty
            where
                $ty: Read<Out = $gives>,
            {
                fn into_value(self) -> Value<$gives> {
                    Value::Bound(Rc::new(move || self.get()))
                }

                #[track_caller]
                fn into_setting<W>(self, write: W, keep: bool) -> Setting<$gives>
                where
                    $gives: Clone + PartialEq + 'static,
                    W: Fn(&mut Style, $gives) + Copy + 'static,
                {
                    setting(self, write, keep)
                }
            }

            impl<$($g)*, Target: 'static> IntoValue<Target, marker::SourceVia<$gives>> for $ty
            where
                $ty: Read<Out = $gives>,
                $gives: Convert<Target>,
            {
                fn into_value(self) -> Value<Target> {
                    let read = Via::new(self);
                    Value::Bound(Rc::new(move || read.get()))
                }

                #[track_caller]
                fn into_setting<W>(self, write: W, keep: bool) -> Setting<Target>
                where
                    Target: Clone + PartialEq + 'static,
                    W: Fn(&mut Style, Target) + Copy + 'static,
                {
                    setting(Via::new(self), write, keep)
                }
            }
        )*
    };
}

source_values! {
    [T: Clone + 'static] Toggle<T> => T;
    [S: 'static, T: 'static, F: Fn(&S) -> T + 'static] Mapped<S, F> => T;
    [G: 'static, T: 'static, F: Fn(&G) -> T + 'static] Grouped<G, F> => T;
}

/// A signal given to a setter is read as `signal.map(T::clone)`.
impl<T: Clone + 'static> IntoValue<T, marker::Source> for Signal<T> {
    fn into_value(self) -> Value<T> {
        Value::Bound(Rc::new(move || self.get()))
    }

    #[track_caller]
    fn into_setting<W>(self, write: W, keep: bool) -> Setting<T>
    where
        T: Clone + PartialEq + 'static,
        W: Fn(&mut Style, T) + Copy + 'static,
    {
        setting(self.map(T::clone), write, keep)
    }
}

/// A value known where it is given, or a source of one: what a provider
/// returns when only some of what it gives follows a signal (a corner that
/// follows the theme's radius; a fixed one registers nowhere).
#[derive(Clone, Debug)]
pub enum FixedOr<T, S> {
    Fixed(T),
    Source(S),
}

impl<T, S: IntoValue<T, marker::Source>> IntoValue<T, marker::Source> for FixedOr<T, S> {
    fn into_value(self) -> Value<T> {
        match self {
            Self::Fixed(v) => Value::Fixed(v),
            Self::Source(s) => s.into_value(),
        }
    }

    #[track_caller]
    fn into_setting<W>(self, write: W, keep: bool) -> Setting<T>
    where
        T: Clone + PartialEq + 'static,
        W: Fn(&mut Style, T) + Copy + 'static,
    {
        match self {
            Self::Fixed(v) => Setting(Given::Fixed(v)),
            Self::Source(s) => s.into_setting(write, keep),
        }
    }
}

impl<T: Convert<Target>, S: IntoValue<Target, marker::SourceVia<T>>, Target> IntoValue<Target, marker::SourceVia<T>>
    for FixedOr<T, S>
{
    fn into_value(self) -> Value<Target> {
        match self {
            Self::Fixed(v) => Value::Fixed(v.convert()),
            Self::Source(s) => s.into_value(),
        }
    }

    #[track_caller]
    fn into_setting<W>(self, write: W, keep: bool) -> Setting<Target>
    where
        Target: Clone + PartialEq + 'static,
        W: Fn(&mut Style, Target) + Copy + 'static,
    {
        match self {
            Self::Fixed(v) => Setting(Given::Fixed(v.convert())),
            Self::Source(s) => s.into_setting(write, keep),
        }
    }
}

impl<S: Clone + Convert<T> + 'static, T: 'static> IntoValue<T, marker::SourceVia<S>> for Signal<S> {
    fn into_value(self) -> Value<T> {
        Value::Bound(Rc::new(move || self.get().convert()))
    }

    #[track_caller]
    fn into_setting<W>(self, write: W, keep: bool) -> Setting<T>
    where
        T: Clone + PartialEq + 'static,
        W: Fn(&mut Style, T) + Copy + 'static,
    {
        setting(Via::new(self.map(S::clone)), write, keep)
    }
}

impl<T> Convert<Option<T>> for T {
    fn convert(self) -> Option<T> {
        Some(self)
    }
}

/// A short list kept inline, so that writing it (a binding that sets a
/// node's shadows on hover) allocates nothing; a longer one is kept on the
/// heap once and shared, so that copying it (a style resolved again, the
/// node paint records from) copies a pointer. Writing a shared list copies
/// it first. Shadows, filters and grid tracks are kept so.
#[derive(Clone)]
pub struct Inline<T, const N: usize>(Items<T, N>);

/// What an [`Inline`] holds: at most `N` items in place, or more, shared.
#[derive(Clone)]
enum Items<T, const N: usize> {
    /// Never spilled: a list longer than `N` is [`Items::Shared`].
    Inline(SmallVec<[T; N]>),
    Shared(Arc<Vec<T>>),
}

impl<T, const N: usize> Inline<T, N> {
    /// The empty list.
    #[must_use]
    pub const fn new() -> Self {
        Self(Items::Inline(SmallVec::new_const()))
    }

    /// `n` copies of `item` (`repeat(3, Track::Fr(1.0))` is CSS's `repeat(3, 1fr)`).
    #[must_use]
    pub fn repeat(n: usize, item: T) -> Self
    where
        T: Clone,
    {
        Self::of(SmallVec::from_elem(item, n))
    }

    /// `items`, shared when they do not fit in place.
    fn of(items: SmallVec<[T; N]>) -> Self {
        if items.spilled() {
            Self(Items::Shared(Arc::new(items.into_vec())))
        } else {
            Self(Items::Inline(items))
        }
    }

    /// Whether it holds anything on the heap: it never does at `N` items or fewer.
    #[must_use]
    pub fn spilled(&self) -> bool {
        matches!(self.0, Items::Shared(_))
    }

    pub fn push(&mut self, item: T)
    where
        T: Clone,
    {
        match &mut self.0 {
            Items::Inline(v) if v.len() < N => v.push(item),
            Items::Inline(v) => {
                let mut heap = Vec::with_capacity(v.len().saturating_add(1).max(N.saturating_mul(2)));
                heap.extend(v.drain(..));
                heap.push(item);
                self.0 = Items::Shared(Arc::new(heap));
            }
            Items::Shared(v) => Arc::make_mut(v).push(item),
        }
    }

    pub fn clear(&mut self) {
        self.0 = Items::Inline(SmallVec::new_const());
    }
}

impl<T: PartialEq, const N: usize> PartialEq for Inline<T, N> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Items::Shared(a), Items::Shared(b)) if Arc::ptr_eq(a, b) => true,
            _ => **self == **other,
        }
    }
}

impl<T, const N: usize> Default for Inline<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: std::fmt::Debug, const N: usize> std::fmt::Debug for Inline<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T, const N: usize> std::ops::Deref for Inline<T, N> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        match &self.0 {
            Items::Inline(v) => v,
            Items::Shared(v) => v,
        }
    }
}

/// Writing a shared list in place copies it first, if it is shared.
impl<T: Clone, const N: usize> std::ops::DerefMut for Inline<T, N> {
    fn deref_mut(&mut self) -> &mut [T] {
        match &mut self.0 {
            Items::Inline(v) => v,
            Items::Shared(v) => Arc::make_mut(v).as_mut_slice(),
        }
    }
}

impl<T, const N: usize> FromIterator<T> for Inline<T, N> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::of(iter.into_iter().collect())
    }
}

impl<'a, T, const N: usize> IntoIterator for &'a Inline<T, N> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
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
        std::iter::once(self).collect()
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
    /// A closure: each node it is mounted on runs it on its own
    /// ([`Mounted::eval`]) and writes what it gave there where the writes
    /// replay ([`Mounted::write`]).
    Bound(Rc<dyn Bound>),
}

/// A bound setter, as the element keeps it: what it reads (a closure, or
/// a source: [`source`]) and the write of what that gives, one allocation
/// behind one pointer, shared by every clone of the element and every
/// node it is mounted on. It holds no state: what one mount last gave is
/// that mount's ([`Mounted`]), so a node never sees another's value and a
/// cold build (the cold check, a fork) runs it without touching a live
/// node's.
///
/// Where the types go: a setter keeps the type of what it was given and
/// the setter's write `W` up to [`IntoValue::into_setting`], which makes the
/// [`BoundWrite`] and erases it here, once per definition. A mount
/// ([`Bound::mount`]) makes a [`Mount`] typed by the same read and write,
/// erased once more behind [`Mounted`]: reading, comparing what it gave
/// with what that mount last gave, and writing it are static calls inside
/// one dynamic one, with no downcast.
pub(crate) trait Bound: Any {
    /// A mount of it on one node: the typed cell of what it last gave
    /// there, empty until its first run.
    fn mount(self: Rc<Self>) -> Box<dyn Mounted>;
    /// Read and write what it gives, keeping nothing, tracked by whatever
    /// runs (a bare [`Style`], a span, a write replayed without its mount).
    fn apply(&self, style: &mut Style);
}

/// One mount of a [`Bound`] on one node: the definition and what it last
/// gave there, typed, and for a source its registration. Owned by that
/// node's binding state (`layout::binding::BoundRun`), which holds the
/// tracker a closure runs under; a source needs none.
pub(crate) trait Mounted {
    /// Whether it is a mount of `def`.
    fn is(&self, def: &Rc<dyn Bound>) -> bool;
    /// Mount `def` here in place of its own definition, when it is of the
    /// same read and write (the same setter given the same closure again,
    /// by an element built anew): what it last gave stays, to be compared
    /// with what `def` gives, and a source's registration stays while it
    /// reads the same source. False when `def` is of another type.
    fn rebind(&mut self, def: &Rc<dyn Bound>) -> bool;
    /// Whether it runs under a tracker of its own (a closure, or the one
    /// mount of a signal), which says when it must run again. Another
    /// source registers with what it reads.
    fn tracked(&self) -> bool;
    /// It is about to run: a source registers with what it reads, to wake
    /// `mark` (its node's), and its wake is taken. Before [`Self::tracked`]
    /// is asked: registering decides whether a signal's mount reads it
    /// alone, tracked.
    fn prepare(&mut self, mark: &crate::reactive::Mark);
    /// A source's mount: whether it must run (it never ran, or what it
    /// reads woke it since). Never for a closure: its tracker says so.
    fn due(&self) -> bool;
    /// Read, keeping what it gave; whether that differs from what it last
    /// gave (true on the first run), once [`Self::prepare`]d. A closure
    /// runs tracked by whatever is running (its tracker), as does the one
    /// mount of a signal; another source reads untracked. Allocates
    /// nothing past what the closure does, and in steady state nothing for
    /// a source.
    ///
    /// The cutoff is `PartialEq`, as a signal's is: a value equal to the
    /// last is not written, and the node's style as last replayed stands.
    /// So a type a setter binds must hold values equal under `PartialEq`
    /// interchangeable for all a restyle or a replay may observe (an
    /// `f32` that is `-0.0` where `+0.0` stood keeps `+0.0`); a type that
    /// breaks this must not be bound.
    fn eval(&mut self) -> bool;
    /// Write what it last gave; false, writing nothing, before its first run.
    fn write(&self, style: &mut Style) -> bool;
    /// What it last gave, as a mount of its own that registers with
    /// nothing: a held write a cold build reuses in its place (ADR-0015,
    /// rule 12, a hold per setter), never running its closure.
    fn snapshot(&self) -> Box<dyn Mounted>;
    /// A source's registration, taken out: its node is hidden, and its
    /// parent waits in its place.
    fn take_wait(&mut self) -> Option<source::Wait>;
}

/// What a bound setter reads, `R`, and the setter's write `W` of what it
/// gives.
struct BoundWrite<R, W> {
    read: R,
    write: W,
}

impl<R: Read, W> BoundWrite<R, W> {
    const fn new(read: R, write: W) -> Self {
        Self { read, write }
    }
}

impl<R, W> Bound for BoundWrite<R, W>
where
    R: Read,
    R::Out: Clone + PartialEq + 'static,
    W: Fn(&mut Style, R::Out) + Copy + 'static,
{
    fn mount(self: Rc<Self>) -> Box<dyn Mounted> {
        Box::new(Mount {
            def: self,
            last: None,
            wait: None,
        })
    }

    fn apply(&self, style: &mut Style) {
        (self.write)(style, self.read.get());
    }
}

/// [`Mounted`] for a [`BoundWrite`]: the typed comparison cell, and a
/// source's registration.
struct Mount<R: Read, W> {
    def: Rc<BoundWrite<R, W>>,
    last: Option<R::Out>,
    wait: Option<source::Wait>,
}

impl<R, W> Mounted for Mount<R, W>
where
    R: Read,
    R::Out: Clone + PartialEq + 'static,
    W: Fn(&mut Style, R::Out) + Copy + 'static,
{
    fn is(&self, def: &Rc<dyn Bound>) -> bool {
        std::ptr::addr_eq(Rc::as_ptr(&self.def), Rc::as_ptr(def))
    }

    fn rebind(&mut self, def: &Rc<dyn Bound>) -> bool {
        let any: Rc<dyn Any> = Rc::<dyn Bound>::clone(def);
        match any.downcast::<BoundWrite<R, W>>() {
            Ok(def) => {
                self.def = def;
                true
            }
            Err(_) => false,
        }
    }

    fn tracked(&self) -> bool {
        R::TRACKED || self.wait.as_ref().is_some_and(|w| w.is_solo())
    }

    fn prepare(&mut self, mark: &crate::reactive::Mark) {
        source::watch(&self.def.read, mark, &mut self.wait);
    }

    fn due(&self) -> bool {
        source::due(self.tracked(), self.wait.as_ref())
    }

    fn eval(&mut self) -> bool {
        let solo = self.wait.as_ref().is_some_and(|w| w.is_solo());
        let v = self.def.read.peek(solo);
        if self.last.as_ref() == Some(&v) {
            return false;
        }
        self.last = Some(v);
        true
    }

    fn take_wait(&mut self) -> Option<source::Wait> {
        self.wait.take()
    }

    fn write(&self, style: &mut Style) -> bool {
        match &self.last {
            Some(v) => {
                (self.def.write)(style, v.clone());
                true
            }
            None => false,
        }
    }

    fn snapshot(&self) -> Box<dyn Mounted> {
        Box::new(Self {
            def: Rc::clone(&self.def),
            last: self.last.clone(),
            wait: None,
        })
    }
}

impl Op {
    /// Its definition, for a bound setter.
    pub(crate) fn bound(&self) -> Option<&Rc<dyn Bound>> {
        match &self.run {
            Run::Bound(b) => Some(b),
            Run::Write(_) => None,
        }
    }

    /// Whether it is a parent's restyle: the one write that may read
    /// signals as it replays (a fixed write writes a value; a bound one,
    /// what its mount last gave).
    pub(crate) fn restyles(&self) -> bool {
        self.kind == Kind::Restyle
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

    /// `fields` were written with a fixed value. It owns what an earlier
    /// write of no more than them wrote, which goes ([`owned`]); what is
    /// left decides the rest, so the check runs after the drop: nothing
    /// bound left, nothing replays (the fixed style holds every write); a
    /// write left that touches these fields, or a restyle, and it replays
    /// after them; else the fixed style holds it as it stands under the
    /// replay. A list shared with a clone is copied only when it changes.
    fn fixed<T, W>(&mut self, fields: Fields, value: &T, write: W)
    where
        T: Clone + 'static,
        W: Fn(&mut Style, T) + 'static,
    {
        let Some(ops) = &mut self.0 else {
            return;
        };
        let left = || ops.iter().filter(|o| !owned(fields, o));
        if left().all(|o| o.kind == Kind::Fixed) {
            self.0 = None;
            return;
        }
        let replays = left().any(|o| o.kind == Kind::Restyle || o.fields.meets(fields));
        if !replays && !ops.iter().any(|o| owned(fields, o)) {
            return;
        }
        let ops = Rc::make_mut(ops);
        ops.retain(|o| !owned(fields, o));
        if replays {
            let value = value.clone();
            ops.push(Op {
                fields,
                kind: Kind::Fixed,
                run: Run::Write(Rc::new(move |s| write(s, value.clone()))),
            });
        }
    }

    /// `fields` were bound to `def`: it owns what an earlier write of no
    /// more than them wrote, and replays after what is left.
    fn bound(&mut self, fields: Fields, def: Rc<dyn Bound>) {
        let ops = Rc::make_mut(self.0.get_or_insert_default());
        ops.retain(|o| !owned(fields, o));
        ops.push(Op {
            fields,
            kind: Kind::Bound,
            run: Run::Bound(def),
        });
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

/// Whether a later write of every one of `fields` owns `op`: it wrote no
/// more than them. A restyle (which writes what it writes) it never owns;
/// that stays before it.
fn owned(fields: Fields, op: &Op) -> bool {
    op.kind != Kind::Restyle && fields.covers(op.fields)
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
#[track_caller]
pub(crate) fn set<T, M, W>(
    style: &mut Style,
    bindings: Option<&mut Bindings>,
    fields: Fields,
    value: impl IntoValue<T, M>,
    write: W,
) where
    T: Clone + PartialEq + 'static,
    W: Fn(&mut Style, T) + Copy + 'static,
{
    match value.into_setting(write, bindings.is_some()).0 {
        Given::Fixed(v) => {
            // Cloned only when a binding keeps a replay of it too (a
            // spilled list would allocate one for nothing).
            if let Some(b) = bindings {
                b.fixed(fields, &v, write);
            }
            write(style, v);
        }
        Given::Bound(def) => match bindings {
            Some(b) => b.bound(fields, def),
            // Not made: `into_setting` without `keep` gives a value.
            None => def.apply(style),
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
        assert_eq!(
            b.ops().map(|o| o.len()),
            Some(1),
            "the binding it covered went, and with it what the fixed write replayed over"
        );
    }

    /// Whether a fixed write replays is decided by what is left once the
    /// bindings it covers are dropped: here the one it overlapped goes, so
    /// nothing is left that it touches and nothing of it is kept. A clone
    /// that shared the writes keeps them.
    #[test]
    fn a_fixed_write_checks_overlap_after_dropping_what_it_covers() {
        let (mut style, mut b) = (Style::default(), Bindings::default());
        let opacity_fields = Fields::of(&[Field::Opacity]);
        set(&mut style, Some(&mut b), opacity_fields, || 0.5_f32, opacity);
        set(
            &mut style,
            Some(&mut b),
            Fields::of(&[Field::Rotate]),
            || 45.0_f32,
            rotate,
        );
        let shared = b.clone();
        set(&mut style, Some(&mut b), opacity_fields, 0.9_f32, opacity);
        assert_eq!(b.ops().map(|o| o.len()), Some(1), "the rotate binding alone");
        assert_eq!(shared.ops().map(|o| o.len()), Some(2), "the clone keeps its writes");
        let s = resolved(&style, &b);
        assert!((s.opacity - 0.9).abs() < f32::EPSILON);
        assert!((s.transform.rotate - 45.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_fixed_write_after_a_restyle_replays_after_it() {
        let (mut style, mut b) = (Style::default(), Bindings::default());
        set(
            &mut style,
            Some(&mut b),
            Fields::of(&[Field::Opacity]),
            || 0.5_f32,
            opacity,
        );
        assert!(b.restyle(std::rc::Rc::new(|mut s: Style| {
            s.opacity = 0.3;
            s
        })));
        set(
            &mut style,
            Some(&mut b),
            Fields::of(&[Field::Opacity]),
            0.9_f32,
            opacity,
        );
        let s = resolved(&style, &b);
        assert!(
            (s.opacity - 0.9).abs() < f32::EPSILON,
            "written after the parent's restyle, the fixed value is the last word: {}",
            s.opacity
        );
        assert_eq!(b.ops().map(|o| o.len()), Some(2), "the restyle and the fixed write");
    }

    #[test]
    fn a_write_of_some_of_a_bindings_fields_replays_over_it() {
        use crate::styled::Styled;
        let left = std::rc::Rc::new(std::cell::Cell::new(4.0_f32));
        let l = std::rc::Rc::clone(&left);
        // Padding bound, then its sides fixed: the binding stays for the
        // top and bottom, the fixed sides replay over it.
        let mut d = crate::div().padding(move || l.get()).padding_x(10.0);
        let (style, b) = d.style_and_bindings();
        let b = b.expect("a node keeps bindings");
        assert_eq!(b.ops().map(|o| o.len()), Some(2));
        let s = resolved(style, b);
        assert!((s.padding.top - 4.0).abs() < f32::EPSILON);
        assert!((s.padding.left - 10.0).abs() < f32::EPSILON);
        assert!((s.padding.right - 10.0).abs() < f32::EPSILON);
        left.set(6.0);
        let s = resolved(style, b);
        assert!((s.padding.bottom - 6.0).abs() < f32::EPSILON, "the binding runs again");
        assert!(
            (s.padding.left - 10.0).abs() < f32::EPSILON,
            "and the fixed sides stand"
        );
    }

    #[test]
    fn shape_setters_own_the_corners_they_write() {
        use crate::shape::Shape;
        use crate::style::Corners;
        use crate::styled::Styled;
        // Two corners bound, then all four fixed: the fixed radius owns them.
        let mut d = crate::div().radius_top(move || 4.0).radius(8.0);
        let (style, b) = d.style_and_bindings();
        let b = b.expect("a node keeps bindings");
        assert!(b.is_empty(), "the binding it covered went: nothing replays");
        assert_eq!(resolved(style, b).shape, Shape::Rounded(Corners::all(8.0)));
        // The shape bound, then two corners fixed: both replay, in order.
        let mut d = crate::div()
            .shape(move || Shape::Rounded(Corners::all(2.0)))
            .radius_top(6.0);
        let (style, b) = d.style_and_bindings();
        let b = b.expect("a node keeps bindings");
        assert_eq!(b.ops().map(|o| o.len()), Some(2));
        let r = resolved(style, b).shape.radii();
        assert!((r.top_left - 6.0).abs() < f32::EPSILON && (r.top_right - 6.0).abs() < f32::EPSILON);
        assert!((r.bottom_left - 2.0).abs() < f32::EPSILON && (r.bottom_right - 2.0).abs() < f32::EPSILON);
    }
}
