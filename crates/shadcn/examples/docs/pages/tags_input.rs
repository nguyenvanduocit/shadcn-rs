use gui::prelude::*;
use gui::{Effect, Icon, Modifiers, NamedKey, ScenarioError, div, icon};
use shadcn::{
    ButtonSize, ButtonVariant, CommandState, PopoverState, TagsInputState, button, command, command_item, popover,
    popover_content, popover_trigger, tags_input, tags_input_input, tags_input_item, tags_input_item_delete,
    tags_input_item_text, theme,
};

use crate::{Render, Shot, demo, page, section};

/// TagsInputWithListbox's frameworks: the listbox's options are their labels.
const FRAMEWORKS: [&str; 5] = ["Next.js", "SvelteKit", "Nuxt", "Remix", "Astro"];

fn tags(state: &TagsInputState, placeholder: &str, invalid: bool) -> gui::Div {
    tags_input(state, invalid)
        .w_px(300.0)
        .children(state.tags.get().iter().map(|tag| tag_item(state, tag)))
        .child(tags_input_input(state).placeholder(placeholder))
}

fn tag_item(state: &TagsInputState, tag: &str) -> gui::Div {
    tags_input_item()
        .key(gui::key_of(tag))
        .child(tags_input_item_text(tag.to_owned()))
        .child(tags_input_item_delete(state, tag))
}

pub fn render() -> Render {
    let fruits = TagsInputState::new(["Apple", "Banana"]);
    let invalid = TagsInputState::new(["shadcn"]);
    // TagsInputWithListbox: the tags and the listbox share one model, and the
    // text typed in the tags field is the listbox's search (`ListboxFilter
    // as-child`), so the command's search field is the tags' field.
    let listbox = CommandState::new();
    let frameworks = TagsInputState {
        tags: gui::Signal::new(vec![String::from("Nuxt"), String::from("Remix")]),
        field: listbox.search.clone(),
    };
    let open = PopoverState::without_auto_focus();
    // `watch(searchTerm, f => { if (f) open = true })`.
    let typed = (listbox.search.clone(), open.clone());
    let open_on_search = Effect::new(move || {
        if !typed.0.text().is_empty() && !typed.1.is_open() {
            typed.1.open();
        }
    });
    Box::new(move || {
        let _keep = &open_on_search;
        let t = theme();
        let chosen = frameworks.tags.get();
        // The command's root first: it starts the frame its items register in.
        // `ListboxContent max-h-[300px] overflow-y-auto`, as a command lists its items.
        let list = command(&listbox).max_h_px(300.0).overflow_y_scroll();
        let options = FRAMEWORKS.map(|label| {
            let toggle = frameworks.tags.clone();
            command_item(&listbox, label, false, move || {
                toggle.update(|mut tags| {
                    if tags.iter().any(|t| t == label) {
                        tags.retain(|t| t != label);
                    } else {
                        tags.push(label.to_owned());
                    }
                    tags
                });
            })
            .map(|i| {
                i.child(label).child(chosen.iter().any(|c| c == label).then(|| {
                    div()
                        .ml_auto()
                        .inline_flex()
                        .items_center()
                        .justify_center()
                        .child(icon(Icon::Check).text_color(t.muted_foreground))
                }))
            })
        });
        let (keys, down) = (listbox.clone(), open.clone());
        let filter = tags_input_input(&frameworks)
            .placeholder("Frameworks...")
            .on_key_down(move |ev| {
                if !down.is_open() {
                    if ev.is(NamedKey::ArrowDown) {
                        down.open();
                        return true;
                    }
                    // `@keydown.enter.prevent`: Enter never adds the typed text as a tag.
                    return ev.is(NamedKey::Enter);
                }
                keys.key(ev) || ev.is(NamedKey::Enter)
            });
        let with_listbox = popover()
            .w_px(300.0)
            .child(
                tags_input(&frameworks, false)
                    .w_full()
                    .children(chosen.iter().map(|tag| tag_item(&frameworks, tag)))
                    .child(filter)
                    .child(popover_trigger(
                        &open,
                        // `order-last self-start ml-auto`.
                        button(ButtonVariant::Ghost, ButtonSize::IconSm)
                            .self_start()
                            .ml_auto()
                            .child(icon(Icon::ChevronDown).size(3.5)),
                    )),
            )
            .child(popover_content(&open, |c| {
                c.p(1.0).child(list.children(options.into_iter().flatten()))
            }));
        page(
            "Tags Input",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo("TagsInputDemo.vue", tags(&fruits, "Fruits...", false)))
                .child(demo("TagsInputWithListbox.vue", with_listbox))
                .child(section("Invalid", tags(&invalid, "Labels...", true))),
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
