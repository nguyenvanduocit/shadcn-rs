//! shadcn-vue's typography: no component, the Tailwind classes its docs put
//! on prose (`components/demo/Typography*.vue`), set here as gui styles.

use gui::prelude::*;
use gui::{Div, ScenarioError, Signal, Text, Track, div, span, text};
use shadcn::theme;

use crate::{Render, Shot, demo, page};

const LEAD: &str = "Once upon a time, in a far-off land, there was a very lazy king who spent all day lounging on his throne. One day, his advisors came to him with a problem: the kingdom was running out of money.";
const QUOTE: &str = "\"After all,\" he said, \"everyone enjoys a good joke, so it's only fair that they should pay for the privilege.\"";
const LEVELS: [&str; 3] = [
    "1st level of puns: 5 gold coins",
    "2nd level of jokes: 10 gold coins",
    "3rd level of one-liners : 20 gold coins",
];
const TREASURY: [[&str; 2]; 4] = [
    ["King's Treasury", "People's happiness"],
    ["Empty", "Overflowing"],
    ["Modest", "Satisfied"],
    ["Full", "Ecstatic"],
];

/// `text-4xl font-extrabold tracking-tight text-balance`.
fn h1(title: &str) -> Text {
    text(title).text_4xl().font_extrabold().tracking_tight().text_balance()
}

/// `border-b pb-2 text-3xl font-semibold tracking-tight`.
fn h2(title: &str) -> Div {
    div()
        .border_b()
        .border_color(theme().border)
        .pb(2.0)
        .child(text(title).text_3xl().font_semibold().tracking_tight())
}

/// `text-2xl font-semibold tracking-tight`.
fn h3(title: &str) -> Text {
    text(title).text_2xl().font_semibold().tracking_tight()
}

/// `leading-7 [&:not(:first-child)]:mt-6`, after a first child.
fn p(content: &str) -> Text {
    text(content).leading_px(28.0).mt(6.0)
}

/// `mt-6 border-l-2 pl-6 italic`.
fn blockquote() -> Div {
    div()
        .mt(6.0)
        .border_l_w(2.0)
        .border_color(theme().border)
        .pl(6.0)
        .child(text(QUOTE).italic())
}

/// `my-6 ml-6 list-disc [&>li]:mt-2`: each item's disc hangs outside it,
/// Chrome's `::marker` ("• ") ending 6px before the item starts (measured).
fn list() -> Div {
    div().my(6.0).ml(6.0).children(LEVELS.map(|item| {
        div()
            .relative()
            .mt(2.0)
            .child(
                text("\u{2022} ")
                    .absolute()
                    .top(0.0)
                    .right_len(gui::relative(1.0))
                    .mr(1.5)
                    .whitespace_nowrap(),
            )
            .child(text(item))
    }))
}

/// `<table class="w-full">` under `my-6 w-full`: bordered cells `px-4 py-2`,
/// the header bold, every even body row `bg-muted`. Borders collapse
/// (Tailwind's preflight), so each cell draws its right and bottom edge and
/// the table its top and left. `gui` has no table layout: Chrome's automatic
/// one gives each column a share of the width in proportion to its
/// max-content width, measured there as 352 and 415 of 767.
fn table() -> Div {
    let t = theme();
    let cells = TREASURY.iter().enumerate().flat_map(|(row, cells)| {
        cells.map(|cell| {
            div()
                .border_r()
                .border_b()
                .border_color(t.border)
                .px(4.0)
                .py(2.0)
                .when(row > 0 && row % 2 == 0, |d| d.bg(t.muted))
                .child(text(cell).when(row == 0, Styled::font_bold))
        })
    });
    div().my(6.0).w_full().child(
        div()
            .grid()
            .grid_cols_tracks(vec![Track::Fr(352.0), Track::Fr(415.0)])
            .w_full()
            .border_t()
            .border_l()
            .border_color(t.border)
            .children(cells),
    )
}

pub fn render() -> Render {
    let followed = Signal::labeled("plan link followed", 0_u32);
    Box::new(move || {
        let t = theme();
        let follow = followed.clone();
        let plan = text("The king thought long and hard, and finally came up with ")
            .leading_px(28.0)
            .mt(6.0)
            .span(
                span("a brilliant plan")
                    .text_color(t.primary)
                    .font_medium()
                    .underline()
                    .underline_offset(4.0)
                    .cursor_pointer()
                    .on_click(move |_| follow.update(|n| n.saturating_add(1))),
            )
            .span(": he would tax the jokes in the kingdom.");
        let story = div()
            .child(h1("Taxing Laughter: The Joke Tax Chronicles"))
            .child(text(LEAD).text_color(t.muted_foreground).text_xl().leading_px(28.0).mt(6.0))
            .child(h2("The King's Plan").mt(10.0))
            .child(plan)
            .child(blockquote())
            .child(h3("The Joke Tax").mt(8.0))
            .child(p("The king's subjects were not amused. They grumbled and complained, but the king was firm:"))
            .child(list())
            .child(p("As a result, people stopped telling jokes, and the kingdom fell into a gloom. But there was one person who refused to let the king's foolishness get him down: a court jester named Jokester."))
            .child(h3("Jokester's Revolt").mt(8.0))
            .child(p("Jokester began sneaking into the castle in the middle of the night and leaving jokes all over the place: under the king's pillow, in his soup, even in the royal toilet. The king was furious, but he couldn't seem to stop Jokester."))
            .child(p("And then, one day, the people of the kingdom discovered that the jokes left by Jokester were so funny that they couldn't help but laugh. And once they started laughing, they couldn't stop."))
            .child(h3("The People's Rebellion").mt(8.0))
            .child(p("The people of the kingdom, feeling uplifted by the laughter, started to tell jokes and puns again, and soon the entire kingdom was in on the joke."))
            .child(table())
            .child(p("The king, seeing how much happier his subjects were, realized the error of his ways and repealed the joke tax. Jokester was declared a hero, and the kingdom lived happily ever after."))
            .child(p("The moral of the story is: never underestimate the power of a good laugh and always be careful of bad ideas."));
        page(
            "Typography",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo("TypographyDemo.vue", div().w_full().child(story)))
                .child(demo(
                    "TypographyH1.vue",
                    h1("Taxing Laughter: The Joke Tax Chronicles").text_center(),
                ))
                .child(demo("TypographyH2.vue", h2("The People of the Kingdom")))
                .child(demo("TypographyH3.vue", h3("The Joke Tax")))
                .child(demo(
                    "TypographyH4.vue",
                    text("People stopped telling jokes").text_xl().font_semibold().tracking_tight(),
                ))
                .child(demo(
                    "TypographyP.vue",
                    text("The king, seeing how much happier his subjects were, realized the error of his ways and repealed the joke tax.")
                        .leading_px(28.0),
                ))
                .child(demo("TypographyBlockquote.vue", blockquote()))
                .child(demo("TypographyList.vue", list()))
                .child(demo(
                    "TypographyInlineCode.vue",
                    div()
                        .relative()
                        .rounded(4.0)
                        .bg(t.muted)
                        .px(1.2)
                        .py(0.8)
                        .font_mono()
                        .text_sm()
                        .font_semibold()
                        .child(text("reka-ui")),
                ))
                .child(demo(
                    "TypographyLead.vue",
                    text("A modal dialog that interrupts the user with important content and expects a response.")
                        .text_xl()
                        .text_color(t.muted_foreground),
                ))
                .child(demo(
                    "TypographyLarge.vue",
                    text("Are you absolutely sure?").text_lg().font_semibold(),
                ))
                .child(demo(
                    "TypographySmall.vue",
                    text("Email address").text_sm().font_medium().leading_none(),
                ))
                .child(demo(
                    "TypographyMuted.vue",
                    text("Enter your email address.").text_sm().text_color(t.muted_foreground),
                ))
                .child(demo("TypographyTable.vue", table())),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    // The link sits inside its sentence; clicking its words follows it.
    let (x, y) = s.locate("a brilliant plan")?;
    s.hover(x, y);
    s.frame("hover link")?;
    s.click_text("a brilliant plan")?;
    s.frame("followed")?;
    Ok(())
}
