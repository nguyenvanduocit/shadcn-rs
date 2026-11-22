use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, SideAlign, Signal, div, icon, text_bound};
use shadcn::{
    ButtonSize, ButtonVariant, MenuItem, button, dropdown_menu, dropdown_menu_checkbox_item, dropdown_menu_content,
    dropdown_menu_group, dropdown_menu_item, dropdown_menu_label, dropdown_menu_radio_group, dropdown_menu_radio_item,
    dropdown_menu_separator, dropdown_menu_shortcut, dropdown_menu_sub, dropdown_menu_sub_content,
    dropdown_menu_sub_trigger, dropdown_menu_trigger, theme,
};

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page, section};

pub fn render() -> Render {
    let last = Signal::labeled("last action", String::from("nothing yet"));
    let status_bar = Signal::labeled("status bar", true);
    let activity_bar = Signal::labeled("activity bar", false);
    let panel = Signal::labeled("panel", false);
    let position = Signal::labeled("position", String::from("bottom"));
    Box::new(move || {
        let t = theme();
        let item = |name: &'static str| {
            dropdown_menu_item(name, MenuItem::default())
                .on_select(move || last.set(name.to_owned()))
                .child(name)
        };
        let with_shortcut = |name: &'static str, keys: &str| item(name).child(dropdown_menu_shortcut(keys));
        let with_icon = |glyph: Icon, name: &'static str| {
            dropdown_menu_item(name, MenuItem::default())
                .on_select(move || last.set(name.to_owned()))
                .child(icon(glyph).text_color(t.muted_foreground))
                .child(name)
        };
        let open = |name: &str| button(ButtonVariant::Outline, ButtonSize::Default).child(name.to_owned());

        let account_menu = dropdown_menu().child(dropdown_menu_trigger(open("Open"))).child(
            dropdown_menu_content()
                .align(SideAlign::Start)
                .w(56.0)
                .child(dropdown_menu_label("My Account", false))
                .child(
                    dropdown_menu_group()
                        .child(with_shortcut("Profile", "⇧⌘P"))
                        .child(with_shortcut("Billing", "⌘B"))
                        .child(with_shortcut("Settings", "⌘S"))
                        .child(with_shortcut("Keyboard shortcuts", "⌘K")),
                )
                .child(dropdown_menu_separator())
                .child(
                    dropdown_menu_group()
                        .child(item("Team"))
                        .child(
                            dropdown_menu_sub()
                                .child(dropdown_menu_sub_trigger("Invite users", MenuItem::default()))
                                .child(
                                    dropdown_menu_sub_content()
                                        .child(item("Email"))
                                        .child(item("Message"))
                                        .child(dropdown_menu_separator())
                                        .child(item("More...")),
                                ),
                        )
                        .child(with_shortcut("New Team", "⌘+T")),
                )
                .child(dropdown_menu_separator())
                .child(item("GitHub"))
                .child(item("Support"))
                .child(dropdown_menu_item("API", MenuItem::DISABLED).child("API"))
                .child(dropdown_menu_separator())
                .child(with_shortcut("Log out", "⇧⌘Q")),
        );

        let checkbox_menu = dropdown_menu().child(dropdown_menu_trigger(open("Checkboxes"))).child(
            dropdown_menu_content()
                .align(SideAlign::Start)
                .w(56.0)
                .child(
                    dropdown_menu_group()
                        .child(dropdown_menu_label("Account", false))
                        .child(with_icon(Icon::User, "Profile"))
                        .child(with_icon(Icon::CreditCard, "Billing"))
                        .child(with_icon(Icon::Settings, "Settings")),
                )
                .child(dropdown_menu_separator())
                .child(
                    dropdown_menu_group()
                        .child(dropdown_menu_label("Appearance", false))
                        .child(dropdown_menu_checkbox_item("Status Bar", false).model(status_bar))
                        .child(dropdown_menu_checkbox_item("Activity Bar", true).model(activity_bar))
                        .child(dropdown_menu_checkbox_item("Panel", false).model(panel)),
                )
                .child(dropdown_menu_separator())
                .child(dropdown_menu_group().child(with_icon(Icon::LogOut, "Sign Out"))),
        );

        let radio_menu = dropdown_menu().child(dropdown_menu_trigger(open("Radio Group"))).child(
            dropdown_menu_content()
                .align(SideAlign::Start)
                .w(56.0)
                .child(dropdown_menu_label("Panel Position", true))
                .child(
                    dropdown_menu_group().child(
                        dropdown_menu_radio_group()
                            .model(position)
                            .child(dropdown_menu_radio_item("Top", "top", false))
                            .child(dropdown_menu_radio_item("Bottom", "bottom", false))
                            .child(dropdown_menu_radio_item("Right", "right", true)),
                    ),
                ),
        );

        page(
            "Dropdown Menu",
            div()
                .flex_col()
                .gap(6.0)
                .child(demo("DropdownMenuDemo.vue", account_menu))
                .child(section(
                    "Checkboxes, radio group",
                    div()
                        .flex()
                        .flex_wrap()
                        .items_start()
                        .gap(4.0)
                        .child(checkbox_menu)
                        .child(radio_menu),
                ))
                .child(
                    text_bound(move || {
                        format!(
                            "Last action: {}. Position: {}. Panel: {}.",
                            last.get(),
                            position.get(),
                            panel.get()
                        )
                    })
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
