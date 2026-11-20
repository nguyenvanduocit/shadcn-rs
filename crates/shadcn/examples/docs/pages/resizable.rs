use gui::prelude::*;
use gui::{ScenarioError, div, text};
use shadcn::{Orientation, ResizableState, resizable_handle, resizable_panel, resizable_panel_group, theme};

use crate::{Render, Shot, page, section};

fn label(s: &str) -> gui::Div {
    div()
        .flex()
        .h_full()
        .items_center()
        .justify_center()
        .p(6.0)
        .child(text(s.to_owned()).font_semibold())
}

pub fn render() -> Render {
    let outer = ResizableState::new(Orientation::Horizontal, &[50.0, 50.0]);
    let inner = ResizableState::new(Orientation::Vertical, &[25.0, 75.0]);
    let grip = ResizableState::new(Orientation::Horizontal, &[25.0, 75.0]);
    let grip_v = ResizableState::new(Orientation::Vertical, &[25.0, 75.0]);
    Box::new(move || {
        let t = theme();
        let demo = resizable_panel_group(&outer)
            .max_w_px(448.0)
            .min_w_px(450.0)
            .rounded_lg()
            .border()
            .border_color(t.border)
            .child(
                resizable_panel(&outer, 0).child(
                    div()
                        .flex()
                        .h_px(200.0)
                        .items_center()
                        .justify_center()
                        .p(6.0)
                        .child(text("One").font_semibold()),
                ),
            )
            .child(resizable_handle(&outer, 0, false))
            .child(
                resizable_panel(&outer, 1).child(
                    resizable_panel_group(&inner)
                        .child(resizable_panel(&inner, 0).child(label("Two")))
                        .child(resizable_handle(&inner, 0, false))
                        .child(resizable_panel(&inner, 1).child(label("Three"))),
                ),
            );
        let handle = resizable_panel_group(&grip)
            .min_h_px(200.0)
            .max_w_px(448.0)
            .min_w_px(450.0)
            .rounded_lg()
            .border()
            .border_color(t.border)
            .child(resizable_panel(&grip, 0).child(label("Sidebar")))
            .child(resizable_handle(&grip, 0, true))
            .child(resizable_panel(&grip, 1).child(label("Content")));
        let vertical = resizable_panel_group(&grip_v)
            .h_px(200.0)
            .w_px(400.0)
            .rounded_lg()
            .border()
            .border_color(t.border)
            .child(resizable_panel(&grip_v, 0).child(label("Header")))
            .child(resizable_handle(&grip_v, 0, true))
            .child(resizable_panel(&grip_v, 1).child(label("Content")));
        let sizes = |s: &ResizableState| {
            s.sizes
                .get()
                .iter()
                .map(|v| format!("{v:.0}"))
                .collect::<Vec<_>>()
                .join(" / ")
        };
        page(
            "Resizable",
            div()
                .flex()
                .items_start()
                .gap(8.0)
                .child(
                    div()
                        .flex_col()
                        .gap(6.0)
                        .child(section(
                            &format!("Demo ({})", sizes(&outer)),
                            crate::demo("ResizableDemo.vue", demo),
                        ))
                        .child(section(&format!("Handle ({})", sizes(&grip)), handle)),
                )
                .child(section(&format!("Vertical ({})", sizes(&grip_v)), vertical)),
        )
    })
}

/// The centre of the `nth` handle in the last frame.
fn handle(s: &Shot, nth: usize) -> Result<(f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .filter(|n| n.name == Some("resizable-handle"))
        .nth(nth)
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound(format!("resizable-handle #{nth}")))
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = handle(s, 0)?;
    s.drag((x, y), (x - 100.0, y));
    s.frame("demo handle dragged left 100")?;
    let (x, y) = handle(s, 1)?;
    s.drag((x, y), (x, y + 60.0));
    s.frame("inner handle dragged down 60")?;
    let (x, y) = handle(s, 2)?;
    s.hover(x, y);
    s.drag((x, y + 40.0), (x + 150.0, y + 40.0));
    s.frame("grip dragged right 150")?;
    let (x, y) = handle(s, 3)?;
    s.drag((x, y), (x, y + 50.0));
    s.frame("vertical grip dragged down 50")?;
    s.press(gui::NamedKey::Tab, gui::Modifiers::default());
    s.frame("tab to first handle")?;
    s.press(gui::NamedKey::ArrowRight, gui::Modifiers::default());
    s.frame("arrow right 10")?;
    Ok(())
}
