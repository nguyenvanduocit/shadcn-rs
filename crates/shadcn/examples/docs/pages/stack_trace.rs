//! An error's stack as an agent's step shows it: the error line, a link per
//! frame of the app's own code, and the library frames folded away.

use gui::prelude::*;
use gui::{Div, Element, Icon, Model, ScenarioError, Signal, div, icon, text};
use shadcn::{
    TraceStatus, trace, trace_collapsible, trace_collapsible_content, trace_collapsible_trigger, trace_icon,
    trace_line, trace_title,
};

use super::thread::{Script, assistant_turn, chunks, frames, output, replay, said, thread, user_turn};
use crate::{Render, Shot, demo, page};

const QUESTION: &str = "Reset the database and seed it.";
const ANSWER: &str =
    "The migrations ran, but the seed failed in seedUsers. One fixture has no profile, so profile.email is undefined.";
const RESET: &str = "pnpm prisma migrate reset --skip-seed";
const SEED: &str = "pnpm prisma db seed";

const SEED_STACK: &str = "TypeError: Cannot read properties of undefined (reading 'email')
    at seedUsers (prisma/seed.ts:27:34)
    at main (prisma/seed.ts:51:9)
    at Module._compile (node:internal/modules/cjs/loader:1554:14)
    at Object.transformer (node_modules/tsx/dist/register.cjs:3:1104)
    at Module.load (node:internal/modules/cjs/loader:1289:32)
    at process.processTicksAndRejections (node:internal/process/task_queues:105:5)";

const RESET_OUTPUT: &str = "Database reset successful

The following migrations have been applied:

migrations/
  └─ 20260912_init/
  └─ 20260920_add_profiles/";

const INVOICE_STACK: &str = "TypeError: Cannot read properties of undefined (reading 'total')
    at formatInvoice (app/utils/billing.ts:42:31)
    at setup (app/components/InvoiceRow.vue:18:12)
    at callWithErrorHandling (node_modules/@vue/runtime-core/dist/runtime-core.cjs.js:200:19)
    at setupStatefulComponent (node_modules/@vue/runtime-core/dist/runtime-core.cjs.js:7848:25)
    at setupComponent (node_modules/@vue/runtime-core/dist/runtime-core.cjs.js:7809:36)
    at mountComponent (node_modules/@vue/runtime-core/dist/runtime-core.cjs.js:5159:7)";

/// One `at name (file:line:column)` line of a stack.
struct Frame {
    line: String,
    /// `name in file:line`, when the line has that shape.
    place: Option<String>,
    /// In a dependency or Node itself, not the app's own code.
    internal: bool,
}

/// `at name (file:line:column)` as its name, file and line.
fn place(line: &str) -> Option<String> {
    let rest = line.strip_prefix("at ")?.strip_suffix(')')?;
    let (name, location) = rest.split_once(" (")?;
    let mut parts = location.rsplitn(3, ':');
    let (_column, number, file) = (parts.next()?, parts.next()?, parts.next()?);
    number.parse::<u32>().ok()?;
    Some(format!("{name} in {file}:{number}"))
}

/// A stack: its error line, then the app's frames and the internal ones.
struct Stack {
    error: String,
    app: Vec<Frame>,
    internal: Vec<Frame>,
}

/// Split `stack` into its frames; a frame is internal when its line names a
/// path that `internal` matches.
fn parse(stack: &str, internal: impl Fn(&str) -> bool) -> Stack {
    let mut lines = stack.split('\n').map(str::trim);
    let error = lines.next().unwrap_or_default().to_owned();
    let (internal, app) = lines
        .map(|line| Frame {
            line: line.to_owned(),
            place: place(line),
            internal: internal(line),
        })
        .partition(|f| f.internal);
    Stack { error, app, internal }
}

/// What a failed step shows when opened: the error, a link per app frame,
/// and the internal frames folded away under `frames`.
fn failure(content: Div, stack: &Stack, frames: &Signal<bool>) -> Div {
    let t = shadcn::theme();
    content
        .flex_col()
        .gap(2.0)
        .child(
            text(stack.error.clone())
                .font_mono()
                .text_xs()
                .text_color(t.destructive),
        )
        .child(
            trace()
                .children(stack.app.iter().map(|f| {
                    // An `<a href>` around the line: a link the browser follows.
                    div().cursor_pointer().focusable().child(
                        trace_line(None)
                            .child(trace_icon(icon(Icon::FileCode)))
                            .child(trace_title(f.place.clone().unwrap_or_else(|| f.line.clone()))),
                    )
                }))
                .child(
                    trace_collapsible()
                        .model(Model::bind(*frames))
                        .child(trace_collapsible_trigger(
                            false,
                            trace_line(None)
                                .child(trace_icon(icon(Icon::Layers)))
                                .child(trace_title(format!("{} framework frames", stack.internal.len()))),
                        ))
                        .child(trace_collapsible_content().map_root(|c| {
                            c.child(
                                div()
                                    .flex_col()
                                    .gap(1.0)
                                    .overflow_x_scroll()
                                    .rounded_md()
                                    .bg(t.muted)
                                    .p(3.0)
                                    .font_mono()
                                    .text_xs()
                                    .children(stack.internal.iter().map(|f| text(f.line.clone()).whitespace_nowrap())),
                            )
                        })),
                ),
        )
}

/// Where a step of the turn is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Called, no result yet: running while the turn plays, failed after.
    Called,
    Ran,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
struct Turn {
    reset: Option<Step>,
    seed: Option<Step>,
    text: String,
}

impl Turn {
    fn finished() -> Self {
        Self {
            reset: Some(Step::Ran),
            seed: Some(Step::Failed),
            text: ANSWER.to_owned(),
        }
    }

    /// The reset runs for 900 ms, the seed for 1200 ms, then the answer
    /// streams a word every 20 ms.
    fn replay() -> Vec<(u64, Self)> {
        let at = |reset, seed| Self {
            reset: Some(reset),
            seed,
            text: String::new(),
        };
        let mut steps = vec![
            (0, at(Step::Called, None)),
            (900, at(Step::Ran, Some(Step::Called))),
            (1200, at(Step::Ran, Some(Step::Failed))),
        ];
        let mut text = String::new();
        for word in chunks(ANSWER) {
            text.push_str(word);
            steps.push((
                20,
                Self {
                    text: text.clone(),
                    ..Self::finished()
                },
            ));
        }
        steps
    }
}

/// The open state of the demo's foldable steps.
#[derive(Clone)]
struct Folds {
    reset: Signal<bool>,
    seed: Signal<bool>,
    frames: Signal<bool>,
}

impl Folds {
    /// As each step first shows: the failed seed open, the rest closed.
    fn set_default(&self) {
        self.reset.set(false);
        self.seed.set(true);
        self.frames.set(false);
    }
}

fn called(command: &str, busy: bool) -> Div {
    trace_line(Some(if busy { TraceStatus::Running } else { TraceStatus::Error }))
        .child(trace_icon(icon(Icon::Terminal)))
        .child(trace_title(format!(
            "{} {command}",
            if busy { "Running" } else { "Failed to run" }
        )))
}

fn ran(open: &Signal<bool>, command: &str, out: &str) -> Element {
    trace_collapsible()
        .model(Model::bind(*open))
        .child(trace_collapsible_trigger(
            false,
            trace_line(None)
                .child(trace_icon(icon(Icon::Terminal)))
                .child(trace_title(format!("Ran {command}"))),
        ))
        .child(trace_collapsible_content().map_root(|c| c.child(output(out))))
        .into_element()
}

fn failed(open: &Signal<bool>, frames: &Signal<bool>, stack: &Stack, glyph: Icon) -> Element {
    trace_collapsible()
        .model(Model::bind(*open))
        .child(trace_collapsible_trigger(
            true,
            trace_line(Some(TraceStatus::Error))
                .child(trace_icon(icon(glyph)))
                .child(trace_title(format!("Failed: {}", stack.error))),
        ))
        .child(trace_collapsible_content().map_root(|c| failure(c, stack, frames)))
        .into_element()
}

fn stack_trace_demo(turn: &Signal<Turn>, folds: &Folds, script: &Script) -> Div {
    let now = turn.get();
    let busy = script.busy();
    let seed_stack = parse(SEED_STACK, |l| l.contains("node_modules") || l.contains("node:"));
    let part = |step: Option<Step>, command: &str| match step {
        None => None,
        Some(Step::Called) => Some(called(command, busy).into_element()),
        Some(Step::Ran) => Some(ran(&folds.reset, command, RESET_OUTPUT)),
        Some(Step::Failed) => Some(failed(&folds.seed, &folds.frames, &seed_stack, Icon::Terminal)),
    };
    let steps = now
        .reset
        .is_some()
        .then(|| trace().child(part(now.reset, RESET)).child(part(now.seed, SEED)));
    let (turn, folds, play) = (*turn, folds.clone(), script.clone());
    thread()
        .child(user_turn(QUESTION))
        .child(assistant_turn([steps.into_element(), said(&now.text)]))
        .child(replay(script, move || {
            let (turn, folds) = (turn, folds.clone());
            play.play(frames(Turn::replay(), move |step: &Turn| {
                // A step that changes shape mounts anew, in its default state.
                let before = turn.get();
                if before.reset != step.reset || before.seed != step.seed {
                    folds.set_default();
                }
                turn.set(step.clone());
            }));
        }))
}

pub fn render() -> Render {
    let turn = Signal::labeled("stack turn", Turn::finished());
    let folds = Folds {
        reset: Signal::labeled("reset open", false),
        seed: Signal::labeled("seed open", true),
        frames: Signal::labeled("seed frames open", false),
    };
    let script = Script::new("stack replay");
    let invoice_open = Signal::labeled("invoice open", true);
    let invoice_frames = Signal::labeled("invoice frames open", false);
    Box::new(move || {
        let invoice = parse(INVOICE_STACK, |l| l.contains("node_modules"));
        page(
            "Stack trace",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo("StackTraceDemo.vue", stack_trace_demo(&turn, &folds, &script)))
                .child(demo(
                    "StackTraceFramesDemo.vue",
                    trace()
                        .max_w(96.0)
                        .child(failed(&invoice_open, &invoice_frames, &invoice, Icon::CircleX)),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("4 framework frames")?;
    s.click(x, y);
    s.frame("seed frames opened")?;
    s.click_text("Replay")?;
    s.frame("replay started")?;
    Ok(())
}
