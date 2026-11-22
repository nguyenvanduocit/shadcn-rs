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
    text(title)
        .text_size(36.0)
        .line_height(40.0)
        .font_weight(800)
        .tracking(-0.025)
        .text_wrap(WrapStyle::Balance)
}

/// `border-b pb-2 text-3xl font-semibold tracking-tight`.
fn h2(title: &str) -> Div {
    div()
        .border_bottom(1.0)
        .border_color(theme().border)
        .padding_bottom(8.0)
        .child(
            text(title)
                .text_size(30.0)
                .line_height(36.0)
                .font_weight(600)
                .tracking(-0.025),
        )
}

/// `text-2xl font-semibold tracking-tight`.
fn h3(title: &str) -> Text {
    text(title)
        .text_size(24.0)
        .line_height(32.0)
        .font_weight(600)
        .tracking(-0.025)
}

/// `leading-7 [&:not(:first-child)]:mt-6`, after a first child.
fn p(content: &str) -> Text {
    text(content).line_height(28.0).margin_top(24.0)
}

/// `mt-6 border-l-2 pl-6 italic`.
fn blockquote() -> Div {
    div()
        .margin_top(24.0)
        .border_left(2.0)
        .border_color(theme().border)
        .padding_left(24.0)
        .child(text(QUOTE).italic(true))
}

/// `my-6 ml-6 list-disc [&>li]:mt-2`: each item's disc hangs outside it,
/// Chrome's `::marker` ("• ") ending 6px before the item starts (measured).
fn list() -> Div {
    div().margin_y(24.0).margin_left(24.0).children(LEVELS.map(|item| {
        div()
            .position(Position::Relative)
            .margin_top(8.0)
            .child(
                text("\u{2022} ")
                    .position(Position::Absolute)
                    .top(0.0)
                    .right(gui::Length::Fraction(1.0))
                    .margin_right(6.0)
                    .nowrap(true),
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
                .border_right(1.0)
                .border_bottom(1.0)
                .border_color(t.border)
                .padding_x(16.0)
                .padding_y(8.0)
                .when(row > 0 && row % 2 == 0, |d| d.background(t.muted))
                .child(text(cell).when(row == 0, |e| e.font_weight(700)))
        })
    });
    div().margin_y(24.0).width(pct(100.0)).child(
        div()
            .display(Display::Grid)
            .grid_columns(vec![Track::Fr(352.0), Track::Fr(415.0)])
            .width(pct(100.0))
            .border_top(1.0)
            .border_left(1.0)
            .border_color(t.border)
            .children(cells),
    )
}

pub fn render() -> Render {
    let followed = Signal::labeled("plan link followed", 0_u32);
    Box::new(move || {
        let t = theme();
        let follow = followed;
        let plan = text("The king thought long and hard, and finally came up with ")
            .line_height(28.0)
            .margin_top(24.0)
            .span(
                span("a brilliant plan")
                    .text_color(t.primary)
                    .font_weight(500)
                    .underline(true)
                    .underline_offset(4.0)
                    .cursor(Cursor::Pointer)
                    .on_click(move |_| follow.update(|n| *n = n.saturating_add(1))),
            )
            .span(": he would tax the jokes in the kingdom.");
        let story = div()
            .child(h1("Taxing Laughter: The Joke Tax Chronicles"))
            .child(text(LEAD).text_color(t.muted_foreground).text_size(20.0).line_height(28.0).line_height(28.0).margin_top(24.0))
            .child(h2("The King's Plan").margin_top(40.0))
            .child(plan)
            .child(blockquote())
            .child(h3("The Joke Tax").margin_top(32.0))
            .child(p("The king's subjects were not amused. They grumbled and complained, but the king was firm:"))
            .child(list())
            .child(p("As a result, people stopped telling jokes, and the kingdom fell into a gloom. But there was one person who refused to let the king's foolishness get him down: a court jester named Jokester."))
            .child(h3("Jokester's Revolt").margin_top(32.0))
            .child(p("Jokester began sneaking into the castle in the middle of the night and leaving jokes all over the place: under the king's pillow, in his soup, even in the royal toilet. The king was furious, but he couldn't seem to stop Jokester."))
            .child(p("And then, one day, the people of the kingdom discovered that the jokes left by Jokester were so funny that they couldn't help but laugh. And once they started laughing, they couldn't stop."))
            .child(h3("The People's Rebellion").margin_top(32.0))
            .child(p("The people of the kingdom, feeling uplifted by the laughter, started to tell jokes and puns again, and soon the entire kingdom was in on the joke."))
            .child(table())
            .child(p("The king, seeing how much happier his subjects were, realized the error of his ways and repealed the joke tax. Jokester was declared a hero, and the kingdom lived happily ever after."))
            .child(p("The moral of the story is: never underestimate the power of a good laugh and always be careful of bad ideas."));
        page(
            "Typography",
            div()
                .display(Display::Flex).flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo("TypographyDemo.vue", div().width(pct(100.0)).child(story)))
                .child(demo(
                    "TypographyH1.vue",
                    h1("Taxing Laughter: The Joke Tax Chronicles").text_align(TextAlign::Center),
                ))
                .child(demo("TypographyH2.vue", h2("The People of the Kingdom")))
                .child(demo("TypographyH3.vue", h3("The Joke Tax")))
                .child(demo(
                    "TypographyH4.vue",
                    text("People stopped telling jokes").text_size(20.0).line_height(28.0).font_weight(600).tracking(-0.025),
                ))
                .child(demo(
                    "TypographyP.vue",
                    text("The king, seeing how much happier his subjects were, realized the error of his ways and repealed the joke tax.")
                        .line_height(28.0),
                ))
                .child(demo("TypographyBlockquote.vue", blockquote()))
                .child(demo("TypographyList.vue", list()))
                .child(demo(
                    "TypographyInlineCode.vue",
                    div()
                        .position(Position::Relative)
                        .radius(4.0)
                        .background(t.muted)
                        .padding_x(4.8)
                        .padding_y(3.2)
                        .font_family(FontFamily::GEIST_MONO)
                        .text_size(14.0).line_height(20.0)
                        .font_weight(600)
                        .child(text("reka-ui")),
                ))
                .child(demo(
                    "TypographyLead.vue",
                    text("A modal dialog that interrupts the user with important content and expects a response.")
                        .text_size(20.0).line_height(28.0)
                        .text_color(t.muted_foreground),
                ))
                .child(demo(
                    "TypographyLarge.vue",
                    text("Are you absolutely sure?").text_size(18.0).line_height(28.0).font_weight(600),
                ))
                .child(demo(
                    "TypographySmall.vue",
                    text("Email address").text_size(14.0).line_height(20.0).font_weight(500).line_height(LineHeight::Relative(1.0)),
                ))
                .child(demo(
                    "TypographyMuted.vue",
                    text("Enter your email address.").text_size(14.0).line_height(20.0).text_color(t.muted_foreground),
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
