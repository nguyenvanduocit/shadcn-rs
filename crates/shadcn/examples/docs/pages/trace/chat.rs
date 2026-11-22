//! The trace demos that play a conversation: an agent thinks, runs tools and
//! answers, streamed by timers in place of a model (Vue's simulated `Chat`).

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gui::prelude::*;
use gui::{Div, Element, Icon, Signal, div, icon, text};
use shadcn::{
    BubbleAlign, BubbleProps, BubbleVariant, Button, ButtonSize, ButtonVariant, CodeBlockVariant, MessageProps,
    TraceStatus, bubble, bubble_content, button, message, message_content, spinner, trace, trace_collapsible,
    trace_collapsible_content, trace_collapsible_trigger, trace_icon, trace_line, trace_title,
};

use super::{output, sources_trace, step};

/// What a tool call has come to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Call {
    Running,
    Output(&'static str),
    Error(&'static str),
}

/// One part of a message, as the AI SDK streams it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Reasoning { text: String, streaming: bool },
    Tool { command: &'static str, call: Call },
    Source { title: &'static str },
}

#[derive(Debug, Clone)]
struct Msg {
    id: String,
    user: bool,
    parts: Vec<Part>,
}

impl Msg {
    fn user(id: String, question: &str) -> Self {
        Self {
            id,
            user: true,
            parts: vec![Part::Text(question.to_owned())],
        }
    }

    fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// A conversation and the stream writing into it.
#[derive(Clone)]
struct Chat {
    messages: Signal<Vec<Msg>>,
    busy: Signal<bool>,
    /// Bumped by every send, so the steps of an older stream stop.
    run: Rc<Cell<u64>>,
    /// The collapsible rows a reader or the stream opened or closed, by
    /// `<message id>-<row>`; a row not in it shows its default. One signal
    /// made with the chat, so a render only reads it.
    open: Signal<HashMap<String, bool>>,
}

/// One step of a stream: wait, then change the chat.
type Step = (u64, Rc<dyn Fn(&Chat)>);

impl Chat {
    fn new(messages: Vec<Msg>) -> Self {
        Self {
            messages: Signal::labeled("chat messages", messages),
            busy: Signal::labeled("chat busy", false),
            run: Rc::new(Cell::new(0)),
            open: Signal::labeled("chat rows open", HashMap::new()),
        }
    }

    /// Whether row `key` is open: `default` until it is opened or closed.
    fn is_open(&self, key: &str, default: bool) -> bool {
        self.open.with(|o| o.get(key).copied().unwrap_or(default))
    }

    /// Open or close row `key`.
    fn set_open(&self, key: &str, open: bool) {
        self.open.update(|o| {
            o.insert(key.to_owned(), open);
        });
    }

    /// A collapsible row whose open state the chat keeps under `key`.
    fn row(&self, key: &str, default: bool) -> shadcn::Part<shadcn::CollapsibleProps> {
        let (chat, key_owned) = (self.clone(), key.to_owned());
        trace_collapsible()
            .open(self.is_open(key, default))
            .on_change(move |open| chat.set_open(&key_owned, open))
    }

    /// Start a new stream: `messages` now, then `steps` one after another.
    fn play(&self, messages: Vec<Msg>, steps: Vec<Step>) {
        let id = self.run.get().wrapping_add(1);
        self.run.set(id);
        self.open.update(HashMap::clear);
        self.messages.replace(messages);
        self.busy.set(true);
        next(self.clone(), id, Rc::new(steps), 0);
    }

    /// Stop any stream and clear the conversation.
    fn reset(&self) {
        self.run.set(self.run.get().wrapping_add(1));
        self.open.update(HashMap::clear);
        self.messages.replace(Vec::new());
        self.busy.set(false);
    }

    /// Change the parts of the last message.
    fn edit_last(&self, f: impl FnOnce(&mut Vec<Part>)) {
        self.messages.update(|m| {
            if let Some(last) = m.last_mut() {
                f(&mut last.parts);
            }
        });
    }

    /// Whether `m` is the message being written.
    fn live(&self, m: &Msg) -> bool {
        self.busy.get() && self.messages.get().last().is_some_and(|l| l.id == m.id)
    }

    fn last_id(&self) -> String {
        self.messages.get().last().map(|m| m.id.clone()).unwrap_or_default()
    }
}

fn next(chat: Chat, id: u64, steps: Rc<Vec<Step>>, i: usize) {
    let Some((ms, _)) = steps.get(i) else {
        return;
    };
    gui::clock::after(Duration::from_millis(*ms), move || {
        if chat.run.get() != id {
            return;
        }
        if let Some((_, f)) = steps.get(i) {
            f(&chat);
        }
        next(chat.clone(), id, steps.clone(), i.saturating_add(1));
    });
}

fn step_of(ms: u64, f: impl Fn(&Chat) + 'static) -> Step {
    (ms, Rc::new(f))
}

/// Steps that append `words` to the last part, `ms` apart, a word at a time.
fn stream_words(words: &'static str, ms: u64) -> impl Iterator<Item = Step> {
    words.split_inclusive(' ').map(move |chunk| {
        step_of(ms, move |c| {
            c.edit_last(|parts| {
                if let Some(Part::Text(t) | Part::Reasoning { text: t, .. }) = parts.last_mut() {
                    t.push_str(chunk);
                }
            });
        })
    })
}

/// The column the conversation sits in: `flex min-h-72 w-full max-w-sm
/// flex-col gap-4 py-6 text-sm`.
fn thread() -> Div {
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .min_height(288.0)
        .width(pct(100.0))
        .max_width(384.0)
        .gap(16.0)
        .padding_y(24.0)
        .text_size(14.0)
        .line_height(20.0)
}

fn user_message(m: &Msg) -> Div {
    let p = MessageProps {
        align: BubbleAlign::End,
        ..MessageProps::default()
    };
    let b = BubbleProps {
        variant: BubbleVariant::Muted,
        align: BubbleAlign::End,
    };
    message(p).child(message_content(p).child(bubble(b).child(bubble_content(b).child(text(m.text())))))
}

/// The assistant's turn: `MessageContent class="gap-4"` around `body`.
fn assistant_message(body: impl IntoIterator<Item = Element>) -> Div {
    let p = MessageProps::default();
    message(p).child(message_content(p).gap(16.0).children(body))
}

/// The outline button under the thread (`mt-auto self-start`).
fn thread_button(label: &str, f: impl Fn() + 'static) -> Button {
    button(ButtonVariant::Outline, ButtonSize::Sm)
        .margin_top(Length::Auto)
        .align_self(Align::Start)
        .child(label.to_owned())
        .on_click(move |_| f())
}

/// How a finished command's step looks: whether it opens at first, its
/// icon (`None`: a check, or an alert when it failed), and the code block
/// variant its error output shows in.
#[derive(Clone, Copy)]
struct ToolLook {
    default_open: bool,
    glyph: Option<Icon>,
    error: CodeBlockVariant,
}

/// A command's step once it has run: opens to its output.
fn tool_row(chat: &Chat, key: &str, look: ToolLook, command: &str, call: Call) -> Element {
    let failed = matches!(call, Call::Error(_));
    let (out, variant) = match call {
        Call::Output(o) => (o, CodeBlockVariant::Default),
        Call::Error(e) => (e, look.error),
        Call::Running => ("", CodeBlockVariant::Default),
    };
    let ToolLook {
        default_open, glyph, ..
    } = look;
    let glyph = glyph.unwrap_or(if failed { Icon::CircleAlert } else { Icon::CircleCheck });
    let title = format!("{} {command}", if failed { "Failed to run" } else { "Ran" });
    chat.row(key, default_open)
        .child(trace_collapsible_trigger(
            failed,
            step(failed.then_some(TraceStatus::Error), icon(glyph), &title),
        ))
        .child(trace_collapsible_content().map_root(|c| c.child(output(out, variant))))
        .into_element()
}

/// A command still running, or cut off when the stream stopped.
fn pending_tool(live: bool, glyph: Option<Icon>, command: &str) -> Element {
    let status = if live { TraceStatus::Running } else { TraceStatus::Error };
    let mark: Element = match (glyph, live) {
        (Some(g), _) => icon(g).into_element(),
        (None, true) => spinner().into_element(),
        (None, false) => icon(Icon::CircleAlert).into_element(),
    };
    let verb = if live { "Running" } else { "Failed to run" };
    trace_line(Some(status))
        .child(trace_icon(mark))
        .child(trace_title(format!("{verb} {command}")))
        .into_element()
}

/// A thinking step: `glyph` when done, `running` while streaming.
fn thinking_line(running: bool, done_glyph: Icon, running_glyph: Option<Icon>, done: &str) -> Div {
    let mark: Element = match (running, running_glyph) {
        (true, Some(g)) => icon(g).into_element(),
        (true, None) => spinner().into_element(),
        (false, _) => icon(done_glyph).into_element(),
    };
    trace_line(running.then_some(TraceStatus::Running))
        .child(trace_icon(mark))
        .child(trace_title(if running { "Thinking" } else { done }))
}

/// The reasoning row: a line until there is text, then a step that opens to
/// it (open while it streams, unless the reader toggled it).
fn reasoning_row(chat: &Chat, key: &str, text_so_far: &str, line: Div) -> Element {
    if text_so_far.is_empty() {
        return line.into_element();
    }
    chat.row(key, false)
        .child(trace_collapsible_trigger(false, line))
        .child(trace_collapsible_content().map_root(|c| c.child(text(text_so_far.to_owned()))))
        .into_element()
}

const TRACE_QUESTION: &str = "Why does pnpm install fail in CI?";
const TRACE_REASONING: &str = "CI installs with a frozen lockfile, so the install stops when package.json and pnpm-lock.yaml disagree. I should check what the last commit changed, then reproduce the install.";
const TRACE_TOOLS: [(&str, u64, Call); 3] = [
    ("pnpm --version", 700, Call::Output("10.18.0")),
    (
        "git diff --stat HEAD~1 -- package.json pnpm-lock.yaml",
        900,
        Call::Output(" package.json | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)"),
    ),
    (
        "pnpm install --frozen-lockfile",
        1200,
        Call::Error(
            "ERR_PNPM_OUTDATED_LOCKFILE  Cannot install with \"frozen-lockfile\" because pnpm-lock.yaml is not up to date with package.json",
        ),
    ),
];
const TRACE_SOURCES: [&str; 2] = ["pnpm install | pnpm.io", "Continuous Integration | pnpm.io"];
const TRACE_ANSWER: &str = "The last commit bumped a version in package.json without updating pnpm-lock.yaml, so the frozen install fails. Run pnpm install locally and commit the lockfile.";

/// `TraceDemo.vue`: the finished conversation, replayed on demand.
pub struct TraceDemo(Chat);

impl TraceDemo {
    pub fn new() -> Self {
        let mut parts = vec![Part::Reasoning {
            text: TRACE_REASONING.to_owned(),
            streaming: false,
        }];
        parts.extend(
            TRACE_TOOLS
                .iter()
                .map(|&(command, _, call)| Part::Tool { command, call }),
        );
        parts.extend(TRACE_SOURCES.iter().map(|&title| Part::Source { title }));
        parts.push(Part::Text(TRACE_ANSWER.to_owned()));
        Self(Chat::new(vec![
            Msg::user("user-0".to_owned(), TRACE_QUESTION),
            Msg {
                id: "assistant-0".to_owned(),
                user: false,
                parts,
            },
        ]))
    }

    fn replay(chat: &Chat) {
        let id = chat.run.get().wrapping_add(1);
        let mut steps = vec![step_of(0, |c| {
            c.edit_last(|p| {
                p.push(Part::Reasoning {
                    text: String::new(),
                    streaming: true,
                });
            });
            c.set_open(&format!("{}-0", c.last_id()), true);
        })];
        steps.extend(stream_words(TRACE_REASONING, 40));
        steps.push(step_of(0, |c| {
            c.edit_last(|p| {
                if let Some(Part::Reasoning { streaming, .. }) = p.last_mut() {
                    *streaming = false;
                }
            });
            c.set_open(&format!("{}-0", c.last_id()), false);
        }));
        for (command, delay, call) in TRACE_TOOLS {
            steps.push(step_of(0, move |c| {
                c.edit_last(|p| {
                    p.push(Part::Tool {
                        command,
                        call: Call::Running,
                    });
                });
            }));
            steps.push(step_of(delay, move |c| {
                c.edit_last(|p| {
                    if let Some(Part::Tool { call: now, .. }) = p.last_mut() {
                        *now = call;
                    }
                });
            }));
        }
        steps.push(step_of(0, |c| {
            c.edit_last(|p| p.extend(TRACE_SOURCES.iter().map(|&title| Part::Source { title })));
        }));
        steps.push(step_of(1000, |c| c.edit_last(|p| p.push(Part::Text(String::new())))));
        steps.extend(stream_words(TRACE_ANSWER, 20));
        steps.push(step_of(0, |c| c.busy.set(false)));
        chat.play(
            vec![
                Msg::user(format!("user-{id}"), TRACE_QUESTION),
                Msg {
                    id: format!("assistant-{id}"),
                    user: false,
                    parts: Vec::new(),
                },
            ],
            steps,
        );
    }

    fn assistant(&self, m: &Msg) -> Div {
        let chat = &self.0;
        let live = chat.live(m);
        let answer = m.text();
        let mut rows: Vec<Element> = Vec::new();
        let steps = m
            .parts
            .iter()
            .filter(|p| matches!(p, Part::Reasoning { .. } | Part::Tool { .. }));
        for (index, part) in steps.enumerate() {
            let key = format!("{}-{index}", m.id);
            rows.push(match part {
                Part::Reasoning {
                    text: so_far,
                    streaming,
                } => {
                    let running = live && *streaming;
                    let line = thinking_line(running, Icon::CircleCheck, None, "Thought for 5s");
                    reasoning_row(chat, &key, so_far, line)
                }
                Part::Tool {
                    command,
                    call: Call::Running,
                } => pending_tool(live, None, command),
                Part::Tool { command, call } => {
                    let look = ToolLook {
                        default_open: index == 2,
                        glyph: None,
                        error: CodeBlockVariant::Destructive,
                    };
                    tool_row(chat, &key, look, command, *call)
                }
                Part::Text(_) | Part::Source { .. } => Element::Empty,
            });
        }
        let sources: Vec<&str> = m
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::Source { title } => Some(*title),
                _ => None,
            })
            .collect();
        if !sources.is_empty() {
            let using = live && answer.is_empty();
            let mark: Element = if using {
                spinner().into_element()
            } else {
                icon(Icon::Globe).into_element()
            };
            let title = format!("{} {} sources", if using { "Using" } else { "Used" }, sources.len());
            rows.push(
                chat.row(&format!("{}-sources", m.id), false)
                    .child(trace_collapsible_trigger(
                        false,
                        trace_line(using.then_some(TraceStatus::Running))
                            .child(trace_icon(mark))
                            .child(trace_title(title)),
                    ))
                    .child(trace_collapsible_content().map_root(|c| c.child(sources_trace(&sources))))
                    .into_element(),
            );
        }
        let traced = (!rows.is_empty()).then(|| trace().children(rows).into_element());
        let said = (!answer.is_empty()).then(|| text(answer).into_element());
        assistant_message([traced.unwrap_or_default(), said.unwrap_or_default()])
    }

    pub fn render(&self) -> Div {
        let chat = self.0.clone();
        let busy = chat.busy.get();
        let replay = chat.clone();
        thread()
            .children(
                chat.messages
                    .get()
                    .iter()
                    .map(|m| if m.user { user_message(m) } else { self.assistant(m) }),
            )
            .when(!busy, |d| {
                d.child(thread_button("Replay", move || Self::replay(&replay)))
            })
    }
}

/// A conversation that starts empty: one question, sent by its button, and
/// a Reset button once the answer is in.
fn send_or_reset(chat: &Chat, question: &'static str, send: fn(&Chat, &'static str)) -> Option<Button> {
    if chat.busy.get() {
        return None;
    }
    let c = chat.clone();
    Some(if chat.messages.get().is_empty() {
        thread_button("Send message", move || send(&c, question))
    } else {
        thread_button("Reset", move || c.reset())
    })
}

/// The assistant's parts grouped as Vue's `groupParts` does: consecutive
/// steps in one trace, each text part a paragraph.
fn grouped(m: &Msg, row: impl Fn(&str, &Part) -> Option<Element>) -> Vec<Element> {
    let mut out: Vec<Element> = Vec::new();
    let mut steps: Vec<Element> = Vec::new();
    let mut trace_run = 0_usize;
    let mut index = 0_usize;
    for part in &m.parts {
        if let Part::Text(t) = part {
            if !steps.is_empty() {
                out.push(trace().children(std::mem::take(&mut steps)).into_element());
                trace_run = trace_run.saturating_add(1);
                index = 0;
            }
            out.push(text(t.clone()).into_element());
            trace_run = trace_run.saturating_add(1);
            continue;
        }
        let key = format!("{}-{trace_run}-{index}", m.id);
        if let Some(e) = row(&key, part) {
            steps.push(e);
        }
        index = index.saturating_add(1);
    }
    if !steps.is_empty() {
        out.push(trace().children(steps).into_element());
    }
    out
}

const THINK_QUESTION: &str = "Why is the checkout deploy so slow?";
const THINK_REASONING: &str = "The build log shows install and compile finish in under a minute. Image optimization takes almost four, and it reruns on every deploy because the cache key includes the commit hash.";
const THINK_ANSWER: &str = "Image optimization is the bottleneck. Key its cache on the lockfile and the image directory instead of the commit hash.";

/// `TraceThinkingDemo.vue`: reasoning that streams open, then folds away.
pub struct ThinkingDemo(Chat);

impl ThinkingDemo {
    pub fn new() -> Self {
        Self(Chat::new(Vec::new()))
    }

    fn send(chat: &Chat, question: &'static str) {
        let id = chat.run.get().wrapping_add(1);
        let key = format!("assistant-{id}-0-0");
        let open_key = key.clone();
        let mut steps = vec![step_of(0, move |c| {
            c.edit_last(|p| {
                p.push(Part::Reasoning {
                    text: String::new(),
                    streaming: true,
                });
            });
            c.set_open(&open_key, true);
        })];
        steps.extend(stream_words(THINK_REASONING, 60));
        steps.push(step_of(0, move |c| {
            c.edit_last(|p| {
                if let Some(Part::Reasoning { streaming, .. }) = p.last_mut() {
                    *streaming = false;
                }
            });
            c.set_open(&key, false);
        }));
        steps.push(step_of(400, |c| c.edit_last(|p| p.push(Part::Text(String::new())))));
        steps.extend(stream_words(THINK_ANSWER, 20));
        steps.push(step_of(0, |c| c.busy.set(false)));
        let mut messages = chat.messages.get();
        messages.push(Msg::user(format!("user-{id}"), question));
        messages.push(Msg {
            id: format!("assistant-{id}"),
            user: false,
            parts: Vec::new(),
        });
        chat.play(messages, steps);
    }

    pub fn render(&self) -> Div {
        let chat = self.0.clone();
        thread()
            .children(chat.messages.get().iter().map(|m| {
                if m.user {
                    return user_message(m);
                }
                let live = chat.live(m);
                assistant_message(grouped(m, |key, part| match part {
                    Part::Reasoning {
                        text: so_far,
                        streaming,
                    } => {
                        let line = thinking_line(live && *streaming, Icon::Brain, Some(Icon::Brain), "Thought");
                        Some(reasoning_row(&chat, key, so_far, line))
                    }
                    _ => None,
                }))
            }))
            .child(send_or_reset(&chat, THINK_QUESTION, Self::send))
    }
}

const TOOLS_QUESTION: &str = "Which package managers are installed?";
const TOOLS: [(&str, Call); 3] = [
    ("node --version", Call::Output("v24.18.0")),
    ("npm --version", Call::Output("11.6.2")),
    ("pnpm --version", Call::Error("command not found: pnpm")),
];
const TOOLS_ANSWER: &str = "Node 24 and npm 11 are installed. pnpm is not.";

/// `TraceToolCallsDemo.vue`: one step per command, each opening to its output.
pub struct ToolCallsDemo(Chat);

impl ToolCallsDemo {
    pub fn new() -> Self {
        Self(Chat::new(Vec::new()))
    }

    fn send(chat: &Chat, question: &'static str) {
        let id = chat.run.get().wrapping_add(1);
        let mut steps = Vec::new();
        for (command, call) in TOOLS {
            steps.push(step_of(0, move |c| {
                c.edit_last(|p| {
                    p.push(Part::Tool {
                        command,
                        call: Call::Running,
                    });
                });
            }));
            steps.push(step_of(900, move |c| {
                c.edit_last(|p| {
                    if let Some(Part::Tool { call: now, .. }) = p.last_mut() {
                        *now = call;
                    }
                });
            }));
        }
        steps.push(step_of(0, |c| c.edit_last(|p| p.push(Part::Text(String::new())))));
        steps.extend(stream_words(TOOLS_ANSWER, 20));
        steps.push(step_of(0, |c| c.busy.set(false)));
        let mut messages = chat.messages.get();
        messages.push(Msg::user(format!("user-{id}"), question));
        messages.push(Msg {
            id: format!("assistant-{id}"),
            user: false,
            parts: Vec::new(),
        });
        chat.play(messages, steps);
    }

    pub fn render(&self) -> Div {
        let chat = self.0.clone();
        thread()
            .children(chat.messages.get().iter().map(|m| {
                if m.user {
                    return user_message(m);
                }
                let live = chat.live(m);
                assistant_message(grouped(m, |key, part| match part {
                    Part::Tool {
                        command,
                        call: Call::Running,
                    } => Some(pending_tool(live, Some(Icon::Terminal), command)),
                    Part::Tool { command, call } => {
                        let look = ToolLook {
                            default_open: false,
                            glyph: Some(Icon::Terminal),
                            error: CodeBlockVariant::Default,
                        };
                        Some(tool_row(&chat, key, look, command, *call))
                    }
                    _ => None,
                }))
            }))
            .child(send_or_reset(&chat, TOOLS_QUESTION, Self::send))
    }
}
