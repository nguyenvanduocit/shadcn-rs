use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, Signal, div, icon, text};
use shadcn::{
    ContextMenuState, MenuItem, MenuState, context_menu_checkbox_item, context_menu_content, context_menu_item,
    context_menu_label, context_menu_radio_group, context_menu_radio_item, context_menu_separator,
    context_menu_shortcut, context_menu_sub_content, context_menu_sub_trigger, context_menu_trigger, theme,
};

use crate::{Render, Shot, demo, page};

pub fn render() -> Render {
    let menu = ContextMenuState::new();
    let tools = menu.sub();
    let last = Signal::labeled("last action", String::from("nothing yet"));
    let bookmarks = Signal::labeled("show bookmarks", true);
    let full_urls = Signal::labeled("show full urls", false);
    let person = Signal::labeled("person", String::from("pedro"));
    Box::new(move || {
        let t = theme();
        let act = |name: &'static str| {
            let last = last.clone();
            move || last.set(name.to_owned())
        };
        let item =
            |m: &MenuState, name: &'static str, opts: MenuItem| context_menu_item(m, name, opts, act(name)).child(name);
        let with_icon = |m: &MenuState, glyph: Icon, name: &'static str, opts: MenuItem| {
            let muted = opts.variant == shadcn::MenuItemVariant::Default;
            context_menu_item(m, name, opts, act(name))
                .child(icon(glyph).when(muted, |i| i.text_color(t.muted_foreground)))
                .child(name)
        };

        let content = context_menu_content(&menu)
            .w(52.0)
            .child(item(&menu, "Back", MenuItem::INSET).child(context_menu_shortcut("⌘[")))
            .child(
                item(
                    &menu,
                    "Forward",
                    MenuItem {
                        inset: true,
                        ..MenuItem::DISABLED
                    },
                )
                .child(context_menu_shortcut("⌘]")),
            )
            .child(item(&menu, "Reload", MenuItem::INSET).child(context_menu_shortcut("⌘R")))
            .child(context_menu_sub_trigger(
                &tools,
                "More Tools",
                MenuItem::INSET,
                context_menu_sub_content(&tools)
                    .w(44.0)
                    .child(item(&tools, "Save Page...", MenuItem::INSET).child(context_menu_shortcut("⇧⌘S")))
                    .child(with_icon(&tools, Icon::Plus, "Create Shortcut...", MenuItem::default()))
                    .child(item(&tools, "Name Window...", MenuItem::INSET))
                    .child(context_menu_separator())
                    .child(with_icon(&tools, Icon::Code2, "Developer Tools", MenuItem::default()))
                    .child(context_menu_separator())
                    .child(with_icon(&tools, Icon::Trash, "Delete", MenuItem::DESTRUCTIVE)),
            ))
            .child(context_menu_separator())
            .child(
                context_menu_checkbox_item(&menu, "Show Bookmarks", &bookmarks, false)
                    .child(context_menu_shortcut("⌘⇧B")),
            )
            .child(context_menu_checkbox_item(&menu, "Show Full URLs", &full_urls, false))
            .child(context_menu_separator())
            .child(
                context_menu_radio_group()
                    .child(context_menu_label("People", true))
                    .child(context_menu_radio_item(&menu, "Pedro Duarte", "pedro", &person, false))
                    .child(context_menu_radio_item(&menu, "Colm Tuite", "colm", &person, false)),
            );

        let area = div()
            .flex()
            .h_px(150.0)
            .w_px(300.0)
            .items_center()
            .justify_center()
            .rounded_md()
            // `border-dashed`: gui strokes borders solid only.
            .border()
            .border_color(t.border)
            .text_sm()
            .child("Right click here");
        page(
            "Context Menu",
            div()
                .flex_col()
                .gap(6.0)
                .child(demo("ContextMenuDemo.vue", context_menu_trigger(&menu, area, content)))
                .child(
                    text(format!(
                        "Last action: {}. Bookmarks: {}. Person: {}.",
                        last.get(),
                        bookmarks.get(),
                        person.get()
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
    let (ax, ay) = s.locate("Right click here")?;
    s.secondary_click(ax - 100.0, ay - 40.0);
    s.frame("open at pointer")?;
    let (x, y) = s.locate("Reload")?;
    s.hover(x, y);
    s.frame("hover reload")?;
    key(s, NamedKey::ArrowDown);
    key(s, NamedKey::ArrowRight);
    s.frame("submenu by keyboard")?;
    key(s, NamedKey::End);
    s.frame("destructive highlighted")?;
    key(s, NamedKey::Escape);
    s.frame("escape closes")?;
    s.secondary_click(ax + 60.0, ay + 20.0);
    s.settle()?;
    s.click_text("Show Full URLs")?;
    s.settle()?;
    s.secondary_click(ax + 60.0, ay + 20.0);
    s.settle()?;
    s.click_text("Colm Tuite")?;
    s.settle()?;
    s.secondary_click(ax + 60.0, ay + 20.0);
    s.settle()?;
    let (hx, hy) = s.locate("Colm Tuite")?;
    s.hover(hx, hy);
    s.frame("checkbox and radio picked")?;
    s.click(ax - 120.0, ay + 60.0);
    s.frame("left click in the area closes")?;
    Ok(())
}
