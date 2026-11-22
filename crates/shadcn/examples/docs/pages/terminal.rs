//! The commands an agent runs and what they print, each folded under its
//! command line and open while it runs, until the reader opens or closes it.

use gui::prelude::*;
use gui::{Div, Element, Icon, Model, ScenarioError, Signal, div, icon};
use shadcn::{
    TraceStatus, trace, trace_collapsible, trace_collapsible_content, trace_collapsible_trigger, trace_icon,
    trace_line, trace_title,
};

use super::thread::{Follow, Script, assistant_turn, chunks, frames, output, replay, said, thread, user_turn};
use crate::{Render, Shot, demo, page};

const QUESTION: &str = "Run the checks, then build the app.";
const ANSWER: &str = "Lint and tests pass, and the build compiled both routes.";

/// Each command the agent runs and the lines it prints.
const COMMANDS: [(&str, &[&str]); 3] = [
    ("pnpm lint", &["> acme-chat@0.1.0 lint", "> eslint ."]),
    (
        "pnpm test",
        &[
            "> acme-chat@0.1.0 test",
            "> vitest run",
            "",
            " ✓ utils/ai.test.ts (6 tests) 41ms",
            " ✓ components/Chat.test.ts (9 tests) 212ms",
            "",
            " Test Files  2 passed (2)",
            "      Tests  15 passed (15)",
        ],
    ),
    (
        "pnpm build",
        &[
            "> acme-chat@0.1.0 build",
            "> nuxt build",
            "",
            "Nuxt 4.2.0 with Nitro 2.12.0",
            "ℹ Building client...",
            "✔ Client built in 4210ms",
            "ℹ Building server...",
            "✔ Server built in 1180ms",
            "",
            "✔ Generated public .output/public",
            "├─ .output/server/chunks/routes/index.mjs (1.9 kB)",
            "└─ .output/server/chunks/routes/api/chat.post.mjs (141 B)",
        ],
    ),
];

const LINT_OUTPUT: &str = "> acme-chat@0.1.0 lint
> eslint .

server/api/chat.post.ts
  14:7  error  'model' is assigned a value but never used  no-unused-vars

components/Chat.vue
  42:5  error  'messages' is defined but never used  vue/no-unused-vars

✖ 2 problems (2 errors, 0 warnings)

 ELIFECYCLE  Command failed with exit code 1.";

/// One command as the transcript shows it.
#[derive(Clone, Debug, PartialEq)]
struct Command {
    command: &'static str,
    /// It exited (with code 0); until then it is still running.
    done: bool,
    output: String,
}

/// The agent's turn at one moment of the replay.
#[derive(Clone, Debug, PartialEq)]
struct Turn {
    commands: Vec<Command>,
    text: String,
}

impl Turn {
    /// The finished turn the demo opens with.
    fn finished() -> Self {
        Self {
            commands: COMMANDS
                .iter()
                .map(|&(command, lines)| Command {
                    command,
                    done: true,
                    output: lines.join("\n"),
                })
                .collect(),
            text: ANSWER.to_owned(),
        }
    }

    /// Every moment of the replay and how many ms after the last it comes:
    /// each command prints a line every 200 ms, then the answer streams a
    /// word every 20 ms.
    fn replay() -> Vec<(u64, Self)> {
        let mut steps = Vec::new();
        let mut ran: Vec<Command> = Vec::new();
        for (command, lines) in COMMANDS {
            for shown in 1..=lines.len() {
                let mut commands = ran.clone();
                commands.push(Command {
                    command,
                    done: false,
                    output: lines.get(..shown).unwrap_or_default().join("\n"),
                });
                let delay = if steps.is_empty() { 0 } else { 200 };
                steps.push((
                    delay,
                    Self {
                        commands,
                        text: String::new(),
                    },
                ));
            }
            ran.push(Command {
                command,
                done: true,
                output: lines.join("\n"),
            });
        }
        steps.push((
            200,
            Self {
                commands: ran.clone(),
                text: String::new(),
            },
        ));
        let mut text = String::new();
        for word in chunks(ANSWER) {
            text.push_str(word);
            steps.push((
                20,
                Self {
                    commands: ran.clone(),
                    text: text.clone(),
                },
            ));
        }
        steps
    }
}

/// A command's row, open while it runs and prints until the user opens or
/// closes it (Vue's `openState`).
fn command_row(c: &Command, busy: bool, fold: &Follow) -> Element {
    let running = busy && !c.done;
    let failed = !running && !c.done;
    let title = if c.done {
        format!("Ran {}", c.command)
    } else if running {
        format!("Running {}", c.command)
    } else {
        format!("Failed to run {}", c.command)
    };
    let status = if running {
        Some(TraceStatus::Running)
    } else if failed {
        Some(TraceStatus::Error)
    } else {
        None
    };
    trace_collapsible()
        .model(Model::bind(fold.open))
        .child(trace_collapsible_trigger(
            failed,
            trace_line(status)
                .child(trace_icon(icon(Icon::Terminal)))
                .child(trace_title(title)),
        ))
        .child(trace_collapsible_content().map_root(|c2| c2.child(output(&c.output))))
        .into_element()
}

fn terminal_demo(turn: &Signal<Turn>, folds: &[Follow], script: &Script) -> Div {
    let now = turn.get();
    let busy = script.busy();
    let rows = (!now.commands.is_empty()).then(|| {
        trace().children(
            now.commands
                .iter()
                .zip(folds)
                .map(|(c, fold)| command_row(c, busy, fold)),
        )
    });
    let (turn, folds, play) = (*turn, folds.to_vec(), script.clone());
    thread()
        .child(user_turn(QUESTION))
        .child(assistant_turn([rows.into_element(), said(&now.text)]))
        .child(replay(script, move || {
            for f in &folds {
                f.reset(f.open.get());
            }
            let (turn, folds) = (turn, folds.clone());
            play.play(frames(Turn::replay(), move |step: &Turn| {
                for (c, f) in step.commands.iter().zip(&folds) {
                    f.follow(!c.done && !c.output.is_empty());
                }
                turn.set(step.clone());
            }));
        }))
}

fn error_demo(open: &Signal<bool>) -> Div {
    trace()
        .max_width(384.0)
        .child(
            trace_line(None)
                .child(trace_icon(icon(Icon::Terminal)))
                .child(trace_title("Ran pnpm install")),
        )
        .child(
            trace_collapsible()
                .model(Model::bind(*open))
                .child(trace_collapsible_trigger(
                    true,
                    trace_line(Some(TraceStatus::Error))
                        .child(trace_icon(icon(Icon::Terminal)))
                        .child(trace_title("pnpm lint exited with code 1")),
                ))
                .child(trace_collapsible_content().map_root(|c| c.child(output(LINT_OUTPUT)))),
        )
}

pub fn render() -> Render {
    let turn = Signal::labeled("terminal turn", Turn::finished());
    // Vue opens the last command when the transcript is not streaming.
    let folds = [Follow::new(false), Follow::new(false), Follow::new(true)];
    let script = Script::new("terminal replay");
    let lint = Signal::labeled("lint open", true);
    Box::new(move || {
        page(
            "Terminal",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo("TerminalDemo.vue", terminal_demo(&turn, &folds, &script)))
                .child(demo("TerminalErrorDemo.vue", error_demo(&lint))),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.click_text("Ran pnpm lint")?;
    s.frame("lint opened")?;
    s.click_text("pnpm lint exited with code 1")?;
    s.frame("error closed")?;
    s.click_text("Replay")?;
    s.frame("replay started")?;
    Ok(())
}
