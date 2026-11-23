//! The pieces around an agent's answer: where it can be rolled back to, the
//! sources it cites, what to ask next, and how much context it has used.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gui::prelude::*;
use gui::{Div, Element, Icon, Model, ScenarioError, Side, SideAlign, Signal, TextField, div, icon, text};
use shadcn::{
    BubbleAlign, BubbleProps, BubbleVariant, ButtonSize, ButtonVariant, InputGroupAlign, InputGroupButtonSize,
    MarkerVariant, MenuItem, MessageProps, MessageScrollerDirection, MessageScrollerProps, TraceStatus, bubble,
    bubble_content, button, dropdown_menu, dropdown_menu_content, dropdown_menu_item, dropdown_menu_separator,
    dropdown_menu_trigger, empty, empty_description, empty_header, empty_media_icon, empty_title, hover_card,
    hover_card_content, hover_card_trigger, input_group, input_group_addon, input_group_button, input_group_textarea,
    marker, marker_content, message, message_content, message_scroller, message_scroller_button,
    message_scroller_provider, message_scroller_viewport, progress, spinner, theme, tooltip, tooltip_content,
    tooltip_trigger, trace, trace_action, trace_collapsible, trace_collapsible_content, trace_collapsible_trigger,
    trace_end_icon, trace_icon, trace_line, trace_title,
};

use super::markdown::markdown;
use super::thread::{
    Follow, Script, after, assistant_turn, click_named, output, replay, said, stream, thread, user_turn, wait,
};
use crate::{Render, Shot, demo, page};

/// CheckpointDemo's conversation: who said it, what, and the checkpoint the
/// agent's turn leaves.
const CHECKPOINTED: [(bool, &str, Option<&str>); 4] = [
    (true, "Rename the button component to action.", None),
    (
        false,
        "Renamed Button to Action in components/ui/button/Button.vue.",
        Some("Button renamed"),
    ),
    (true, "Update the docs to match.", None),
    (false, "Updated the Button page to say Action.", None),
];

/// CheckpointDemo: a turn the conversation can be rolled back to.
#[derive(Clone)]
struct Checkpoints {
    script: Script,
    /// How many messages of [`CHECKPOINTED`] are in the thread.
    count: Signal<usize>,
    /// How much of the last message has streamed in, in characters.
    shown: Signal<usize>,
}

impl Checkpoints {
    fn new() -> Self {
        Self {
            script: Script::new("checkpoint busy"),
            count: Signal::new(CHECKPOINTED.len()),
            shown: Signal::new(usize::MAX),
        }
    }

    fn restore(&self, index: usize) {
        self.count.set(index.saturating_add(1));
        self.shown.set(usize::MAX);
    }

    /// Send the next scripted message and stream the agent's reply, three
    /// characters every 20ms (Vue's `useIntervalFn`).
    fn send(&self) {
        let sent = self.count.get().saturating_add(1);
        let reply = CHECKPOINTED.get(sent).filter(|(user, ..)| !user);
        let Some((_, words, _)) = reply else {
            self.count.set(sent);
            return;
        };
        self.count.set(sent.saturating_add(1));
        self.shown.set(0);
        let total = words.chars().count();
        let steps = (1..=total.div_ceil(3))
            .map(|i| {
                let shown = self.shown;
                after(20, move || shown.set(i.saturating_mul(3).min(total)))
            })
            .collect();
        self.script.play(steps);
    }

    fn view(&self) -> Div {
        let count = self.count.get();
        let last = count.saturating_sub(1);
        let rows = CHECKPOINTED
            .iter()
            .take(count)
            .enumerate()
            .flat_map(|(i, (user, words, checkpoint))| {
                let words: String = if i == last {
                    words.chars().take(self.shown.get()).collect()
                } else {
                    (*words).to_owned()
                };
                let turn = if *user {
                    user_turn(&words)
                } else {
                    let p = MessageProps::default();
                    message(p).child(message_content(p).child(said(&words)))
                };
                let me = self.clone();
                let mark = checkpoint.map(|label| {
                    marker(
                        MarkerVariant::Separator,
                        Element::Empty,
                        marker_content(MarkerVariant::Separator)
                            .display(Display::Flex)
                            .align_items(Align::Center)
                            .gap(8.0)
                            .child(label)
                            .when(i < last, |c| {
                                c.child(
                                    button(ButtonVariant::Outline, ButtonSize::Xs)
                                        .child("Restore")
                                        .on_click(move |_| me.restore(i)),
                                )
                            }),
                    )
                });
                [Some(turn), mark].into_iter().flatten()
            });
        let next = CHECKPOINTED.get(count).is_some_and(|(user, ..)| *user);
        let me = self.clone();
        thread().children(rows).when(!self.script.busy() && next, |t| {
            t.child(
                button(ButtonVariant::Outline, ButtonSize::Sm)
                    .margin_top(Length::Auto)
                    .align_self(Align::Start)
                    .child("Continue")
                    .on_click(move |_| me.send()),
            )
        })
    }
}

const SOURCES_QUESTION: &str = "How do I start an Astro project with pnpm?";
const SOURCES_ANSWER: &str =
    "Run pnpm create astro@latest and follow the prompts. It scaffolds the project and installs the dependencies.";
/// The pages the agent cites, by URL and title; it cites the Astro docs twice.
const SOURCES: [(&str, &str); 4] = [
    (
        "https://docs.astro.build/en/install-and-setup/",
        "Install Astro | docs.astro.build",
    ),
    ("https://www.npmjs.com/package/create-astro", "create-astro | npmjs.com"),
    (
        "https://docs.astro.build/en/install-and-setup/",
        "Install Astro | docs.astro.build",
    ),
    ("https://pnpm.io/cli/create", "pnpm create | pnpm.io"),
];

/// SourcesDemo: an answer and the pages it used, each cited once.
#[derive(Clone)]
struct Sources {
    script: Script,
    replied: Signal<bool>,
    answer: Signal<String>,
    /// How many source parts have arrived.
    cited: Signal<usize>,
    open: Signal<bool>,
}

impl Sources {
    fn new() -> Self {
        Self {
            script: Script::new("sources busy"),
            replied: Signal::new(true),
            answer: Signal::new(SOURCES_ANSWER.to_owned()),
            cited: Signal::new(SOURCES.len()),
            open: Signal::new(true),
        }
    }

    fn replay(&self) {
        self.replied.set(false);
        self.answer.set(String::new());
        self.cited.set(0);
        self.open.set(true);
        let replied = self.replied;
        let mut steps = vec![after(300, move || replied.set(true))];
        steps.extend(stream(&self.answer, SOURCES_ANSWER, 20));
        for i in 1..=SOURCES.len() {
            let cited = self.cited;
            steps.push(after(20, move || cited.set(i)));
        }
        self.script.play(steps);
    }

    fn view(&self) -> Div {
        // Each URL once, in the order first cited.
        let mut unique: Vec<(&str, &str)> = Vec::new();
        for &(url, title) in SOURCES.iter().take(self.cited.get()) {
            if !unique.iter().any(|(u, _)| *u == url) {
                unique.push((url, title));
            }
        }
        let used = (!unique.is_empty()).then(|| {
            let head = trace_line(None)
                .child(trace_icon(icon(Icon::Globe)))
                .child(trace_title(format!("Used {} sources", unique.len())));
            let links = unique.iter().map(|(_, title)| {
                trace_action(
                    false,
                    trace_line(None)
                        .child(trace_title(*title))
                        .child(trace_end_icon(icon(Icon::ArrowUpRight))),
                )
            });
            trace()
                .child(
                    trace_collapsible()
                        .model(Model::bind(self.open))
                        .child(trace_collapsible_trigger(false, head))
                        .child(trace_collapsible_content().map_root(|c| c.child(trace().children(links)))),
                )
                .into_element()
        });
        let me = self.clone();
        thread()
            .child(user_turn(SOURCES_QUESTION))
            .when(self.replied.get(), |t| {
                t.child(assistant_turn([
                    said(&self.answer.get()),
                    used.unwrap_or(Element::Empty),
                ]))
            })
            .child(replay(&self.script, move || me.replay()))
    }
}

const SUGGESTIONS: [(&str, &str); 6] = [
    (
        "Add a dark mode toggle",
        "Added a ModeToggle to the site header. It calls useColorMode from @vueuse/core.",
    ),
    (
        "Fix the failing build",
        "The build failed on a missing import in pages/index.vue. I added it and the build passes.",
    ),
    (
        "Write tests for the form",
        "Added four tests in SignupForm.test.ts. They cover the empty, invalid and valid states.",
    ),
    (
        "Upgrade to Nuxt 4",
        "Ran the codemod and bumped nuxt to 4. Two pages needed the new app/ directory, so I moved them.",
    ),
    (
        "Add a loading state",
        "Added a loading state to the dashboard page. It shows skeletons while the data loads.",
    ),
    (
        "Find unused dependencies",
        "Three packages are not imported anywhere: lodash, moment and classnames. Should I remove them?",
    ),
];

/// SuggestionDemo: prompts to send in one press, above the composer.
#[derive(Clone)]
struct Suggestions {
    draft: TextField,
    /// The last prompt sent and the reply to it.
    turn: Signal<Option<(String, String)>>,
}

impl Suggestions {
    fn new() -> Self {
        Self {
            draft: TextField::new(""),
            turn: Signal::new(None),
        }
    }

    fn send(&self, prompt: &str) {
        let reply = SUGGESTIONS
            .iter()
            .find(|(p, _)| *p == prompt)
            .map_or("This demo only knows the suggestions. Pick one.", |(_, r)| *r);
        self.turn.set(Some((prompt.to_owned(), reply.to_owned())));
        self.draft.set_text("");
    }

    fn view(&self) -> Div {
        let typed = self.draft.text();
        let can_send = !typed.trim().is_empty();
        let chips = SUGGESTIONS.iter().map(|(prompt, _)| {
            let me = self.clone();
            button(ButtonVariant::Outline, ButtonSize::Sm)
                .flex_shrink(0.0)
                .child(*prompt)
                .on_click(move |_| me.send(prompt))
        });
        let me = self.clone();
        let enter = (self.clone(), typed.trim().to_owned());
        let composer = input_group(
            input_group_textarea(&self.draft)
                .placeholder("Ask the agent to change your code...")
                .padding(12.0)
                .on_submit(move |()| {
                    if !enter.1.is_empty() {
                        enter.0.send(&enter.1);
                    }
                }),
        )
        .addon(
            input_group_addon(InputGroupAlign::BlockEnd)
                .button(
                    input_group_button(ButtonVariant::Outline, InputGroupButtonSize::IconSm)
                        .radius(9999.0)
                        .child(icon(Icon::Plus)),
                )
                .button(
                    input_group_button(ButtonVariant::Default, InputGroupButtonSize::IconSm)
                        .margin_left(Length::Auto)
                        .radius(9999.0)
                        .disabled(!can_send)
                        .child(icon(Icon::ArrowUp))
                        .on_click(move |_| me.send(typed.trim())),
                ),
        );
        let p = MessageProps::default();
        div()
            .margin_x(Length::Auto)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .width(pct(100.0))
            .max_width(512.0)
            .gap(24.0)
            .padding_y(24.0)
            .text_size(14.0)
            .line_height(20.0)
            .when_some(self.turn.get(), |d, (prompt, reply)| {
                d.child(
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(16.0)
                        .child(user_turn(&prompt))
                        .child(message(p).child(message_content(p).child(text(reply)))),
                )
            })
            .child(
                div()
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .gap(8.0)
                    .child(
                        div()
                            .display(Display::Flex)
                            .mask(ScrollFade {
                                axis: Axis::Horizontal,
                                start: true,
                                end: true,
                                size: None,
                            })
                            .gap(8.0)
                            .overflow_x(Overflow::Scroll)
                            .overflow_y(Overflow::Hidden)
                            .children(chips),
                    )
                    .child(composer),
            )
    }
}

/// A source a citation points to.
struct Source {
    host: &'static str,
    title: &'static str,
    description: &'static str,
}

const STREAMING: Source = Source {
    host: "nextjs.org",
    title: "Linking and Navigating",
    description: "How the App Router streams a route to the browser.",
};
const SUSPENSE: Source = Source {
    host: "react.dev",
    title: "<Suspense>",
    description: "Display a fallback until its children have finished loading.",
};
const SERVER_COMPONENTS: Source = Source {
    host: "react.dev",
    title: "Server Components",
    description: "Components that render ahead of time, on the server.",
};
const LOADING: Source = Source {
    host: "nextjs.org",
    title: "loading.js",
    description: "Show an instant loading state for a route segment.",
};
const CHUNKED: Source = Source {
    host: "developer.mozilla.org",
    title: "Transfer-Encoding",
    description: "Send a response as a series of chunks.",
};

/// A citation inside a paragraph: a secondary `xs` button set in the line
/// as an atomic inline (`gui::Text::child`), on its baseline, opening a
/// hover card with the source's title and summary.
fn citation(source: &Source) -> Element {
    let t = theme();
    hover_card()
        .open_delay(Duration::from_millis(10))
        .close_delay(Duration::from_millis(100))
        .child(hover_card_trigger(
            button(ButtonVariant::Secondary, ButtonSize::Xs).child(source.host),
        ))
        .child(
            hover_card_content()
                .width(256.0)
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(4.0)
                .child(text(source.title).font_weight(500))
                .child(text(source.description).text_color(t.muted_foreground))
                .child(
                    text(source.host)
                        .text_size(12.0)
                        .line_height(16.0)
                        .text_color(t.muted_foreground),
                ),
        )
        .into_element()
}

fn citations() -> Div {
    div()
        .margin_x(Length::Auto)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .max_width(384.0)
        .gap(12.0)
        .padding_y(24.0)
        .text_size(14.0)
        .line_height(20.0)
        .line_height(LineHeight::Relative(1.625))
        .child(
            text("Next.js streams a route in chunks, so the page shows up before every request finishes. ")
                .child(citation(&STREAMING)),
        )
        .child(
            text(
                "Wrap a slow part of the page in a Suspense boundary and React sends the fallback first. When the \
                 data resolves, the finished HTML streams into the same response and replaces the fallback, with no \
                 extra request from the browser. ",
            )
            .child(citation(&SUSPENSE))
            .span(" Server Components help here, because their code never ships to the browser. ")
            .child(citation(&SERVER_COMPONENTS)),
        )
        .child(
            text("A loading.tsx file does the same for a whole route segment. ")
                .child(citation(&LOADING))
                .span(" It all runs over chunked transfer encoding, so there is nothing extra to set up. ")
                .child(citation(&CHUNKED)),
        )
}

/// Vue's `formatTokens`: `8420` → `8.4k`, `128000` → `128k`.
fn tokens(value: u32) -> String {
    if value < 1000 {
        return value.to_string();
    }
    let tenths = (value.saturating_add(50)) / 100;
    let (whole, part) = (tenths / 10, tenths % 10);
    if part == 0 {
        format!("{whole}k")
    } else {
        format!("{whole}.{part}k")
    }
}

fn context() -> Div {
    const LIMIT: u32 = 128_000;
    const ROWS: [(&str, u32); 3] = [("Input", 8420), ("Output", 1260), ("Reasoning", 980)];
    let used: u32 = ROWS.iter().map(|(_, v)| v).sum();
    let percent = (used.saturating_mul(100).saturating_add(LIMIT / 2)) / LIMIT;
    let t = theme();
    div()
        .display(Display::Flex)
        .justify_content(Justify::Center)
        .padding_y(48.0)
        .child(
            hover_card()
                // The demo's delays: open almost at once, close soon after.
                .open_delay(Duration::from_millis(10))
                .close_delay(Duration::from_millis(100))
                .child(hover_card_trigger(
                    button(ButtonVariant::Outline, ButtonSize::Sm).child(format!("{percent}% context")),
                ))
                .child(
                    hover_card_content()
                        .width(256.0)
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(12.0)
                        .child(
                            div()
                                .display(Display::Flex)
                                .align_items(Align::Center)
                                .justify_content(Justify::Between)
                                .gap(12.0)
                                .text_size(14.0)
                                .line_height(20.0)
                                .child(text("Context").font_weight(500))
                                .child(
                                    text(format!("{} / {}", tokens(used), tokens(LIMIT)))
                                        .text_color(t.muted_foreground),
                                ),
                        )
                        .child(progress(f32::from(u16::try_from(percent).unwrap_or(100))))
                        .child(
                            div()
                                .display(Display::Flex)
                                .flex_direction(FlexDirection::Column)
                                .gap(4.0)
                                .text_size(14.0)
                                .line_height(20.0)
                                .children(ROWS.iter().map(|(label, value)| {
                                    div()
                                        .display(Display::Flex)
                                        .justify_content(Justify::Between)
                                        .gap(12.0)
                                        .child(text(*label).text_color(t.muted_foreground))
                                        .child(text(tokens(*value)))
                                })),
                        ),
                ),
        )
}

pub fn render() -> Render {
    let agent = Agent::new();
    let checkpoints = Checkpoints::new();
    let sources = Sources::new();
    let suggestions = Suggestions::new();
    Box::new(move || {
        page(
            "Chat",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(40.0)
                .child(demo("ChatDemo.vue", agent.view()))
                .child(demo("CheckpointDemo.vue", checkpoints.view()))
                .child(demo("SourcesDemo.vue", sources.view()))
                .child(demo("SuggestionDemo.vue", suggestions.view()))
                .child(demo("InlineCitationDemo.vue", citations()))
                .child(demo("ContextDemo.vue", context())),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    // `side="top"`: the attach menu opens above its button.
    click_named(s, "ChatDemo.vue", "dropdown-menu-trigger")?;
    s.frame("attach menu above")?;
    s.press(gui::NamedKey::Escape, gui::Modifiers::default());
    click_named(s, "ChatDemo.vue", "input-group-button")?;
    wait(s, 1200);
    s.frame("agent thinking")?;
    wait(s, 5000);
    s.frame("agent replied")?;
    s.click_text("Restore")?;
    s.frame("restored")?;
    s.click_text("Continue")?;
    wait(s, 400);
    s.frame("continued")?;
    s.click_text("Fix the failing build")?;
    s.frame("suggestion sent")?;
    let (x, y) = s.locate("8% context")?;
    s.hover(x, y);
    wait(s, 50);
    s.frame("context card")?;
    Ok(())
}

// ── ChatDemo ───────────────────────────────────────────────────────────────

/// One step of a scripted agent turn; each streams into the reply in order.
#[derive(Clone, Copy)]
enum Step {
    Reasoning(&'static str),
    Sleep(u64),
    Tool {
        command: &'static str,
        ms: u64,
        stdout: &'static str,
    },
    Text(&'static str),
}

/// What the agent is doing: idle, about to reply, replying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ready,
    Submitted,
    Streaming,
}

/// A part of the agent's turn, shaped like AI SDK UI message parts.
#[derive(Debug, Clone)]
enum Part {
    /// The thought so far; open while it runs unless the reader chose.
    Reasoning {
        text: String,
        done: bool,
        open: Follow,
    },
    /// A command, and what it printed once it ran.
    Tool {
        command: &'static str,
        stdout: Option<&'static str>,
        open: Signal<bool>,
    },
    Text(String),
}

#[derive(Debug, Clone)]
struct Turn {
    id: String,
    user: bool,
    parts: Vec<Part>,
}

impl Turn {
    /// Vue's `getMessageText`: the text parts, joined.
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

const AGENT_SCRIPT: [(&str, &[Step]); 3] = [
    (
        "Add a dark mode toggle to the settings page.",
        &[
            Step::Reasoning(
                "I need to know how themes are set up before I add a toggle. If @nuxtjs/color-mode is already installed, the toggle only has to set colorMode.preference.",
            ),
            Step::Sleep(400),
            Step::Tool {
                command: "rg -l colorMode nuxt.config.ts app",
                ms: 800,
                stdout: "nuxt.config.ts",
            },
            Step::Sleep(500),
            Step::Text(
                "@nuxtjs/color-mode is already set up in `nuxt.config.ts`, so the toggle only has to set `colorMode.preference`.\n\n**Changes**\n\n- Added `components/ModeToggle.vue` with a Light and Dark option.\n- Put `<ModeToggle />` next to the Appearance heading in `pages/settings.vue`.\n\n```ts\nconst colorMode = useColorMode()\n```\n",
            ),
        ],
    ),
    (
        "Make it follow the system setting by default.",
        &[
            Step::Reasoning("The default theme lives in the colorMode options. I should check what it is set to now."),
            Step::Sleep(400),
            Step::Tool {
                command: "rg -n preference nuxt.config.ts",
                ms: 800,
                stdout: "12:    preference: 'light',",
            },
            Step::Sleep(500),
            Step::Text(
                "It was set to `light`. I updated the `colorMode` options in `nuxt.config.ts`:\n\n```ts\ncolorMode: { classSuffix: '', preference: 'system' },\n```\n\nNew visitors now get the theme from their OS. The toggle also has a **System** option, so anyone who picked Light or Dark can switch back.\n",
            ),
        ],
    ),
    (
        "Run the build before I ship it.",
        &[
            Step::Tool {
                command: "pnpm build",
                ms: 1600,
                stdout: "Nuxt 4.0.0 with Nitro 2.12.0\n\n✓ Client built in 4.2s\n✓ Server built in 1.8s\n✓ Prerendered 12 routes",
            },
            Step::Sleep(500),
            Step::Text(
                "The build passes, so it is ready to ship.\n\n- Built the client in **4.2s**.\n- No build or type errors.\n- Prerendered all **12** routes.\n",
            ),
        ],
    ),
];

const FALLBACK: &str = "This demo only knows the scripted turns. Press reset to start over.";

/// ChatDemo: an agent thread scripted turn by turn, its reasoning and commands
/// streaming in before its Markdown reply.
#[derive(Clone)]
struct Agent {
    turns: Signal<Vec<Turn>>,
    status: Signal<Status>,
    draft: TextField,
    /// Bumped by reset, so a running script stops at its next wait.
    run: Rc<Cell<u64>>,
    /// Numbers the turns, as Vue's `Date.now()` ids.
    made: Rc<Cell<u64>>,
}

impl Agent {
    fn new() -> Self {
        Self {
            turns: Signal::labeled("agent turns", Vec::new()),
            status: Signal::labeled("agent status", Status::Ready),
            draft: TextField::new(AGENT_SCRIPT[0].0),
            run: Rc::new(Cell::new(0)),
            made: Rc::new(Cell::new(0)),
        }
    }

    fn sent(&self) -> usize {
        self.turns.get().iter().filter(|t| t.user).count()
    }

    /// The next scripted ask, put in the composer once a turn ends.
    fn next_ask(&self) -> &'static str {
        AGENT_SCRIPT.get(self.sent()).map_or("", |(ask, _)| ask)
    }

    fn id(&self, role: &str) -> String {
        let n = self.made.get().wrapping_add(1);
        self.made.set(n);
        format!("{role}-{n}")
    }

    fn submit(&self) {
        let words = self.draft.text().trim().to_owned();
        if words.is_empty() || self.status.get() != Status::Ready {
            return;
        }
        // A turn the script knows only when the composer still holds its ask.
        let steps: Vec<Step> = match AGENT_SCRIPT.get(self.sent()) {
            Some((ask, steps)) if *ask == words => steps.to_vec(),
            _ => vec![Step::Text(FALLBACK)],
        };
        let id = self.id("user");
        self.turns.update(|t| {
            t.push(Turn {
                id,
                user: true,
                parts: vec![Part::Text(words)],
            });
        });
        self.draft.set_text(self.next_ask());
        self.respond(steps);
    }

    fn respond(&self, steps: Vec<Step>) {
        let run = self.run.get();
        self.status.set(Status::Submitted);
        let me = self.clone();
        let steps: Rc<[Step]> = steps.into();
        self.after(600, run, move || {
            let id = me.id("assistant");
            me.turns.update(|t| {
                t.push(Turn {
                    id,
                    user: false,
                    parts: Vec::new(),
                });
            });
            me.status.set(Status::Streaming);
            me.step(Rc::clone(&steps), 0, run);
        });
    }

    /// Run `f` after `ms`, unless a reset came first.
    fn after(&self, ms: u64, run: u64, f: impl Fn() + 'static) {
        let live = Rc::clone(&self.run);
        gui::clock::after(Duration::from_millis(ms), move || {
            if live.get() == run {
                f();
            }
        });
    }

    /// Change the reply's last part.
    fn edit_last(&self, f: impl FnOnce(&mut Part)) {
        self.turns.update(|t| {
            if let Some(part) = t.last_mut().and_then(|turn| turn.parts.last_mut()) {
                f(part);
            }
        });
    }

    fn push(&self, part: Part) {
        self.turns.update(|t| {
            if let Some(turn) = t.last_mut() {
                turn.parts.push(part);
            }
        });
    }

    fn step(&self, steps: Rc<[Step]>, i: usize, run: u64) {
        let step = steps.get(i).copied();
        let me = self.clone();
        let next = move || me.step(Rc::clone(&steps), i.saturating_add(1), run);
        match step {
            None => self.status.set(Status::Ready),
            Some(Step::Sleep(ms)) => self.after(ms, run, next),
            Some(Step::Reasoning(words)) => {
                self.push(Part::Reasoning {
                    text: String::new(),
                    done: false,
                    open: Follow::new(true),
                });
                let me = self.clone();
                self.stream(
                    words,
                    0,
                    run,
                    Rc::new(move || {
                        me.edit_last(|p| {
                            if let Part::Reasoning { done, open, .. } = p {
                                *done = true;
                                open.follow(false);
                            }
                        });
                        next();
                    }),
                );
            }
            Some(Step::Tool { command, ms, stdout }) => {
                self.push(Part::Tool {
                    command,
                    stdout: None,
                    open: Signal::new(false),
                });
                let me = self.clone();
                self.after(ms, run, move || {
                    me.edit_last(|p| {
                        if let Part::Tool { stdout: out, .. } = p {
                            *out = Some(stdout);
                        }
                    });
                    next();
                });
            }
            Some(Step::Text(words)) => {
                self.push(Part::Text(String::new()));
                self.stream(words, 0, run, Rc::new(next));
            }
        }
    }

    /// Vue's `stream`: three more characters of `words` into the last part
    /// every 16ms, then `done`.
    fn stream(&self, words: &'static str, shown: usize, run: u64, done: Rc<dyn Fn()>) {
        let count = shown.saturating_add(3);
        let end = words.char_indices().nth(count).map_or(words.len(), |(i, _)| i);
        let part = words.get(..end).unwrap_or(words).to_owned();
        self.edit_last(|p| match p {
            Part::Reasoning { text, .. } => *text = part,
            Part::Text(text) => *text = part,
            Part::Tool { .. } => {}
        });
        let me = self.clone();
        let finished = end >= words.len();
        self.after(16, run, move || {
            if finished {
                done();
            } else {
                me.stream(words, count, run, Rc::clone(&done));
            }
        });
    }

    fn reset(&self) {
        self.run.set(self.run.get().wrapping_add(1));
        self.turns.replace(Vec::new());
        self.status.set(Status::Ready);
        self.draft.set_text(AGENT_SCRIPT[0].0);
    }

    fn reasoning(text: &str, running: bool, open: &Follow) -> Element {
        let head = trace_line(running.then_some(TraceStatus::Running))
            .child(trace_icon(if running {
                spinner().into_element()
            } else {
                icon(Icon::CircleCheck).into_element()
            }))
            .child(trace_title(if running { "Thinking" } else { "Thought" }));
        trace_collapsible()
            .model(Model::bind(open.open))
            .child(trace_collapsible_trigger(false, head))
            .child(trace_collapsible_content().map_root(|c| c.child(text.to_owned())))
            .into_element()
    }

    fn tool(command: &str, stdout: Option<&str>, open: &Signal<bool>, streaming: bool) -> Element {
        match stdout {
            Some(out) => {
                let head = trace_line(None)
                    .child(trace_icon(icon(Icon::Terminal)))
                    .child(trace_title(format!("Ran {command}")));
                trace_collapsible()
                    .model(Model::bind(*open))
                    .child(trace_collapsible_trigger(false, head))
                    .child(trace_collapsible_content().map_root(|c| c.child(output(out))))
                    .into_element()
            }
            None => {
                let (status, verb) = if streaming {
                    (TraceStatus::Running, "Running")
                } else {
                    (TraceStatus::Error, "Failed to run")
                };
                trace_line(Some(status))
                    .child(trace_icon(icon(Icon::Terminal)))
                    .child(trace_title(format!("{verb} {command}")))
                    .into_element()
            }
        }
    }

    fn turn(&self, turn: &Turn, live: bool) -> Div {
        if turn.user {
            let p = MessageProps {
                align: BubbleAlign::End,
                ..MessageProps::default()
            };
            let b = BubbleProps {
                variant: BubbleVariant::Muted,
                align: BubbleAlign::End,
            };
            return message(p).child(message_content(p).child(bubble(b).child(bubble_content(b).child(turn.text()))));
        }
        let steps: Vec<Element> = turn
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Reasoning { text, done, open } => Some(Self::reasoning(text, !done, open)),
                Part::Tool { command, stdout, open } => Some(Self::tool(command, *stdout, open, live)),
                Part::Text(_) => None,
            })
            .collect();
        let words = turn.text();
        let p = MessageProps::default();
        message(p).child(
            message_content(p)
                .gap(12.0)
                .when(!steps.is_empty(), |c| c.child(trace().children(steps)))
                .when(!words.is_empty(), |c| c.child(markdown(&words, 14.0))),
        )
    }

    fn view(&self) -> Div {
        let t = theme();
        let turns = self.turns.get();
        let busy = self.status.get() != Status::Ready;
        let can_send = !self.draft.text().trim().is_empty() && !busy;
        let (reset, send) = (self.clone(), self.clone());
        let last = turns.len().saturating_sub(1);
        let body = if turns.is_empty() {
            empty().height(pct(100.0)).child(
                empty_header()
                    .child(empty_media_icon(Icon::MessageCircleDashed))
                    .child(empty_title("What should we build?"))
                    .child(empty_description(
                        "The agent can read your code, run commands and make changes. Press send to start.",
                    )),
            )
        } else {
            let me = self.clone();
            message_scroller()
                .child(
                    message_scroller_viewport(
                        turns,
                        |turn: &Turn| (turn.id.as_str(), turn.user),
                        move |i, turn| me.turn(turn, busy && i == last),
                    )
                    .gap(24.0)
                    .viewport(|v| v.padding_x(32.0)),
                )
                .child(message_scroller_button(MessageScrollerDirection::End))
        };
        // The provider outlives the empty state, as Vue's wraps the card.
        let body = message_scroller_provider(MessageScrollerProps::default(), body);
        let attach = |glyph: Icon, label: &str| {
            dropdown_menu_item(label, MenuItem::default())
                .child(icon(glyph))
                .child(label.to_owned())
        };
        let context = dropdown_menu_content()
            .side(Side::Top)
            .align(SideAlign::Start)
            .width(176.0)
            .child(attach(Icon::Paperclip, "Add Photos & Files"))
            .child(dropdown_menu_separator())
            .child(attach(Icon::Telescope, "Deep Research"))
            .child(attach(Icon::Globe, "Web Search"));
        let composer = input_group(
            input_group_textarea(&self.draft)
                .placeholder("Ask the agent...")
                .min_height(56.0)
                .on_submit({
                    let send = send.clone();
                    move |()| send.submit()
                }),
        )
        .addon(
            input_group_addon(InputGroupAlign::BlockEnd)
                .padding_top(4.0)
                .button(
                    dropdown_menu()
                        .child(dropdown_menu_trigger(
                            input_group_button(ButtonVariant::Outline, InputGroupButtonSize::IconSm)
                                .child(icon(Icon::Plus)),
                        ))
                        .child(context),
                )
                .button(
                    input_group_button(ButtonVariant::Default, InputGroupButtonSize::IconSm)
                        .margin_left(Length::Auto)
                        .disabled(!can_send)
                        .child(icon(Icon::ArrowUp))
                        .on_click(move |_| send.submit()),
                ),
        );
        div()
            .position(Position::Relative)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .width(pct(100.0))
            .gap(16.0)
            .child(
                div()
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .height(800.0)
                    .width(pct(100.0))
                    .gap(16.0)
                    .child(
                        div()
                            .display(Display::Flex)
                            .align_items(Align::Start)
                            .justify_content(Justify::Between)
                            .gap(16.0)
                            .padding(32.0)
                            .padding_bottom(16.0)
                            .child(
                                div()
                                    .display(Display::Flex)
                                    .flex_direction(FlexDirection::Column)
                                    .gap(4.0)
                                    .child(text("New Thread").font_weight(500))
                                    .child(
                                        text("Ask the agent to change your code.")
                                            .text_size(14.0)
                                            .line_height(20.0)
                                            .text_color(t.muted_foreground),
                                    ),
                            )
                            .child(
                                tooltip()
                                    .child(tooltip_trigger(
                                        button(ButtonVariant::Outline, ButtonSize::Icon)
                                            .disabled(busy)
                                            .child(icon(Icon::RotateCw))
                                            .on_click(move |_| reset.reset()),
                                    ))
                                    .child(tooltip_content().child(text("Reset"))),
                            ),
                    )
                    .child(div().min_height(0.0).flex(Flex::grow(1.0)).child(body))
                    .child(
                        div()
                            .padding(32.0)
                            .padding_top(0.0)
                            .child(div().width(pct(100.0)).child(composer)),
                    ),
            )
    }
}
