use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{
    SelectSize, select, select_content, select_group, select_item, select_label, select_multiple, select_separator,
    select_value, theme,
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
    let fruit = Signal::labeled("fruit", None::<String>);
    let small = Signal::labeled("timezone", Some("est".to_owned()));
    Box::new(move || {
        let t = theme();
        let fruits = || {
            select_content().child(
                select_group()
                    .child(select_label("Fruits"))
                    .children(FRUITS.map(|(v, l)| select_item(v, l))),
            )
        };
        let zones = select_content()
            .child(
                select_group()
                    .child(select_label("North America"))
                    .child(select_item("est", "Eastern Standard Time (EST)"))
                    .child(select_item("cst", "Central Standard Time (CST)"))
                    .child(select_item("pst", "Pacific Standard Time (PST)").disabled(true)),
            )
            .child(select_separator())
            .child(
                select_group()
                    .child(select_label("Europe & Africa"))
                    .child(select_item("gmt", "Greenwich Mean Time (GMT)"))
                    .child(select_item("cet", "Central European Time (CET)")),
            );
        page(
            "Select",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(24.0)
                .child(demo(
                    "SelectDemo.vue",
                    select()
                        .model(fruit)
                        .width(180.0)
                        .child(select_value("Select a fruit"))
                        .child(fruits()),
                ))
                .child(demo(
                    "SelectMultipleDemo.vue",
                    select_multiple()
                        .width(180.0)
                        .child(select_value("Select a fruit"))
                        .child(fruits()),
                ))
                .child(demo(
                    "SelectScrollable.vue",
                    select()
                        .width(280.0)
                        .child(select_value("Select a timezone"))
                        .child(select_content().children(ZONES.map(|(region, zones)| {
                            select_group()
                                .child(select_label(region))
                                .children(zones.iter().map(|(v, l)| select_item(*v, *l)))
                        }))),
                ))
                .child(section(
                    "Small, groups, separator, disabled item",
                    div().display(Display::Flex).child(
                        select()
                            .model(small)
                            .size(SelectSize::Sm)
                            .width(280.0)
                            .child(select_value("Select a timezone"))
                            .child(zones),
                    ),
                ))
                .child(
                    text(format!(
                        "Fruit: {}. Timezone: {}.",
                        fruit.get().unwrap_or_default(),
                        small.get().unwrap_or_default()
                    ))
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
