//! Message scroller: chat threads that follow a streaming reply, settle a new
//! turn near the top with a peek of the one before, open where asked, jump to
//! a message, keep the reader's place as history loads, and say what is in view.

use gui::prelude::*;
use std::rc::Rc;

use gui::{Div, Element, Icon, Model, ScenarioError, Side, SideAlign, Signal, Text, component, div, icon, text};
use shadcn::{
    BubbleAlign, BubbleProps, BubbleVariant, Button, ButtonSize, ButtonVariant, InputGroupAlign, InputGroupButtonSize,
    MarkerVariant, MenuItem, MessageProps, MessageScroller, MessageScrollerDefaultScrollPosition,
    MessageScrollerDirection, MessageScrollerProps, MessageScrollerScrollAlign, MessageScrollerScrollOptions,
    MessageScrollerScrollable, Orientation, Toast, ToastKind, ToggleGroupItem, bubble, bubble_content, button, card,
    card_action, card_content, card_description, card_footer, card_header, card_title, dropdown_menu,
    dropdown_menu_content, dropdown_menu_group, dropdown_menu_item, dropdown_menu_label, dropdown_menu_separator,
    dropdown_menu_trigger, empty, empty_description, empty_header, empty_media_icon, empty_title, hover_card,
    hover_card_content, hover_card_trigger, input_group_addon, input_group_button, input_group_content, marker,
    marker_content, message, message_content, message_header, message_scroller, message_scroller_button,
    message_scroller_provider, message_scroller_viewport, select, select_content, select_group, select_item,
    select_value, slider, tabs, tabs_list, tabs_trigger, theme, toast, toaster, toggle_group_item,
    toggle_group_multiple, tooltip, tooltip_content, tooltip_trigger,
};

use super::demo_chat::{DemoChat, DemoMessage, Role, Turns, animated};
use super::thread::{click_named, wait};
use crate::{Render, Shot, demo, page};
use shadcn::focus::StateLook;

/// A saved conversation: id, role and text of each message.
type Transcript = &'static [(&'static str, Role, &'static str)];

fn transcript(t: Transcript) -> Vec<DemoMessage> {
    t.iter()
        .map(|(id, role, words)| DemoMessage {
            id: (*id).to_owned(),
            role: *role,
            text: (*words).to_owned(),
        })
        .collect()
}

/// `child` under a provider with `props` (Vue's `MessageScrollerProvider`),
/// keyed by the demo's resets (Vue's `:key="demoKey"`).
fn provider(props: MessageScrollerProps, key: u32, child: impl IntoElement) -> Element {
    message_scroller_provider(props, child).key(key).into_element()
}

/// A control inside a provider, rendered by `f` with its scroller (a Vue
/// child component calling `useMessageScroller`).
fn scrolled(f: impl Fn(&MessageScroller) -> Element + 'static) -> Element {
    #[derive(Clone)]
    struct View(Rc<dyn Fn(&MessageScroller) -> Element>);
    impl PartialEq for View {
        fn eq(&self, _: &Self) -> bool {
            false
        }
    }
    component(View(Rc::new(f)), |cx| {
        let view = cx.props();
        let scroller = shadcn::use_message_scroller();
        move || match &scroller {
            Some(s) => view.with(|v| (v.0)(s)),
            None => Element::Empty,
        }
    })
    .into_element()
}

/// Bump a demo's reset key.
fn bump(key: Signal<u32>) {
    key.update(|k| *k = k.wrapping_add(1));
}

// ── The card every demo sits in ─────────────────────────────────────────────

/// `CardHeader class="border-b"`, with `gap-1` when `tight`.
fn header(title: &str, description: &str, tight: bool) -> Div {
    card_header()
        .when(tight, |h| h.gap(4.0))
        .border_bottom(1.0)
        .border_color(theme().border)
        .padding_bottom(24.0)
        .child(card_title(title))
        .child(card_description(description))
}

/// `Card class="h-140 w-full gap-0"`: a header, the thread
/// (`CardContent class="min-h-0 flex-1 overflow-hidden p-0"`) and a footer.
fn shell(header: Div, body: impl IntoElement, footer: Option<Div>) -> Div {
    card()
        .height(560.0)
        .width(pct(100.0))
        .gap(0.0)
        .child(header)
        .child(
            card_content()
                .min_height(0.0)
                .flex(Flex::grow(1.0))
                .overflow(Overflow::Hidden)
                .padding(0.0)
                .child(body),
        )
        .when_some(footer, Div::child)
}

/// `Card class="mx-auto h-140 w-full max-w-sm gap-0"`.
fn centred(card: Div) -> Div {
    card.margin_x(Length::Auto).max_width(384.0)
}

/// `CardFooter class="border-t"`.
fn footer_ruled() -> Div {
    card_footer()
        .border_top(1.0)
        .border_color(theme().border)
        .padding_top(24.0)
}

/// The note under a demo: `px-0.5 text-center text-xs text-muted-foreground`.
fn caption(words: &str) -> Text {
    text(words)
        .padding_x(2.0)
        .text_align(TextAlign::Center)
        .text_size(12.0)
        .line_height(16.0)
        .text_color(theme().muted_foreground)
}

/// An outline icon button resetting the demo, with a "Reset" tooltip.
fn reset_button(disabled: bool, on: impl Fn() + 'static) -> Div {
    card_action().child(
        tooltip()
            .child(tooltip_trigger(
                button(ButtonVariant::Outline, ButtonSize::Icon)
                    .disabled(disabled)
                    .child(icon(Icon::RotateCw))
                    .on_click(move |_| on()),
            ))
            .child(tooltip_content().child(text("Reset"))),
    )
}

/// What a thread shows before its first message.
fn empty_thread(title: &str, description: &str) -> Div {
    empty().height(pct(100.0)).child(
        empty_header()
            .child(empty_media_icon(Icon::MessageCircleDashed))
            .child(empty_title(title))
            .child(empty_description(description)),
    )
}

/// `MessageScroller` holding `items` (`meta`: id and anchor, `row`: the
/// message) and the button back to the end.
fn thread<T: 'static>(
    items: impl Into<Rc<[T]>>,
    meta: impl for<'a> Fn(&'a T) -> (&'a str, bool) + 'static,
    row: impl Fn(usize, &T) -> Div + 'static,
) -> Div {
    message_scroller()
        .child(message_scroller_viewport(items, meta, row))
        .child(message_scroller_button(MessageScrollerDirection::End))
}

/// A message's id, and whether it anchors (a user turn, as Vue's default).
fn turn_meta(m: &DemoMessage) -> (&str, bool) {
    (m.id.as_str(), m.role == Role::User)
}

/// A message as `MessageAnimated` with Vue's default bubbles.
fn turn_row(_: usize, m: &DemoMessage) -> Div {
    animated(m, BubbleVariant::Muted, BubbleVariant::Ghost)
}

/// The thread of `messages` as `MessageAnimated` turns.
fn turns(messages: Vec<DemoMessage>) -> Div {
    thread(messages, turn_meta, turn_row)
}

/// The send button of a footer: `size="icon" class="ml-auto"`.
fn send_button(disabled: bool, on: impl Fn() + 'static) -> Button {
    button(ButtonVariant::Default, ButtonSize::Icon)
        .margin_left(Length::Auto)
        .disabled(disabled)
        .child(icon(Icon::ArrowUp))
        .on_click(move |_| on())
}

/// The read-only composer: the next scripted turn (or `idle` when none is
/// left), a menu of attachments that does nothing, `extra`, and send.
fn composer(chat: &DemoChat, idle: &str, extra: Option<Div>) -> Div {
    let t = theme();
    let next = chat.next_message();
    let busy = chat.streaming.get();
    let line = match next {
        Some(words) => text(words),
        None => text(idle).text_color(t.muted_foreground),
    }
    .line_clamp(2)
    .when(busy, |l| l.opacity(0.6));
    let attach = |glyph: Icon, label: &str| {
        dropdown_menu_item(label, MenuItem::default())
            .child(icon(glyph))
            .child(label.to_owned())
    };
    let files = dropdown_menu_content()
        .side(Side::Top)
        .align(SideAlign::Start)
        .width(176.0)
        .child(attach(Icon::Paperclip, "Add Photos & Files"))
        .child(dropdown_menu_separator())
        .child(attach(Icon::Image, "Create Image"))
        .child(attach(Icon::Telescope, "Deep Research"))
        .child(attach(Icon::Globe, "Web Search"));
    let addon = input_group_addon(InputGroupAlign::BlockEnd).padding_top(4.0).button(
        dropdown_menu()
            .child(dropdown_menu_trigger(
                input_group_button(ButtonVariant::Outline, InputGroupButtonSize::IconSm).child(icon(Icon::Plus)),
            ))
            .child(files),
    );
    let addon = match extra {
        Some(e) => addon.child(e),
        None => addon,
    };
    let send = chat.clone();
    let addon = addon.button(
        input_group_button(ButtonVariant::Default, InputGroupButtonSize::IconSm)
            .margin_left(Length::Auto)
            .disabled(next.is_none() || busy)
            .child(icon(Icon::ArrowUp))
            .on_click(move |_| send.send()),
    );
    card_footer()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(8.0)
        .child(
            div().width(pct(100.0)).child(
                input_group_content(
                    div()
                        .height(56.0)
                        .width(pct(100.0))
                        .padding_x(12.0)
                        .padding_y(10.0)
                        .child(line),
                )
                .addon(addon),
            ),
        )
}

// ── MessageScrollerDemo, MessageScrollerStreamingDemo ──────────────────────

/// What tells the two scripted chats apart.
struct Copy {
    title: &'static str,
    description: &'static str,
    empty_title: &'static str,
    empty_description: &'static str,
    idle: &'static str,
    caption: &'static str,
    /// Reset is off while the thread is empty too.
    reset_when_empty: bool,
}

/// A scripted chat sent turn by turn (the provider outlives the empty state).
struct Chat {
    chat: DemoChat,
    props: MessageScrollerProps,
    copy: Copy,
}

impl Chat {
    fn view(&self) -> Div {
        let messages = self.chat.messages.get();
        let busy = self.chat.streaming.get();
        let c = &self.copy;
        let chat = self.chat.clone();
        let body = if messages.is_empty() {
            empty_thread(c.empty_title, c.empty_description)
        } else {
            turns(messages.clone())
        };
        div()
            .position(Position::Relative)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(provider(
                self.props,
                0,
                centred(shell(
                    header(c.title, c.description, true).child(reset_button(
                        busy || (c.reset_when_empty && messages.is_empty()),
                        move || chat.reset(),
                    )),
                    body,
                    Some(composer(&self.chat, c.idle, None)),
                )),
            ))
            .child(caption(c.caption))
    }
}

// ── MessageScrollerPreviousContextDemo ─────────────────────────────────────

const DEFAULT_PEEK: f32 = 64.0;

/// A chat that opens on its first exchange, with a slider for how much of the
/// previous reply stays in view above a new turn.
struct Context {
    chat: DemoChat,
    /// The provider's key, bumped by the demo's reset.
    key: Signal<u32>,
    peek: Signal<Vec<f32>>,
}

impl Context {
    fn new() -> Self {
        Self {
            chat: DemoChat::new(CONTEXT, 2, 35),
            key: Signal::new(0),
            peek: Signal::new(vec![DEFAULT_PEEK]),
        }
    }

    fn reset(&self) {
        self.chat.reset();
        self.peek.set(vec![DEFAULT_PEEK]);
        bump(self.key);
    }

    fn view(&self) -> Div {
        let messages = self.chat.messages.get();
        let busy = self.chat.streaming.get();
        let peek = self.peek.get().first().copied().unwrap_or(DEFAULT_PEEK);
        let props = MessageScrollerProps {
            scroll_margin: 24.0,
            scroll_previous_item_peek: peek,
            ..MessageScrollerProps::default()
        };
        let me = Self {
            chat: self.chat.clone(),
            key: self.key,
            peek: self.peek,
        };
        let amount = div()
            .display(Display::Flex)
            .width(112.0)
            .align_items(Align::Center)
            .gap(8.0)
            .child(
                text(format!("{peek:.0}px"))
                    .text_size(12.0)
                    .line_height(16.0)
                    .text_color(theme().muted_foreground),
            )
            .child(
                slider(Orientation::Horizontal)
                    .model(self.peek)
                    .min(64.0)
                    .max(128.0)
                    .disabled(busy),
            );
        div()
            .position(Position::Relative)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(provider(
                props,
                self.key.get(),
                centred(shell(
                    header(
                        "Keeping Context Visible",
                        "New turns keep part of the previous reply in view.",
                        true,
                    )
                    .child(reset_button(busy, move || me.reset())),
                    turns(messages),
                    Some(composer(
                        &self.chat,
                        "No messages queued. Reset the context.",
                        Some(amount),
                    )),
                )),
            ))
            .child(caption("Adjust the slider and send. Observe the previous message peek"))
    }
}

// ── MessageScrollerAnchoringDemo ───────────────────────────────────────────

/// Turns appended one message at a time, the chosen role anchoring.
#[derive(Clone)]
struct Anchoring {
    messages: Signal<Vec<DemoMessage>>,
    role: Signal<Role>,
    /// The provider's key, bumped by the demo's reset.
    key: Signal<u32>,
}

impl Anchoring {
    fn new() -> Self {
        Self {
            messages: Signal::new(Vec::new()),
            role: Signal::new(Role::User),
            key: Signal::new(0),
        }
    }

    fn next(&self) -> Option<DemoMessage> {
        transcript(ANCHORING).into_iter().nth(self.messages.get().len())
    }

    fn reset(&self) {
        self.messages.set(Vec::new());
        bump(self.key);
    }

    fn send(&self) {
        if let Some(m) = self.next() {
            self.messages.update(|all| {
                all.push(m);
            });
        }
    }

    /// A toggle item. Vue's `onValueChange` takes the first value of the
    /// multiple group's model: pressing the other role keeps this one and
    /// resets the turns; releasing this one changes nothing.
    fn toggle(&self, value: &str, label: &str, role: Role) -> ToggleGroupItem {
        let me = self.clone();
        toggle_group_item(value).child(label.to_owned()).on_click(move || {
            if me.role.get() != role {
                me.reset();
            }
        })
    }

    fn view(&self) -> Div {
        let messages = self.messages.get();
        let role = self.role.get();
        let (reset, send) = (self.clone(), self.clone());
        let body = if messages.is_empty() {
            empty_thread(
                "No anchored messages yet",
                "Send the first message to see the selected role anchor.",
            )
            .into_element()
        } else {
            // Mounted with the first message, as Vue's `v-else` provider.
            provider(
                MessageScrollerProps::default(),
                self.key.get(),
                thread(
                    messages.clone(),
                    move |m: &DemoMessage| (m.id.as_str(), m.role == role),
                    turn_row,
                ),
            )
        };
        div()
            .position(Position::Relative)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(centred(shell(
                header("Anchoring Turns", "Choose which role settles near the top edge.", false).child(
                    card_action().child(
                        button(ButtonVariant::Outline, ButtonSize::Icon)
                            .disabled(messages.is_empty())
                            .child(icon(Icon::RotateCw))
                            .on_click(move |_| reset.reset()),
                    ),
                ),
                body,
                Some(
                    card_footer()
                        .child(
                            toggle_group_multiple()
                                .default_value(vec!["user".to_owned()])
                                .child(self.toggle("user", "User", Role::User))
                                .child(self.toggle("assistant", "Assistant", Role::Assistant)),
                        )
                        .child(send_button(self.next().is_none(), move || send.send())),
                ),
            )))
            .child(
                caption("Toggle the anchor role, then send messages to compare where turns settle.")
                    .margin_x(Length::Auto)
                    .max_width(320.0),
            )
    }
}

// ── MessageScrollerAnimationDemo ───────────────────────────────────────────

const PRESETS: [(&str, &str); 7] = [
    ("fade", "Fade"),
    ("slide-up", "Slide Up"),
    ("slide-side", "Slide Side"),
    ("pop", "Pop"),
    ("spring-bounce", "Spring Bounce"),
    ("blur-fade", "Blur Fade"),
    ("scale-fade", "Scale Fade"),
];

/// A scripted chat with a choice of entrance animation for user turns.
struct Animation {
    chat: DemoChat,
    /// The provider's key, bumped by the demo's reset.
    key: Signal<u32>,
    preset: Signal<Option<String>>,
}

impl Animation {
    fn view(&self) -> Div {
        let messages = self.chat.messages.get();
        let busy = self.chat.streaming.get();
        let (chat, fresh) = (self.chat.clone(), self.key);
        let send = self.chat.clone();
        let body = if messages.is_empty() {
            empty_thread("No Messages Yet", "Click the button below to send the first message.").into_element()
        } else {
            provider(MessageScrollerProps::default(), self.key.get(), turns(messages.clone()))
        };
        let presets =
            select_content().child(select_group().children(PRESETS.iter().map(|(id, name)| select_item(*id, *name))));
        div()
            .position(Position::Relative)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(centred(shell(
                header(
                    "Animation",
                    "Choose how user messages are animated when they are added to the conversation.",
                    false,
                )
                .child(
                    card_action()
                        .display(Display::Flex)
                        .align_items(Align::Center)
                        .gap(8.0)
                        .child(
                            button(ButtonVariant::Outline, ButtonSize::Icon)
                                .disabled(messages.is_empty() || busy)
                                .child(icon(Icon::RotateCw))
                                .on_click(move |_| {
                                    chat.reset();
                                    bump(fresh);
                                }),
                        ),
                ),
                body,
                Some(
                    footer_ruled()
                        .child(select().model(self.preset).child(select_value("")).child(presets))
                        .child(send_button(self.chat.next_message().is_none() || busy, move || {
                            send.send()
                        })),
                ),
            )))
            .child(
                caption("Select an animation then click send to see it in action.")
                    .margin_x(Length::Auto)
                    .max_width(384.0)
                    .text_wrap(WrapStyle::Balance),
            )
    }
}

// ── MessageScrollerCommandsDemo ────────────────────────────────────────────

/// The first 39 characters and an ellipsis, past 42.
fn trimmed(words: &str) -> String {
    if words.chars().count() > 42 {
        format!("{}...", words.chars().take(39).collect::<String>())
    } else {
        words.to_owned()
    }
}

/// The user turns of `t`: their index in it, id and words.
fn user_turns(t: Transcript) -> impl Iterator<Item = (usize, &'static str, &'static str)> {
    t.iter()
        .enumerate()
        .filter(|(_, (_, role, _))| *role == Role::User)
        .map(|(i, (id, _, words))| (i, *id, *words))
}

/// A saved transcript driven from a menu outside it.
struct Commands;

impl Commands {
    fn view(&self) -> Div {
        let messages = transcript(COMMANDS);
        // `CommandMenu`, a child of the provider driving it.
        let menu = scrolled(move |scroller| {
            let jumps = dropdown_menu_content().align(SideAlign::End).width(256.0).child(
                dropdown_menu_group()
                    .child(dropdown_menu_label("Conversations", false))
                    .children(user_turns(COMMANDS).map(|(index, id, words)| {
                        let s = scroller.clone();
                        dropdown_menu_item(id, MenuItem::default())
                            .on_select(move || {
                                s.scroll_to_message(index, MessageScrollerScrollOptions::default());
                            })
                            .child(text(trimmed(words)).line_clamp(1).min_width(0.0))
                    })),
            );
            dropdown_menu()
                .child(dropdown_menu_trigger(
                    button(ButtonVariant::Secondary, ButtonSize::Default).child("Jump to..."),
                ))
                .child(jumps)
                .into_element()
        });
        let props = MessageScrollerProps {
            default_scroll_position: MessageScrollerDefaultScrollPosition::End,
            ..MessageScrollerProps::default()
        };
        div()
            .position(Position::Relative)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(provider(
                props,
                0,
                centred(shell(
                    header("Commands", "Drive the transcript from outside.", true).child(card_action().child(menu)),
                    turns(messages),
                    None,
                )),
            ))
            .child(
                caption("Use the controls to jump to any message in the conversation.")
                    .margin_x(Length::Auto)
                    .max_width(384.0)
                    .text_wrap(WrapStyle::Balance),
            )
    }
}

// ── MessageScrollerLoadHistoryDemo ─────────────────────────────────────────

const INITIAL_VISIBLE_COUNT: usize = 5;

fn history() -> Vec<DemoMessage> {
    HISTORY
        .iter()
        .enumerate()
        .map(|(i, (role, words))| DemoMessage {
            id: format!("history-{i}"),
            role: *role,
            text: (*words).to_owned(),
        })
        .collect()
}

fn recent() -> Vec<DemoMessage> {
    let all = history();
    all.get(all.len().saturating_sub(INITIAL_VISIBLE_COUNT)..)
        .map(<[DemoMessage]>::to_vec)
        .unwrap_or_default()
}

/// The last turns of a conversation, the earlier ones loaded above on demand.
#[derive(Clone)]
struct History {
    messages: Signal<Vec<DemoMessage>>,
    /// The provider's key, bumped by the demo's reset.
    key: Signal<u32>,
}

impl History {
    fn load(&self) {
        self.messages.set(history());
        toast(Toast::new(ToastKind::Default, "History loaded").description("Scroll up to see earlier messages."));
    }

    fn view(&self) -> Div {
        let t = theme();
        let messages = self.messages.get();
        let can_load = messages.len() < HISTORY.len();
        let (reset, load) = (self.clone(), self.clone());
        let rows: Vec<Option<DemoMessage>> = messages.iter().cloned().map(Some).chain([None]).collect();
        div()
            .position(Position::Relative)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(centred(shell(
                header("Load History", "Prepended messages keep your place.", true).child(reset_button(
                    messages.len() <= INITIAL_VISIBLE_COUNT,
                    move || {
                        reset.messages.set(recent());
                        bump(reset.key);
                    },
                )),
                provider(
                    MessageScrollerProps::default(),
                    self.key.get(),
                    thread(
                        rows,
                        |r: &Option<DemoMessage>| r.as_ref().map_or(("end-of-conversation", false), turn_meta),
                        |i, r| match r {
                            Some(m) => turn_row(i, m),
                            None => marker(
                                MarkerVariant::Separator,
                                Element::Empty,
                                marker_content(MarkerVariant::Separator).child("End of Conversation"),
                            ),
                        },
                    ),
                ),
                Some(
                    footer_ruled()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .align_items(Align::Center)
                        .gap(8.0)
                        .child(
                            button(ButtonVariant::Secondary, ButtonSize::Default)
                                .width(pct(100.0))
                                .disabled(!can_load)
                                .child(if can_load { "Load History" } else { "History Loaded" })
                                .on_click(move |_| load.load()),
                        )
                        .child(
                            text("Restore earlier messages while keeping your place.")
                                .text_size(12.0)
                                .line_height(16.0)
                                .text_color(t.muted_foreground),
                        ),
                ),
            )))
            .child(
                caption("Click Load History to load the entire conversation")
                    .margin_x(Length::Auto)
                    .max_width(384.0)
                    .text_wrap(WrapStyle::Balance),
            )
    }
}

// ── MessageScrollerOpeningPositionDemo ─────────────────────────────────────

/// Bring `scroller` to `position`: the start, the end or "open-3".
fn apply(scroller: &MessageScroller, position: &str) {
    match position {
        "start" => scroller.scroll_to_start(),
        "end" => scroller.scroll_to_end(),
        // "open-3".
        _ => scroller.scroll_to_message(
            2,
            MessageScrollerScrollOptions {
                align: MessageScrollerScrollAlign::Start,
                scroll_margin: Some(64.0),
            },
        ),
    };
}

/// The position tabs, a child of the provider: they open the transcript at
/// the last anchor as they mount (Vue's `onMounted(applyPosition)`) and move
/// it as the reader picks another.
fn opening_tabs(position: Signal<String>) -> Element {
    component(position, |cx| {
        let position = cx.props().get_untracked();
        let scroller = shadcn::use_message_scroller();
        if let Some(s) = &scroller {
            apply(s, &position.get_untracked());
        }
        move || {
            let s = scroller.clone();
            let model = Model::bind(position).on_change(move |p| {
                if let Some(s) = &s {
                    apply(s, &p);
                }
            });
            let triggers = ["start", "end", "last-anchor"].map(|p| tabs_trigger(p).child(p));
            tabs()
                .model(model)
                .width(pct(100.0))
                .child(tabs_list().width(pct(100.0)).children(triggers))
                .into_element()
        }
    })
    .into_element()
}

/// A saved transcript opened at the start, the end or its last anchored turn.
fn opening(position: Signal<String>) -> Div {
    let messages = transcript(OPENING);
    div()
        .position(Position::Relative)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(16.0)
        .child(provider(
            MessageScrollerProps::default(),
            0,
            centred(shell(
                header("Opening Position", "Choose where a saved transcript opens.", true),
                turns(messages),
                Some(
                    footer_ruled()
                        .justify_content(Justify::Center)
                        .child(opening_tabs(position)),
                ),
            )),
        ))
        .child(
            caption("Toggle the defaultScrollPosition to see where the transcript starts when you open the thread")
                .margin_x(Length::Auto)
                .max_width(384.0),
        )
}

// ── MessageScrollerScrollableDemo ──────────────────────────────────────────

fn checkpoints() -> Vec<DemoMessage> {
    (1..=12_usize)
        .map(|n| {
            let user = n % 2 == 1;
            DemoMessage {
                id: format!("scrollable-{n}"),
                role: if user { Role::User } else { Role::Assistant },
                text: if user {
                    format!("Review scroll checkpoint {n}.")
                } else {
                    format!(
                        "Checkpoint {n} is synced. The scrollable hook updates as the viewport moves.\n\nWhen the reader is at the first message, the footer should only point them down. Once they move into the middle of the transcript, it should explain that both directions are available.\n\nAt the latest message, the footer should switch again and only point them back up."
                    )
                },
            }
        })
        .collect()
}

fn scroll_status(s: MessageScrollerScrollable) -> &'static str {
    match (s.start, s.end) {
        (true, true) => "You can scroll both ways.",
        (false, true) => "You are at the top. You can only scroll down.",
        (true, false) => "You are at the bottom. You can only scroll up.",
        (false, false) => "All messages fit in the viewport.",
    }
}

fn scrollable() -> Div {
    let t = theme();
    let messages = checkpoints();
    let props = MessageScrollerProps {
        default_scroll_position: MessageScrollerDefaultScrollPosition::Start,
        ..MessageScrollerProps::default()
    };
    let card = shell(
        header(
            "Scroll Status",
            "Where the reader can scroll to based on current scroll position.",
            true,
        ),
        message_scroller()
            .child(message_scroller_viewport(messages, turn_meta, turn_row).gap(16.0))
            .child(message_scroller_button(MessageScrollerDirection::End)),
        Some(
            footer_ruled()
                .justify_content(Justify::Center)
                .text_align(TextAlign::Center)
                .text_size(14.0)
                .line_height(20.0)
                .text_color(t.muted_foreground)
                .child(scrolled(|s| text(scroll_status(s.scrollable())).into_element())),
        ),
    )
    .overflow(Overflow::Hidden);
    div()
        .margin_x(Length::Auto)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .max_width(384.0)
        .gap(16.0)
        .child(provider(props, 0, card))
        .child(caption("Scroll the transcript to see the footer update."))
}

// ── MessageScrollerVisibilityDemo ──────────────────────────────────────────

/// A transcript with an outline beside it marking the turn being read.
struct Visibility;

impl Visibility {
    /// `TranscriptOutline`: a bar per user turn, the current one dark; hovered,
    /// a card listing them to jump to.
    fn outline(&self, scroller: &MessageScroller) -> gui::Element {
        let t = theme();
        let current = scroller.visibility().current_anchor;
        let is_current = |index: usize| current == Some(index);
        let bars = div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .size(36.0)
            .align_items(Align::Center)
            .justify_content(Justify::Center)
            .gap(4.0)
            .radius(8.0)
            .children(user_turns(VISIBILITY).map(|(index, _, _)| {
                div()
                    .height(2.0)
                    .width(16.0)
                    .radius(9999.0)
                    .background(if is_current(index) {
                        t.foreground
                    } else {
                        t.muted_foreground.opacity(0.4)
                    })
            }));
        let card = hover_card_content()
            .side(Side::Left)
            .align(SideAlign::Center)
            .side_offset(-28.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .width(256.0)
            .gap(4.0)
            .radius(18.0)
            .padding(4.0)
            .children(user_turns(VISIBILITY).map(|(index, _, words)| {
                let s = scroller.clone();
                div()
                    .display(Display::Flex)
                    .min_height(28.0)
                    .align_items(Align::Center)
                    .radius(14.0)
                    .padding_x(8.0)
                    .padding_y(6.0)
                    .text_size(14.0)
                    .line_height(20.0)
                    .when(is_current(index), |d| {
                        d.background(t.accent).text_color(t.accent_foreground)
                    })
                    .with_interaction(|d, i| {
                        d.background_while(move || i.hovered(), t.accent)
                            .text_color_while(move || i.hovered(), t.accent_foreground)
                    })
                    .child(text(trimmed(words)).line_clamp(1).min_width(0.0))
                    .on_click(move |_| {
                        s.scroll_to_message(index, MessageScrollerScrollOptions::default());
                    })
            }));
        hover_card().child(hover_card_trigger(bars)).child(card).into_element()
    }

    fn view(&self) -> Div {
        let messages = transcript(VISIBILITY);
        let props = MessageScrollerProps {
            scroll_margin: 12.0,
            ..MessageScrollerProps::default()
        };
        let me = Self;
        div()
            .position(Position::Relative)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(provider(
                props,
                0,
                div()
                    .position(Position::Relative)
                    .margin_x(Length::Auto)
                    .width(pct(100.0))
                    .max_width(384.0)
                    .child(shell(
                        header("Transcript Outline", "Track the current anchored turn.", true),
                        turns(messages),
                        None,
                    ))
                    // `absolute top-1/2 -right-12 -translate-y-1/2` around the 36px trigger.
                    .child(
                        div()
                            .position(Position::Absolute)
                            .top(gui::Length::Fraction(0.5))
                            .right(-48.0)
                            .margin_top(-18.0)
                            .child(scrolled(move |s| me.outline(s).into_element())),
                    ),
            ))
            .child(
                caption("Open the outline to jump between anchored turns as you read.")
                    .margin_x(Length::Auto)
                    .max_width(384.0),
            )
    }
}

// ── MessageScrollerGroupChatDemo ───────────────────────────────────────────

const ME: &str = "Grace";

/// Who said what, or a line in the thread.
enum Item {
    Said {
        id: &'static str,
        sender: &'static str,
        assistant: bool,
        words: &'static str,
        anchor: bool,
    },
    Event {
        id: &'static str,
        words: &'static str,
    },
}

const OPENING_GROUP: [Item; 3] = [
    Item::Said {
        id: "group-1",
        sender: "Grace",
        assistant: false,
        words: "@mary, the astrophage line keeps matching Venus energy output. Can you check my math?",
        anchor: false,
    },
    Item::Said {
        id: "group-2",
        sender: "Mary (Agent)",
        assistant: true,
        words: "Yes. Confirmed. The curve points to a microorganism harvesting stellar energy and breeding near carbon dioxide. If @rocky agrees, this is the clue we need.",
        anchor: false,
    },
    Item::Said {
        id: "group-3",
        sender: "Grace",
        assistant: false,
        words: "ping @rocky",
        anchor: true,
    },
];
const JOINED: Item = Item::Event {
    id: "group-4",
    words: "Rocky has joined the chat",
};
const ROCKY: Item = Item::Said {
    id: "group-5",
    sender: "Rocky",
    assistant: false,
    words: "Amaze. Astrophage eats light, makes heat, goes to carbon dioxide. Rocky has fuel model. Grace is smart.",
    anchor: false,
};

/// An item's id, and whether it anchors: a marker always does.
fn group_meta<'a>(item: &'a &'static Item) -> (&'a str, bool) {
    match **item {
        Item::Event { id, .. } => (id, true),
        Item::Said { id, anchor, .. } => (id, anchor),
    }
}

fn group_row(item: &Item) -> Div {
    match *item {
        Item::Event { words, .. } => marker(
            MarkerVariant::Separator,
            Element::Empty,
            marker_content(MarkerVariant::Separator).child(words),
        ),
        Item::Said {
            sender,
            assistant,
            words,
            ..
        } => {
            let mine = sender == ME;
            let variant = match (mine, assistant) {
                (true, _) => BubbleVariant::Muted,
                (false, true) => BubbleVariant::Ghost,
                (false, false) => BubbleVariant::Tinted,
            };
            let align = if mine { BubbleAlign::End } else { BubbleAlign::Start };
            let p = MessageProps {
                align,
                bubble: variant,
                ..MessageProps::default()
            };
            let b = BubbleProps { variant, align };
            message(p).child(
                message_content(p)
                    .child((!mine).then(|| message_header(p).child(sender)))
                    .child(bubble(b).child(bubble_content(b).child(words))),
            )
        }
    }
}

/// A group chat where a participant joining is a marker the thread anchors on.
#[derive(Clone)]
struct GroupChat {
    /// 0: idle, 1: Rocky joined, 2: Rocky replied.
    turn: Signal<u8>,
    /// The provider's key, bumped by the demo's reset.
    key: Signal<u32>,
}

impl GroupChat {
    fn view(&self) -> Div {
        let t = theme();
        let step = self.turn.get();
        let items: Vec<&'static Item> = OPENING_GROUP
            .iter()
            .chain((step >= 1).then_some(&JOINED))
            .chain((step >= 2).then_some(&ROCKY))
            .collect();
        let (advance, reset) = (self.clone(), self.clone());
        div()
            .position(Position::Relative)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(centred(shell(
                header(
                    "Group Chat",
                    "A group chat with several participants and an assistant. The Marker is marked as a turn.",
                    true,
                )
                .child(reset_button(step == 0, move || {
                    reset.turn.set(0);
                    bump(reset.key);
                })),
                provider(
                    MessageScrollerProps::default(),
                    self.key.get(),
                    thread(items, group_meta, |_, i| group_row(i)),
                ),
                Some(
                    footer_ruled()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .align_items(Align::Center)
                        .gap(8.0)
                        .child(
                            button(ButtonVariant::Secondary, ButtonSize::Default)
                                .width(pct(100.0))
                                .disabled(step >= 2)
                                .child(if step == 0 {
                                    "Add Rocky"
                                } else {
                                    "Send Message as Rocky"
                                })
                                .on_click(move |_| advance.turn.update(|t| *t = t.saturating_add(1).min(2))),
                        )
                        .child(
                            text(if step == 0 {
                                "This will create a marker and make it the anchor"
                            } else {
                                "Now send Rocky's reply into the conversation"
                            })
                            .text_size(12.0)
                            .line_height(16.0)
                            .text_color(t.muted_foreground),
                        ),
                ),
            )))
            .child(
                caption("When a user joins, a marker is created. scrollAnchor on the marker marks it as the next turn")
                    .margin_x(Length::Auto)
                    .max_width(384.0)
                    .text_wrap(WrapStyle::Balance),
            )
    }
}

// ── The page ───────────────────────────────────────────────────────────────

pub fn render() -> Render {
    let basic = Chat {
        chat: DemoChat::new(CHAT, 0, 20),
        props: MessageScrollerProps::default(),
        copy: Copy {
            title: "New Chat",
            description: "How can I help you today?",
            empty_title: "Morning, shadcn!",
            empty_description: "What are we working on today? Press send to start a new conversation",
            idle: "No messages queued. Reset the conversation.",
            caption: "Demo is read only. Press send to send messages.",
            reset_when_empty: false,
        },
    };
    let streaming = Chat {
        chat: DemoChat::new(STREAMING, 0, 20),
        props: MessageScrollerProps {
            auto_scroll: true,
            ..MessageScrollerProps::default()
        },
        copy: Copy {
            title: "Streaming Messages",
            description: "Auto-scroll follows the live edge of the conversation.",
            empty_title: "Ready to Stream",
            empty_description: "Press send to stream a scripted launch summary.",
            idle: "No messages queued. Reset the stream.",
            caption: "Streaming is simulated. `autoScroll` is enabled.",
            reset_when_empty: true,
        },
    };
    let context = Context::new();
    let anchoring = Anchoring::new();
    let animation = Animation {
        chat: DemoChat::new(ANIMATION, 0, 15),
        key: Signal::new(0),
        preset: Signal::new(Some("fade".to_owned())),
    };
    let commands = Commands;
    let history = History {
        messages: Signal::new(recent()),
        key: Signal::new(0),
    };
    let position = Signal::labeled("opening position", "last-anchor".to_owned());
    let visibility = Visibility;
    let group = GroupChat {
        turn: Signal::labeled("rocky turn", 0_u8),
        key: Signal::new(0),
    };
    Box::new(move || {
        page(
            "Message scroller",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(40.0)
                .child(demo("MessageScrollerDemo.vue", basic.view()))
                .child(demo("MessageScrollerStreamingDemo.vue", streaming.view()))
                .child(demo("MessageScrollerPreviousContextDemo.vue", context.view()))
                .child(demo("MessageScrollerAnchoringDemo.vue", anchoring.view()))
                .child(demo("MessageScrollerAnimationDemo.vue", animation.view()))
                .child(demo("MessageScrollerCommandsDemo.vue", commands.view()))
                .child(demo("MessageScrollerLoadHistoryDemo.vue", history.view()))
                .child(demo("MessageScrollerOpeningPositionDemo.vue", opening(position)))
                .child(demo("MessageScrollerScrollableDemo.vue", scrollable()))
                .child(demo("MessageScrollerVisibilityDemo.vue", visibility.view()))
                .child(demo("MessageScrollerGroupChatDemo.vue", group.view()))
                .child(toaster()),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    // A turn sent: it settles near the top, its reply streams in below.
    click_named(s, "MessageScrollerDemo.vue", "input-group-button")?;
    wait(s, 600);
    s.frame("chat sent, streaming")?;
    wait(s, 6000);
    click_named(s, "MessageScrollerDemo.vue", "input-group-button")?;
    wait(s, 400);
    s.frame("second turn anchored with a peek")?;
    // Auto-scroll follows the stream.
    click_named(s, "MessageScrollerStreamingDemo.vue", "input-group-button")?;
    wait(s, 2500);
    s.frame("streaming followed")?;
    click_named(s, "MessageScrollerLoadHistoryDemo.vue", "button")?;
    s.frame("history loaded above")?;
    s.click_text("start")?;
    s.frame("opened at the start")?;
    s.click_text("Jump to...")?;
    s.frame("jump menu")?;
    s.click_text("What should I compare before we change ...")?;
    s.frame("jumped")?;
    let (x, y) = s.locate("Review scroll checkpoint 1.")?;
    s.hover(x, y);
    s.wheel(600.0);
    s.frame("scrolled to the middle")?;
    s.click_text("Add Rocky")?;
    s.frame("rocky joined")?;
    Ok(())
}

// ── Conversations (from the shadcn-vue demos) ──────────────────────────────
const CHAT: Turns = &[
    (
        Role::User,
        "I'm building a chat for our app and the scroll behavior is driving me nuts. Every time the AI streams a reply, the whole thread jumps around.",
    ),
    (
        Role::Assistant,
        "That's the classic streaming scroll problem. Wrap your message list in `MessageScroller` and turn on `autoScroll` — the viewport pins to the bottom as tokens arrive, so users always see the latest text land in place.\n\nThe important part: it only auto-scrolls while the reader is already at the bottom. The moment they scroll up to read something earlier, auto-scroll backs off and their position is preserved.",
    ),
    (
        Role::User,
        "Okay, but when someone sends a new message the view still feels jarring — like the whole conversation reloads from the top.",
    ),
    (
        Role::Assistant,
        "MessageScrollerItem fixes that with turn anchoring. Set `scrollAnchor` on the turn that should settle near the top instead of blindly snapping to the document bottom.\n\nIt also leaves a small peek of the previous exchange visible above the anchor, so context isn't lost. The reply starts in view without that disorienting jump you get from a plain overflow container.",
    ),
    (
        Role::User,
        "And if they've scrolled up to re-read an older answer? I don't want to yank them back down.",
    ),
    (
        Role::Assistant,
        "You won't. Auto-scroll only runs when the viewport is already pinned to the bottom, so scrolling up is a deliberate opt-out — their place in the thread stays put even as new tokens keep arriving below.\n\nWhen there is content they haven't seen yet, `MessageScrollerButton` appears at the bottom of the viewport. One tap jumps them back to the newest message and re-engages auto-scroll.",
    ),
    (Role::User, "Last one — does this work with assistive tech?"),
    (
        Role::Assistant,
        "`MessageScrollerContent` sets `role=\"log\"` and `aria-relevant=\"additions\"` by default, so screen readers announce new messages as they stream in.\n\nThe scroll button is a real `<button>` with an sr-only label, and it's removed from the tab order when you're already at the bottom — no ghost focus stops.",
    ),
];

const STREAMING: Turns = &[
    (
        Role::User,
        "I'm building a chat for our app and the scroll behavior is driving me nuts. Every time the AI streams a reply, the whole thread jumps around.",
    ),
    (
        Role::Assistant,
        "That's the classic streaming scroll problem. Wrap your message list in `MessageScroller` and turn on `autoScroll` — the viewport pins to the bottom as tokens arrive, so users always see the latest text land in place.\n\nThe important part: it only auto-scrolls while the reader is already at the bottom. The moment they scroll up to read something earlier, auto-scroll backs off and their position is preserved. You get smooth streaming without fighting the user's intent.",
    ),
    (
        Role::User,
        "Okay, but when someone sends a new message the view still feels jarring — like the whole conversation reloads from the top.",
    ),
    (
        Role::Assistant,
        "MessageScrollerItem fixes that with turn anchoring. Set `scrollAnchor` on the turn that should settle near the top instead of blindly snapping to the document bottom.\n\nIt also leaves a small peek of the previous exchange visible above the anchor, so context isn't lost. The reply starts in view without that disorienting jump you get from a plain overflow container.",
    ),
    (
        Role::User,
        "And if they've scrolled up to re-read an older answer? I don't want to yank them back down.",
    ),
    (
        Role::Assistant,
        "You won't. Auto-scroll only runs when the viewport is already pinned to the bottom, so scrolling up is a deliberate opt-out — their place in the thread stays put even as new tokens keep arriving below.\n\nWhen there is content they haven't seen yet, `MessageScrollerButton` appears at the bottom of the viewport. One tap jumps them back to the newest message and re-engages auto-scroll. Same pattern as Slack or iMessage: quiet when you're caught up, helpful when you're not.",
    ),
    (Role::User, "Last one — does this work with assistive tech?"),
    (
        Role::Assistant,
        "`MessageScrollerContent` sets `role=\"log\"` and `aria-relevant=\"additions\"` by default, so screen readers announce new messages as they stream in.\n\nThe scroll button is a real `<button>` with an sr-only label, and it's removed from the tab order when you're already at the bottom — no ghost focus stops.",
    ),
];

const CONTEXT: Turns = &[
    (
        Role::User,
        "I'm building a chat for our app and the scroll behavior is driving me nuts. Every time the AI streams a reply, the whole thread jumps around.",
    ),
    (
        Role::Assistant,
        "That's the classic streaming scroll problem. Wrap your message list in `MessageScroller` and turn on `autoScroll` — the viewport pins to the bottom as tokens arrive, so users always see the latest text land in place.\n\nThe important part: it only auto-scrolls while the reader is already at the bottom. The moment they scroll up to read something earlier, auto-scroll backs off and their position is preserved. You get smooth streaming without fighting the user's intent.",
    ),
    (
        Role::User,
        "Okay, but when someone sends a new message the view still feels jarring — like the whole conversation reloads from the top.",
    ),
    (
        Role::Assistant,
        "MessageScrollerItem fixes that with turn anchoring. Set `scrollAnchor` on the turn that should settle near the top instead of blindly snapping to the document bottom.\n\nIt also leaves a small peek of the previous exchange visible above the anchor, so context isn't lost. The reply starts in view without that disorienting jump you get from a plain overflow container.",
    ),
    (
        Role::User,
        "And if they've scrolled up to re-read an older answer? I don't want to yank them back down.",
    ),
    (
        Role::Assistant,
        "You won't. Auto-scroll only runs when the viewport is already pinned to the bottom, so scrolling up is a deliberate opt-out — their place in the thread stays put even as new tokens keep arriving below.\n\nWhen there is content they haven't seen yet, `MessageScrollerButton` appears at the bottom of the viewport. One tap jumps them back to the newest message and re-engages auto-scroll. Same pattern as Slack or iMessage: quiet when you're caught up, helpful when you're not.",
    ),
    (Role::User, "Last one — does this work with assistive tech?"),
    (
        Role::Assistant,
        "`MessageScrollerContent` sets `role=\"log\"` and `aria-relevant=\"additions\"` by default, so screen readers announce new messages as they stream in.\n\nThe scroll button is a real `<button>` with an sr-only label, and it's removed from the tab order when you're already at the bottom — no ghost focus stops.",
    ),
];

const ANIMATION: Turns = &[
    (
        Role::User,
        "Can user messages pop in like iMessage without breaking anchoring?",
    ),
    (
        Role::Assistant,
        "Yes. Animate the user row with transform and opacity, and let the assistant response stream normally below it.\n\nThat keeps the row measurement predictable while still giving the newly sent bubble a more tactile entrance.",
    ),
    (Role::User, "What makes the animation feel more like iMessage?"),
    (
        Role::Assistant,
        "Use a quick spring from the trailing edge: a little scale, a small upward move, and no layout animation.\n\nThe bubble feels tactile, but the measured row stays predictable, so anchoring and auto-scroll do not have to fight a changing layout.",
    ),
    (
        Role::User,
        "Can I switch between presets while testing the same thread?",
    ),
    (
        Role::Assistant,
        "Yes. Keep the conversation in place while you change the preset, then send the next message to compare the new entrance against the same context.\n\nThat makes it easier to judge the difference between a subtle fade, a snappy pop, and a more dramatic 3D tilt without rebuilding the scenario each time.",
    ),
];

const HISTORY: Turns = &[
    (Role::User, "Can you summarize the incident channel?"),
    (
        Role::Assistant,
        "The first alert was a delayed export job. It started backing up around 09:42 UTC and triggered the warning once the retry queue crossed the threshold.\n\nNo customer-facing checkout paths were affected, but exports for larger workspaces were running about 12 minutes behind.",
    ),
    (Role::User, "Was checkout affected?"),
    (
        Role::Assistant,
        "No checkout errors were reported. Payment authorization, order creation, and confirmation emails stayed inside their normal latency bands.\n\nThe only elevated metric was export queue depth, which maps to analytics downloads instead of checkout.",
    ),
    (Role::User, "What changed in the last deploy?"),
    (
        Role::Assistant,
        "Only the export queue worker changed. The deploy moved large CSV jobs onto the shared retry policy, which made each failed attempt hold a worker slot longer than before.\n\nThe app deploy did not include checkout, pricing, or billing API changes.",
    ),
    (Role::User, "Do we need to roll back?"),
    (
        Role::Assistant,
        "Not yet. Queue depth is recovering after we reduced retry concurrency, and the oldest pending job is now under five minutes old.\n\nKeep rollback ready if the queue starts climbing again, but the current trend points toward recovery.",
    ),
    (Role::User, "Keep watching for customer-visible issues."),
    (
        Role::Assistant,
        "I will watch the queue and support tags for another 15 minutes. I am tracking export failures, delayed download requests, and any support thread that mentions missing reports.\n\nIf those stay quiet through the next batch window, we can close this as an internal degradation.",
    ),
];

const OPENING: Transcript = &[
    (
        "open-1",
        Role::User,
        "This is the first message the user sent in the conversation.",
    ),
    (
        "open-2",
        Role::Assistant,
        "Workspace creation rose 8%, but first invite completion only rose 2%.",
    ),
    (
        "open-3",
        Role::User,
        "This is the last message the user sent in the conversation.",
    ),
    (
        "open-4",
        Role::Assistant,
        "Start with the invite step. Teams are creating workspaces but waiting to add collaborators.\n\nRecommended follow-up:\n\n1. Compare invite drop-off by account size.\n2. Check whether users who skip invites still return within 24 hours.\n3. Review the empty-state copy on the first project screen.\n4. Segment activation by template, since template users may not need invites right away.\n\nIf that pattern holds, the next experiment should make collaboration useful earlier instead of prompting for invites harder.",
    ),
];

const COMMANDS: Transcript = &[
    (
        "command-activation",
        Role::User,
        "We're seeing activation dip after workspace creation. Can you help me find the likely step?",
    ),
    (
        "command-activation-reply",
        Role::Assistant,
        "The sharpest drop is between creating the workspace and inviting the first teammate.\n\nWorkspace creation is still healthy, but the invite step is where users pause. That suggests the product is asking for collaboration before the user has enough confidence in the workspace.",
    ),
    (
        "command-compare",
        Role::User,
        "What should I compare before we change the onboarding flow?",
    ),
    (
        "command-compare-reply",
        Role::Assistant,
        "Compare three cohorts:\n\n1. Users who choose a template before inviting teammates.\n2. Users who start from a blank workspace.\n3. Users who skip invites and return within 24 hours.\n\nIf template users invite faster, the fix is probably better first-run guidance rather than a louder invite prompt.",
    ),
    (
        "command-experiment",
        Role::User,
        "Can you turn that into an experiment?",
    ),
    (
        "command-experiment-reply",
        Role::Assistant,
        "Yes. Create a variant that shows a short checklist after workspace creation:\n\n- Pick a template.\n- Add one project detail.\n- Invite a teammate when the workspace has context.\n\nMeasure first invite completion, 24-hour return rate, and whether teams create a second project.",
    ),
    (
        "command-risk",
        Role::User,
        "What's the risk if we delay the invite prompt?",
    ),
    (
        "command-risk-reply",
        Role::Assistant,
        "The main risk is reducing team creation for accounts that already know who they want to invite.\n\nTo protect that path, keep the invite action visible in the header and only change the primary empty-state guidance. That gives confident teams a direct route without forcing uncertain users through the invite step too early.",
    ),
];

const VISIBILITY: Transcript = &[
    (
        "vis-brief",
        Role::User,
        "Review the incident handoff and tell me what to read first.",
    ),
    (
        "vis-brief-reply",
        Role::Assistant,
        "Start with the summary and the impact section. The regression affected the upload queue, but the recovery path completed for every queued job.",
    ),
    ("vis-impact", Role::User, "What was the customer impact?"),
    (
        "vis-impact-reply",
        Role::Assistant,
        "Impact was limited to delayed processing.\n\nNo records were dropped, and the reconciliation worker confirmed each retry batch. Support saw confusion from two customers, but there were no checkout or billing errors.",
    ),
    ("vis-actions", Role::User, "What actions are open?"),
    (
        "vis-actions-reply",
        Role::Assistant,
        "Keep the retry window enabled until the next deploy, then add a queue-depth alert as the long-term fix.\n\nThe alert should fire on sustained queue growth, not a single short spike.",
    ),
    ("vis-checklist", Role::User, "Give me the follow-up checklist."),
    (
        "vis-checklist-reply",
        Role::Assistant,
        "After that, compare the queue recovery graph with the deploy timeline so the handoff shows exactly when processing returned to baseline. That makes it easier for support and engineering to answer the same customer questions without re-reading the whole incident thread.\n\nI would also add a short owner note beside each follow-up item. The checklist is small, but ownership keeps the retry-window decision, alert tuning, and support macro from drifting into separate follow-up conversations.\n\nKeep the retry window enabled until the next deploy, then add a queue-depth alert as the long-term fix.\n\nThe alert should fire on sustained queue growth, not a single short spike.",
    ),
];

const ANCHORING: Transcript = &[
    (
        "anchor-1-user",
        Role::User,
        "Can you show me how anchoring behaves when a new prompt starts the turn?",
    ),
    (
        "anchor-1-assistant",
        Role::Assistant,
        "Append the user prompt first, then append the assistant response. With User selected, the prompt settles near the top and the assistant response fills in below it.",
    ),
    (
        "anchor-2-user",
        Role::User,
        "What changes when assistant messages are the anchor?",
    ),
    (
        "anchor-2-assistant",
        Role::Assistant,
        "Now each assistant response is the item `MessageScroller` keeps in view. This is useful when the reply is the moment you want readers to land on after each turn.",
    ),
    ("anchor-3-user", Role::User, "Can I switch roles and keep adding turns?"),
    (
        "anchor-3-assistant",
        Role::Assistant,
        "Yes. The next appended message with the selected role becomes the anchor, so you can compare user and assistant anchoring without resetting the demo.",
    ),
];
