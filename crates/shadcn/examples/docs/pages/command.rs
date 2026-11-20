use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, Signal, div, icon, text};
use shadcn::{
    CommandState, command, command_empty, command_group, command_input, command_item, command_list, command_separator,
    command_shortcut, theme,
};

use crate::{Render, Shot, demo, page};

pub fn render() -> Render {
    let cmd = CommandState::new();
    let last = Signal::labeled("last command", String::from("nothing yet"));
    Box::new(move || {
        let t = theme();
        let item = |value: &'static str, glyph: Icon, label: &'static str, disabled: bool| {
            let last = last.clone();
            command_item(&cmd, value, disabled, move || last.set(label.to_owned()))
                .map(|i| i.child(icon(glyph).text_color(t.muted_foreground)).child(text(label)))
        };
        let with_shortcut =
            |value, glyph, label, keys: &str| item(value, glyph, label, false).map(|i| i.child(command_shortcut(keys)));
        let palette = command(&cmd)
            .rounded_lg()
            .border()
            .border_color(t.border)
            .shadow_md()
            .min_w_px(450.0)
            .child(command_input(&cmd, "Type a command or search..."))
            .child(
                command_list()
                    .child(command_group(
                        Some("Suggestions"),
                        [
                            item("calendar", Icon::Calendar, "Calendar", false),
                            item("emoji", Icon::Smile, "Search Emoji", false),
                            item("calculator", Icon::Calculator, "Calculator", true),
                        ],
                    ))
                    .child(command_separator())
                    .child(command_group(
                        Some("Settings"),
                        [
                            with_shortcut("profile", Icon::User, "Profile", "⌘P"),
                            with_shortcut("billing", Icon::CreditCard, "Billing", "⌘B"),
                            with_shortcut("settings", Icon::Settings, "Settings", "⌘S"),
                        ],
                    ))
                    .child(command_empty(&cmd, "No results found.")),
            );
        page(
            "Command",
            div().flex_col().gap(6.0).child(demo("CommandDemo.vue", palette)).child(
                text(format!("Last command: {}.", last.get()))
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
    let (x, y) = s.locate("Search Emoji")?;
    s.hover(x, y);
    s.frame("hover emoji")?;
    let (x, y) = s.locate("Type a command or search...")?;
    s.click(x, y);
    s.type_text("s");
    s.frame("typed s highlights first")?;
    key(s, NamedKey::ArrowDown);
    key(s, NamedKey::ArrowDown);
    s.frame("arrow down twice")?;
    s.type_text("ett");
    s.frame("typed sett")?;
    s.type_text("x");
    s.frame("no results")?;
    key(s, NamedKey::Backspace);
    key(s, NamedKey::Backspace);
    key(s, NamedKey::Backspace);
    key(s, NamedKey::Backspace);
    key(s, NamedKey::Backspace);
    s.settle()?;
    key(s, NamedKey::ArrowDown);
    key(s, NamedKey::ArrowDown);
    s.frame("cleared, down to profile")?;
    key(s, NamedKey::Enter);
    s.frame("enter picks")?;
    Ok(())
}
