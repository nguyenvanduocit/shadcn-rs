//! The debug overlay: outlines every box, highlights what the last frame
//! changed, flashes where paint recorded again, and labels the node under
//! the pointer. The frame HUD: its lines and a graph of the last frames'
//! times, drawn straight into a scene (no element tree, no layout).

use std::time::Duration;

use crate::color::Color;
use crate::geometry::Bounds;
use crate::inspect::{Change, LayoutDiff, Repaint, describe};
use crate::layout::FrameNode;
use crate::scene::Scene;
use crate::style::{FontFamily, ResolvedText};
use crate::text::{Target, TextSystem};

const OUTLINE: u32 = 0x0093_99b2;
const ADDED: u32 = 0x00a6_e3a1;
const MOVED: u32 = 0x00f9_e2af;
const RESIZED: u32 = 0x00fa_b387;
const REPAINTED: u32 = 0x0089_b4fa;
const REMOVED: u32 = 0x00f3_8ba8;
const HOVER: u32 = 0x00cb_a6f7;
const LABEL_BG: u32 = 0x0011_111b;
const LABEL_TEXT: u32 = 0x00cd_d6f4;
const LABEL_PAD: f32 = 6.0;
/// Paint flashing, as Chrome's rendering tools colour it: recorded again green,
/// only moved (re-composited) teal.
const RECORDED: u32 = 0x0040_c057;
const COMPOSITED: u32 = 0x0022_b8cf;
/// The HUD graph: one bar per frame, 0 to two 60 Hz frames tall.
const GRAPH_BAR: f32 = 2.0;
const GRAPH_HEIGHT: f32 = 48.0;
const GRAPH_SPAN: Duration = Duration::from_micros(33_333);
const GRAPH_BUDGET: Duration = Duration::from_micros(16_667);
const GRAPH_OK: u32 = 0x00a6_e3a1;
const GRAPH_SLOW: u32 = 0x00fa_b387;
const GRAPH_LATE: u32 = 0x00f3_8ba8;

/// What the overlay shows for one frame.
pub struct Overlay<'a> {
    pub nodes: &'a [FrameNode],
    /// How this frame differs from the one before it.
    pub diff: &'a LayoutDiff,
    /// The node under the pointer, if any.
    pub hovered: Option<&'a FrameNode>,
    /// Where this frame's paint recorded again or only moved what it kept.
    pub painted: &'a [Repaint],
}

fn change_color(change: &Change) -> u32 {
    match change {
        Change::Added { .. } => ADDED,
        Change::Removed { .. } => REMOVED,
        Change::Updated { .. } if change.resized() => RESIZED,
        Change::Updated { .. } if change.moved() => MOVED,
        Change::Updated { .. } => REPAINTED,
    }
}

fn changed_bounds(change: &Change) -> Bounds {
    match change {
        Change::Added { after } | Change::Updated { after, .. } => *after,
        Change::Removed { before } => *before,
    }
}

fn label_text() -> ResolvedText {
    ResolvedText {
        size: 12.0,
        line_height: 16.0,
        family: FontFamily::Mono,
        nowrap: true,
        ..ResolvedText::root(Color::hex(LABEL_TEXT))
    }
}

fn stroke(target: &mut Target<'_>, b: Bounds, width: f32, color: Color) {
    target.fill_rect(Bounds { height: width, ..b }, color);
    target.fill_rect(
        Bounds {
            y: b.bottom() - width,
            height: width,
            ..b
        },
        color,
    );
    target.fill_rect(Bounds { width, ..b }, color);
    target.fill_rect(
        Bounds {
            x: b.right() - width,
            width,
            ..b
        },
        color,
    );
}

/// Record `overlay`, to paint on top of a frame `size` physical pixels at `scale`.
pub fn record_overlay(size: (u32, u32), scale: f32, text: &mut TextSystem, overlay: &Overlay<'_>) -> Scene {
    let mut scene = Scene::new(size, scale, None);
    let viewport = scene.logical_size();
    let mut items = Vec::new();
    let mut target = Target {
        items: &mut items,
        scale,
        size,
        cull: None,
        snap: true,
    };
    for n in overlay.nodes {
        stroke(&mut target, n.bounds, 1.0, Color::hex(OUTLINE).opacity(0.3));
    }
    for c in overlay.diff.changes.iter() {
        let color = Color::hex(change_color(&c.change));
        let b = changed_bounds(&c.change);
        target.fill_rect(b, color.opacity(0.18));
        stroke(&mut target, b, 2.0, color);
    }
    for p in overlay.painted {
        if p.recorded {
            target.fill_rect(p.bounds, Color::hex(RECORDED).opacity(0.25));
            stroke(&mut target, p.bounds, 1.0, Color::hex(RECORDED));
        } else {
            stroke(&mut target, p.bounds, 1.0, Color::hex(COMPOSITED));
        }
    }
    if let Some(n) = overlay.hovered {
        stroke(&mut target, n.bounds, 2.0, Color::hex(HOVER));
        let label = format!("{}  {}  {}", n.path, describe(n), n.bounds);
        chip(
            &mut target,
            text,
            &label,
            (n.bounds.x, n.bounds.bottom() + 4.0),
            viewport,
            None,
        );
    }
    let recorded = overlay.painted.iter().filter(|p| p.recorded).count();
    let head = format!(
        "F12 overlay  {} nodes, {} changed, {recorded} painted, {} moved:",
        overlay.diff.nodes,
        overlay.diff.changes.len(),
        overlay.painted.len().saturating_sub(recorded)
    );
    let t = label_text();
    let y = viewport.1 - t.line_height - LABEL_PAD * 2.0;
    let mut x = chip(&mut target, text, &head, (0.0, y), viewport, None);
    for (name, color) in [
        ("added", ADDED),
        ("moved", MOVED),
        ("resized", RESIZED),
        ("repainted", REPAINTED),
        ("removed", REMOVED),
        ("painted", RECORDED),
        ("composited", COMPOSITED),
    ] {
        x = chip(&mut target, text, name, (x, y), viewport, Some(Color::hex(color)));
    }
    scene.push(items);
    scene
}

/// Record `lines` on chips stacked in the window's top-right corner, and
/// under them a bar per frame in `times` (oldest first): green within a
/// 60 Hz frame, orange within two, red past two, with a line at 16.7 ms
/// (the frame HUD).
pub fn record_hud(size: (u32, u32), scale: f32, text: &mut TextSystem, lines: &[String], times: &[Duration]) -> Scene {
    let mut scene = Scene::new(size, scale, None);
    let viewport = scene.logical_size();
    let mut items = Vec::new();
    let mut target = Target {
        items: &mut items,
        scale,
        size,
        cull: None,
        snap: true,
    };
    let step = label_text().line_height + LABEL_PAD;
    // The label face is monospaced: lines padded to one length make one block.
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let mut y = 0.0;
    for line in lines {
        chip(
            &mut target,
            text,
            &format!("{line:<width$}"),
            (viewport.0, y),
            viewport,
            None,
        );
        y += step;
    }
    graph(&mut target, times, (viewport.0, y));
    scene.push(items);
    scene
}

/// The frame-time graph with its top-right corner at `at`.
fn graph(target: &mut Target<'_>, times: &[Duration], at: (f32, f32)) {
    if times.is_empty() {
        return;
    }
    let bars = crate::convert::usize_to_f32(times.len());
    let width = bars * GRAPH_BAR + LABEL_PAD * 2.0;
    let x0 = (at.0 - width).max(0.0);
    let top = at.1 + LABEL_PAD / 2.0;
    let frame = Bounds {
        x: x0,
        y: top,
        width,
        height: GRAPH_HEIGHT + LABEL_PAD,
    };
    target.fill_rect(frame, Color::hex(LABEL_BG).opacity(0.9));
    let floor = top + LABEL_PAD / 2.0 + GRAPH_HEIGHT;
    let tall = |d: Duration| {
        let f = d.as_secs_f32() / GRAPH_SPAN.as_secs_f32();
        f.clamp(0.0, 1.0) * GRAPH_HEIGHT
    };
    let mut x = x0 + LABEL_PAD;
    for &t in times {
        let color = if t <= GRAPH_BUDGET {
            GRAPH_OK
        } else if t <= GRAPH_SPAN {
            GRAPH_SLOW
        } else {
            GRAPH_LATE
        };
        let h = tall(t).max(1.0);
        target.fill_rect(
            Bounds {
                x,
                y: floor - h,
                width: GRAPH_BAR,
                height: h,
            },
            Color::hex(color),
        );
        x += GRAPH_BAR;
    }
    target.fill_rect(
        Bounds {
            x: x0 + LABEL_PAD,
            y: floor - tall(GRAPH_BUDGET),
            width: bars * GRAPH_BAR,
            height: 1.0,
        },
        Color::hex(LABEL_TEXT).opacity(0.6),
    );
}

/// Draw `s` on a dark chip at `at`, kept inside the window; returns the x past it.
fn chip(
    target: &mut Target<'_>,
    text: &mut TextSystem,
    s: &str,
    at: (f32, f32),
    viewport: (f32, f32),
    ink: Option<Color>,
) -> f32 {
    let t = label_text();
    let (w, h) = text.measure(s, &t, None);
    let (w, h) = (w + LABEL_PAD * 2.0, h + LABEL_PAD);
    let x = at.0.min(viewport.0 - w).max(0.0);
    let y = at.1.min(viewport.1 - h).max(0.0);
    target.fill_rect(
        Bounds {
            x,
            y,
            width: w,
            height: h,
        },
        Color::hex(LABEL_BG).opacity(0.9),
    );
    let color = ink.unwrap_or(t.color);
    let t = ResolvedText { color, ..t };
    text.draw(target, s, &t, None, (x + LABEL_PAD, y + LABEL_PAD / 2.0), 1.0);
    x + w
}
