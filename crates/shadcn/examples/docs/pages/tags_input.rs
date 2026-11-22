use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, Signal, div, icon};
use shadcn::{
    ButtonSize, ButtonVariant, CommandItem, CommandSection, Part, TagsInputProps, button, command, command_list,
    popover, popover_content, popover_trigger, tags_input, tags_input_input, tags_input_item, tags_input_item_delete,
    tags_input_item_text, theme,
};

use crate::{Render, Shot, demo, page, section};

/// TagsInputWithListbox's frameworks: the listbox's options are their labels.
const FRAMEWORKS: [&str; 5] = ["Next.js", "SvelteKit", "Nuxt", "Remix", "Astro"];

fn tags(model: Signal<Vec<String>>, placeholder: &str, invalid: bool) -> Part<TagsInputProps> {
    tags_input()
        .model(model)
        .invalid(invalid)
        .width(300.0)
        .children(model.get().iter().map(|tag| tag_item(tag)))
        .child(tags_input_input().placeholder(placeholder))
}

fn tag_item(tag: &str) -> gui::Div {
    tags_input_item()
        .key(gui::key_of(tag))
        .child(tags_input_item_text(tag.to_owned()))
        .child(tags_input_item_delete(tag))
}

pub fn render() -> Render {
    let fruits = Signal::labeled("fruits", vec![String::from("Apple"), String::from("Banana")]);
    let invalid = Signal::labeled("labels", vec![String::from("shadcn")]);
    // TagsInputWithListbox: the tags and the listbox share one model, and the
    // text typed in the tags field is the listbox's search (`ListboxFilter
    // v-model="searchTerm" as-child`).
    let frameworks = Signal::labeled("frameworks", vec![String::from("Nuxt"), String::from("Remix")]);
    let search = Signal::labeled("search", String::new());
    let open = Signal::labeled("open", false);
    // `watch(searchTerm, f => { if (f) open = true })`.
    gui::watch(
        move || search.get(),
        move |typed, _| {
            if !typed.is_empty() {
                open.set(true);
            }
        },
    );
    // `ListboxContent max-h-[300px] overflow-y-auto`: the options straight in the list.
    let options = std::rc::Rc::new(vec![CommandSection::Items(
        FRAMEWORKS.iter().map(|l| CommandItem::new(*l)).collect(),
    )]);
    Box::new(move || {
        let t = theme();
        let list = command_list(std::rc::Rc::clone(&options), move |item, row| {
            let label = item.value.clone();
            let on = frameworks.with(|f| f.contains(&label));
            row.child(label).child(on.then(|| {
                div()
                    .margin_left(Length::Auto)
                    .display(Display::Flex)
                    .inline(true)
                    .align_items(Align::Center)
                    .justify_content(Justify::Center)
                    .child(icon(Icon::Check).text_color(t.muted_foreground))
            }))
        })
        .on_select(move |item| {
            let label = item.value.clone();
            frameworks.update(|tags| {
                if tags.contains(&label) {
                    tags.retain(|t| *t != label);
                } else {
                    tags.push(label.clone());
                }
            });
        })
        .max_h_px(300.0);
        // `@keydown.down="open = true"`; the other keys go on to the listbox.
        let filter = tags_input_input().placeholder("Frameworks...").on_key_down(move |ev| {
            let opens = !open.get_untracked() && ev.is(NamedKey::ArrowDown);
            if opens {
                open.set(true);
            }
            opens
        });
        let with_listbox = popover().open(open).width(300.0).child(
            command()
                .search(search)
                .child(
                    tags_input()
                        .model(frameworks)
                        .text(search)
                        // `@keydown.enter.prevent`: Enter picks from the listbox, never adds the typed text.
                        .add_on_enter(false)
                        .width(pct(100.0))
                        .children(frameworks.get().iter().map(|tag| tag_item(tag)))
                        .child(filter)
                        .child(popover_trigger(
                            // `order-last self-start ml-auto`.
                            button(ButtonVariant::Ghost, ButtonSize::IconSm)
                                .align_self(Align::Start)
                                .margin_left(Length::Auto)
                                .child(icon(Icon::ChevronDown).size(14.0)),
                        )),
                )
                .child(popover_content().open_auto_focus(false).padding(4.0).child(list)),
        );
        page(
            "Tags Input",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo("TagsInputDemo.vue", tags(fruits, "Fruits...", false)))
                .child(demo("TagsInputWithListbox.vue", with_listbox))
                .child(section("Invalid", tags(invalid, "Labels...", true))),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Fruits...")?;
    s.click(x, y);
    s.frame("focused")?;
    s.type_text("Cherry");
    s.press(NamedKey::Enter, Modifiers::default());
    s.frame("enter adds cherry")?;
    s.type_text("Apple");
    s.press(NamedKey::Enter, Modifiers::default());
    s.frame("duplicate apple refused")?;
    let delete = s
        .last()
        .nodes
        .iter()
        .find(|n| n.name == Some("tags-input-item-delete"))
        .map(|n| n.bounds);
    let Some(b) = delete else {
        return Err(ScenarioError::TextNotFound("tags-input-item-delete".to_owned()));
    };
    s.click(b.x + b.width / 2.0, b.y + b.height / 2.0);
    s.frame("x removes apple")?;
    // The listbox: typing opens it filtered, Enter toggles the highlighted
    // option into the tags and clears the search, the list stays open.
    let (x, y) = s.locate("Frameworks...")?;
    s.click(x, y);
    s.type_text("ast");
    s.frame("typed ast: astro highlighted")?;
    s.press(NamedKey::Enter, Modifiers::default());
    s.frame("enter adds astro, list stays open")?;
    let remix = s
        .last()
        .nodes
        .iter()
        .filter(|n| n.name == Some("command-item"))
        .nth(3)
        .map(|n| n.bounds)
        .ok_or_else(|| ScenarioError::TextNotFound("remix option".to_owned()))?;
    s.click(remix.x + remix.width / 2.0, remix.y + remix.height / 2.0);
    s.frame("clicking the remix option removes its tag")?;
    Ok(())
}
