use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, Signal, div, text_bound};
use shadcn::{
    ButtonSize, ButtonVariant, alert_dialog, alert_dialog_action, alert_dialog_cancel, alert_dialog_content,
    alert_dialog_description, alert_dialog_footer, alert_dialog_header, alert_dialog_title, button, dialog_trigger,
};

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page};

pub fn render() -> Render {
    let confirmed = Signal::labeled("confirmed", 0_u32);
    Box::new(move || {
        let count = confirmed;
        page(
            "Alert Dialog",
            div()
                .flex_col()
                .items_start()
                .gap(4.0)
                .child(demo(
                    "AlertDialogDemo.vue",
                    alert_dialog()
                        .child(dialog_trigger(button(ButtonVariant::Outline, ButtonSize::Default).child("Show Dialog")))
                        .child(alert_dialog_content()
                            .child(alert_dialog_header().child(alert_dialog_title("Are you absolutely sure?")).child(
                                alert_dialog_description(
                                    "This action cannot be undone. This will permanently delete your account and remove your data from our servers.",
                                ),
                            ))
                            .child(
                                alert_dialog_footer()
                                    .child(alert_dialog_cancel().child("Cancel"))
                                    .child(alert_dialog_action(move || count.update(|n| *n = n.saturating_add(1))).child("Continue")),
                            )),
                ))
                .child(text_bound(move || format!("Continued {} times", confirmed.get())).text_sm()),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    motion_frames(s, "opening", &MOTION_MS, |s| s.click_text("Show Dialog"))?;
    motion_frames(s, "closing", &MOTION_MS, |s| {
        s.press(NamedKey::Escape, Modifiers::default());
        Ok(())
    })?;
    s.click_text("Show Dialog")?;
    s.frame("open")?;
    s.click(20.0, 600.0);
    s.frame("outside press keeps it open")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab to cancel")?;
    s.press(NamedKey::Escape, Modifiers::default());
    s.frame("escape closed")?;
    s.click_text("Show Dialog")?;
    s.settle()?;
    s.click_text("Continue")?;
    s.frame("continued")?;
    Ok(())
}
