use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, TextField, div};
use shadcn::{
    ButtonSize, ButtonVariant, TabsState, button, card, card_content, card_description, card_footer, card_header,
    card_title, input, label, tabs, tabs_content, tabs_list, tabs_trigger,
};

use crate::{Render, Shot, demo, page};

fn field(name: &str, field: &TextField) -> gui::Div {
    div().grid().gap(3.0).child(label(name)).child(input(field))
}

pub fn render() -> Render {
    let state = TabsState::new("account");
    let name = TextField::new("Pedro Duarte");
    let username = TextField::new("@peduarte");
    let current = TextField::new("");
    let new = TextField::new("");
    Box::new(move || {
        page(
            "Tabs",
            demo(
                "TabsDemo.vue",
                div().flex_col().w_full().max_w_px(384.0).gap(6.0).child(
                    tabs()
                        .child(
                            tabs_list()
                                .child(tabs_trigger(&state, "account").child("Account"))
                                .child(tabs_trigger(&state, "password").child("Password")),
                        )
                        .child(tabs_content(&state, "account").map(|c| {
                            c.child(
                                card()
                                    .child(card_header().child(card_title("Account")).child(card_description(
                                        "Make changes to your account here. Click save when you're done.",
                                    )))
                                    .child(
                                        card_content()
                                            .grid()
                                            .gap(6.0)
                                            .child(field("Name", &name))
                                            .child(field("Username", &username)),
                                    )
                                    .child(card_footer().child(
                                        button(ButtonVariant::Default, ButtonSize::Default).child("Save changes"),
                                    )),
                            )
                        }))
                        .child(tabs_content(&state, "password").map(|c| {
                            c.child(
                                card()
                                    .child(card_header().child(card_title("Password")).child(card_description(
                                        "Change your password here. After saving, you'll be logged out.",
                                    )))
                                    .child(
                                        card_content()
                                            .grid()
                                            .gap(6.0)
                                            .child(field("Current password", &current))
                                            .child(field("New password", &new)),
                                    )
                                    .child(card_footer().child(
                                        button(ButtonVariant::Default, ButtonSize::Default).child("Save password"),
                                    )),
                            )
                        })),
                ),
            ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("account")?;
    s.click_text("Password")?;
    s.frame("clicked password")?;
    s.press(NamedKey::ArrowRight, Modifiers::default());
    s.frame("arrow right wraps to account")?;
    s.press(NamedKey::End, Modifiers::default());
    s.frame("end selects password")?;
    Ok(())
}
