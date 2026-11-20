//! From an element tree to a [`Frame`]: every node's box in window pixels,
//! what it paints, and what it does when acted on.
//!
//! The tree is laid out by Taffy (block, flex and grid), text measured by
//! [`TextSystem`]. Interactive state from the last frame (hover, press,
//! focus, scroll offsets) is applied first, as style refinements and offsets.
//! [`Deferred`] subtrees are laid out afterwards, each on a layer of its own,
//! placed against their anchor and kept inside the window.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

use taffy::{AvailableSpace, NodeId, TaffyTree};

use crate::color::Color;
use crate::edit::{TextEdit, TextField};
use crate::element::{Common, Deferred, Element, ElementKey, Handler, Handlers, InputMode, Placement};
use crate::events::ClickEvent;
use crate::focus::{FocusHandle, FocusId};
use crate::geometry::{Bounds, NodePath, PathSeg};
use crate::icon::Icon;
use crate::motion::Motion;
use crate::placement::{EDGE_MARGIN, fitted_height, over, placed, position};
use crate::scroll::{ScrollChild, ScrollHandle, ScrollLayout};
use crate::style::{
    Corners, Cursor, Direction, Display, Edges, Effects, Length, Opened, Overflow, Painted, Position, ResolvedText,
    Ring, Shadow, Style, TextAlign,
};
use crate::text::{Paragraph, Run, TextSystem};

/// The layout engine failed.
#[derive(Debug)]
pub struct LayoutError(taffy::TaffyError);

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "layout error: {}", self.0)
    }
}

impl std::error::Error for LayoutError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl From<taffy::TaffyError> for LayoutError {
    fn from(e: taffy::TaffyError) -> Self {
        Self(e)
    }
}

/// How a node's box is painted.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub background: Option<Color>,
    pub border: Edges<f32>,
    pub border_color: Option<Color>,
    pub radius: Corners,
    pub shadows: Vec<Shadow>,
    pub ring: Option<Ring>,
    /// Its own opacity (CSS `opacity`): below 1 it and its children paint
    /// aside as one group, composited at it, so an ancestor's is not theirs.
    pub opacity: f32,
    pub invisible: bool,
    pub effects: Effects,
    /// The scale and turn its painting takes, with its children's.
    pub transform: Painted,
    /// The inherited text properties the effects resolve against
    /// (`currentColor`, the `ch` unit).
    pub text: ResolvedText,
}

/// What a node draws inside its box.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    None,
    Text {
        content: String,
        text: ResolvedText,
        /// Parts of `content` styled on their own; empty when it has one style.
        runs: Vec<Run>,
    },
    Icon {
        icon: Icon,
        color: Color,
        stroke_width: f32,
    },
    Shapes(Vec<crate::shape::Shape>),
    Image {
        source: crate::image::ImageSource,
        radius: Corners,
    },
    Input {
        /// What the input shows: its value with any composition in it.
        edit: TextEdit,
        /// The byte range of `edit.text` an input method is composing.
        preedit: Option<(usize, usize)>,
        placeholder: String,
        placeholder_color: Color,
        selection_color: Color,
        text: ResolvedText,
        focused: bool,
        multiline: bool,
        mode: InputMode,
    },
}

/// Scroll state of a scroll container.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scroll {
    pub offset: (f32, f32),
    /// The largest offset on each axis.
    pub max: (f32, f32),
    /// Colour of the scrollbar thumb: the container's text colour, faded.
    pub thumb: Color,
}

/// One laid-out node.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameNode {
    pub path: NodePath,
    pub name: Option<&'static str>,
    /// The border box in window pixels.
    pub bounds: Bounds,
    /// The box inside padding and border, where text is laid out.
    pub content_box: Bounds,
    /// What this node may paint into, when an ancestor clips.
    pub clip: Option<Bounds>,
    /// The corner radii of that clip (the clipping ancestor's inner corners).
    pub clip_radius: Corners,
    /// 0 for the main tree; each deferred subtree is one higher than the last.
    pub layer: usize,
    /// The parent within the same layer.
    pub parent: Option<usize>,
    pub look: Look,
    pub content: Content,
    pub scroll: Option<Scroll>,
}

/// What a node does, kept beside [`FrameNode`] because closures cannot be compared.
#[derive(Clone, Default)]
pub struct Behaviour {
    pub(crate) handlers: Handlers,
    pub(crate) focus: Option<FocusHandle>,
    pub(crate) disabled: bool,
    pub(crate) pass_through: bool,
    pub(crate) cursor: Option<Cursor>,
    pub(crate) field: Option<TextField>,
    /// The spans of a paragraph that act on the pointer (links).
    pub(crate) spans: Vec<SpanTarget>,
    /// Holds this scroll container's offset.
    pub(crate) scroll: Option<ScrollHandle>,
}

/// A span of a text node that takes clicks or sets the cursor.
#[derive(Clone)]
pub(crate) struct SpanTarget {
    pub(crate) range: std::ops::Range<usize>,
    pub(crate) click: Option<Handler<ClickEvent>>,
    pub(crate) cursor: Option<Cursor>,
    /// Where its text lies in the window, one box per line it takes.
    pub(crate) boxes: Vec<Bounds>,
}

impl Behaviour {
    /// The span at window point `(x, y)`.
    pub(crate) fn span_at(&self, x: f32, y: f32) -> Option<&SpanTarget> {
        self.spans.iter().find(|s| s.boxes.iter().any(|b| b.contains(x, y)))
    }
}

/// A layer: its root node and the box it was placed against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub root: usize,
    pub anchor: Option<Bounds>,
}

/// A whole laid-out frame, in paint order: the main tree depth-first, then
/// each layer.
#[derive(Clone, Default)]
pub struct Frame {
    pub nodes: Vec<FrameNode>,
    pub(crate) behaviour: Vec<Behaviour>,
    pub layers: Vec<Layer>,
    pub viewport: Bounds,
}

impl Frame {
    pub(crate) fn behaviour(&self, i: usize) -> Option<&Behaviour> {
        self.behaviour.get(i)
    }

    /// The boxes of the nodes that act on the pointer (a click, press, drag
    /// or hover handler, a cursor of their own, focus or a text field), in
    /// paint order: where a hover can change what the screen shows.
    #[must_use]
    pub fn pointer_targets(&self) -> Vec<Bounds> {
        self.nodes
            .iter()
            .zip(&self.behaviour)
            .filter(|(_, b)| {
                let h = &b.handlers;
                !b.disabled
                    && (h.click.is_some()
                        || h.mouse_down.is_some()
                        || h.drag.is_some()
                        || h.hover.is_some()
                        || b.cursor.is_some()
                        || b.focus.is_some()
                        || b.field.is_some())
            })
            .map(|(n, _)| n.bounds)
            .collect()
    }

    /// Where the first pointer-taking span showing exactly `text` (a link)
    /// lies: the box of its first line.
    #[must_use]
    pub fn span_bounds(&self, text: &str) -> Option<Bounds> {
        self.nodes.iter().zip(&self.behaviour).find_map(|(node, b)| {
            let Content::Text { content, .. } = &node.content else {
                return None;
            };
            b.spans
                .iter()
                .find(|s| content.get(s.range.clone()) == Some(text))
                .and_then(|s| s.boxes.first().copied())
        })
    }

    /// The node and its ancestors within its layer, innermost first.
    pub fn ancestors(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(Some(i), |&n| self.nodes.get(n).and_then(|node| node.parent))
    }
}

/// The interactive state a frame is built with.
#[derive(Clone, Debug, Default)]
pub struct UiState {
    pub hovered: HashSet<NodePath>,
    pub active: HashSet<NodePath>,
    pub scroll: HashMap<NodePath, (f32, f32)>,
    pub focused: Option<FocusId>,
    pub focus_visible: bool,
}

enum Measure {
    Text {
        content: String,
        text: ResolvedText,
        runs: Vec<Run>,
    },
    Image(ImageMeasure),
    Input {
        content: String,
        text: ResolvedText,
        multiline: bool,
    },
}

/// What an image's box is sized from: its natural size, and which of its
/// sides its style sets.
#[derive(Clone, Copy)]
struct ImageMeasure {
    natural: (f32, f32),
    /// The style sets its width, height: a definite space on that axis is
    /// that side's size.
    sized: (bool, bool),
    /// Its width or max-width is a percentage: it shrinks to any width, so
    /// adds nothing to its parent's min-content width (css-sizing-3 §5.2.2).
    compressible: bool,
}

impl ImageMeasure {
    /// The content size: a side its style or the layout sets, the other kept
    /// to the natural ratio, as CSS sizes a replaced element.
    fn size(&self, known: taffy::Size<Option<f32>>, available: taffy::Size<AvailableSpace>) -> (f32, f32) {
        let set = |known: Option<f32>, sized: bool, space: AvailableSpace| {
            known.or(match space {
                AvailableSpace::Definite(v) if sized => Some(v),
                _ => None,
            })
        };
        let (nw, nh) = (self.natural.0.max(1.0), self.natural.1.max(1.0));
        match (
            set(known.width, self.sized.0, available.width),
            set(known.height, self.sized.1, available.height),
        ) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * nh / nw),
            (None, Some(h)) => (h * nw / nh, h),
            (None, None) if self.compressible && available.width == AvailableSpace::MinContent => (0.0, 0.0),
            (None, None) => self.natural,
        }
    }
}

enum Pending {
    None,
    Text { content: String, runs: Vec<Run> },
    Icon { icon: Icon, stroke_width: f32 },
    Input(Box<PendingInput>),
    Shapes(Vec<crate::shape::Shape>),
    Image(crate::image::ImageSource),
}

struct PendingInput {
    field: TextField,
    placeholder: String,
    placeholder_color: Option<Color>,
    selection_color: Option<Color>,
    multiline: bool,
    mode: InputMode,
}

struct Built {
    taffy: NodeId,
    path: NodePath,
    name: Option<&'static str>,
    style: Style,
    text: ResolvedText,
    opacity: f32,
    invisible: bool,
    content: Pending,
    behaviour: Behaviour,
    parent: Option<usize>,
    layer: usize,
    focused: bool,
}

struct LayerTodo<'e> {
    deferred: &'e Deferred,
    path: NodePath,
    anchor: usize,
}

struct Builder<'e, 's> {
    tree: TaffyTree<Measure>,
    state: &'s UiState,
    motion: &'s mut Motion,
    built: Vec<Built>,
    todo: VecDeque<LayerTodo<'e>>,
    /// How the layer being placed was placed against its anchor.
    placed: Opened,
}

/// The focus handle of a node: its own, or one from its path when it is
/// focusable by place.
fn focus_of(common: &Common, path: &NodePath) -> Option<FocusHandle> {
    common
        .focus
        .or_else(|| common.focusable.then(|| FocusHandle::for_path(path)))
}

/// The width a text input's content is measured in: a textarea wraps to
/// nothing at its min-content width.
fn width_constraint(known: Option<f32>, available: AvailableSpace) -> Option<f32> {
    known.or(match available {
        AvailableSpace::Definite(w) => Some(w),
        AvailableSpace::MinContent => Some(0.0),
        AvailableSpace::MaxContent => None,
    })
}

/// The width text is laid out in, given its box's content width: none for
/// one unbroken left-aligned line, else the box (to wrap, cut or align in).
fn wrap_width(text: &ResolvedText, content_width: f32) -> Option<f32> {
    if text.nowrap && !text.truncate && text.physical_align() == TextAlign::Left {
        None
    } else {
        Some(content_width)
    }
}

impl<'e> Builder<'e, '_> {
    fn resolve(&self, el: &Element, common: &Common, path: &NodePath) -> (Style, bool) {
        let mut style = common.style.clone();
        let is_input = matches!(el, Element::Input(_));
        if let Element::Div(d) = el
            && d.kind.iter().any(|c| matches!(c, Element::Icon(_)))
            && let Some(f) = &common.states.has_icon
        {
            style = f(style);
        }
        let enabled = !common.disabled;
        let focused = focus_of(common, path).is_some_and(|h| Some(h.id()) == self.state.focused);
        if enabled
            && self.state.hovered.contains(path)
            && let Some(f) = &common.states.hover
        {
            style = f(style);
        }
        if enabled
            && self.state.active.contains(path)
            && let Some(f) = &common.states.active
        {
            style = f(style);
        }
        if focused && let Some(f) = &common.states.focus {
            style = f(style);
        }
        if focused
            && (self.state.focus_visible || is_input)
            && let Some(f) = &common.states.focus_visible
        {
            style = f(style);
        }
        if !enabled && let Some(f) = &common.states.disabled {
            style = f(style);
        }
        (style, focused)
    }

    /// Add `el` (at `path`) under `parent`; returns its Taffy node, or `None`
    /// for nodes that take no space.
    fn build(
        &mut self,
        el: &'e Element,
        path: NodePath,
        parent: Option<usize>,
        layer: usize,
    ) -> Result<Option<NodeId>, LayoutError> {
        let Some(common) = el.common() else {
            return Ok(None);
        };
        let (mut style, focused) = self.resolve(el, common, &path);
        if style.display == crate::style::Display::None {
            return Ok(None);
        }
        self.motion.sample(&path, &mut style);
        let inherited = parent.and_then(|p| self.built.get(p));
        let text = inherited
            .map_or_else(|| ResolvedText::root(Color::BLACK), |p| p.text)
            .inherit(&style.text);
        let opacity = style.opacity;
        let invisible = inherited.is_some_and(|p| p.invisible) || style.invisible;
        if matches!(el, Element::Input(_)) && style.cursor.is_none() {
            style.cursor = Some(Cursor::Text);
        }
        let mut taffy_style = style.to_taffy(text.direction);
        if matches!(el, Element::Icon(_)) {
            let size = taffy::Dimension::length(text.icon_size);
            if style.width == crate::style::Length::Auto {
                taffy_style.size.width = size;
            }
            if style.height == crate::style::Length::Auto {
                taffy_style.size.height = size;
            }
        }
        let in_block_flow = inherited.is_none_or(|p| p.style.display == crate::style::Display::Block);
        if style.inline && in_block_flow && style.justify_self.is_none() {
            taffy_style.justify_self = Some(taffy::AlignItems::START);
        }
        let index = self.built.len();
        let runs: Vec<Run> = match el {
            Element::Text(n) => n
                .kind
                .spans
                .iter()
                .map(|sp| Run {
                    range: sp.range.clone(),
                    text: text.inherit(&sp.style.text),
                    background: sp.style.background,
                    radius: sp.style.radius.top_left.resolve(text.radius),
                    padding: sp.style.padding,
                })
                .collect(),
            _ => Vec::new(),
        };
        let span_targets: Vec<SpanTarget> = match el {
            Element::Text(n) => n
                .kind
                .spans
                .iter()
                .filter(|sp| sp.click.is_some() || sp.style.cursor.is_some())
                .map(|sp| SpanTarget {
                    range: sp.range.clone(),
                    click: sp.click.clone(),
                    cursor: sp.style.cursor,
                    boxes: Vec::new(),
                })
                .collect(),
            _ => Vec::new(),
        };
        let content = match el {
            Element::Text(n) => Pending::Text {
                content: n.kind.content.clone(),
                runs: runs.clone(),
            },
            Element::Icon(n) => Pending::Icon {
                icon: n.kind.icon,
                stroke_width: n.kind.stroke_width,
            },
            Element::Input(n) => Pending::Input(Box::new(PendingInput {
                field: n.kind.field.clone(),
                placeholder: n.kind.placeholder.clone(),
                placeholder_color: n.kind.placeholder_color,
                selection_color: n.kind.selection_color,
                multiline: n.kind.multiline,
                mode: n.kind.mode,
            })),
            Element::Shapes(n) => Pending::Shapes(n.kind.clone()),
            Element::Image(n) => Pending::Image(n.kind.clone()),
            _ => Pending::None,
        };
        let field = match &content {
            Pending::Input(i) => Some(i.field.clone()),
            _ => None,
        };
        let node = match el {
            Element::Text(n) => self.tree.new_leaf_with_context(
                taffy_style,
                Measure::Text {
                    content: n.kind.content.clone(),
                    text,
                    runs,
                },
            )?,
            Element::Input(n) => self.tree.new_leaf_with_context(
                taffy_style,
                Measure::Input {
                    content: n.kind.mode.shown(&n.kind.field.edit.get()).0.text,
                    text,
                    multiline: n.kind.multiline,
                },
            )?,
            Element::Image(n) => {
                let (w, h) = n.kind.size();
                let percent = |l: Length| matches!(l, Length::Fraction(_));
                self.tree.new_leaf_with_context(
                    taffy_style,
                    Measure::Image(ImageMeasure {
                        natural: (crate::convert::u32_to_f32(w), crate::convert::u32_to_f32(h)),
                        sized: (style.width != Length::Auto, style.height != Length::Auto),
                        compressible: percent(style.width) || percent(style.max_width),
                    }),
                )?
            }
            _ => self.tree.new_leaf(taffy_style)?,
        };
        self.built.push(Built {
            taffy: node,
            path: path.clone(),
            name: common.name,
            behaviour: Behaviour {
                handlers: common.handlers.clone(),
                focus: focus_of(common, &path).filter(|_| !common.disabled),
                disabled: common.disabled,
                pass_through: common.pass_through,
                cursor: style.cursor,
                field,
                spans: span_targets,
                scroll: common.scroll.clone(),
            },
            style,
            text,
            opacity,
            invisible,
            content,
            parent,
            layer,
            focused,
        });

        if let Element::Div(div) = el {
            let mut kids = Vec::with_capacity(div.kind.len());
            for (i, child) in div.kind.iter().enumerate() {
                let seg = child.key().map_or(PathSeg::Index(i), PathSeg::Key);
                let child_path = path.child(seg);
                if let Element::Deferred(d) = child {
                    self.todo.push_back(LayerTodo {
                        deferred: d,
                        path: child_path,
                        anchor: index,
                    });
                } else if let Some(id) = self.build(child, child_path, Some(index), layer)? {
                    kids.push(id);
                }
            }
            self.tree.set_children(node, &kids)?;
        }
        Ok(Some(node))
    }
}

/// Lay out `root` in a `width`×`height` window.
pub fn layout(
    root: &Element,
    width: f32,
    height: f32,
    state: &UiState,
    motion: &mut Motion,
    text: &mut TextSystem,
) -> Result<Frame, LayoutError> {
    let viewport = Bounds {
        x: 0.0,
        y: 0.0,
        width,
        height,
    };
    // Boxes keep fractional sizes and places, as Blink lays out in 1/64 px;
    // painting snaps their edges to device pixels (ADR-0006).
    let mut tree = TaffyTree::new();
    tree.disable_rounding();
    let mut b = Builder {
        tree,
        state,
        motion,
        built: Vec::new(),
        todo: VecDeque::new(),
        placed: None,
    };
    let mut frame = Frame {
        viewport,
        ..Frame::default()
    };

    // The root fills the window.
    if let Some(root_id) = b.build(root, NodePath(vec![PathSeg::Index(0)]), None, 0)? {
        let mut s = b.tree.style(root_id)?.clone();
        s.size = taffy::Size {
            width: taffy::Dimension::length(width),
            height: taffy::Dimension::length(height),
        };
        b.tree.set_style(root_id, s)?;
        compute(
            &mut b.tree,
            root_id,
            AvailableSpace::Definite(width),
            AvailableSpace::Definite(height),
            text,
        )?;
        place(&b, 0, (0.0, 0.0), &mut frame)?;
        locate_spans(&mut frame, 0, text);
    }

    let mut layer = 0_usize;
    while let Some(todo) = b.todo.pop_front() {
        layer = layer.saturating_add(1);
        let anchor = frame_bounds_of(&frame, &b, todo.anchor);
        let first = b.built.len();
        // A layer inherits from the window's root, not its anchor, as content
        // portalled to `<body>` does: a menu opened from a bold button is not bold.
        let Some(id) = b.build(&todo.deferred.child, todo.path.clone(), Some(0), layer)? else {
            continue;
        };
        // Bubbling stops at a layer's root.
        if let Some(root) = b.built.get_mut(first) {
            root.parent = None;
        }
        let placement = todo.deferred.placement;
        let mut s = b.tree.style(id)?.clone();
        if placement == Placement::Fill {
            s.size = taffy::Size {
                width: taffy::Dimension::length(width),
                height: taffy::Dimension::length(height),
            };
            s.position = taffy::Position::Relative;
        } else {
            s.max_size = taffy::Size {
                width: taffy::LengthPercentageAuto::length(width - 2.0 * EDGE_MARGIN),
                height: taffy::LengthPercentageAuto::length(height - 2.0 * EDGE_MARGIN),
            };
        }
        b.tree.set_style(id, s)?;
        compute(
            &mut b.tree,
            id,
            AvailableSpace::MaxContent,
            AvailableSpace::MaxContent,
            text,
        )?;
        let size = b.tree.layout(id)?.size;
        let mut size = (size.width, size.height);
        // At least as wide as the anchor, and still as wide as its own min
        // width asks: the width laid out so far already holds that.
        if let (Placement::Anchored { match_width: true, .. }, Some(a)) = (placement, anchor)
            && a.width > size.0
        {
            size = relayout(&mut b.tree, id, text, |s| {
                s.min_size.width = taffy::LengthPercentageAuto::length(a.width);
            })?;
        }
        let origin = match (placement, anchor) {
            (Placement::Over { part, anchor_part }, Some(a)) => {
                let anchor_part = frame_bounds_by_key(&frame, &b, todo.anchor, anchor_part).unwrap_or(a);
                let part = layer_bounds_by_key(&b, first, part)?.unwrap_or_default();
                let o = over(a, anchor_part, part, size, viewport);
                relayout(&mut b.tree, id, text, |s| {
                    s.min_size.width = taffy::LengthPercentageAuto::length(o.min_width.max(size.0));
                    if let Some(h) = o.height {
                        s.max_size.height = taffy::LengthPercentageAuto::length(h);
                    }
                })?;
                // Scrolled once, as it opens: the reader scrolls it after that.
                if let Some(handle) = b
                    .built
                    .get(first..)
                    .and_then(|layer| layer.iter().find_map(|n| n.behaviour.scroll.as_ref()))
                    && handle.layout().is_none()
                {
                    handle.scroll_to((0.0, o.scroll));
                }
                o.origin
            }
            _ => {
                let size = match fitted_height(placement, anchor, size, viewport) {
                    Some(h) => relayout(&mut b.tree, id, text, |s| {
                        s.max_size.height = taffy::LengthPercentageAuto::length(h);
                    })?,
                    None => size,
                };
                position(placement, anchor, size, viewport)
            }
        };
        b.placed = placed(placement, anchor, size, viewport);
        frame.layers.push(Layer {
            root: frame.nodes.len(),
            anchor,
        });
        let placed_from = frame.nodes.len();
        place(&b, first, origin, &mut frame)?;
        locate_spans(&mut frame, placed_from, text);
    }
    Ok(frame)
}

/// Find where the pointer-taking spans of frame nodes from `from` on lie.
fn locate_spans(frame: &mut Frame, from: usize, text: &mut TextSystem) {
    for i in from..frame.nodes.len() {
        let (Some(node), Some(behaviour)) = (frame.nodes.get(i), frame.behaviour.get_mut(i)) else {
            continue;
        };
        let Content::Text { content, text: t, runs } = &node.content else {
            continue;
        };
        let area = node.content_box;
        let p = Paragraph { content, runs };
        for span in &mut behaviour.spans {
            span.boxes = text
                .selection(p, t, text_width(t, &area), span.range.start, span.range.end)
                .into_iter()
                .map(|b| Bounds {
                    x: area.x + b.x,
                    y: area.y + b.y,
                    ..b
                })
                .collect();
        }
    }
}

/// The box of built node `i`, once placed.
fn frame_bounds_of(frame: &Frame, b: &Builder<'_, '_>, i: usize) -> Option<Bounds> {
    let path = &b.built.get(i)?.path;
    frame.nodes.iter().find(|n| &n.path == path).map(|n| n.bounds)
}

fn compute(
    tree: &mut TaffyTree<Measure>,
    root: NodeId,
    w: AvailableSpace,
    h: AvailableSpace,
    text: &mut TextSystem,
) -> Result<(), LayoutError> {
    tree.compute_layout_with_measure(root, taffy::Size { width: w, height: h }, |inputs, _, ctx, style| {
        taffy::compute_leaf_layout(
            inputs,
            style,
            |_, _| 0.0,
            |known, available| {
                let Some(ctx) = ctx else {
                    return taffy::Size::ZERO;
                };
                let (w, h) = match ctx {
                    Measure::Text { content, text: t, runs } => {
                        let p = Paragraph { content, runs };
                        // The min-content width is the widest piece no line breaks.
                        let width = match (known.width, available.width) {
                            (Some(w), _) | (None, AvailableSpace::Definite(w)) => Some(w),
                            (None, AvailableSpace::MinContent) => Some(text.min_content(p, t)),
                            (None, AvailableSpace::MaxContent) => None,
                        };
                        let (w, h) = text.measure(p, t, width.and_then(|w| wrap_width(t, w)));
                        // Text wrapped because it did not fit takes all the width it
                        // was given, as a CSS block does, not just its longest line.
                        match width {
                            Some(given) if h > t.line_height && !t.nowrap => {
                                let (free, _) = text.measure(p, t, None);
                                (if free > given { given } else { w }, h)
                            }
                            _ => (w, h),
                        }
                    }
                    Measure::Image(image) => image.size(known, available),
                    Measure::Input {
                        content,
                        text: t,
                        multiline,
                    } => {
                        if *multiline {
                            let (w, h) =
                                text.measure(content.as_str(), t, width_constraint(known.width, available.width));
                            (w, h.max(t.line_height))
                        } else {
                            // A browser `<input>` is `size=20` wide: 171px of content
                            // for Inter at 14px, measured in Chrome (NumberFieldDemo: 168 = 166 + border).
                            (t.size * INPUT_WIDTH_EMS, t.line_height)
                        }
                    }
                };
                taffy::Size { width: w, height: h }
            },
        )
    })?;
    Ok(())
}

/// Lay a layer's root `id` out again with its style changed by `f`; its new size.
fn relayout(
    tree: &mut TaffyTree<Measure>,
    id: NodeId,
    text: &mut TextSystem,
    f: impl FnOnce(&mut taffy::Style),
) -> Result<(f32, f32), LayoutError> {
    let mut s = tree.style(id)?.clone();
    f(&mut s);
    tree.set_style(id, s)?;
    compute(tree, id, AvailableSpace::MaxContent, AvailableSpace::MaxContent, text)?;
    let size = tree.layout(id)?.size;
    Ok((size.width, size.height))
}

/// The box of the node keyed `key` inside built node `anchor`, once placed.
fn frame_bounds_by_key(frame: &Frame, b: &Builder<'_, '_>, anchor: usize, key: ElementKey) -> Option<Bounds> {
    let within = &b.built.get(anchor)?.path.0;
    frame
        .nodes
        .iter()
        .find(|n| n.path.0.starts_with(within) && n.path.0.last() == Some(&PathSeg::Key(key)))
        .map(|n| n.bounds)
}

/// The box of the node keyed `key` in the layer whose root is built node
/// `root`, from the root's top-left corner, as laid out (not yet placed).
fn layer_bounds_by_key(b: &Builder<'_, '_>, root: usize, key: ElementKey) -> Result<Option<Bounds>, LayoutError> {
    let layer = b.built.get(root..).unwrap_or_default();
    let Some(found) = layer.iter().position(|n| n.path.0.last() == Some(&PathSeg::Key(key))) else {
        return Ok(None);
    };
    let mut i = root.saturating_add(found);
    let Some(node) = b.built.get(i) else {
        return Ok(None);
    };
    let size = b.tree.layout(node.taffy)?.size;
    let (mut x, mut y) = (0.0, 0.0);
    while i != root
        && let Some(n) = b.built.get(i)
    {
        let at = b.tree.layout(n.taffy)?.location;
        x += at.x;
        y += at.y;
        let Some(parent) = n.parent else { break };
        i = parent;
    }
    Ok(Some(Bounds {
        x,
        y,
        width: size.width,
        height: size.height,
    }))
}

/// Where a placed node puts its children.
#[derive(Clone, Copy)]
struct Placed {
    /// The node's index in the frame.
    index: usize,
    /// The children's origin: the node's box less its scroll offset.
    origin: (f32, f32),
    /// The clip its children paint inside, and its corners.
    clip: Option<Bounds>,
    clip_radius: Corners,
}

/// How far a node's children reach from its border box's top-left corner.
#[derive(Clone, Copy)]
struct Reach {
    left: f32,
    right: f32,
    down: f32,
}

impl Default for Reach {
    /// No children: nothing reaches anywhere.
    fn default() -> Self {
        Self {
            left: f32::INFINITY,
            right: 0.0,
            down: 0.0,
        }
    }
}

/// Where `node`'s border box sits in its parent's: Taffy's place for it,
/// mirrored in a right-to-left block container (Taffy lays block children
/// out from the left whatever the `direction`), where an in-flow child keeps
/// its right margin to the content box's right edge, as CSS places it.
fn location(b: &Builder<'_, '_>, node: &Built, l: &taffy::Layout) -> Result<(f32, f32), LayoutError> {
    let parent = node.parent.and_then(|p| b.built.get(p));
    match parent {
        Some(p)
            if p.style.display == Display::Block
                && p.text.direction == Direction::Rtl
                && node.style.position == Position::Relative =>
        {
            let pl = b.tree.layout(p.taffy)?;
            let right = pl.size.width - pl.padding.right - pl.border.right;
            Ok((right - l.margin.right - l.size.width, l.location.y))
        }
        _ => Ok((l.location.x, l.location.y)),
    }
}

/// Turn built nodes from `first` on (one layer, depth-first) into frame
/// nodes, with the layer's root at `origin`.
fn place(b: &Builder<'_, '_>, first: usize, origin: (f32, f32), frame: &mut Frame) -> Result<(), LayoutError> {
    let layer = b.built.get(first).map_or(0, |n| n.layer);
    let count = b.built.iter().skip(first).take_while(|n| n.layer == layer).count();
    let range = first..first.saturating_add(count);

    // How far each node's children reach (left, right, down), for scroll
    // limits, and where the children of a container with a scroll handle lie.
    let mut extent: HashMap<usize, Reach> = HashMap::new();
    let mut tracked: HashMap<usize, Vec<ScrollChild>> = HashMap::new();
    for i in range.clone() {
        let Some(node) = b.built.get(i) else { continue };
        let Some(parent) = node.parent.filter(|p| range.contains(p)) else {
            continue;
        };
        let l = b.tree.layout(node.taffy)?;
        let (x, y) = location(b, node, l)?;
        let e = extent.entry(parent).or_default();
        e.left = e.left.min(x - l.margin.left);
        e.right = e.right.max(x + l.size.width + l.margin.right);
        e.down = e.down.max(y + l.size.height + l.margin.bottom);
        if b.built.get(parent).is_some_and(|p| p.behaviour.scroll.is_some()) {
            tracked.entry(parent).or_default().push(ScrollChild {
                key: node.path.0.last().and_then(|seg| match seg {
                    PathSeg::Key(k) => Some(*k),
                    PathSeg::Index(_) => None,
                }),
                bounds: Bounds {
                    x,
                    y,
                    width: l.size.width,
                    height: l.size.height,
                },
            });
        }
    }

    let mut placed: HashMap<usize, Placed> = HashMap::new();
    for i in range {
        let Some(node) = b.built.get(i) else { continue };
        let l = b.tree.layout(node.taffy)?;
        let (x, y) = location(b, node, l)?;
        let parent = node.parent.and_then(|p| placed.get(&p).copied());
        let (base, clip, clip_radius) = parent.map_or((origin, None, Corners::default()), |p| {
            (p.origin, p.clip, p.clip_radius)
        });
        let shift = node.style.transform.offset((l.size.width, l.size.height), b.placed);
        let bounds = Bounds {
            x: base.0 + x + shift.0,
            y: base.1 + y + shift.1,
            width: l.size.width,
            height: l.size.height,
        };
        let padding_box = bounds.inset(l.border.top, l.border.right, l.border.bottom, l.border.left);
        let content_box = padding_box.inset(l.padding.top, l.padding.right, l.padding.bottom, l.padding.left);

        let scrolls = node.style.overflow_x == Overflow::Scroll || node.style.overflow_y == Overflow::Scroll;
        let rtl = node.text.direction == Direction::Rtl;
        let scroll = scrolls.then(|| {
            let e = extent.get(&i).copied().unwrap_or_default();
            // Offsets run from the inline start: right to left content
            // overflows to the left, and scrolls that way from 0.
            let max = (
                if rtl {
                    (l.border.left + l.padding.left - e.left).max(0.0)
                } else {
                    (e.right + l.padding.right + l.border.right - l.size.width).max(0.0)
                },
                (e.down + l.padding.bottom + l.border.bottom - l.size.height).max(0.0),
            );
            let want = match &node.behaviour.scroll {
                Some(h) => h.offset(),
                None => b.state.scroll.get(&node.path).copied().unwrap_or_default(),
            };
            Scroll {
                offset: (want.0.clamp(0.0, max.0), want.1.clamp(0.0, max.1)),
                max,
                thumb: if node.style.scrollbar_none {
                    Color::TRANSPARENT
                } else {
                    node.text.color.opacity(0.25)
                },
            }
        });
        let offset = scroll.map_or((0.0, 0.0), |s| s.offset);
        if let (Some(h), Some(s)) = (&node.behaviour.scroll, scroll) {
            h.laid_out(ScrollLayout {
                bounds,
                padding: Edges {
                    top: l.padding.top,
                    right: l.padding.right,
                    bottom: l.padding.bottom,
                    left: l.padding.left,
                },
                offset: s.offset,
                max: s.max,
                children: tracked.remove(&i).unwrap_or_default(),
            });
        }
        let (child_clip, child_radius) = if node.style.clips() {
            let r = node.style.radius.resolve(node.text.radius);
            let inner = Corners {
                top_left: (r.top_left - l.border.top.max(l.border.left)).max(0.0),
                top_right: (r.top_right - l.border.top.max(l.border.right)).max(0.0),
                bottom_right: (r.bottom_right - l.border.bottom.max(l.border.right)).max(0.0),
                bottom_left: (r.bottom_left - l.border.bottom.max(l.border.left)).max(0.0),
            };
            (Some(clip.map_or(padding_box, |c| c.intersect(&padding_box))), inner)
        } else {
            (clip, clip_radius)
        };

        let content = match &node.content {
            Pending::None => Content::None,
            Pending::Text { content, runs } => Content::Text {
                content: content.clone(),
                text: node.text,
                runs: runs.clone(),
            },
            Pending::Icon { icon, stroke_width } => Content::Icon {
                icon: *icon,
                color: node.text.color,
                stroke_width: *stroke_width,
            },
            Pending::Shapes(shapes) => Content::Shapes(shapes.clone()),
            Pending::Image(source) => Content::Image {
                source: source.clone(),
                radius: node.style.radius.resolve(node.text.radius),
            },
            Pending::Input(input) => {
                let (edit, preedit) = input.mode.shown(&input.field.edit.get());
                Content::Input {
                    edit,
                    preedit,
                    placeholder: input.placeholder.clone(),
                    placeholder_color: input.placeholder_color.unwrap_or(node.text.color.opacity(0.5)),
                    selection_color: input.selection_color.unwrap_or(Color::hex(0x0003_66d6).opacity(0.3)),
                    text: node.text,
                    focused: node.focused,
                    multiline: input.multiline,
                    mode: input.mode,
                }
            }
        };
        let frame_index = frame.nodes.len();
        frame.nodes.push(FrameNode {
            path: node.path.clone(),
            name: node.name,
            bounds,
            content_box,
            clip,
            clip_radius,
            layer: node.layer,
            parent: parent.map(|p| p.index),
            look: Look {
                background: node.style.background,
                border: node.style.border_widths,
                border_color: node.style.border_color,
                radius: node.style.radius.resolve(node.text.radius),
                shadows: node.style.shadows.clone(),
                ring: node.style.ring,
                opacity: node.opacity,
                invisible: node.invisible,
                effects: node.style.effects.clone(),
                transform: node.style.transform.painted(bounds, b.placed),
                text: node.text,
            },
            content,
            scroll,
        });
        frame.behaviour.push(node.behaviour.clone());
        placed.insert(
            i,
            Placed {
                index: frame_index,
                origin: (
                    if rtl { bounds.x + offset.0 } else { bounds.x - offset.0 },
                    bounds.y - offset.1,
                ),
                clip: child_clip,
                clip_radius: child_radius,
            },
        );
    }
    Ok(())
}

/// How a text input lays out its text: the style, the wrap width, and where
/// the text box starts, shifted left so the caret stays in view.
pub(crate) struct InputText {
    pub t: ResolvedText,
    pub width: Option<f32>,
    pub origin: (f32, f32),
}

/// Width of an `<input>` that sets none, in ems of its font size.
const INPUT_WIDTH_EMS: f32 = 166.0 / 14.0;

pub(crate) fn input_text(
    text: &mut TextSystem,
    content: &str,
    t: &ResolvedText,
    area: Bounds,
    multiline: bool,
    caret: usize,
) -> InputText {
    if multiline {
        return InputText {
            t: *t,
            width: Some(area.width),
            origin: (area.x, area.y),
        };
    }
    let line = ResolvedText {
        nowrap: true,
        truncate: false,
        ..*t
    };
    // Text that fits is aligned in the box; text that does not scrolls to the caret.
    let (free, _) = text.measure(content, &line, None);
    if line.physical_align() != TextAlign::Left && free <= area.width {
        return InputText {
            t: line,
            width: Some(area.width),
            origin: (area.x, area.y + ((area.height - t.line_height) / 2.0).max(0.0)),
        };
    }
    let caret_x = text.caret(content, &line, None, caret).x;
    let shift = (caret_x - area.width + 1.0).max(0.0);
    InputText {
        t: line,
        width: None,
        origin: (area.x - shift, area.y + ((area.height - t.line_height) / 2.0).max(0.0)),
    }
}

/// The wrap width a text node is drawn with, given its area.
#[must_use]
pub fn text_width(text: &ResolvedText, area: &Bounds) -> Option<f32> {
    wrap_width(text, area.width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{IntoElement, Side, SideAlign, deferred, div, text};
    use crate::styled::Styled;

    fn frame(root: impl IntoElement) -> Frame {
        let mut ts = TextSystem::new();
        layout(
            &root.into_element(),
            400.0,
            300.0,
            &UiState::default(),
            &mut Motion::default(),
            &mut ts,
        )
        .expect("layout")
    }

    fn find<'a>(f: &'a Frame, name: &str) -> &'a FrameNode {
        f.nodes.iter().find(|n| n.name == Some(name)).expect("node")
    }

    #[test]
    fn flex_row_places_children_side_by_side_with_gap() {
        let f = frame(
            div()
                .flex()
                .gap(2.0)
                .p(4.0)
                .child(div().named("a").size(10.0))
                .child(div().named("b").size(10.0)),
        );
        let (a, b) = (find(&f, "a").bounds, find(&f, "b").bounds);
        assert_eq!((a.x, a.y, a.width), (16.0, 16.0, 40.0));
        assert_eq!(b.x, 64.0, "40px box + 8px gap after the first");
    }

    #[test]
    fn text_wraps_inside_a_narrow_box() {
        let f = frame(div().child(div().named("box").w_px(60.0).child(text("several words that wrap"))));
        let b = find(&f, "box").bounds;
        assert!(b.height >= 48.0, "at least two 24px lines: {b}");
    }

    #[test]
    fn a_flex_item_shrinks_to_its_widest_word_and_no_further() {
        let f = frame(
            div().child(
                div()
                    .flex()
                    .items_start()
                    .w_px(20.0)
                    .child(div().named("prefix").child(text("https://")))
                    .child(div().named("rest").flex_1()),
            ),
        );
        let b = find(&f, "prefix").bounds;
        assert!(b.width > 20.0, "as wide as the word, overflowing: {b}");
        assert_eq!(b.height, 24.0, "the word on one line: {b}");
    }

    #[test]
    fn a_deferred_layer_opens_below_its_anchor_and_stays_on_screen() {
        let f = frame(
            div()
                .p(2.0)
                .child(div().named("anchor").ml(80.0).w_px(60.0).h_px(20.0).child(deferred(
                    Placement::Anchored {
                        side: Side::Bottom,
                        align: SideAlign::Start,
                        offset: 4.0,
                        match_width: false,
                        fit_height: false,
                    },
                    div().named("menu").w_px(200.0).h_px(50.0),
                ))),
        );
        let anchor = find(&f, "anchor").bounds;
        let menu = find(&f, "menu");
        assert_eq!(menu.layer, 1);
        assert_eq!(menu.bounds.y, anchor.bottom() + 4.0);
        assert_eq!(
            menu.bounds.right(),
            400.0 - EDGE_MARGIN,
            "pushed back inside the window"
        );
    }

    #[test]
    fn scrolled_children_move_up_and_are_clipped() {
        let mut state = UiState::default();
        let mut ts = TextSystem::new();
        let tree = || {
            div()
                .child(
                    div()
                        .named("list")
                        .h_px(100.0)
                        .overflow_y_scroll()
                        .children((0..10).map(|i| div().named(if i == 0 { "first" } else { "item" }).h_px(40.0))),
                )
                .into_element()
        };
        let before = layout(&tree(), 400.0, 300.0, &state, &mut Motion::default(), &mut ts).expect("layout");
        let path = find(&before, "list").path.clone();
        state.scroll.insert(path, (0.0, 1000.0));
        let after = layout(&tree(), 400.0, 300.0, &state, &mut Motion::default(), &mut ts).expect("layout");
        let list = find(&after, "list");
        assert_eq!(list.scroll.map(|s| s.offset.1), Some(300.0), "clamped to 400 - 100");
        assert_eq!(find(&after, "first").bounds.y, -300.0);
        assert_eq!(find(&after, "first").clip, Some(list.bounds));
    }
}
