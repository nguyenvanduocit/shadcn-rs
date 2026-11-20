use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{
    MenuItem, MenuState, MenubarState, menubar, menubar_checkbox_item, menubar_content, menubar_item,
    menubar_radio_group, menubar_radio_item, menubar_separator, menubar_shortcut, menubar_sub_content,
    menubar_sub_trigger, menubar_trigger, theme,
};

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page};

pub fn render() -> Render {
    let bar = MenubarState::new();
    let file = bar.menu();
    let share = file.sub();
    let edit = bar.menu();
    let find = edit.sub();
    let view = bar.menu();
    let profiles = bar.menu();
    let last = Signal::labeled("last action", String::from("nothing yet"));
    let bookmarks = Signal::labeled("bookmarks bar", false);
    let full_urls = Signal::labeled("full urls", true);
    let profile = Signal::labeled("profile", String::from("benoit"));
    Box::new(move || {
        let t = theme();
        let act = |name: &'static str| {
            let last = last.clone();
            move || last.set(name.to_owned())
        };
        let item =
            |m: &MenuState, name: &'static str, opts: MenuItem| menubar_item(m, name, opts, act(name)).child(name);
        let short = |m: &MenuState, name: &'static str, opts: MenuItem, keys: &str| {
            item(m, name, opts).child(menubar_shortcut(keys))
        };
        let plain = MenuItem::default();

        let file_menu = menubar_content(&file)
            .child(short(&file, "New Tab", plain, "⌘T"))
            .child(short(&file, "New Window", plain, "⌘N"))
            .child(item(&file, "New Incognito Window", MenuItem::DISABLED))
            .child(menubar_separator())
            .child(menubar_sub_trigger(
                &share,
                "Share",
                plain,
                menubar_sub_content(&share)
                    .child(item(&share, "Email link", plain))
                    .child(item(&share, "Messages", plain))
                    .child(item(&share, "Notes", plain)),
            ))
            .child(menubar_separator())
            .child(short(&file, "Print...", plain, "⌘P"));

        let edit_menu = menubar_content(&edit)
            .child(short(&edit, "Undo", plain, "⌘Z"))
            .child(short(&edit, "Redo", plain, "⇧⌘Z"))
            .child(menubar_separator())
            .child(menubar_sub_trigger(
                &find,
                "Find",
                plain,
                menubar_sub_content(&find)
                    .child(item(&find, "Search the web", plain))
                    .child(menubar_separator())
                    .child(item(&find, "Find...", plain))
                    .child(item(&find, "Find Next", plain))
                    .child(item(&find, "Find Previous", plain)),
            ))
            .child(menubar_separator())
            .child(item(&edit, "Cut", plain))
            .child(item(&edit, "Copy", plain))
            .child(item(&edit, "Paste", plain));

        let view_menu = menubar_content(&view)
            .child(menubar_checkbox_item(
                &view,
                "Always Show Bookmarks Bar",
                &bookmarks,
                false,
            ))
            .child(menubar_checkbox_item(&view, "Always Show Full URLs", &full_urls, false))
            .child(menubar_separator())
            .child(short(&view, "Reload", MenuItem::INSET, "⌘R"))
            .child(short(
                &view,
                "Force Reload",
                MenuItem {
                    inset: true,
                    ..MenuItem::DISABLED
                },
                "⇧⌘R",
            ))
            .child(menubar_separator())
            .child(item(&view, "Toggle Fullscreen", MenuItem::INSET))
            .child(menubar_separator())
            .child(item(&view, "Hide Sidebar", MenuItem::INSET));

        let profiles_menu = menubar_content(&profiles)
            .child(
                menubar_radio_group()
                    .child(menubar_radio_item(&profiles, "Andy", "andy", &profile, false))
                    .child(menubar_radio_item(&profiles, "Benoit", "benoit", &profile, false))
                    .child(menubar_radio_item(&profiles, "Luis", "Luis", &profile, false)),
            )
            .child(menubar_separator())
            .child(item(&profiles, "Edit...", MenuItem::INSET))
            .child(menubar_separator())
            .child(item(&profiles, "Add Profile...", MenuItem::INSET));

        page(
            "Menubar",
            div()
                .flex_col()
                .gap(6.0)
                .child(demo(
                    "MenubarDemo.vue",
                    menubar(&bar)
                        .child(menubar_trigger(&bar, &file, "File", file_menu))
                        .child(menubar_trigger(&bar, &edit, "Edit", edit_menu))
                        .child(menubar_trigger(&bar, &view, "View", view_menu))
                        .child(menubar_trigger(&bar, &profiles, "Profiles", profiles_menu)),
                ))
                .child(
                    text(format!(
                        "Last action: {}. Bookmarks bar: {}. Profile: {}.",
                        last.get(),
                        bookmarks.get(),
                        profile.get()
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
