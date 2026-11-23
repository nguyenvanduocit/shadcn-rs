use std::time::Duration;

use gui::prelude::*;
use gui::{ScenarioError, div};
use shadcn::{
    ButtonSize, ButtonVariant, Toast, ToastKind, button, dialog, dialog_content, dialog_description, dialog_header,
    dialog_title, dialog_trigger, toast, toaster,
};

use crate::{Render, Shot, demo, motion_frames, page};

/// The toast `SonnerDemo.vue` and `SonnerWithDialogDemo.vue` raise.
fn event_created() -> Toast {
    Toast::new(ToastKind::Default, "Event has been created")
        .description("Sunday, December 03, 2023 at 9:00 AM")
        .action("Undo", |()| {})
}

pub fn render() -> Render {
    Box::new(move || {
        let outline = |label: &str| button(ButtonVariant::Outline, ButtonSize::Default).child(label.to_owned());
        let types = div()
            .display(Display::Flex)
            .flex_wrap(true)
            .gap(8.0)
            .child(outline("Default").on_click(|_| {
                toast("Event has been created");
            }))
            .child(outline("Success").on_click(|_| {
                toast::success("Event has been created");
            }))
            .child(outline("Info").on_click(|_| {
                toast::info("Be at the area 10 minutes before the event time");
            }))
            .child(outline("Warning").on_click(|_| {
                toast::warning("Event start time cannot be earlier than 8am");
            }))
            .child(outline("Error").on_click(|_| {
                toast::error("Event has not been created");
            }))
            .child(outline("Promise").on_click(|_| {
                // `toast.promise`: loading until the work settles, 2s later.
                let id = toast::loading("Loading...");
                gui::clock::after(Duration::from_secs(2), move || {
                    toast::update(id, Toast::new(ToastKind::Success, "Event has been created"));
                });
            }));
        page(
            "Sonner",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "SonnerDemo.vue",
                    outline("Show Toast").on_click(|_| {
                        toast(event_created());
                    }),
                ))
                .child(demo("SonnerTypesDemo.vue", types))
                .child(demo(
                    "SonnerWithDialogDemo.vue",
                    // A press on a toast is no press outside the dialog: the
                    // toaster's layer is above it, so the dialog stays open.
                    div().display(Display::Flex).child(
                        dialog().child(dialog_trigger(outline("Dialog with toast"))).child(
                            dialog_content()
                                .max_width(448.0)
                                .child(
                                    dialog_header()
                                        .child(dialog_title("Vue Sonner Toast"))
                                        .child(dialog_description("Dialog with toast")),
                                )
                                .child(
                                    div().display(Display::Grid).gap(16.0).child(
                                        button(ButtonVariant::Default, ButtonSize::Sm)
                                            .padding_x(12.0)
                                            .on_click(|_| {
                                                toast(event_created());
                                            })
                                            .child("Toast"),
                                    ),
                                ),
                        ),
                    ),
                ))
                .child(toaster()),
        )
    })
}

/// Let `ms` pass, running the timers that come due.
fn wait(s: &mut Shot, ms: u64) {
    s.wait(Duration::from_millis(ms));
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.click_text("Show Toast")?;
    s.frame("one toast")?;
    for kind in ["Success", "Info", "Warning", "Error"] {
        s.click_text(kind)?;
        s.settle()?;
    }
    s.hover(20.0, 600.0);
    s.frame("stack of three")?;
    s.hover(780.0, 600.0);
    s.frame("hover spreads them")?;
    s.hover(20.0, 600.0);
    s.click_text("Promise")?;
    s.frame("loading")?;
    wait(s, 2100);
    s.frame("promise settled")?;
    wait(s, 4100);
    s.frame("all expired")?;
    s.click_text("Show Toast")?;
    s.settle()?;
    s.click_text("Undo")?;
    s.frame("action dismissed it")?;
    motion_frames(s, "another toast", &[0, 100, 200, 400], |s| s.click_text("Show Toast"))?;
    // Gone again before the dialog: the action dismisses it, sinking out.
    motion_frames(s, "dismissed", &[0, 100, 200, 400], |s| s.click_text("Undo"))?;
    s.click_text("Dialog with toast")?;
    s.settle()?;
    s.click_text("Toast")?;
    s.frame("toast over the dialog")?;
    let (x, y) = s.locate("Event has been created")?;
    s.click(x, y);
    s.frame("pressing the toast keeps the dialog")?;
    Ok(())
}
