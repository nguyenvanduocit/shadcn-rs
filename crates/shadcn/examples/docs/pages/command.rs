use std::rc::Rc;

use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, Signal, div, icon, text, text_bound};
use shadcn::{CommandItem, CommandSection, command, command_input, command_list, command_shortcut, theme};

use crate::{Render, Shot, demo, page};

/// The palette's items: value, icon, label, shortcut.
const SUGGESTIONS: [(&str, Icon, &str, Option<&str>); 3] = [
    ("calendar", Icon::Calendar, "Calendar", None),
    ("emoji", Icon::Smile, "Search Emoji", None),
    ("calculator", Icon::Calculator, "Calculator", None),
];
const SETTINGS: [(&str, Icon, &str, Option<&str>); 3] = [
    ("profile", Icon::User, "Profile", Some("⌘P")),
    ("billing", Icon::CreditCard, "Billing", Some("⌘B")),
    ("settings", Icon::Settings, "Settings", Some("⌘S")),
];

fn find(value: &str) -> Option<&'static (&'static str, Icon, &'static str, Option<&'static str>)> {
    SUGGESTIONS.iter().chain(SETTINGS.iter()).find(|e| e.0 == value)
}

pub fn render() -> Render {
    let last = Signal::labeled("last command", String::from("nothing yet"));
    let sections = Rc::new(vec![
        CommandSection::group(
            Some("Suggestions"),
            SUGGESTIONS.iter().map(|e| {
                if e.0 == "calculator" {
                    CommandItem::disabled(e.0)
                } else {
                    CommandItem::new(e.0)
                }
            }),
        ),
        CommandSection::Separator,
        CommandSection::group(Some("Settings"), SETTINGS.iter().map(|e| CommandItem::new(e.0))),
        CommandSection::empty("No results found."),
    ]);
    Box::new(move || {
        let t = theme();
        let palette = command()
            .radius(10.0)
            .border(1.0)
            .border_color(t.border)
            .shadow([
                Shadow {
                    x: 0.0,
                    y: 4.0,
                    blur: 6.0,
                    spread: -1.0,
                    color: gui::Color::BLACK.opacity(0.1),
                },
                Shadow {
                    x: 0.0,
                    y: 2.0,
                    blur: 4.0,
                    spread: -2.0,
                    color: gui::Color::BLACK.opacity(0.1),
                },
            ])
            .min_width(450.0)
            .child(command_input("Type a command or search..."))
            .child(
                command_list(Rc::clone(&sections), move |item, row| match find(&item.value) {
                    Some(&(_, glyph, label, keys)) => row
                        .child(icon(glyph).text_color(t.muted_foreground))
                        .child(text(label))
                        .child(keys.map(command_shortcut)),
                    None => row,
                })
                .on_select(move |item| {
                    if let Some(e) = find(&item.value) {
                        last.set(e.2.to_owned());
                    }
                }),
            );
        page(
            "Command",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(24.0)
                .child(demo("CommandDemo.vue", palette))
                .child(
                    text_bound(move || format!("Last command: {}.", last.get()))
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
