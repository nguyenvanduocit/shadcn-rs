//! An agent that hands work to a subagent: the subagent's own steps, folded
//! under one line, and what it reported.

use gui::prelude::*;
use gui::{Div, Element, Icon, Model, ScenarioError, Signal, div, icon, text};
use shadcn::{
    BadgeVariant, TraceStatus, badge, spinner, trace, trace_collapsible, trace_collapsible_content,
    trace_collapsible_trigger, trace_description, trace_icon, trace_line, trace_title,
};

use super::thread::{Script, after, assistant_turn, output, replay, said, stream, thread, user_turn, wait};
use crate::{Render, Shot, demo, page};

fn line(status: Option<TraceStatus>, glyph: impl IntoElement, title: &str) -> Div {
    trace_line(status).child(trace_icon(glyph)).child(trace_title(title))
}

fn done(title: &str) -> Div {
    line(None, icon(Icon::CircleCheck), title)
}

fn details(open: &Signal<bool>) -> Div {
    let head = done("Ran the research agent").child(trace_description("48s"));
    trace().max_width(384.0).child(
        trace_collapsible()
            .model(Model::bind(*open))
            .child(trace_collapsible_trigger(false, head))
            .child(trace_collapsible_content().map_root(|c| {
                c.display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .gap(12.0)
                    .child(
                        div()
                            .display(Display::Flex)
                            .flex_wrap(true)
                            .gap(6.0)
                            .child(badge(BadgeVariant::Secondary).child("anthropic/claude-sonnet-4.5"))
                            .child(
                                badge(BadgeVariant::Outline)
                                    .font_family(FontFamily::GEIST_MONO)
                                    .child("runCommand"),
                            ),
                    )
                    .child(text("Find out why image optimization reruns on every deploy."))
                    .child(
                        trace()
                            .child(done("Ran grep -rn cacheKey scripts"))
                            .child(done("Ran sed -n 13,15p scripts/optimize-images.ts")),
                    )
                    .child(text("The cache is keyed on the commit SHA."))
            })),
    )
}

fn running(open: &Signal<bool>) -> Div {
    let head = line(Some(TraceStatus::Running), spinner(), "Running the research agent");
    trace().max_width(384.0).child(
        trace_collapsible()
            .model(Model::bind(*open))
            .child(trace_collapsible_trigger(false, head))
            .child(trace_collapsible_content().map_root(|c| {
                c.child(trace().child(done("Ran grep -rn cacheKey scripts")).child(line(
                    Some(TraceStatus::Running),
                    spinner(),
                    "Running sed -n 13,15p scripts/optimize-images.ts",
                )))
            })),
    )
}

const QUESTION: &str = "Why does image optimization rerun on every deploy?";
const REPORT: &str =
    "The cache is keyed on the commit SHA, so every deploy misses it and optimizes all 1,240 images again.";
const ANSWER: &str = "Image optimization reruns because its cache is keyed on the commit SHA, which changes on every deploy. Key the cache on a hash of public/images instead, and unchanged images will be skipped.";

/// The subagent's commands and what each printed.
const STEPS: [(&str, &str); 3] = [
    (
        "grep -rn cacheKey scripts",
        "scripts/optimize-images.ts:14:  const cacheKey = process.env.COMMIT_SHA",
    ),
    (
        "sed -n 13,15p scripts/optimize-images.ts",
        "const cacheDir = \".nuxt/cache/images\"\nconst cacheKey = process.env.COMMIT_SHA\nconst cached = await restoreCache(cacheDir, cacheKey)",
    ),
    ("ls public/images | wc -l", "1240"),
];

/// How far the subagent has come.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Agent {
    /// Called, nothing back yet.
    Started,
    /// Its first `n` commands ran; the next is running.
    Working(usize),
    /// Every command ran and it reported.
    Reported,
}

/// SubagentDemo: the subagent's steps stream in under its line.
#[derive(Clone)]
struct Research {
    script: Script,
    agent: Signal<Option<Agent>>,
    answer: Signal<String>,
    open: Signal<bool>,
    steps: [Signal<bool>; 3],
}

impl Research {
    fn new() -> Self {
        Self {
            script: Script::new("subagent busy"),
            agent: Signal::new(Some(Agent::Reported)),
            answer: Signal::new(ANSWER.to_owned()),
            open: Signal::new(true),
            steps: std::array::from_fn(|_| Signal::new(false)),
        }
    }

    fn replay(&self) {
        self.agent.set(Some(Agent::Started));
        self.answer.set(String::new());
        self.open.set(true);
        for s in &self.steps {
            s.set(false);
        }
        let mut steps = Vec::new();
        for i in 0..STEPS.len() {
            let agent = self.agent;
            steps.push(after(if i == 0 { 0 } else { 1000 }, move || {
                agent.set(Some(Agent::Working(i)))
            }));
        }
        let agent = self.agent;
        steps.push(after(1000, move || agent.set(Some(Agent::Reported))));
        steps.extend(stream(&self.answer, ANSWER, 20).into_iter().map(|s| s.lead(20)));
        self.script.play(steps);
    }

    fn row(&self, agent: Agent) -> Div {
        let busy = self.script.busy();
        let finished = agent == Agent::Reported;
        let live = !finished && busy;
        let status = if finished {
            None
        } else if live {
            Some(TraceStatus::Running)
        } else {
            Some(TraceStatus::Error)
        };
        let mark = |status: Option<TraceStatus>| -> Element {
            match status {
                None => icon(Icon::CircleCheck).into_element(),
                Some(TraceStatus::Running) => spinner().into_element(),
                _ => icon(Icon::CircleAlert).into_element(),
            }
        };
        let verb = if finished {
            "Ran"
        } else if live {
            "Running"
        } else {
            "Failed to run"
        };
        let head = line(status, mark(status), &format!("{verb} the research subagent"));
        let ran = match agent {
            Agent::Started => return trace().child(head),
            Agent::Working(n) => n,
            Agent::Reported => STEPS.len(),
        };
        let rows = STEPS
            .iter()
            .zip(&self.steps)
            .take(ran.saturating_add(1))
            .enumerate()
            .map(|(i, ((command, out), open))| {
                if i < ran {
                    trace_collapsible()
                        .model(Model::bind(*open))
                        .child(trace_collapsible_trigger(false, done(&format!("Ran {command}"))))
                        .child(trace_collapsible_content().map_root(|c| c.child(output(out))))
                        .into_element()
                } else {
                    let s = if live { TraceStatus::Running } else { TraceStatus::Error };
                    let verb = if live { "Running" } else { "Failed to run" };
                    line(Some(s), mark(Some(s)), &format!("{verb} {command}")).into_element()
                }
            });
        let failed = status == Some(TraceStatus::Error);
        trace().child(
            trace_collapsible()
                .model(Model::bind(self.open))
                .child(trace_collapsible_trigger(failed, head))
                .child(trace_collapsible_content().map_root(|c| {
                    c.display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(12.0)
                        .child(trace().children(rows))
                        .when(finished, |c| c.child(text(REPORT)))
                })),
        )
    }

    fn view(&self) -> Div {
        let answer = self.answer.get();
        let agent = self.agent.get();
        let me = self.clone();
        thread()
            .child(user_turn(QUESTION))
            .when(agent.is_some() || !answer.is_empty(), |t| {
                t.child(assistant_turn([
                    agent.map_or(Element::Empty, |a| self.row(a).into_element()),
                    said(&answer),
                ]))
            })
            .child(replay(&self.script, move || me.replay()))
    }
}

pub fn render() -> Render {
    let research = Research::new();
    let details_open = Signal::labeled("details open", true);
    let running_open = Signal::labeled("running open", true);
    Box::new(move || {
        page(
            "Subagent",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(40.0)
                .child(demo("SubagentDemo.vue", research.view()))
                .child(demo("SubagentDetailsDemo.vue", details(&details_open)))
                .child(demo("SubagentRunningDemo.vue", running(&running_open))),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.click_text("Ran ls public/images | wc -l")?;
    s.frame("step output open")?;
    s.click_text("Replay")?;
    wait(s, 1300);
    s.frame("subagent working")?;
    Ok(())
}
