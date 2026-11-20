//! What an agent thinks before it answers: its reasoning, how long it took,
//! and the steps it researched with.

use gui::prelude::*;
use gui::{Div, Element, Icon, ScenarioError, Signal, div, icon, text};
use shadcn::{
    BadgeVariant, CodeBlockContentProps, CodeBlockProps, CodeBlockSize, TraceStatus, badge, code_block,
    code_block_content, code_block_lines, spinner, trace, trace_action, trace_collapsible, trace_collapsible_content,
    trace_collapsible_trigger, trace_description, trace_end_icon, trace_icon, trace_line, trace_title,
};

use super::thread::{Follow, Script, Step, after, assistant_turn, replay, said, stream, thread, user_turn, wait};
use crate::{Render, Shot, demo, page};

const QUESTION: &str = "Why is the checkout deploy so slow?";

/// ReasoningDemo: a reply whose thinking streams first, then folds away.
#[derive(Clone)]
struct Reasoning {
    script: Script,
    /// Whether the agent's turn has started.
    replied: Signal<bool>,
    thought: Signal<String>,
    /// The thinking part is still being written.
    thinking: Signal<bool>,
    answer: Signal<String>,
    open: Follow,
}

const REASONING: &str = "The deploy log has three phases. Install takes 38s and next build takes 52s, both normal for this repo.\n\nImage optimization takes 3m 50s. It reruns on every deploy because its cache key includes the commit hash, so the cache never hits.";
const REASONING_ANSWER: &str = "Image optimization is the bottleneck. Key its cache on the lockfile and the image directory instead of the commit hash, and the deploy should take under two minutes.";

impl Reasoning {
    /// The finished transcript, its thinking open.
    fn new() -> Self {
        Self {
            script: Script::new("reasoning busy"),
            replied: Signal::new(true),
            thought: Signal::new(REASONING.to_owned()),
            thinking: Signal::new(false),
            answer: Signal::new(REASONING_ANSWER.to_owned()),
            open: Follow::new(true),
        }
    }

    fn replay(&self) {
        self.replied.set(false);
        self.thought.set(String::new());
        self.thinking.set(true);
        self.answer.set(String::new());
        self.open.reset(false);
        let (replied, open, thinking) = (self.replied.clone(), self.open.clone(), self.thinking.clone());
        let mut steps: Vec<Step> = vec![after(300, move || replied.set(true))];
        let first = open.clone();
        steps.push(after(0, move || first.follow(true)));
        steps.extend(stream(&self.thought, REASONING, 40));
        steps.push(after(40, move || {
            thinking.set(false);
            open.follow(false);
        }));
        steps.extend(
            stream(&self.answer, REASONING_ANSWER, 20)
                .into_iter()
                .enumerate()
                .map(|(i, s)| if i == 0 { s.lead(400) } else { s }),
        );
        self.script.play(steps);
    }

    fn view(&self) -> Div {
        let live = self.script.busy() && self.thinking.get();
        let status = live.then_some(TraceStatus::Running);
        let title = if live { "Thinking" } else { "Thought for 5s" };
        let thought = self.thought.get();
        let row = if thought.is_empty() {
            trace_line(status).child(trace_title(title))
        } else {
            let open = &self.open.open;
            trace_collapsible()
                .child(trace_collapsible_trigger(
                    open,
                    false,
                    trace_line(status).child(trace_title(title)),
                ))
                .child(trace_collapsible_content(open, |c| c.child(text(thought))))
        };
        let me = self.clone();
        thread()
            .child(user_turn(QUESTION))
            .when(self.replied.get(), |t| {
                t.child(assistant_turn([
                    trace().child(row).into_element(),
                    said(&self.answer.get()),
                ]))
            })
            .child(replay(&self.script, move || me.replay()))
    }
}

fn duration(open: &Signal<bool>) -> Div {
    let line = trace_line(None)
        .child(trace_icon(icon(Icon::CircleCheck)))
        .child(trace_title("Thought"))
        .child(trace_description("12s"));
    trace().max_w(96.0).child(
        trace_collapsible().child(trace_collapsible_trigger(open, false, line)).child(
            trace_collapsible_content(open, |c| {
                c.child(text(
                    "The build log shows install and compile finish in under a minute. Image optimization takes almost four, and it reruns on every deploy because the cache key includes the commit hash.",
                ))
            }),
        ),
    )
}

/// What a research step of ChainOfThoughtDemo did and found.
#[derive(Clone, Copy)]
enum Research {
    ReadBuildLog {
        deployment: &'static str,
        log: &'static str,
    },
    SearchWeb {
        query: &'static str,
        results: [&'static str; 2],
    },
    SearchCode {
        query: &'static str,
        files: [&'static str; 2],
    },
}

const RESEARCH: [Research; 3] = [
    Research::ReadBuildLog {
        deployment: "dpl_8Kx2",
        log: "install    41s\ncompile    38s\nimages   3m 52s",
    },
    Research::SearchWeb {
        query: "next.js image cache key",
        results: ["Image component | nextjs.org", "Build cache | vercel.com"],
    },
    Research::SearchCode {
        query: "cacheKey",
        files: ["next.config.ts", "scripts/optimize-images.ts"],
    },
];

const RESEARCH_ANSWER: &str = "Image optimization takes almost four minutes because its cache key includes the commit hash. Key it on the image directory instead.";

impl Research {
    /// The step's line title in `status`.
    fn title(self, status: Option<TraceStatus>) -> String {
        let verb = |running: &str, done: &str, failed: &str| match status {
            Some(TraceStatus::Running) => running.to_owned(),
            Some(TraceStatus::Error) => failed.to_owned(),
            _ => done.to_owned(),
        };
        match self {
            Self::SearchWeb { query, .. } => {
                format!(
                    "{} the web for {query}",
                    verb("Searching", "Searched", "Failed to search")
                )
            }
            Self::ReadBuildLog { deployment, .. } => {
                format!(
                    "{} the build log for {deployment}",
                    verb("Reading", "Read", "Failed to read")
                )
            }
            Self::SearchCode { query, .. } => {
                format!(
                    "{} the code for {query}",
                    verb("Searching", "Searched", "Failed to search")
                )
            }
        }
    }

    /// What the step found.
    fn output(self) -> Div {
        match self {
            Self::ReadBuildLog { log, .. } => {
                let p = CodeBlockProps {
                    size: CodeBlockSize::Sm,
                    ..CodeBlockProps::default()
                };
                let c = CodeBlockContentProps {
                    wrap: true,
                    ..CodeBlockContentProps::default()
                };
                code_block(p).child(code_block_content(p, c, code_block_lines(log)).max_h(64.0))
            }
            Self::SearchWeb { results, .. } => trace().children(results.iter().map(|title| {
                trace_action(
                    false,
                    trace_line(None)
                        .child(trace_icon(icon(Icon::Globe)))
                        .child(trace_title(*title))
                        .child(trace_end_icon(icon(Icon::ArrowUpRight))),
                )
            })),
            Self::SearchCode { files, .. } => div().flex().flex_wrap().gap(1.5).children(
                files
                    .iter()
                    .map(|f| badge(BadgeVariant::Secondary).font_mono().child(*f)),
            ),
        }
    }
}

/// A line with the icon its status takes.
fn status_line(status: Option<TraceStatus>, title: impl Into<String>) -> Div {
    let mark: Element = match status {
        Some(TraceStatus::Running) => spinner().into_element(),
        Some(TraceStatus::Error) => icon(Icon::CircleAlert).into_element(),
        _ => icon(Icon::CircleCheck).into_element(),
    };
    trace_line(status).child(trace_icon(mark)).child(trace_title(title))
}

/// ChainOfThoughtDemo: research steps folded under one summary line.
#[derive(Clone)]
struct ChainOfThought {
    script: Script,
    replied: Signal<bool>,
    /// How many steps have started, and how many of those have finished.
    started: Signal<usize>,
    finished: Signal<usize>,
    answer: Signal<String>,
    summary: Follow,
    /// Each finished step's open state; the build log opens first.
    steps: [Signal<bool>; 3],
}

fn step_opens() -> [Signal<bool>; 3] {
    [Signal::new(true), Signal::new(false), Signal::new(false)]
}

impl ChainOfThought {
    fn new() -> Self {
        Self {
            script: Script::new("chain of thought busy"),
            replied: Signal::new(true),
            started: Signal::new(RESEARCH.len()),
            finished: Signal::new(RESEARCH.len()),
            answer: Signal::new(RESEARCH_ANSWER.to_owned()),
            summary: Follow::new(false),
            steps: step_opens(),
        }
    }

    fn replay(&self) {
        self.replied.set(false);
        self.started.set(0);
        self.finished.set(0);
        self.answer.set(String::new());
        self.summary.reset(false);
        for (open, fresh) in self.steps.iter().zip(step_opens()) {
            open.set(fresh.get());
        }
        let (replied, summary) = (self.replied.clone(), self.summary.clone());
        let mut steps = vec![after(300, move || {
            replied.set(true);
            summary.follow(true);
        })];
        for i in 1..=RESEARCH.len() {
            let (started, finished) = (self.started.clone(), self.finished.clone());
            steps.push(after(0, move || started.set(i)));
            steps.push(after(900, move || finished.set(i)));
        }
        let summary = self.summary.clone();
        let words = stream(&self.answer, RESEARCH_ANSWER, 20);
        let mut words = words.into_iter();
        if let Some(first) = words.next() {
            steps.push(first);
            steps.push(after(0, move || summary.follow(false)));
        }
        steps.extend(words);
        self.script.play(steps);
    }

    fn view(&self) -> Div {
        let streaming = self.script.busy();
        let answer = self.answer.get();
        let running = streaming && answer.is_empty();
        let started = self.started.get();
        let finished = self.finished.get();
        let summary_status = running.then_some(TraceStatus::Running);
        let title = if running {
            "Researching".to_owned()
        } else {
            format!("Researched in {started} steps")
        };
        let rows = RESEARCH
            .iter()
            .zip(&self.steps)
            .take(started)
            .enumerate()
            .map(|(i, (r, open))| {
                if i >= finished {
                    let status = if streaming {
                        TraceStatus::Running
                    } else {
                        TraceStatus::Error
                    };
                    return status_line(Some(status), r.title(Some(status))).into_element();
                }
                trace_collapsible()
                    .child(trace_collapsible_trigger(open, false, status_line(None, r.title(None))))
                    .child(trace_collapsible_content(open, |c| c.child(r.output())))
                    .into_element()
            });
        let open = &self.summary.open;
        let body = (started > 0).then(|| {
            trace()
                .child(
                    trace_collapsible()
                        .child(trace_collapsible_trigger(
                            open,
                            false,
                            status_line(summary_status, title),
                        ))
                        .child(trace_collapsible_content(open, |c| c.child(trace().children(rows)))),
                )
                .into_element()
        });
        let me = self.clone();
        thread()
            .child(user_turn(QUESTION))
            .when(self.replied.get(), |t| {
                t.child(assistant_turn([body.unwrap_or(Element::Empty), said(&answer)]))
            })
            .child(replay(&self.script, move || me.replay()))
    }
}

pub fn render() -> Render {
    let reasoning = Reasoning::new();
    let thought = Signal::labeled("thought open", false);
    let chain = ChainOfThought::new();
    Box::new(move || {
        page(
            "Reasoning",
            div()
                .flex_col()
                .gap(10.0)
                .child(demo("ReasoningDemo.vue", reasoning.view()))
                .child(demo("ReasoningDurationDemo.vue", duration(&thought)))
                .child(demo("ChainOfThoughtDemo.vue", chain.view())),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.click_text("Thought")?;
    s.frame("duration open")?;
    s.click_text("Researched in 3 steps")?;
    s.frame("research open")?;
    let (x, y) = s.locate("Replay")?;
    s.click(x, y);
    wait(s, 400);
    s.frame("thinking")?;
    wait(s, 1200);
    s.frame("thought streaming")?;
    wait(s, 3500);
    s.frame("answered")?;
    Ok(())
}
