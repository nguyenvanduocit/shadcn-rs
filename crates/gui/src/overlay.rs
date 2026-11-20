//! The debug overlay: outlines every box, highlights what the last frame
//! changed and labels the node under the pointer.

use tiny_skia::Pixmap;

use crate::color::Color;
use crate::convert::u32_to_f32;
use crate::geometry::Bounds;
use crate::inspect::{Change, LayoutDiff, describe};
use crate::layout::FrameNode;
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

/// What the overlay shows for one frame.
pub struct Overlay<'a> {
    pub nodes: &'a [FrameNode],
    /// How this frame differs from the one before it.
    pub diff: &'a LayoutDiff,
    /// The node under the pointer, if any.
    pub hovered: Option<&'a FrameNode>,
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

/// Paint `overlay` on top of an already painted frame.
pub fn paint_overlay(pixmap: &mut Pixmap, scale: f32, text: &mut TextSystem, overlay: &Overlay<'_>) {
    let viewport = (u32_to_f32(pixmap.width()) / scale, u32_to_f32(pixmap.height()) / scale);
    let mut target = Target {
        pixmap,
        scale,
        clip: None,
        visible: None,
    };
    for n in overlay.nodes {
        stroke(&mut target, n.bounds, 1.0, Color::hex(OUTLINE).opacity(0.3));
    }
    for c in &overlay.diff.changes {
        let color = Color::hex(change_color(&c.change));
        let b = changed_bounds(&c.change);
        target.fill_rect(b, color.opacity(0.18));
        stroke(&mut target, b, 2.0, color);
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
    let head = format!(
        "F12 overlay  {} nodes, {} changed:",
        overlay.diff.nodes,
        overlay.diff.changes.len()
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
    ] {
        x = chip(&mut target, text, name, (x, y), viewport, Some(Color::hex(color)));
    }
}

/// Paint `lines` on chips stacked in the window's top-right corner (the frame HUD).
pub fn paint_hud(pixmap: &mut Pixmap, scale: f32, text: &mut TextSystem, lines: &[String]) {
    let viewport = (u32_to_f32(pixmap.width()) / scale, u32_to_f32(pixmap.height()) / scale);
    let mut target = Target {
        pixmap,
        scale,
        clip: None,
        visible: None,
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
