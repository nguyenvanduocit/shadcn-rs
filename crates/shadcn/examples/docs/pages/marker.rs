use gui::prelude::*;
use gui::{Div, Element, Icon, ScenarioError, Signal, div, icon, text};
use shadcn::{MarkerVariant, marker, marker_button, marker_content, marker_icon, marker_link, spinner};

use crate::{Render, Shot, demo, page};

use MarkerVariant as V;

/// The demos' column: `flex w-full max-w-sm flex-col py-12`.
fn column(gap: f32) -> Div {
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .max_width(384.0)
        .gap(gap)
        .padding_y(48.0)
}

fn note(variant: MarkerVariant, which: Option<Icon>, words: &str) -> Div {
    let glyph = which.map_or(Element::Empty, |i| marker_icon(icon(i)).into_element());
    marker(variant, glyph, marker_content(variant).child(words.to_owned()))
}

fn busy(variant: MarkerVariant, words: &str) -> Div {
    marker(
        variant,
        marker_icon(spinner()),
        marker_content(variant).child(words.to_owned()),
    )
}

pub fn render() -> Render {
    let reverted = Signal::labeled("reverted", 0_u32);
    Box::new(move || {
        let r = reverted;
        page(
            "Marker",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .child(
                    text(format!("Reverted {} times", reverted.get()))
                        .text_size(14.0)
                        .line_height(20.0),
                )
                .child(demo(
                    "MarkerLinkButtonDemo.vue",
                    column(32.0)
                        .child(marker_link(note(
                            V::Default,
                            Some(Icon::GitBranch),
                            "View the pull request",
                        )))
                        .child(
                            marker_button(note(V::Default, Some(Icon::RotateCcw), "Revert this change"))
                                .on_click(move |_| r.update(|n| *n = n.saturating_add(1))),
                        ),
                ))
                .child(demo(
                    "MarkerDemo.vue",
                    column(32.0)
                        .child(note(V::Default, Some(Icon::GitBranch), "Switched to a new branch"))
                        .child(busy(V::Default, "Thinking..."))
                        .child(note(V::Separator, None, "Conversation compacted"))
                        .child(note(V::Default, Some(Icon::Search), "Explored 4 files")),
                ))
                .child(demo(
                    "MarkerVariantsDemo.vue",
                    column(32.0)
                        .child(note(V::Default, None, "A default marker for inline notes."))
                        .child(note(V::Separator, None, "A separator marker"))
                        .child(note(V::Border, None, "A border marker for row boundaries.")),
                ))
                .child(demo(
                    "MarkerStatusDemo.vue",
                    column(32.0)
                        .child(busy(V::Default, "Compacting conversation"))
                        .child(busy(V::Separator, "Running tests")),
                ))
                .child(demo(
                    "MarkerSeparatorDemo.vue",
                    column(32.0)
                        .child(note(V::Separator, None, "Today"))
                        .child(note(V::Separator, None, "Worked for 42s"))
                        .child(note(V::Separator, None, "Conversation compacted")),
                ))
                .child(demo(
                    "MarkerBorderDemo.vue",
                    column(12.0)
                        .child(note(V::Border, Some(Icon::GitBranch), "Switched to release-candidate"))
                        .child(note(V::Border, Some(Icon::Search), "Reviewed 8 related files"))
                        .child(note(V::Border, Some(Icon::FileText), "Opened implementation notes")),
                ))
                .child(demo(
                    "MarkerIconDemo.vue",
                    column(48.0)
                        .child(note(V::Default, Some(Icon::GitBranch), "Switched to a new branch"))
                        .child(note(V::Separator, Some(Icon::Search), "Explored 4 files"))
                        .child(
                            note(V::Default, Some(Icon::BookOpenCheck), "Syncing completed")
                                .display(Display::Flex)
                                .flex_direction(FlexDirection::Column),
                        ),
                ))
                .child(demo(
                    "MarkerShimmerDemo.vue",
                    column(32.0).child(note(V::Default, None, "Thinking...")).child(note(
                        V::Separator,
                        None,
                        "Reading 4 files",
                    )),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Revert this change")?;
    s.hover(x, y);
    s.frame("hover revert")?;
    s.click_text("Revert this change")?;
    s.frame("reverted")?;
    Ok(())
}
