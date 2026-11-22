use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, Signal, div, icon, text_bound};
use shadcn::{
    MenuItem, context_menu, context_menu_checkbox_item, context_menu_content, context_menu_item, context_menu_label,
    context_menu_radio_group, context_menu_radio_item, context_menu_separator, context_menu_shortcut, context_menu_sub,
    context_menu_sub_content, context_menu_sub_trigger, context_menu_trigger, theme,
};

use crate::{Render, Shot, demo, page};

pub fn render() -> Render {
    let last = Signal::labeled("last action", String::from("nothing yet"));
    let bookmarks = Signal::labeled("show bookmarks", true);
    let full_urls = Signal::labeled("show full urls", false);
    let person = Signal::labeled("person", String::from("pedro"));
    Box::new(move || {
        let t = theme();
        let act = |name: &'static str| move || last.set(name.to_owned());
        let item = |name: &'static str, opts: MenuItem| context_menu_item(name, opts).on_select(act(name)).child(name);
        let with_icon = |glyph: Icon, name: &'static str, opts: MenuItem| {
            let muted = opts.variant == shadcn::MenuItemVariant::Default;
            context_menu_item(name, opts)
                .on_select(act(name))
                .child(icon(glyph).when(muted, |i| i.text_color(t.muted_foreground)))
                .child(name)
        };

        let content = context_menu_content()
            .w(52.0)
            .child(item("Back", MenuItem::INSET).child(context_menu_shortcut("⌘[")))
            .child(
                item(
                    "Forward",
                    MenuItem {
                        inset: true,
                        ..MenuItem::DISABLED
                    },
                )
                .child(context_menu_shortcut("⌘]")),
            )
            .child(item("Reload", MenuItem::INSET).child(context_menu_shortcut("⌘R")))
            .child(
                context_menu_sub()
                    .child(context_menu_sub_trigger("More Tools", MenuItem::INSET))
                    .child(
                        context_menu_sub_content()
                            .w(44.0)
                            .child(item("Save Page...", MenuItem::INSET).child(context_menu_shortcut("⇧⌘S")))
                            .child(with_icon(Icon::Plus, "Create Shortcut...", MenuItem::default()))
                            .child(item("Name Window...", MenuItem::INSET))
                            .child(context_menu_separator())
                            .child(with_icon(Icon::Code2, "Developer Tools", MenuItem::default()))
                            .child(context_menu_separator())
                            .child(with_icon(Icon::Trash, "Delete", MenuItem::DESTRUCTIVE)),
                    ),
            )
            .child(context_menu_separator())
            .child(
                context_menu_checkbox_item("Show Bookmarks", false)
                    .model(bookmarks)
                    .child(context_menu_shortcut("⌘⇧B")),
            )
            .child(context_menu_checkbox_item("Show Full URLs", false).model(full_urls))
            .child(context_menu_separator())
            .child(
                context_menu_radio_group()
                    .model(person)
                    .child(context_menu_label("People", true))
                    .child(context_menu_radio_item("Pedro Duarte", "pedro", false))
                    .child(context_menu_radio_item("Colm Tuite", "colm", false)),
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
                .child(demo(
                    "ContextMenuDemo.vue",
                    context_menu().child(context_menu_trigger(area)).child(content),
                ))
                .child(
                    text_bound(move || {
                        format!(
                            "Last action: {}. Bookmarks: {}. Person: {}.",
                            last.get(),
                            bookmarks.get(),
                            person.get()
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
