//! What an agent means to do and how far it got: a plan, a to-do queue and
//! the tasks it works through.

use gui::prelude::*;
use gui::{Div, Element, Icon, ScenarioError, Signal, div, icon, text};
use shadcn::{
    TraceStatus, spinner, trace, trace_collapsible, trace_collapsible_content, trace_collapsible_trigger,
    trace_description, trace_icon, trace_line, trace_title,
};

use super::thread::{Follow, Script, after, assistant_turn, replay, said, stream, thread, user_turn, wait};
use crate::{Render, Shot, demo, page};

/// How far a step of a plan or a to-do has come.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Progress {
    Done,
    Running,
    Pending,
}

impl Progress {
    /// Step `index` when `current` steps are done.
    fn of(index: usize, current: usize) -> Self {
        match index.cmp(&current) {
            std::cmp::Ordering::Less => Self::Done,
            std::cmp::Ordering::Equal => Self::Running,
            std::cmp::Ordering::Greater => Self::Pending,
        }
    }

    fn glyph(self) -> Icon {
        match self {
            Self::Done => Icon::CircleCheck,
            Self::Running => Icon::CircleDot,
            Self::Pending => Icon::Circle,
        }
    }
}

/// A collapsible row: `line` and, while `open`, `content`.
fn fold(open: &Signal<bool>, line: Div, content: impl FnOnce(Div) -> Div) -> Div {
    trace_collapsible()
        .child(trace_collapsible_trigger(open, false, line))
        .child(trace_collapsible_content(open, content))
}

fn line(status: Option<TraceStatus>, glyph: impl IntoElement, title: &str) -> Div {
    trace_line(status).child(trace_icon(glyph)).child(trace_title(title))
}

const PLAN_STEPS: [&str; 4] = [
    "Install @base-ui/react",
    "Replace the Radix imports",
    "Rename asChild to render",
    "Run the type checker",
];
const PLAN_QUESTION: &str = "Migrate the dialog components to Base UI.";
const PLAN_ANSWER: &str = "Migrated Dialog and AlertDialog. The type checker passes.";

/// PlanDemo: a plan whose steps tick off one by one.
#[derive(Clone)]
struct Plan {
    script: Script,
    replied: Signal<bool>,
    /// Steps done; `None` before the plan arrives.
    done: Signal<Option<usize>>,
    answer: Signal<String>,
    open: Signal<bool>,
}

impl Plan {
    fn new() -> Self {
        Self {
            script: Script::new("plan busy"),
            replied: Signal::new(true),
            done: Signal::new(Some(PLAN_STEPS.len())),
            answer: Signal::new(PLAN_ANSWER.to_owned()),
            open: Signal::new(true),
        }
    }

    fn replay(&self) {
        self.replied.set(false);
        self.done.set(None);
        self.answer.set(String::new());
        self.open.set(true);
        let (replied, done) = (self.replied.clone(), self.done.clone());
        let mut steps = vec![after(300, move || {
            replied.set(true);
            done.set(Some(0));
        })];
        for i in 1..=PLAN_STEPS.len() {
            let done = self.done.clone();
            steps.push(after(1200, move || done.set(Some(i))));
        }
        steps.extend(stream(&self.answer, PLAN_ANSWER, 20));
        self.script.play(steps);
    }

    fn view(&self) -> Div {
        let streaming = self.script.busy();
        let body = self.done.get().map(|done| {
            let status = (streaming && done < PLAN_STEPS.len()).then_some(TraceStatus::Running);
            let head = line(status, icon(Icon::Route), "Plan: migrate to Base UI")
                .child(trace_description(format!("{done} of {} done", PLAN_STEPS.len())));
            let rows = PLAN_STEPS.iter().enumerate().map(|(i, title)| {
                let p = Progress::of(i, done);
                let status = match p {
                    Progress::Pending => Some(TraceStatus::Pending),
                    Progress::Running if streaming => Some(TraceStatus::Running),
                    _ => None,
                };
                line(status, icon(p.glyph()), title)
            });
            trace()
                .child(fold(&self.open, head, |c| c.child(trace().children(rows))))
                .into_element()
        });
        let me = self.clone();
        thread()
            .child(user_turn(PLAN_QUESTION))
            .when(self.replied.get(), |t| {
                t.child(assistant_turn([
                    body.unwrap_or(Element::Empty),
                    said(&self.answer.get()),
                ]))
            })
            .child(replay(&self.script, move || me.replay()))
    }
}

fn plan_description(open: &Signal<bool>) -> Div {
    const STEPS: [&str; 4] = [
        "Add a dark variant to the theme tokens",
        "Read the theme from a cookie on the server",
        "Add a toggle to the settings page",
        "Check contrast on the charts",
    ];
    let head = line(None, icon(Icon::Route), "Plan: add dark mode")
        .child(trace_description(format!("1 of {} done", STEPS.len())));
    let rows = STEPS.iter().enumerate().map(|(i, step)| {
        let (status, glyph) = if i == 0 {
            (None, Icon::CircleCheck)
        } else {
            (Some(TraceStatus::Pending), Icon::Circle)
        };
        line(status, icon(glyph), step)
    });
    trace().max_w(96.0).child(fold(open, head, |c| {
        c.flex_col()
            .gap(3.0)
            .child(text(
                "Store the theme in a cookie so the server renders the right one and the page does not flash on load.",
            ))
            .child(trace().children(rows))
    }))
}

const TODOS: [&str; 4] = [
    "Add a dark variant to the theme tokens",
    "Read the theme from a cookie",
    "Add a toggle to the settings page",
    "Check contrast on the charts",
];
const QUEUE_QUESTION: &str = "Add dark mode to the settings page.";
const QUEUE_ANSWER: &str = "Dark mode is in. The toggle is under Settings, Appearance.";

/// QueueDemo: a to-do list the agent rewrites as it works.
#[derive(Clone)]
struct Queue {
    script: Script,
    /// To-dos done in the last list written; `None` before the first.
    done: Signal<Option<usize>>,
    answer: Signal<String>,
    /// The "to do" and "completed" sections' open state.
    open: [Signal<bool>; 2],
}

impl Queue {
    fn new() -> Self {
        Self {
            script: Script::new("queue busy"),
            done: Signal::new(Some(TODOS.len())),
            answer: Signal::new(QUEUE_ANSWER.to_owned()),
            open: [Signal::new(true), Signal::new(true)],
        }
    }

    fn replay(&self) {
        self.done.set(None);
        self.answer.set(String::new());
        for o in &self.open {
            o.set(true);
        }
        let mut steps = Vec::new();
        for i in 0..=TODOS.len() {
            let done = self.done.clone();
            steps.push(after(if i == 0 { 0 } else { 1400 }, move || done.set(Some(i))));
        }
        steps.extend(stream(&self.answer, QUEUE_ANSWER, 20).into_iter().map(|s| s.lead(20)));
        self.script.play(steps);
    }

    fn view(&self) -> Div {
        let busy = self.script.busy();
        let answer = self.answer.get();
        let body = self.done.get().map(|done| {
            let todos: Vec<(&str, Progress)> = TODOS
                .iter()
                .enumerate()
                .map(|(i, t)| (*t, Progress::of(i, done)))
                .collect();
            let pending: Vec<_> = todos.iter().filter(|(_, p)| *p != Progress::Done).copied().collect();
            let completed: Vec<_> = todos.iter().filter(|(_, p)| *p == Progress::Done).copied().collect();
            let sections = [
                (Icon::ListTodo, format!("{} to do", pending.len()), pending),
                (Icon::ListChecks, format!("{} completed", completed.len()), completed),
            ];
            trace()
                .children(
                    sections
                        .into_iter()
                        .zip(&self.open)
                        .filter(|((_, _, l), _)| !l.is_empty())
                        .map(|((glyph, title, list), open)| {
                            let rows = list.into_iter().map(|(todo, p)| {
                                let status = (busy && p == Progress::Running).then_some(TraceStatus::Running);
                                line(status, icon(p.glyph()), todo)
                            });
                            fold(open, line(None, icon(glyph), &title), |c| {
                                c.child(trace().children(rows))
                            })
                        }),
                )
                .into_element()
        });
        let replied = body.is_some() || !answer.is_empty();
        let me = self.clone();
        thread()
            .child(user_turn(QUEUE_QUESTION))
            .when(replied, |t| {
                t.child(assistant_turn([body.unwrap_or(Element::Empty), said(&answer)]))
            })
            .child(replay(&self.script, move || me.replay()))
    }
}

/// QueueDescriptionsDemo's to-dos: title, what it is about, whether done.
const DESCRIBED: [(&str, Option<&str>, bool); 5] = [
    (
        "Write the migration guide",
        Some("Cover the renamed props and the new data attributes."),
        true,
    ),
    ("Add a changeset", None, true),
    (
        "Fix the focus ring on the select trigger",
        Some("It is clipped by overflow-hidden on the field."),
        false,
    ),
    ("Update the registry snapshots", None, false),
    (
        "Add tests for the combobox",
        Some("Typing, clearing and keyboard selection."),
        false,
    ),
];

/// The open state of QueueDescriptionsDemo's sections and of each to-do.
struct Described {
    sections: [Signal<bool>; 2],
    todos: [Signal<bool>; 5],
}

impl Described {
    fn new() -> Self {
        Self {
            sections: [Signal::new(true), Signal::new(false)],
            todos: std::array::from_fn(|_| Signal::new(false)),
        }
    }

    fn view(&self) -> Div {
        let todo_row = |(title, description, done): (&str, Option<&str>, bool), open: &Signal<bool>| {
            let row = line(None, icon(if done { Icon::CircleCheck } else { Icon::Circle }), title);
            match description {
                Some(d) => fold(open, row, |c| c.child(text(d))),
                None => row,
            }
        };
        let section = |done: bool, glyph: Icon, open: &Signal<bool>| {
            let list: Vec<_> = DESCRIBED
                .iter()
                .zip(&self.todos)
                .filter(|((_, _, d), _)| *d == done)
                .collect();
            let title = if done {
                format!("{} completed", list.len())
            } else {
                format!("{} to do", list.len())
            };
            fold(open, line(None, icon(glyph), &title), |c| {
                c.child(trace().children(list.into_iter().map(|(t, o)| todo_row(*t, o))))
            })
        };
        let [pending, completed] = &self.sections;
        trace()
            .max_w(96.0)
            .child(section(false, Icon::ListTodo, pending))
            .child(section(true, Icon::ListChecks, completed))
    }
}

/// A task's titles while running and when finished, and its steps'.
struct TaskScript {
    title: [&'static str; 2],
    steps: &'static [[&'static str; 2]],
}

const TASKS: [TaskScript; 2] = [
    TaskScript {
        title: ["Finding the project files", "Found the project files"],
        steps: &[
            ["Searching for useSeoMeta", "Searched for useSeoMeta"],
            ["Reading app/app.vue", "Read app/app.vue"],
            ["Reading app/pages/index.vue", "Read app/pages/index.vue"],
        ],
    },
    TaskScript {
        title: ["Updating the page metadata", "Updated the page metadata"],
        steps: &[
            ["Editing app/app.vue", "Edited app/app.vue"],
            ["Editing app/pages/index.vue", "Edited app/pages/index.vue"],
        ],
    },
];
const TASK_QUESTION: &str = "Add Open Graph metadata to the home page.";
const TASK_ANSWER: &str = "Added Open Graph defaults to app/app.vue and a title, description and image for the home page in app/pages/index.vue.";

/// TaskDemo: tasks that open while they run and fold once done.
#[derive(Clone)]
struct Tasks {
    script: Script,
    /// Each task's steps done; `None` before it starts.
    done: [Signal<Option<usize>>; 2],
    answer: Signal<String>,
    open: [Follow; 2],
}

impl Tasks {
    fn new() -> Self {
        Self {
            script: Script::new("task busy"),
            done: TASKS.each_ref().map(|t| Signal::new(Some(t.steps.len()))),
            answer: Signal::new(TASK_ANSWER.to_owned()),
            open: [Follow::new(false), Follow::new(false)],
        }
    }

    fn replay(&self) {
        for (d, o) in self.done.iter().zip(&self.open) {
            d.set(None);
            o.reset(false);
        }
        self.answer.set(String::new());
        let mut steps = Vec::new();
        for ((task, done), open) in TASKS.iter().zip(&self.done).zip(&self.open) {
            for n in 0..=task.steps.len() {
                let (done, open) = (done.clone(), open.clone());
                let running = n < task.steps.len();
                steps.push(after(if steps.is_empty() { 0 } else { 900 }, move || {
                    done.set(Some(n));
                    open.follow(running);
                }));
            }
        }
        steps.extend(stream(&self.answer, TASK_ANSWER, 20).into_iter().map(|s| s.lead(900)));
        self.script.play(steps);
    }

    fn view(&self) -> Div {
        let busy = self.script.busy();
        let answer = self.answer.get();
        let rows: Vec<Div> = TASKS
            .iter()
            .zip(&self.done)
            .zip(&self.open)
            .filter_map(|((task, done), open)| {
                let done = done.get()?;
                let finished = done >= task.steps.len();
                let title = task.title.get(usize::from(finished)).copied().unwrap_or_default();
                let steps: Vec<(&str, Option<TraceStatus>)> = task
                    .steps
                    .iter()
                    .take(done.saturating_add(1))
                    .enumerate()
                    .map(|(i, [running, ended])| {
                        if i < done {
                            (*ended, None)
                        } else {
                            (
                                *running,
                                Some(if busy { TraceStatus::Running } else { TraceStatus::Error }),
                            )
                        }
                    })
                    .collect();
                let status = if steps.iter().any(|(_, s)| *s == Some(TraceStatus::Error)) {
                    Some(TraceStatus::Error)
                } else if steps.iter().any(|(_, s)| *s == Some(TraceStatus::Running)) {
                    Some(TraceStatus::Running)
                } else {
                    None
                };
                let mark: Element = match status {
                    Some(TraceStatus::Running) => spinner().into_element(),
                    Some(TraceStatus::Error) => icon(Icon::CircleAlert).into_element(),
                    _ => icon(Icon::CircleCheck).into_element(),
                };
                let head = trace_line(status).child(trace_icon(mark)).child(trace_title(title));
                let failed = status == Some(TraceStatus::Error);
                Some(
                    trace_collapsible()
                        .child(trace_collapsible_trigger(&open.open, failed, head))
                        .child(trace_collapsible_content(&open.open, |c| {
                            c.child(
                                trace().children(steps.into_iter().map(|(t, s)| trace_line(s).child(trace_title(t)))),
                            )
                        })),
                )
            })
            .collect();
        let replied = !rows.is_empty() || !answer.is_empty();
        let body = (!rows.is_empty()).then(|| trace().children(rows).into_element());
        let me = self.clone();
        thread()
            .child(user_turn(TASK_QUESTION))
            .when(replied, |t| {
                t.child(assistant_turn([body.unwrap_or(Element::Empty), said(&answer)]))
            })
            .child(replay(&self.script, move || me.replay()))
    }
}

pub fn render() -> Render {
    let plan = Plan::new();
    let described_plan = Signal::labeled("plan open", true);
    let queue = Queue::new();
    let described = Described::new();
    let tasks = Tasks::new();
    Box::new(move || {
        page(
            "Plan",
            div()
                .flex_col()
                .gap(10.0)
                .child(demo("PlanDemo.vue", plan.view()))
                .child(demo("PlanDescriptionDemo.vue", plan_description(&described_plan)))
                .child(demo("QueueDemo.vue", queue.view()))
                .child(demo("QueueDescriptionsDemo.vue", described.view()))
                .child(demo("TaskDemo.vue", tasks.view())),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.click_text("Fix the focus ring on the select trigger")?;
    s.frame("to-do description open")?;
    s.click_text("Found the project files")?;
    s.frame("task open")?;
    let (x, y) = s.locate("Replay")?;
    s.click(x, y);
    wait(s, 1500);
    s.frame("plan running")?;
    Ok(())
}
