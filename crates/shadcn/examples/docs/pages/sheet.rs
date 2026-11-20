use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, TextField, div};
use shadcn::{
    ButtonSize, ButtonVariant, DialogState, SheetSide, button, dialog_close, dialog_trigger, input, label,
    sheet_content, sheet_description, sheet_footer, sheet_header, sheet_title,
};

use crate::{Render, Shot, demo, motion_frames, page, section};

const SIDES: [(&str, SheetSide); 4] = [
    ("Top", SheetSide::Top),
    ("Right", SheetSide::Right),
    ("Bottom", SheetSide::Bottom),
    ("Left", SheetSide::Left),
];

pub fn render() -> Render {
    let profile_sheet = DialogState::new();
    let name = TextField::new("Pedro Duarte");
    let username = TextField::new("@peduarte");
    let sides: Vec<DialogState> = SIDES.iter().map(|_| DialogState::new()).collect();
    Box::new(move || {
        let field = |name: &str, f: &TextField| div().grid().gap(3.0).child(label(name)).child(input(f));
        let profile = |state: &DialogState, side: SheetSide| {
            sheet_content(state, side, |c| {
                c.child(
                    sheet_header()
                        .child(sheet_title("Edit profile"))
                        .child(sheet_description(
                            "Make changes to your profile here. Click save when you're done.",
                        )),
                )
                .child(
                    div()
                        .grid()
                        .flex_1()
                        .auto_rows_min()
                        .gap(6.0)
                        .px(4.0)
                        .child(field("Name", &name))
                        .child(field("Username", &username)),
                )
                .child(
                    sheet_footer()
                        .child(button(ButtonVariant::Default, ButtonSize::Default).child("Save changes"))
                        .child(dialog_close(
                            state,
                            button(ButtonVariant::Outline, ButtonSize::Default).child("Close"),
                        )),
                )
            })
        };
        page(
            "Sheet",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "SheetDemo.vue",
                    div()
                        .flex()
                        .child(dialog_trigger(
                            &profile_sheet,
                            button(ButtonVariant::Outline, ButtonSize::Default).child("Open"),
                        ))
                        .child(profile(&profile_sheet, SheetSide::Right)),
                ))
                .child(section(
                    "Sides",
                    div()
                        .flex()
                        .gap(2.0)
                        .children(SIDES.iter().zip(&sides).map(|((name, side), state)| {
                            div()
                                .child(dialog_trigger(
                                    state,
                                    button(ButtonVariant::Outline, ButtonSize::Default).child(*name),
                                ))
                                .child(profile(state, *side))
                        })),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    motion_frames(s, "opening", &[0, 125, 250, 500], |s| s.click_text("Open"))?;
    motion_frames(s, "closing", &[0, 75, 150, 300], |s| {
        s.press(NamedKey::Escape, Modifiers::default());
        Ok(())
    })?;
    s.click_text("Open")?;
    s.frame("right open")?;
    s.press(NamedKey::Escape, Modifiers::default());
    s.frame("escape closed")?;
    for side in ["Top", "Bottom", "Left"] {
        s.click_text(side)?;
        s.frame(&format!("{side} open"))?;
        s.click_text("Close")?;
        s.settle()?;
    }
    s.click_text("Left")?;
    s.settle()?;
    s.click(900.0, 320.0);
    s.frame("overlay press closed")?;
    Ok(())
}
