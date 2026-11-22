use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, Signal, div, text_bound};
use shadcn::{
    MenuItem, menubar, menubar_checkbox_item, menubar_content, menubar_item, menubar_menu, menubar_radio_group,
    menubar_radio_item, menubar_separator, menubar_shortcut, menubar_sub, menubar_sub_content, menubar_sub_trigger,
    menubar_trigger, theme,
};

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page};

pub fn render() -> Render {
    let last = Signal::labeled("last action", String::from("nothing yet"));
    let bookmarks = Signal::labeled("bookmarks bar", false);
    let full_urls = Signal::labeled("full urls", true);
    let profile = Signal::labeled("profile", String::from("benoit"));
    Box::new(move || {
        let t = theme();
        let act = |name: &'static str| move || last.set(name.to_owned());
        let item = |name: &'static str, opts: MenuItem| menubar_item(name, opts).on_select(act(name)).child(name);
        let short = |name: &'static str, opts: MenuItem, keys: &str| item(name, opts).child(menubar_shortcut(keys));
        let plain = MenuItem::default();

        let file_menu = menubar_content()
            .child(short("New Tab", plain, "⌘T"))
            .child(short("New Window", plain, "⌘N"))
            .child(item("New Incognito Window", MenuItem::DISABLED))
            .child(menubar_separator())
            .child(
                menubar_sub().child(menubar_sub_trigger("Share", plain)).child(
                    menubar_sub_content()
                        .child(item("Email link", plain))
                        .child(item("Messages", plain))
                        .child(item("Notes", plain)),
                ),
            )
            .child(menubar_separator())
            .child(short("Print...", plain, "⌘P"));

        let edit_menu = menubar_content()
            .child(short("Undo", plain, "⌘Z"))
            .child(short("Redo", plain, "⇧⌘Z"))
            .child(menubar_separator())
            .child(
                menubar_sub().child(menubar_sub_trigger("Find", plain)).child(
                    menubar_sub_content()
                        .child(item("Search the web", plain))
                        .child(menubar_separator())
                        .child(item("Find...", plain))
                        .child(item("Find Next", plain))
                        .child(item("Find Previous", plain)),
                ),
            )
            .child(menubar_separator())
            .child(item("Cut", plain))
            .child(item("Copy", plain))
            .child(item("Paste", plain));

        let view_menu = menubar_content()
            .child(menubar_checkbox_item("Always Show Bookmarks Bar", false).model(bookmarks))
            .child(menubar_checkbox_item("Always Show Full URLs", false).model(full_urls))
            .child(menubar_separator())
            .child(short("Reload", MenuItem::INSET, "⌘R"))
            .child(short(
                "Force Reload",
                MenuItem {
                    inset: true,
                    ..MenuItem::DISABLED
                },
                "⇧⌘R",
            ))
            .child(menubar_separator())
            .child(item("Toggle Fullscreen", MenuItem::INSET))
            .child(menubar_separator())
            .child(item("Hide Sidebar", MenuItem::INSET));

        let profiles_menu = menubar_content()
            .child(
                menubar_radio_group()
                    .model(profile)
                    .child(menubar_radio_item("Andy", "andy", false))
                    .child(menubar_radio_item("Benoit", "benoit", false))
                    .child(menubar_radio_item("Luis", "Luis", false)),
            )
            .child(menubar_separator())
            .child(item("Edit...", MenuItem::INSET))
            .child(menubar_separator())
            .child(item("Add Profile...", MenuItem::INSET));

        page(
            "Menubar",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(24.0)
                .child(demo(
                    "MenubarDemo.vue",
                    menubar()
                        .child(menubar_menu().child(menubar_trigger("File")).child(file_menu))
                        .child(menubar_menu().child(menubar_trigger("Edit")).child(edit_menu))
                        .child(menubar_menu().child(menubar_trigger("View")).child(view_menu))
                        .child(menubar_menu().child(menubar_trigger("Profiles")).child(profiles_menu)),
                ))
                .child(
                    text_bound(move || {
                        format!(
                            "Last action: {}. Bookmarks bar: {}. Profile: {}.",
                            last.get(),
                            bookmarks.get(),
                            profile.get()
                        )
                    })
                    .text_size(14.0)
                    .line_height(20.0)
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
    motion_frames(s, "opening", &MOTION_MS, |s| s.click_text("File"))?;
    motion_frames(s, "closing", &MOTION_MS, |s| {
        s.press(NamedKey::Escape, Modifiers::default());
        Ok(())
    })?;
    s.click_text("File")?;
    s.frame("file open")?;
    let (x, y) = s.locate("Share")?;
    s.hover(x, y);
    s.frame("share submenu")?;
    let (x, y) = s.locate("Edit")?;
    s.hover(x, y);
    s.frame("hover switches to edit")?;
    key(s, NamedKey::ArrowDown);
    key(s, NamedKey::ArrowDown);
    s.frame("arrow down in edit")?;
    key(s, NamedKey::ArrowRight);
    s.frame("arrow right opens view")?;
    key(s, NamedKey::Enter);
    s.settle()?;
    s.click_text("View")?;
    s.frame("bookmarks bar checked")?;
    key(s, NamedKey::ArrowRight);
    s.frame("profiles")?;
    key(s, NamedKey::Escape);
    s.frame("escape closes, trigger focused")?;
    Ok(())
}
