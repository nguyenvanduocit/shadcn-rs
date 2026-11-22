use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, text};
use shadcn::{accordion, accordion_content, accordion_item, accordion_trigger};

use crate::{Render, Shot, demo, motion_frames, page};

const ITEMS: [(&str, &str, [&str; 2]); 3] = [
    (
        "item-1",
        "Product Information",
        [
            "Our flagship product combines cutting-edge technology with sleek design. Built with premium materials, it offers unparalleled performance and reliability.",
            "Key features include advanced processing capabilities, and an intuitive user interface designed for both beginners and experts.",
        ],
    ),
    (
        "item-2",
        "Shipping Details",
        [
            "We offer worldwide shipping through trusted courier partners. Standard delivery takes 3-5 business days, while express shipping ensures delivery within 1-2 business days.",
            "All orders are carefully packaged and fully insured. Track your shipment in real-time through our dedicated tracking portal.",
        ],
    ),
    (
        "item-3",
        "Return Policy",
        [
            "We stand behind our products with a comprehensive 30-day return policy. If you're not completely satisfied, simply return the item in its original condition.",
            "Our hassle-free return process includes free return shipping and full refunds processed within 48 hours of receiving the returned item.",
        ],
    ),
];

pub fn render() -> Render {
    Box::new(move || {
        let last = ITEMS.len().saturating_sub(1);
        page(
            "Accordion",
            demo(
                "AccordionDemo.vue",
                accordion()
                    .single(true)
                    .default_value(&["item-1"])
                    .w_full()
                    .children(ITEMS.iter().enumerate().map(|(i, (value, title, [first, second]))| {
                        accordion_item(*value)
                            .when(i == last, |d| d.border_0())
                            .child(accordion_trigger(*title))
                            .child(accordion_content().child(text(*first)).child(text(*second).mt(2.0)))
                    })),
            ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default item-1 open")?;
    let (x, y) = s.locate("Shipping Details")?;
    s.hover(x, y);
    s.frame("hover shipping")?;
    s.click_text("Shipping Details")?;
    s.frame("clicked shipping, item-1 closes")?;
    s.click_text("Shipping Details")?;
    s.frame("clicked shipping again, all closed")?;
    // The clicks left focus on Shipping Details.
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab moves to return policy")?;
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.frame("arrow down wraps to product information")?;
    s.press(NamedKey::Enter, Modifiers::default());
    s.frame("enter opens product information")?;
    s.press(NamedKey::Home, Modifiers::default());
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.press(NamedKey::Space, Modifiers::default());
    s.frame("home, down, space opens shipping")?;
    motion_frames(s, "closing shipping, opening return", &[0, 50, 100, 200], |s| {
        s.click_text("Return Policy")
    })?;
    Ok(())
}
