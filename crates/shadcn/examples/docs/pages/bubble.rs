use gui::prelude::*;
use gui::{Div, Icon, Model, ScenarioError, Signal, div, icon, text};
use shadcn::focus::StateLook;
use shadcn::{
    BubbleAlign, BubbleProps, BubbleReactionsSide, BubbleVariant, ButtonSize, ButtonVariant, bubble, bubble_content,
    bubble_content_button, bubble_group, bubble_reactions, bubble_reactions_button, button, collapsible,
    collapsible_trigger_as, popover, popover_content, popover_trigger, theme, toast, toaster, tooltip, tooltip_content,
    tooltip_trigger,
};

use crate::{Render, Shot, demo, page};

use BubbleAlign::{End, Start};
use BubbleVariant as V;

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

fn said(variant: BubbleVariant, align: BubbleAlign, words: &str) -> Div {
    let p = BubbleProps { variant, align };
    bubble(p).child(bubble_content(p).child(words.to_owned()))
}

fn reactions(side: BubbleReactionsSide, at: BubbleAlign, emoji: &[&str]) -> Div {
    bubble_reactions(side, at).children(emoji.iter().map(|&e| e.to_owned()))
}

/// BubbleCollapsibleDemo's long reply.
const REVIEW: &str = "The accessibility review found two focus states that were visually too subtle in dark mode.

I checked the dialog, menu, and drawer paths because each one renders focusable controls inside a layered surface.

The dialog and drawer are fine. The menu needs the hover and focus tokens split so keyboard focus stays visible when the pointer is not involved.

I also recommend keeping the change in the style file instead of the primitive so the other themes can choose their own focus treatment later.";

/// How much of [`REVIEW`] shows while folded.
const PREVIEW_LENGTH: usize = 180;

fn alignment() -> Div {
    column(32.0)
        .child(said(
            V::Muted,
            Start,
            "This bubble is aligned to the start. This is the default alignment.",
        ))
        .child(said(
            V::Default,
            End,
            "This bubble is aligned to the end. Use this for user messages.",
        ))
}

fn collapsed(open: &Signal<bool>) -> Div {
    let shown = open.get();
    let long = REVIEW.chars().count() > PREVIEW_LENGTH;
    let body = if shown || !long {
        REVIEW.to_owned()
    } else {
        format!("{}...", REVIEW.chars().take(PREVIEW_LENGTH).collect::<String>())
    };
    let p = BubbleProps {
        variant: V::Muted,
        align: End,
    };
    let toggle = collapsible_trigger_as(
        button(ButtonVariant::Link, ButtonSize::Default)
            .gap(4.0)
            .padding(0.0)
            .text_color(theme().muted_foreground)
            .child(if shown { "Show less" } else { "Show more" })
            .child(
                icon(Icon::ChevronDown)
                    .size(16.0)
                    .rotate(if shown { 180.0 } else { 0.0 }),
            ),
    );
    column(32.0)
        .child(said(V::Muted, Start, "How can I help you today?"))
        .child(
            bubble(p).child(
                bubble_content(p).child(
                    collapsible()
                        .model(Model::bind(*open))
                        .child(text(body))
                        .when(long, |c| c.child(toggle)),
                ),
            ),
        )
}

fn grouped() -> Div {
    column(32.0)
        .child(said(V::Muted, Start, "Can you tell me what's the issue?"))
        .child(
            bubble_group()
                .child(said(V::Default, End, "You tell me!"))
                .child(said(V::Default, End, "It worked yesterday. You broke it!"))
                .child(said(V::Default, End, "Find the bug and fix it.").child(reactions(
                    BubbleReactionsSide::Bottom,
                    Start,
                    &["👀"],
                ))),
        )
        .child(said(
            V::Muted,
            Start,
            "Want me to diff yesterday's you against today's you? It's a bit embarrassing.",
        ))
}

fn choices() -> Div {
    let p = BubbleProps {
        variant: V::Tinted,
        align: End,
    };
    let choice = |words: &str, said: &'static str| {
        bubble(p).child(bubble_content_button(p).child(words.to_owned()).on_click(move |_| {
            toast(said);
        }))
    };
    column(32.0)
        .child(said(V::Muted, Start, "How can I help you today?"))
        .child(
            bubble_group()
                .child(choice("I forgot my password", "You clicked forgot password"))
                .child(choice(
                    "I need help with my subscription",
                    "You clicked help with subscription",
                ))
                .child(choice(
                    "Something else. Talk to a human.",
                    "You clicked something else. Talk to a human.",
                )),
        )
}

fn failed(details: Signal<bool>) -> Div {
    let t = theme();
    let info = button(ButtonVariant::Ghost, ButtonSize::IconSm);
    let i = info.interaction();
    // The ghost button's hover text over the destructive colour.
    let info = info
        .radius(9999.0)
        .when(details.get(), |b| {
            b.text_color(t.destructive)
                .text_color_while(move || i.hovered(), t.accent_foreground)
        })
        .child(icon(Icon::Info));
    column(16.0)
        .child(said(V::Default, End, "Run the build script."))
        .child(
            said(V::Destructive, Start, "Failed to run the command.").child(
                bubble_reactions_button(BubbleReactionsSide::Bottom, End).child(
                    popover().open(details).child(popover_trigger(info)).child(
                        popover_content().child(
                            div()
                                .display(Display::Flex)
                                .flex_direction(FlexDirection::Column)
                                .gap(4.0)
                                .text_size(14.0)
                                .line_height(20.0)
                                .child(text("Command failed with exit code 1").font_weight(500))
                                .child(
                                    text("ENOENT: no such file or directory, open pnpm-lock.yaml")
                                        .text_color(t.muted_foreground),
                                ),
                        ),
                    ),
                ),
            ),
        )
}

fn read_receipt() -> Div {
    column(16.0)
        .child(said(V::Secondary, Start, "Did you remove the stale route?"))
        .child(
            said(V::Default, End, "Yes, removed it from the registry.").child(
                bubble_reactions_button(BubbleReactionsSide::Bottom, End).child(
                    tooltip()
                        .child(tooltip_trigger(
                            button(ButtonVariant::Ghost, ButtonSize::IconSm)
                                .radius(9999.0)
                                .child(icon(Icon::Check)),
                        ))
                        .child(tooltip_content().child(text("Read on Jan 5, 2026 at 4:32 PM"))),
                ),
            ),
        )
}

pub fn render() -> Render {
    let ran = Signal::labeled("ran", false);
    let open = Signal::labeled("show more", false);
    let details = Signal::labeled("failure details", false);
    Box::new(move || {
        let bottom = BubbleReactionsSide::Bottom;
        let thread = demo(
            "BubbleDemo.vue",
            column(32.0)
                .child(said(V::Default, End, "Hey there! what's up?"))
                .child(
                    bubble_group()
                        .child(said(V::Muted, Start, "Hey! Want to see chat bubbles?"))
                        .child(
                            said(
                                V::Muted,
                                Start,
                                "I can group messages, switch sides, and keep the whole thread easy to scan.",
                            )
                            .child(reactions(bottom, End, &["👍"])),
                        ),
                )
                .child(said(V::Default, End, "Sure. Hit me with your best demo."))
                .child(
                    said(
                        V::Muted,
                        Start,
                        "Yes. You are reading a demo that is demoing itself. Very meta. Very on-brand.",
                    )
                    .child(reactions(bottom, End, &["👍", "🔥", "👀", "+2"])),
                ),
        );
        let variants = demo(
            "BubbleVariantsDemo.vue",
            column(48.0)
                .child(said(V::Default, Start, "This is the default primary bubble."))
                .child(said(V::Secondary, End, "This is the secondary variant."))
                .child(
                    said(
                        V::Muted,
                        Start,
                        "This one is muted. It uses a lower emphasis color for the chat bubble.",
                    )
                    .child(reactions(bottom, End, &["👍"])),
                )
                .child(said(
                    V::Tinted,
                    End,
                    "This one is tinted. The tint is a softer color derived from the primary color.",
                ))
                .child(said(V::Outline, Start, "We can also use an outlined variant."))
                .child(
                    said(V::Destructive, End, "Or a destructive variant with a reaction.").child(reactions(
                        bottom,
                        End,
                        &["🔥"],
                    )),
                )
                // One text run: gui draws neither the <b> nor the <code> inline.
                .child(said(
                    V::Ghost,
                    Start,
                    "Ghost bubbles work for assistant text, markdown, and other content that should not be framed. \
                     This is perfect for assistant messages that should not have a frame and can take the full width \
                     of the container. You can also render code in it. Ghost bubbles are full width and can take the \
                     full width of the container.",
                )),
        );
        let r = ran;
        let reacted = demo(
            "BubbleReactionsDemo.vue",
            column(48.0)
                .child(
                    said(V::Muted, End, "I don't need tests, I know my code works.").child(reactions(
                        bottom,
                        Start,
                        &["👍", "😮"],
                    )),
                )
                .child(
                    said(
                        V::Muted,
                        Start,
                        "Bold. Fine I'll add some tests. I'll let you know when they're done.",
                    )
                    .child(reactions(bottom, End, &["👀", "🚀", "+2"])),
                )
                .child(
                    said(
                        V::Default,
                        End,
                        "Tests passed on the first try. All 142 of them. Looking good!",
                    )
                    .child(reactions(BubbleReactionsSide::Top, Start, &["🎉", "👏"])),
                )
                .child(
                    said(V::Destructive, Start, "Are you sure I can run this command?").child(
                        bubble_reactions_button(bottom, End).child(
                            button(ButtonVariant::Ghost, ButtonSize::Sm)
                                .child("Yes, run it")
                                .on_click(move |_| r.set(true)),
                        ),
                    ),
                ),
        );
        page(
            "Bubble",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .child(
                    text(if ran.get() { "Running command…" } else { "Not run yet" })
                        .text_size(14.0)
                        .line_height(20.0),
                )
                .child(reacted)
                .child(thread)
                .child(variants)
                .child(demo("BubbleAlignmentDemo.vue", alignment()))
                .child(demo("BubbleCollapsibleDemo.vue", collapsed(&open)))
                .child(demo("BubbleGroupDemo.vue", grouped()))
                .child(demo("BubbleLinksAndButtonsDemo.vue", choices()))
                .child(demo("BubblePopoverDemo.vue", failed(details)))
                .child(demo("BubbleTooltipDemo.vue", read_receipt()))
                .child(toaster()),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Yes, run it")?;
    s.hover(x, y);
    s.frame("hover reaction button")?;
    s.click_text("Yes, run it")?;
    s.frame("ran")?;
    s.click_text("Show more")?;
    s.frame("show more")?;
    s.click_text("I forgot my password")?;
    s.frame("suggestion toasts")?;
    let (x, y) = s.locate("Yes, removed it from the registry.")?;
    s.hover(x + 80.0, y + 40.0);
    s.frame("read receipt tooltip")?;
    Ok(())
}
