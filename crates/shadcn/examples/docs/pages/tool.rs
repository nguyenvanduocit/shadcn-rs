//! What an agent ran: commands and their output, a command waiting for the
//! reader's approval, and a file read that opens in a sheet.

use gui::prelude::*;
use gui::{Div, Element, Icon, Model, ScenarioError, Signal, div, icon, text};
use shadcn::{
    ButtonSize, ButtonVariant, CodeBlockContentProps, CodeBlockProps, ItemMediaVariant, ItemSize, ItemVariant,
    SheetSide, TraceStatus, button, code_block, code_block_content, code_block_lines, dialog_trigger, item,
    item_actions, item_content, item_description, item_media, item_title, sheet, sheet_content, sheet_description,
    sheet_header, sheet_title, spinner, trace, trace_action, trace_collapsible, trace_collapsible_content,
    trace_collapsible_trigger, trace_end_icon, trace_icon, trace_line, trace_title,
};

use super::thread::{Follow, Script, Step, after, assistant_turn, output, said, stream, thread, user_turn, wait};
use crate::{Render, Shot, demo, page};

fn line(status: Option<TraceStatus>, glyph: Icon, title: &str) -> Div {
    trace_line(status)
        .child(trace_icon(icon(glyph)))
        .child(trace_title(title))
}

/// A command that ran (or failed) and opens to its output.
fn ran(open: &Signal<bool>, failed: bool, title: &str, out: &str) -> Element {
    let status = failed.then_some(TraceStatus::Error);
    trace_collapsible()
        .model(Model::bind(*open))
        .child(trace_collapsible_trigger(failed, line(status, Icon::Terminal, title)))
        .child(trace_collapsible_content().map_root(|c| c.child(output(out))))
        .into_element()
}

fn states(open: &[Signal<bool>; 2]) -> Div {
    let [status, dev] = open;
    trace()
        .max_width(384.0)
        .child(ran(
            status,
            false,
            "Ran git status --short",
            " M server/api/chat.post.ts\n M components/Chat.vue",
        ))
        .child(ran(
            dev,
            true,
            "Failed to run pnpm dev",
            "Command timed out after 120 seconds.",
        ))
        .child(line(None, Icon::Terminal, "Denied rm -rf dist"))
        .child(line(Some(TraceStatus::Running), Icon::Terminal, "Running pnpm test"))
}

/// ToolDemo's commands: what ran, and its output or error.
const COMMANDS: [(&str, Result<&str, &str>); 3] = [
    ("node --version", Ok("v24.18.0")),
    ("npm --version", Ok("11.6.2")),
    ("pnpm --version", Err("command not found: pnpm")),
];
const TOOL_QUESTION: &str = "Which package managers are installed?";
const TOOL_ANSWER: &str = "Node 24 and npm 11 are installed. pnpm is not.";

/// ToolDemo: commands that run one after another, then the answer.
#[derive(Clone)]
struct Commands {
    script: Script,
    replied: Signal<bool>,
    started: Signal<usize>,
    finished: Signal<usize>,
    answer: Signal<String>,
    /// Each finished command's open state: the failed one starts open in the
    /// first transcript, every one closed in a replay.
    open: [Signal<bool>; 3],
}

impl Commands {
    fn new() -> Self {
        Self {
            script: Script::new("tool busy"),
            replied: Signal::new(true),
            started: Signal::new(COMMANDS.len()),
            finished: Signal::new(COMMANDS.len()),
            answer: Signal::new(TOOL_ANSWER.to_owned()),
            open: COMMANDS.map(|(_, r)| Signal::new(r.is_err())),
        }
    }

    fn replay(&self) {
        self.replied.set(false);
        self.started.set(0);
        self.finished.set(0);
        self.answer.set(String::new());
        for o in &self.open {
            o.set(false);
        }
        let replied = self.replied;
        let mut steps = vec![after(300, move || replied.set(true))];
        for i in 1..=COMMANDS.len() {
            let (started, finished) = (self.started, self.finished);
            steps.push(after(0, move || started.set(i)));
            steps.push(after(900, move || finished.set(i)));
        }
        steps.extend(stream(&self.answer, TOOL_ANSWER, 20));
        self.script.play(steps);
    }

    fn view(&self) -> Div {
        let streaming = self.script.busy();
        let finished = self.finished.get();
        let rows = COMMANDS
            .iter()
            .zip(&self.open)
            .take(self.started.get())
            .enumerate()
            .map(|(i, ((command, result), open))| {
                if i < finished {
                    let (failed, verb, out) = match result {
                        Ok(out) => (false, "Ran", *out),
                        Err(e) => (true, "Failed to run", *e),
                    };
                    ran(open, failed, &format!("{verb} {command}"), out)
                } else if streaming {
                    line(
                        Some(TraceStatus::Running),
                        Icon::Terminal,
                        &format!("Running {command}"),
                    )
                    .into_element()
                } else {
                    line(
                        Some(TraceStatus::Error),
                        Icon::Terminal,
                        &format!("Failed to run {command}"),
                    )
                    .into_element()
                }
            })
            .collect::<Vec<_>>();
        let body = (!rows.is_empty()).then(|| trace().children(rows).into_element());
        let me = self.clone();
        thread()
            .child(user_turn(TOOL_QUESTION))
            .when(self.replied.get(), |t| {
                t.child(assistant_turn([
                    body.unwrap_or(Element::Empty),
                    said(&self.answer.get()),
                ]))
            })
            .child(super::thread::replay(&self.script, move || me.replay()))
    }
}

/// Where a command waiting for approval has come to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Approval {
    Requested,
    /// Answered, the agent not yet going on.
    Responded(bool),
    Ran,
    Denied,
}

const CLEANUP_QUESTION: &str = "Clean up the build folder.";
const CLEANUP_REASONING: &str =
    "The build output lives in dist. Deleting it cannot be undone, so I should ask before I run the command.";

/// ToolApprovalDemo and ConfirmationDemo: the agent asks before it runs a
/// command that cannot be undone.
#[derive(Clone)]
struct Cleanup {
    script: Script,
    asked: Signal<bool>,
    replied: Signal<bool>,
    thought: Signal<String>,
    thinking: Signal<bool>,
    approval: Signal<Option<Approval>>,
    reply: Signal<String>,
    open: Follow,
}

impl Cleanup {
    fn new(label: &'static str) -> Self {
        Self {
            script: Script::new(label),
            asked: Signal::new(false),
            replied: Signal::new(false),
            thought: Signal::new(String::new()),
            thinking: Signal::new(false),
            approval: Signal::new(None),
            reply: Signal::new(String::new()),
            open: Follow::new(false),
        }
    }

    fn send(&self) {
        self.asked.set(true);
        self.open.reset(false);
        let (replied, thinking, open) = (self.replied, self.thinking, self.open.clone());
        let mut steps: Vec<Step> = vec![after(600, move || {
            replied.set(true);
            thinking.set(true);
            open.follow(true);
        })];
        steps.extend(stream(&self.thought, CLEANUP_REASONING, 20));
        let (thinking, open) = (self.thinking, self.open.clone());
        steps.push(after(20, move || {
            thinking.set(false);
            open.follow(false);
        }));
        let approval = self.approval;
        steps.push(after(400, move || approval.set(Some(Approval::Requested))));
        self.script.play(steps);
    }

    fn respond(&self, approved: bool) {
        self.approval.set(Some(Approval::Responded(approved)));
        let approval = self.approval;
        let mut steps = vec![after(600, move || {
            approval.set(Some(if approved { Approval::Ran } else { Approval::Denied }));
        })];
        let words = if approved {
            "Removed the dist folder."
        } else {
            "Okay, I left the dist folder in place."
        };
        steps.extend(stream(&self.reply, words, 20));
        self.script.play(steps);
    }

    fn reset(&self) {
        self.script.stop();
        self.asked.set(false);
        self.replied.set(false);
        self.thought.set(String::new());
        self.thinking.set(false);
        self.approval.set(None);
        self.reply.set(String::new());
    }

    fn request(&self, command: &str) -> Div {
        let (deny, approve) = (self.clone(), self.clone());
        item(ItemVariant::Outline, ItemSize::Sm)
            .child(item_media(ItemMediaVariant::Icon).child(icon(Icon::Terminal)))
            .child(
                item_content()
                    .child(item_title(format!("Run {command}?")))
                    .child(item_description("The command runs in this project.")),
            )
            .child(
                item_actions()
                    .child(
                        button(ButtonVariant::Outline, ButtonSize::Sm)
                            .child("Deny")
                            .on_click(move |_| deny.respond(false)),
                    )
                    .child(
                        button(ButtonVariant::Default, ButtonSize::Sm)
                            .child("Approve")
                            .on_click(move |_| approve.respond(true)),
                    ),
            )
    }

    fn view(&self) -> Div {
        const COMMAND: &str = "rm -rf dist";
        let thinking = self.thinking.get();
        let status = thinking.then_some(TraceStatus::Running);
        let mark: Element = if thinking {
            spinner().into_element()
        } else {
            icon(Icon::CircleCheck).into_element()
        };
        let open = &self.open.open;
        let reasoning = trace()
            .child(
                trace_collapsible()
                    .model(Model::bind(*open))
                    .child(trace_collapsible_trigger(
                        false,
                        trace_line(status)
                            .child(trace_icon(mark))
                            .child(trace_title(if thinking { "Thinking" } else { "Thought" })),
                    ))
                    .child(trace_collapsible_content().map_root(|c| c.child(text(self.thought.get())))),
            )
            .into_element();
        let tool: Element = match self.approval.get() {
            None => Element::Empty,
            Some(Approval::Requested) => self.request(COMMAND).into_element(),
            Some(a) => {
                let denied = matches!(a, Approval::Denied | Approval::Responded(false));
                let (verb, status) = match a {
                    _ if denied => ("Denied", None),
                    Approval::Ran => ("Ran", None),
                    _ => ("Running", Some(TraceStatus::Running)),
                };
                trace()
                    .child(line(status, Icon::Terminal, &format!("{verb} {COMMAND}")))
                    .into_element()
            }
        };
        let asked = self.asked.get();
        let me = self.clone();
        let action = (!self.script.busy()).then(|| {
            button(ButtonVariant::Outline, ButtonSize::Sm)
                .margin_top(Length::Auto)
                .align_self(Align::Start)
                .child(if asked { "Reset" } else { "Send message" })
                .on_click(move |_| if asked { me.reset() } else { me.send() })
        });
        thread()
            .when(asked, |t| t.child(user_turn(CLEANUP_QUESTION)))
            .when(self.replied.get(), |t| {
                t.child(assistant_turn([reasoning, tool, said(&self.reply.get())]))
            })
            .child(action)
    }
}

const PACKAGE_JSON: &str = r#"{
  "name": "acme-chat",
  "private": true,
  "dependencies": {
    "@ai-sdk/vue": "^4.0.0",
    "ai": "^7.0.0",
    "nuxt": "^4.2.0"
  }
}"#;
const SHEET_QUESTION: &str = "Which AI SDK version does this app use?";
const SHEET_ANSWER: &str = "It uses ai 7 with @ai-sdk/vue 4.";

/// ToolSheetDemo: a file the agent read, opened in a sheet.
#[derive(Clone)]
struct FileRead {
    script: Script,
    asked: Signal<bool>,
    replied: Signal<bool>,
    read: Signal<bool>,
    answer: Signal<String>,
}

impl FileRead {
    fn new() -> Self {
        Self {
            script: Script::new("tool sheet busy"),
            asked: Signal::new(false),
            replied: Signal::new(false),
            read: Signal::new(false),
            answer: Signal::new(String::new()),
        }
    }

    fn send(&self) {
        self.asked.set(true);
        let (replied, read) = (self.replied, self.read);
        let mut steps = vec![
            after(300, move || replied.set(true)),
            after(900, move || read.set(true)),
        ];
        steps.extend(stream(&self.answer, SHEET_ANSWER, 20));
        self.script.play(steps);
    }

    fn reset(&self) {
        self.script.stop();
        self.asked.set(false);
        self.replied.set(false);
        self.read.set(false);
        self.answer.set(String::new());
    }

    fn file(&self) -> Div {
        const PATH: &str = "package.json";
        let lines = PACKAGE_JSON.lines().count();
        let content = CodeBlockContentProps {
            line_numbers: true,
            ..CodeBlockContentProps::default()
        };
        let p = CodeBlockProps::default();
        trace().child(
            sheet()
                .child(dialog_trigger(trace_action(
                    false,
                    line(None, Icon::FileText, &format!("Read {PATH}")).child(trace_end_icon(icon(Icon::ChevronRight))),
                )))
                .child(
                    sheet_content(SheetSide::Right)
                        .child(
                            sheet_header()
                                .child(sheet_title(PATH).font_family(FontFamily::GEIST_MONO))
                                .child(sheet_description(format!("{lines} lines"))),
                        )
                        .child(
                            code_block(p)
                                .margin_x(24.0)
                                .margin_bottom(24.0)
                                .width(Length::Auto)
                                .child(code_block_content(p, content, code_block_lines(PACKAGE_JSON))),
                        ),
                ),
        )
    }

    fn view(&self) -> Div {
        let streaming = self.script.busy();
        let tool = if self.read.get() {
            self.file()
        } else if streaming {
            trace().child(line(Some(TraceStatus::Running), Icon::FileText, "Reading package.json"))
        } else {
            trace().child(line(
                Some(TraceStatus::Error),
                Icon::FileText,
                "Failed to read package.json",
            ))
        };
        let asked = self.asked.get();
        let me = self.clone();
        let action = (!streaming).then(|| {
            button(ButtonVariant::Outline, ButtonSize::Sm)
                .margin_top(Length::Auto)
                .align_self(Align::Start)
                .child(if asked { "Reset" } else { "Send message" })
                .on_click(move |_| if asked { me.reset() } else { me.send() })
        });
        thread()
            .when(asked, |t| t.child(user_turn(SHEET_QUESTION)))
            .when(self.replied.get(), |t| {
                t.child(assistant_turn([tool.into_element(), said(&self.answer.get())]))
            })
            .child(action)
    }
}

pub fn render() -> Render {
    let open_states = [
        Signal::labeled("git status open", false),
        Signal::labeled("pnpm dev open", false),
    ];
    let commands = Commands::new();
    let approval = Cleanup::new("tool approval busy");
    let confirmation = Cleanup::new("confirmation busy");
    let file = FileRead::new();
    Box::new(move || {
        page(
            "Tool",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(40.0)
                .child(demo("ToolDemo.vue", commands.view()))
                .child(demo("ToolStatesDemo.vue", states(&open_states)))
                .child(demo("ToolApprovalDemo.vue", approval.view()))
                .child(demo("ConfirmationDemo.vue", confirmation.view()))
                .child(demo("ToolSheetDemo.vue", file.view())),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.click_text("Ran git status --short")?;
    s.frame("output open")?;
    s.click_text("Send message")?;
    wait(s, 900);
    s.frame("thinking")?;
    wait(s, 1600);
    s.frame("approval asked")?;
    s.click_text("Approve")?;
    wait(s, 900);
    s.frame("approved")?;
    Ok(())
}
