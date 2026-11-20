use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, Side, SideAlign, Signal, div, icon, text};
use shadcn::{
    ButtonSize, ButtonVariant, MenuItem, MenuState, button, dropdown_menu_checkbox_item, dropdown_menu_content,
    dropdown_menu_group, dropdown_menu_item, dropdown_menu_label, dropdown_menu_radio_group, dropdown_menu_radio_item,
    dropdown_menu_separator, dropdown_menu_shortcut, dropdown_menu_sub_content, dropdown_menu_sub_trigger,
    dropdown_menu_trigger, theme,
};

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page, section};

pub fn render() -> Render {
    let account = MenuState::new();
    let invite = account.sub();
    let checkboxes = MenuState::new();
    let radio = MenuState::new();
    let last = Signal::labeled("last action", String::from("nothing yet"));
    let status_bar = Signal::labeled("status bar", true);
    let activity_bar = Signal::labeled("activity bar", false);
    let panel = Signal::labeled("panel", false);
    let position = Signal::labeled("position", String::from("bottom"));
    Box::new(move || {
        let t = theme();
        let act = |name: &'static str| {
            let last = last.clone();
            move || last.set(name.to_owned())
        };
        let item = |menu: &MenuState, name: &'static str| {
            dropdown_menu_item(menu, name, MenuItem::default(), act(name)).child(name)
        };
        let with_shortcut =
            |menu: &MenuState, name: &'static str, keys: &str| item(menu, name).child(dropdown_menu_shortcut(keys));
        let with_icon = |menu: &MenuState, glyph: Icon, name: &'static str| {
            dropdown_menu_item(menu, name, MenuItem::default(), act(name))
                .child(icon(glyph).text_color(t.muted_foreground))
                .child(name)
        };

        let a = &account;
        let account_menu = dropdown_menu_content(a)
            .w(56.0)
            .child(dropdown_menu_label("My Account", false))
            .child(
                dropdown_menu_group()
                    .child(with_shortcut(a, "Profile", "⇧⌘P"))
                    .child(with_shortcut(a, "Billing", "⌘B"))
                    .child(with_shortcut(a, "Settings", "⌘S"))
                    .child(with_shortcut(a, "Keyboard shortcuts", "⌘K")),
            )
            .child(dropdown_menu_separator())
            .child(
                dropdown_menu_group()
                    .child(item(a, "Team"))
                    .child(dropdown_menu_sub_trigger(
                        &invite,
                        "Invite users",
                        MenuItem::default(),
                        dropdown_menu_sub_content(&invite)
                            .child(item(&invite, "Email"))
                            .child(item(&invite, "Message"))
                            .child(dropdown_menu_separator())
                            .child(item(&invite, "More...")),
                    ))
                    .child(with_shortcut(a, "New Team", "⌘+T")),
            )
            .child(dropdown_menu_separator())
            .child(item(a, "GitHub"))
            .child(item(a, "Support"))
            .child(dropdown_menu_item(a, "API", MenuItem::DISABLED, || {}).child("API"))
            .child(dropdown_menu_separator())
            .child(with_shortcut(a, "Log out", "⇧⌘Q"));

        let c = &checkboxes;
        let checkbox_menu = dropdown_menu_content(c)
            .w(56.0)
            .child(
                dropdown_menu_group()
                    .child(dropdown_menu_label("Account", false))
                    .child(with_icon(c, Icon::User, "Profile"))
                    .child(with_icon(c, Icon::CreditCard, "Billing"))
                    .child(with_icon(c, Icon::Settings, "Settings")),
            )
            .child(dropdown_menu_separator())
            .child(
                dropdown_menu_group()
                    .child(dropdown_menu_label("Appearance", false))
                    .child(dropdown_menu_checkbox_item(c, "Status Bar", &status_bar, false))
                    .child(dropdown_menu_checkbox_item(c, "Activity Bar", &activity_bar, true))
                    .child(dropdown_menu_checkbox_item(c, "Panel", &panel, false)),
            )
            .child(dropdown_menu_separator())
            .child(dropdown_menu_group().child(with_icon(c, Icon::LogOut, "Sign Out")));

        let r = &radio;
        let radio_menu = dropdown_menu_content(r)
            .w(56.0)
            .child(dropdown_menu_label("Panel Position", true))
            .child(
                dropdown_menu_group().child(
                    dropdown_menu_radio_group()
                        .child(dropdown_menu_radio_item(r, "Top", "top", &position, false))
                        .child(dropdown_menu_radio_item(r, "Bottom", "bottom", &position, false))
                        .child(dropdown_menu_radio_item(r, "Right", "right", &position, true)),
                ),
            );

        let open = |name: &str| button(ButtonVariant::Outline, ButtonSize::Default).child(name.to_owned());
        page(
            "Dropdown Menu",
            div()
                .flex_col()
                .gap(6.0)
                .child(demo(
                    "DropdownMenuDemo.vue",
                    dropdown_menu_trigger(&account, open("Open"), Side::Bottom, SideAlign::Start, account_menu),
                ))
                .child(section(
                    "Checkboxes, radio group",
                    div()
                        .flex()
                        .flex_wrap()
                        .items_start()
                        .gap(4.0)
                        .child(dropdown_menu_trigger(
                            &checkboxes,
                            open("Checkboxes"),
                            Side::Bottom,
                            SideAlign::Start,
                            checkbox_menu,
                        ))
                        .child(dropdown_menu_trigger(
                            &radio,
                            open("Radio Group"),
                            Side::Bottom,
                            SideAlign::Start,
                            radio_menu,
                        )),
                ))
                .child(
                    text(format!(
                        "Last action: {}. Position: {}. Panel: {}.",
                        last.get(),
                        position.get(),
                        panel.get()
                    ))
                    .text_sm()
                    .text_color(t.muted_foreground),
                ),
        )
    })
}

fn key(s: &mut Shot, k: NamedKey) {
    s.press(k, Modifiers::default());
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    motion_frames(s, "opening", &MOTION_MS, |s| s.click_text("Open"))?;
    motion_frames(s, "closing", &MOTION_MS, |s| {
        s.press(NamedKey::Escape, Modifiers::default());
        Ok(())
    })?;
    s.click_text("Open")?;
    s.frame("open")?;
    let (x, y) = s.locate("Billing")?;
    s.hover(x, y);
    s.frame("hover billing")?;
    key(s, NamedKey::ArrowDown);
    key(s, NamedKey::ArrowDown);
    s.frame("arrow down twice")?;
    let (x, y) = s.locate("Invite users")?;
    s.hover(x, y);
    s.frame("submenu open")?;
    key(s, NamedKey::ArrowRight);
    key(s, NamedKey::ArrowDown);
    s.frame("keyboard in submenu")?;
    key(s, NamedKey::ArrowLeft);
    s.frame("back to root")?;
    key(s, NamedKey::Escape);
    s.frame("escape closes")?;
    key(s, NamedKey::Enter);
    s.frame("enter opens on first item")?;
    key(s, NamedKey::End);
    key(s, NamedKey::Enter);
    s.frame("enter selects log out")?;
    s.click_text("Checkboxes")?;
    s.settle()?;
    s.click_text("Panel")?;
    s.settle()?;
    s.click_text("Checkboxes")?;
    s.settle()?;
    let (x, y) = s.locate("Status Bar")?;
    s.hover(x, y);
    s.frame("checkboxes")?;
    s.click(900.0, 600.0);
    s.settle()?;
    s.click_text("Radio Group")?;
    s.settle()?;
    s.click_text("Top")?;
    s.settle()?;
    s.click_text("Radio Group")?;
    s.frame("radio group")?;
    s.click(900.0, 600.0);
    s.frame("click outside closes")?;
    Ok(())
}
