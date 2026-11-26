use gui::prelude::*;
use gui::{Div, Icon, ScenarioError, Signal, div, icon, text};
use shadcn::{
    BubbleAlign, BubbleProps, BubbleReactionsSide, BubbleVariant, Button, ButtonSize, ButtonVariant, MarkerVariant,
    MessageProps, avatar, avatar_fallback, bubble, bubble_content, bubble_group, bubble_reactions, button, marker,
    message, message_avatar, message_content, message_footer, message_group, message_header, theme, tooltip,
    tooltip_content, tooltip_trigger,
};

use super::markdown::markdown;
use crate::{Render, Shot, demo, page};

use BubbleAlign::{End, Start};

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

fn said(p: MessageProps, variant: BubbleVariant, words: &str) -> Div {
    let b = BubbleProps {
        variant,
        align: p.align,
    };
    bubble(b).child(bubble_content(b).child(words.to_owned()))
}

/// The demos' avatars have no picture here (`/avatars/*.png` are the docs
/// site's), so their initials show, as reka shows them until a picture loads.
fn face(p: MessageProps, initials: &str) -> Div {
    message_avatar(p).child(avatar().child(avatar_fallback(initials)))
}

fn turn(align: BubbleAlign, footer: bool, initials: &str, body: impl IntoElement) -> Div {
    let p = MessageProps {
        align,
        footer,
        ..MessageProps::default()
    };
    message(p)
        .child(face(p, initials))
        .child(message_content(p).child(body))
}

fn mine(footer: bool) -> MessageProps {
    MessageProps {
        align: End,
        footer,
        ..MessageProps::default()
    }
}

fn theirs() -> MessageProps {
    MessageProps::default()
}

fn ghost_icon(size: ButtonSize, glyph: Icon) -> Button {
    button(ButtonVariant::Ghost, size).child(icon(glyph))
}

fn actions() -> Div {
    let theirs = theirs();
    let failed = mine(true);
    column(32.0)
        .child(
            message(theirs).child(
                message_content(theirs)
                    .child(said(
                        theirs,
                        BubbleVariant::Muted,
                        "The install failure is coming from the workspace package.",
                    ))
                    .child(
                        message_footer(theirs)
                            .child(ghost_icon(ButtonSize::Icon, Icon::Copy))
                            .child(ghost_icon(ButtonSize::Icon, Icon::ThumbsUp))
                            .child(ghost_icon(ButtonSize::Icon, Icon::ThumbsDown)),
                    ),
            ),
        )
        .child(
            message(failed).child(
                message_content(failed)
                    .child(said(
                        failed,
                        BubbleVariant::Default,
                        "Okay drop me a link. Taking a look...",
                    ))
                    .child(
                        message_footer(failed)
                            .gap(8.0)
                            .child(text("Failed to send").font_weight(400).text_color(theme().destructive))
                            .child(ghost_icon(ButtonSize::IconSm, Icon::RefreshCcw)),
                    ),
            ),
        )
}

fn grouped() -> Div {
    let p = theirs();
    column(24.0).child(
        message_group()
            .child(message(p).child(message_avatar(p)).child(message_content(p).child(said(
                p,
                BubbleVariant::Muted,
                "I checked the registry addresses.",
            ))))
            .child(message(p).child(face(p, "CN")).child(message_content(p).child(said(
                p,
                BubbleVariant::Muted,
                "The component and example JSON now live under the UI registry.",
            )))),
    )
}

/// MessagePatternDemo's reply, in Markdown.
const REPLY: &str = "Install Dialog, then open it from the gear button in `SiteHeader.vue`.

```bash
npx shadcn-vue@latest add dialog
```

Wrap the button in a `DialogTrigger` and put the settings form in `DialogContent`. Focus moves into the dialog when it opens, and **Esc** closes it.";

/// MessagePatternDemo's state: the reply's code block, the copy and
/// feedback buttons and the tooltip of each action.
struct Pattern {
    /// Whether the reply was copied in the last two seconds.
    copied: Signal<bool>,
    /// Owns the timer that clears `copied`.
    owner: gui::Scope,
    feedback: Signal<Option<bool>>,
}

impl Pattern {
    fn new() -> Self {
        Self {
            copied: Signal::labeled("reply copied", false),
            owner: gui::Scope::new(),
            feedback: Signal::labeled("feedback", None),
        }
    }

    /// A ghost action under a message, named by its tooltip.
    fn action(label: &str, glyph: Icon, on_click: impl Fn() + 'static) -> gui::Element {
        tooltip()
            .child(tooltip_trigger(
                ghost_icon(ButtonSize::IconSm, glyph).on_click(move |_| on_click()),
            ))
            .child(tooltip_content().child(text(label.to_owned())))
            .into_element()
    }

    fn view(&self) -> Div {
        let ask = MessageProps {
            align: End,
            ..MessageProps::default()
        };
        let b = BubbleProps {
            variant: BubbleVariant::Muted,
            align: End,
        };
        let reply = MessageProps {
            bubble: BubbleVariant::Ghost,
            ..MessageProps::default()
        };
        let copied = self.copied.get();
        let (flag, owner, feedback) = (self.copied, self.owner, self.feedback);
        let copy = move || {
            if gui::clipboard::copy(REPLY) {
                flag.set(true);
                owner.run(|| gui::clock::after(std::time::Duration::from_secs(2), move || flag.set(false)));
            }
        };
        let (up, down) = (feedback, feedback);
        let vote = |s: &Signal<Option<bool>>, v: bool| s.update(|f| *f = if *f == Some(v) { None } else { Some(v) });
        div()
            .margin_x(Length::Auto)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .width(pct(100.0))
            .max_width(448.0)
            .gap(32.0)
            .padding_y(24.0)
            .text_size(14.0)
            .line_height(20.0)
            .child(
                message(ask).child(
                    message_content(ask)
                        .gap(8.0)
                        .child(bubble(b).child(
                            bubble_content(b).child(
                                "How do I add a settings dialog? It should open from the gear icon in the header.",
                            ),
                        ))
                        .child(
                            message_footer(ask)
                                .child(Self::action("Copy", Icon::Copy, || {}))
                                .child(Self::action("Edit", Icon::Pencil, || {})),
                        ),
                ),
            )
            .child(
                message(reply).child(
                    message_content(reply).gap(8.0).child(markdown(REPLY, 14.0)).child(
                        message_footer(reply)
                            .child(Self::action(
                                if copied { "Copied" } else { "Copy" },
                                if copied { Icon::Check } else { Icon::Copy },
                                copy,
                            ))
                            .child(Self::action("Good response", Icon::ThumbsUp, move || vote(&up, true)))
                            .child(Self::action("Bad response", Icon::ThumbsDown, move || {
                                vote(&down, false);
                            }))
                            .child(Self::action("Retry", Icon::RefreshCcw, || {})),
                    ),
                ),
            )
    }
}

pub fn render() -> Render {
    let pattern = Pattern::new();
    Box::new(move || {
        let delivered = mine(true);
        let main = demo(
            "MessageDemo.vue",
            column(24.0)
                .child(turn(
                    End,
                    false,
                    "ME",
                    said(mine(false), BubbleVariant::Default, "Deploying to prod real quick."),
                ))
                .child(turn(
                    Start,
                    false,
                    "R",
                    said(theirs(), BubbleVariant::Muted, "It's 4:55 PM. On a Friday."),
                ))
                .child(
                    message(delivered).child(face(delivered, "ME")).child(
                        message_content(delivered)
                            .child(said(delivered, BubbleVariant::Default, "It's a one-line change."))
                            .child(message_footer(delivered).child("Delivered")),
                    ),
                )
                .child(turn(
                    Start,
                    false,
                    "R",
                    bubble_group()
                        .child(said(
                            theirs(),
                            BubbleVariant::Muted,
                            "It's always a one-line change 😭.",
                        ))
                        .child(
                            said(theirs(), BubbleVariant::Muted, "Alright, let me take a look.")
                                .child(bubble_reactions(BubbleReactionsSide::Bottom, End).child("👍")),
                        ),
                ))
                .child(marker(
                    MarkerVariant::Default,
                    gui::Element::Empty,
                    div()
                        .display(Display::Flex)
                        .child(text("Oliver").font_weight(500))
                        .child(text("\u{a0}is typing...")),
                )),
        );
        let avatars = demo(
            "MessageAvatarDemo.vue",
            column(24.0)
                .child(turn(
                    Start,
                    false,
                    "R",
                    said(theirs(), BubbleVariant::Muted, "The build failed during dependency installation."),
                ))
                .child(turn(End, false, "R", said(mine(false), BubbleVariant::Default, "Can you share the exact error?")))
                .child(turn(
                    Start,
                    false,
                    "R",
                    bubble_group()
                        .child(said(theirs(), BubbleVariant::Muted, "Here's the error from the logs"))
                        .child(said(
                            theirs(),
                            BubbleVariant::Muted,
                            "Something went wrong with the build. The libraries are not installed correctly. Try running the build again.",
                        )),
                )),
        );
        let read = mine(true);
        let captions = demo(
            "MessageHeaderAndFooterDemo.vue",
            column(32.0)
                .child(
                    message(theirs()).child(
                        message_content(theirs())
                            .child(message_header(theirs()).child("Olivia"))
                            .child(said(theirs(), BubbleVariant::Muted, "I already checked the logs.")),
                    ),
                )
                .child(
                    message(read).child(
                        message_content(read)
                            .child(said(
                                read,
                                BubbleVariant::Default,
                                "Send the report to the team. Ping @shadcn if you need help.",
                            ))
                            .child(
                                message_footer(read)
                                    .child(text("Read\u{a0}"))
                                    .child(text("Yesterday").font_weight(400)),
                            ),
                    ),
                ),
        );
        page(
            "Message",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .child(main)
                .child(avatars)
                .child(captions)
                .child(demo("MessageActionsDemo.vue", actions()))
                .child(demo("MessageGroupDemo.vue", grouped()))
                .child(demo("MessagePatternDemo.vue", pattern.view())),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    // The reply's actions: the lowest row of buttons in MessagePatternDemo.
    let frame = s.last();
    let pattern = frame
        .nodes
        .iter()
        .position(|n| n.name == Some("MessagePatternDemo.vue"));
    let action = frame
        .nodes
        .iter()
        .enumerate()
        .filter(|&(i, n)| n.name == Some("button") && frame.ancestors(i).any(|a| Some(a) == pattern))
        .map(|(_, n)| n.bounds(gui::inspect::OUTSIDE))
        .reduce(|a, b| if (b.y, -b.x) > (a.y, -a.x) { b } else { a })
        .ok_or_else(|| ScenarioError::TextNotFound("MessagePatternDemo.vue's actions".to_owned()))?;
    s.hover(action.x + action.width / 2.0, action.y + action.height / 2.0);
    s.frame("hover an action")?;
    Ok(())
}
