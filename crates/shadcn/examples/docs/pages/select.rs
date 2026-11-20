use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, div, text};
use shadcn::{
    SelectSize, SelectState, select_content, select_group, select_item, select_label, select_separator, select_trigger,
    theme,
};

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page, section};

const FRUITS: [(&str, &str); 5] = [
    ("apple", "Apple"),
    ("banana", "Banana"),
    ("blueberry", "Blueberry"),
    ("grapes", "Grapes"),
    ("pineapple", "Pineapple"),
];

/// SelectScrollable's regions and their time zones.
const ZONES: [(&str, &[(&str, &str)]); 5] = [
    (
        "North America",
        &[
            ("est", "Eastern Standard Time (EST)"),
            ("cst", "Central Standard Time (CST)"),
            ("mst", "Mountain Standard Time (MST)"),
            ("pst", "Pacific Standard Time (PST)"),
            ("akst", "Alaska Standard Time (AKST)"),
            ("hst", "Hawaii Standard Time (HST)"),
        ],
    ),
    (
        "Europe & Africa",
        &[
            ("gmt", "Greenwich Mean Time (GMT)"),
            ("cet", "Central European Time (CET)"),
            ("eet", "Eastern European Time (EET)"),
            ("west", "Western European Summer Time (WEST)"),
            ("cat", "Central Africa Time (CAT)"),
            ("eat", "East Africa Time (EAT)"),
        ],
    ),
    (
        "Asia",
        &[
            ("msk", "Moscow Time (MSK)"),
            ("ist", "India Standard Time (IST)"),
            ("cst_china", "China Standard Time (CST)"),
            ("jst", "Japan Standard Time (JST)"),
            ("kst", "Korea Standard Time (KST)"),
            ("ist_indonesia", "Indonesia Central Standard Time (WITA)"),
        ],
    ),
    (
        "Australia & Pacific",
        &[
            ("awst", "Australian Western Standard Time (AWST)"),
            ("acst", "Australian Central Standard Time (ACST)"),
            ("aest", "Australian Eastern Standard Time (AEST)"),
            ("nzst", "New Zealand Standard Time (NZST)"),
            ("fjt", "Fiji Time (FJT)"),
        ],
    ),
    (
        "South America",
        &[
            ("art", "Argentina Time (ART)"),
            ("bot", "Bolivia Time (BOT)"),
            ("brt", "Brasilia Time (BRT)"),
            ("clt", "Chile Standard Time (CLT)"),
        ],
    ),
];

pub fn render() -> Render {
    let fruit = SelectState::new(None);
    let small = SelectState::new(Some("est"));
    let fruits_many = SelectState::multiple(&[]);
    let zone = SelectState::new(None);
    Box::new(move || {
        let t = theme();
        let fruits = select_content(&fruit).child(
            select_group()
                .child(select_label("Fruits"))
                .children(FRUITS.map(|(v, l)| select_item(&fruit, v, l, false))),
        );
        let zones = select_content(&small)
            .child(
                select_group()
                    .child(select_label("North America"))
                    .child(select_item(&small, "est", "Eastern Standard Time (EST)", false))
                    .child(select_item(&small, "cst", "Central Standard Time (CST)", false))
                    .child(select_item(&small, "pst", "Pacific Standard Time (PST)", true)),
            )
            .child(select_separator())
            .child(
                select_group()
                    .child(select_label("Europe & Africa"))
                    .child(select_item(&small, "gmt", "Greenwich Mean Time (GMT)", false))
                    .child(select_item(&small, "cet", "Central European Time (CET)", false)),
            );
        page(
            "Select",
            div()
                .flex_col()
                .gap(6.0)
                .child(demo(
                    "SelectDemo.vue",
                    select_trigger(&fruit, "Select a fruit", SelectSize::Default, fruits).w_px(180.0),
                ))
                .child(demo(
                    "SelectMultipleDemo.vue",
                    select_trigger(
                        &fruits_many,
                        "Select a fruit",
                        SelectSize::Default,
                        select_content(&fruits_many).child(
                            select_group()
                                .child(select_label("Fruits"))
                                .children(FRUITS.map(|(v, l)| select_item(&fruits_many, v, l, false))),
                        ),
                    )
                    .w_px(180.0),
                ))
                .child(demo(
                    "SelectScrollable.vue",
                    select_trigger(
                        &zone,
                        "Select a timezone",
                        SelectSize::Default,
                        select_content(&zone).children(ZONES.map(|(region, zones)| {
                            select_group()
                                .child(select_label(region))
                                .children(zones.iter().map(|(v, l)| select_item(&zone, v, l, false)))
                        })),
                    )
                    .w_px(280.0),
                ))
                .child(section(
                    "Small, groups, separator, disabled item",
                    div()
                        .flex()
                        .child(select_trigger(&small, "Select a timezone", SelectSize::Sm, zones).w_px(280.0)),
                ))
                .child(
                    text(format!(
                        "Fruit: {}. Timezone: {}.",
                        fruit.value.get().unwrap_or_default(),
                        small.value.get().unwrap_or_default()
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
    motion_frames(s, "opening", &MOTION_MS, |s| s.click_text("Select a fruit"))?;
    motion_frames(s, "closing", &MOTION_MS, |s| {
        s.press(NamedKey::Escape, Modifiers::default());
        Ok(())
    })?;
    s.click_text("Select a fruit")?;
    s.frame("open")?;
    let (x, y) = s.locate("Banana")?;
    s.hover(x, y);
    s.frame("hover banana")?;
    key(s, NamedKey::ArrowDown);
    s.frame("arrow down")?;
    key(s, NamedKey::Enter);
    s.frame("enter picks blueberry")?;
    s.click_text("Blueberry")?;
    s.settle()?;
    s.frame("reopen highlights the chosen")?;
    s.type_text("p");
    s.frame("typeahead p")?;
    key(s, NamedKey::Escape);
    s.frame("escape closes")?;
    s.click_text("Eastern Standard Time (EST)")?;
    s.frame("small open")?;
    s.click_text("Greenwich Mean Time (GMT)")?;
    s.frame("picked gmt")?;
    // A click right after a pick, with no frame between, reaches what is
    // under the pointer now (the multiple select's trigger), not the closed list.
    s.click_text("Blueberry")?;
    s.settle()?;
    s.click_text("Pineapple")?;
    s.click_text("Select a fruit")?;
    s.frame("pick then click at once")?;
    key(s, NamedKey::Escape);
    // `multiple` (the first select shows Blueberry by now): each pick
    // toggles and the list stays open.
    s.click_text("Select a fruit")?;
    s.frame("multiple open")?;
    s.click_text("Apple")?;
    s.click_text("Grapes")?;
    s.frame("apple and grapes, still open")?;
    key(s, NamedKey::Escape);
    s.frame("multiple shows both")?;
    s.click_text("Select a timezone")?;
    s.frame("scrollable open")?;
    Ok(())
}
