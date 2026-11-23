use gui::prelude::*;
use gui::{Div, FocusHandle, Model, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{
    ButtonSize, ButtonVariant, DialogProps, Part, Questionnaire, QuestionnaireAnswer, QuestionnaireItem,
    QuestionnaireItemProps, QuestionnaireProgressState, QuestionnaireProps, QuestionnaireShortcuts,
    QuestionnaireStatus, Toast, ToastKind, button, card, card_action, card_content, card_description, card_footer,
    card_header, dialog, dialog_close, dialog_content, dialog_description, dialog_footer, dialog_header, dialog_title,
    dialog_trigger, native_select, native_select_option, questionnaire, questionnaire_actions, questionnaire_choice,
    questionnaire_choices, questionnaire_description, questionnaire_error, questionnaire_error_with,
    questionnaire_input, questionnaire_item, questionnaire_next, questionnaire_previous, questionnaire_progress,
    questionnaire_progress_with, questionnaire_reset, questionnaire_skip, questionnaire_submit, questionnaire_title,
    theme, toaster,
};

use crate::{Render, Shot, demo, page};

/// The value a single-answer question was submitted with, or `fallback`.
fn value_of(answers: &[QuestionnaireAnswer], name: &str, fallback: &str) -> String {
    answers
        .iter()
        .find(|a| a.name == name)
        .and_then(|a| a.values.first().cloned())
        .unwrap_or_else(|| fallback.to_owned())
}

/// Every value a question was submitted with, joined, or `None`.
fn values_of(answers: &[QuestionnaireAnswer], name: &str) -> String {
    let values = answers
        .iter()
        .find(|a| a.name == name)
        .map(|a| a.values.join(", "))
        .unwrap_or_default();
    if values.is_empty() { "None".to_owned() } else { values }
}

/// `toast(title, { description })`.
fn toast(title: &str, description: String) {
    shadcn::toast(Toast::new(ToastKind::Default, title).description(description));
}

/// A question's choices, each `(value, label)`.
fn choices(entries: &[(&str, &str)]) -> Div {
    questionnaire_choices().children(
        entries
            .iter()
            .map(|(value, label)| questionnaire_choice(value, text((*label).to_owned()), None)),
    )
}

/// The common item: title, an optional description, choices and the error.
fn simple_item(
    name: &str,
    title: &str,
    description: Option<&str>,
    entries: &[(&str, &str)],
) -> Part<QuestionnaireItemProps> {
    questionnaire_item(name)
        .child(questionnaire_title(title.to_owned()))
        .child(description.map(|d| questionnaire_description(d.to_owned())))
        .child(choices(entries))
        .child(questionnaire_error())
}

/// Previous, next and submit.
fn actions(submit: &str) -> Div {
    questionnaire_actions()
        .child(questionnaire_previous("Previous"))
        .child(questionnaire_next("Next"))
        .child(questionnaire_submit(submit))
}

fn item(name: &str, choices: &[&str]) -> QuestionnaireItem {
    QuestionnaireItem::new(name, choices).required()
}

fn the_demo(saved: &Signal<String>) -> Part<QuestionnaireProps> {
    let s = *saved;
    questionnaire(vec![
        QuestionnaireItem::new("direction", &["tool-calls", "approvals", "handoffs"])
            .required()
            .with_input(),
        QuestionnaireItem::new("signals", &["progress", "decisions", "risks", "next-step"]).multiple(),
        QuestionnaireItem::new("timing", &["now", "next-cycle", "backlog"]).required(),
    ])
    .default_item("direction")
    .shortcuts(Some(QuestionnaireShortcuts::Letters))
    .on_submit(move |(_, answers): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        let summary: Vec<String> = answers
            .iter()
            .map(|a| format!("{}: {}", a.name, values_of(&answers, &a.name)))
            .collect();
        s.set(format!("Agent plan saved · {}", summary.join(" · ")));
    })
}

fn questionnaire_demo(root: Part<QuestionnaireProps>) -> Part<QuestionnaireProps> {
    let choice = |name: &str, value: &str, label: &str, description: Option<&str>| {
        let _ = name;
        questionnaire_choice(
            value,
            text(label.to_owned()).when(description.is_some(), |t| t.font_weight(500)),
            description,
        )
    };
    root.margin_x(Length::Auto)
        .max_width(448.0)
        .child(questionnaire_progress())
        .child(
            questionnaire_item("direction")
                .child(questionnaire_title("What should the agent build next?"))
                .child(questionnaire_description(
                    "Choose a direction or describe another task.",
                ))
                .child(
                    questionnaire_choices()
                        .child(choice(
                            "direction",
                            "tool-calls",
                            "Tool call timeline",
                            Some("Show what the agent ran and what came back."),
                        ))
                        .child(choice(
                            "direction",
                            "approvals",
                            "Approval checkpoints",
                            Some("Ask before sensitive or destructive actions."),
                        ))
                        .child(choice(
                            "direction",
                            "handoffs",
                            "Sub-agent handoffs",
                            Some("Make delegated work and results easier to follow."),
                        ))
                        .child(questionnaire_input().placeholder("Describe another feature…")),
                )
                .child(questionnaire_error()),
        )
        .child(
            questionnaire_item("signals")
                .child(questionnaire_title("What should every progress update include?"))
                .child(questionnaire_description(
                    "Select all that apply, or skip this question.",
                ))
                .child(
                    questionnaire_choices()
                        .child(choice("signals", "progress", "Progress", None))
                        .child(choice("signals", "decisions", "Decisions", None))
                        .child(choice("signals", "risks", "Risks", None))
                        .child(choice("signals", "next-step", "Next step", None)),
                )
                .child(questionnaire_error()),
        )
        .child(
            questionnaire_item("timing")
                .child(questionnaire_title("When should work begin?"))
                .child(questionnaire_description(
                    "Choose when the agent should begin the work.",
                ))
                .child(
                    questionnaire_choices()
                        .child(choice("timing", "now", "Start now", None))
                        .child(choice("timing", "next-cycle", "Next development cycle", None))
                        .child(choice("timing", "backlog", "Add it to the backlog", None)),
                )
                .child(questionnaire_error()),
        )
        .child(
            questionnaire_actions()
                .child(questionnaire_previous("Previous"))
                .child(questionnaire_skip("Skip"))
                .child(questionnaire_next("Next"))
                .child(questionnaire_submit("Save plan")),
        )
}

fn animated() -> Part<QuestionnaireProps> {
    questionnaire(vec![
        item("task", &["implement", "debug", "review"]),
        item("review", &["targeted", "complete", "manual"]),
        item("delivery", &["summary", "diff", "handoff"]),
    ])
    .default_item("task")
    .on_submit(move |(_, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        toast(
            "Agent workflow saved",
            format!(
                "Task: {} · Review: {} · Delivery: {}",
                value_of(&a, "task", "None"),
                value_of(&a, "review", "None"),
                value_of(&a, "delivery", "None")
            ),
        );
    })
}

/// Vue fades and slides each item in as it opens; `gui` draws no entrance
/// animations, so an item shows at once, as Vue does under `motion-reduce`.
fn animated_demo(root: Part<QuestionnaireProps>) -> Part<QuestionnaireProps> {
    root.margin_x(Length::Auto)
        .max_width(448.0)
        .child(questionnaire_progress())
        .child(simple_item(
            "task",
            "What should the agent do?",
            Some("Choose the task for this run."),
            &[
                ("implement", "Implement the requested change"),
                ("debug", "Debug the current behavior"),
                ("review", "Review the implementation"),
            ],
        ))
        .child(simple_item(
            "review",
            "How should the work be reviewed?",
            Some("Select the verification depth."),
            &[
                ("targeted", "Targeted checks"),
                ("complete", "Complete test suite"),
                ("manual", "Tests and manual QA"),
            ],
        ))
        .child(simple_item(
            "delivery",
            "How should the result be delivered?",
            Some("Choose the final handoff format."),
            &[
                ("summary", "Concise summary"),
                ("diff", "Summary and changed files"),
                ("handoff", "Detailed review handoff"),
            ],
        ))
        .child(actions("Save workflow"))
}

fn card_questionnaire() -> Part<QuestionnaireProps> {
    questionnaire(vec![
        item("task", &["fix", "refactor", "docs"]),
        item("output", &["summary", "files", "review"]),
    ])
    .default_item("task")
    .shortcuts(Some(QuestionnaireShortcuts::Numbers))
    .on_submit(move |(_, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        toast(
            "Agent task created",
            format!(
                "Task: {} · Handoff: {}",
                value_of(&a, "task", "None"),
                value_of(&a, "output", "None")
            ),
        );
    })
}

/// An item laid out as a card's header and content (`QuestionnaireTitle
/// as-child` on `CardTitle`, and so on).
fn card_item(name: &str, title: &str, description: &str, entries: &[(&str, &str)]) -> Part<QuestionnaireItemProps> {
    questionnaire_item(name)
        .child(
            card_header()
                // `as-child` merges the title's `text-base` into `CardTitle`'s
                // `leading-none`, and `text-base`'s 24px line wins (Chrome).
                .child(questionnaire_title(title.to_owned()).grid_column_start(1))
                .child(card_description(description.to_owned()))
                .child(card_action().child(questionnaire_progress())),
        )
        .child(card_content().child(choices(entries)).child(questionnaire_error()))
}

fn card_demo(root: Part<QuestionnaireProps>) -> Part<QuestionnaireProps> {
    root.margin_x(Length::Auto).max_width(448.0).child(
        card()
            .child(card_item(
                "task",
                "What should the agent work on?",
                "Choose the task that should be handled next.",
                &[
                    ("fix", "Fix the failing tests"),
                    ("refactor", "Refactor the data layer"),
                    ("docs", "Update the integration guide"),
                ],
            ))
            .child(card_item(
                "output",
                "What should the final handoff include?",
                "Pick the level of detail needed for review.",
                &[
                    ("summary", "Summary only"),
                    ("files", "Summary and changed files"),
                    ("review", "Full review handoff"),
                ],
            ))
            .child(card_footer().child(actions("Create task").width(pct(100.0)))),
    )
}

fn conditional() -> Part<QuestionnaireProps> {
    questionnaire(vec![
        item("runtime", &["local", "cloud"]).checked(&["local"]),
        item("environment", &["preview", "staging", "isolated"]).disabled(),
        item("approval", &["writes", "commands", "sensitive"]),
    ])
    .default_item("runtime")
    .on_submit(move |(_, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        toast(
            "Execution plan saved",
            format!(
                "Runtime: {} · Environment: {} · Approval: {}",
                value_of(&a, "runtime", "None"),
                value_of(&a, "environment", "Not applicable"),
                value_of(&a, "approval", "None")
            ),
        );
    })
    // `@change="runtime = …"` and `:disabled="runtime !== 'cloud'"`.
    .on_change(|(q, name, value): (Questionnaire, String, String)| {
        if name == "runtime" {
            q.set_disabled("environment", value != "cloud");
        }
    })
}

fn conditional_demo(root: Part<QuestionnaireProps>) -> Part<QuestionnaireProps> {
    root.margin_x(Length::Auto)
        .max_width(448.0)
        .child(questionnaire_progress())
        .child(simple_item(
            "runtime",
            "Where should the agent run?",
            Some("Cloud runs add an environment question to this flow."),
            &[("local", "Local workspace"), ("cloud", "Cloud workspace")],
        ))
        .child(simple_item(
            "environment",
            "Which cloud environment should it use?",
            None,
            &[
                ("preview", "Preview"),
                ("staging", "Staging"),
                ("isolated", "Isolated sandbox"),
            ],
        ))
        .child(simple_item(
            "approval",
            "When should the agent request approval?",
            None,
            &[
                ("writes", "Before writing files"),
                ("commands", "Before running commands"),
                ("sensitive", "Only for sensitive actions"),
            ],
        ))
        .child(actions("Save execution plan"))
}

fn controlled() -> Part<QuestionnaireProps> {
    questionnaire(vec![
        item("scope", &["component", "tests", "feature"]),
        item("checks", &["targeted", "package", "full"]),
        item("output", &["summary", "diff", "handoff"]),
    ])
    .default_item("scope")
    .on_submit(move |(_, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        toast(
            "Agent workflow configured",
            format!(
                "Scope: {} · Verification: {} · Output: {}",
                value_of(&a, "scope", "None"),
                value_of(&a, "checks", "None"),
                value_of(&a, "output", "None")
            ),
        );
    })
}

fn controlled_demo(root: Part<QuestionnaireProps>, current: Signal<String>) -> Div {
    let t = theme();
    let label = match current.get().as_str() {
        "scope" => "Change scope",
        "checks" => "Verification",
        "output" => "Final output",
        _ => "",
    };
    div()
        .position(Position::Relative)
        .margin_x(Length::Auto)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .max_width(448.0)
        .child(
            text(format!("Current checkpoint: {label}"))
                .position(Position::Absolute)
                .right(0.0)
                .top(0.0)
                .text_size(14.0)
                .line_height(20.0)
                .text_color(t.muted_foreground),
        )
        .child(
            root.margin_top(Length::Auto)
                .child(questionnaire_progress())
                .child(simple_item(
                    "scope",
                    "What may the agent change?",
                    Some("The host stores the active checkpoint while Questionnaire navigates."),
                    &[
                        ("component", "Only the target component"),
                        ("tests", "Component and related tests"),
                        ("feature", "The complete feature area"),
                    ],
                ))
                .child(simple_item(
                    "checks",
                    "Which verification level should it use?",
                    None,
                    &[
                        ("targeted", "Targeted tests"),
                        ("package", "Package tests and typecheck"),
                        ("full", "Full workspace verification"),
                    ],
                ))
                .child(simple_item(
                    "output",
                    "What should the agent return when finished?",
                    None,
                    &[
                        ("summary", "Concise summary"),
                        ("diff", "Summary with changed files"),
                        ("handoff", "Detailed implementation handoff"),
                    ],
                ))
                .child(actions("Save workflow")),
        )
}

fn dialog_questionnaire(open: Signal<bool>) -> Part<QuestionnaireProps> {
    questionnaire(vec![
        item("scope", &["component", "feature", "workspace"]),
        item("tests", &["targeted", "package", "full"]),
    ])
    .default_item("scope")
    .on_submit(move |(_, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        open.set(false);
        toast(
            "Clarification sent",
            format!(
                "Scope: {} · Verification: {}",
                value_of(&a, "scope", "None"),
                value_of(&a, "tests", "None")
            ),
        );
    })
}

/// An item whose title and description are the dialog's.
fn dialog_item(name: &str, title: &str, description: &str, entries: &[(&str, &str)]) -> Part<QuestionnaireItemProps> {
    questionnaire_item(name)
        .child(
            dialog_header()
                .child(questionnaire_progress())
                .child(dialog_title(title.to_owned()))
                .child(dialog_description(description.to_owned())),
        )
        .child(choices(entries))
        .child(questionnaire_error())
}

fn dialog_demo(root: Part<QuestionnaireProps>, open: Signal<bool>) -> Part<DialogProps> {
    let outline = |label: &str| button(ButtonVariant::Outline, ButtonSize::Default).child(label.to_owned());
    dialog()
        .open(open)
        .child(dialog_trigger(outline("Open clarification")))
        .child(
            dialog_content().child(
                root.child(dialog_item(
                    "scope",
                    "Which files are in scope?",
                    "Choose how broadly the agent can update the workspace.",
                    &[
                        ("component", "Component only"),
                        ("feature", "Complete feature directory"),
                        ("workspace", "Any related workspace file"),
                    ],
                ))
                .child(dialog_item(
                    "tests",
                    "How much verification is needed?",
                    "Choose the checks the agent should run before handoff.",
                    &[
                        ("targeted", "Targeted tests"),
                        ("package", "Package tests"),
                        ("full", "Full workspace verification"),
                    ],
                ))
                .child(
                    dialog_footer()
                        .child(dialog_close(outline("Cancel")))
                        .child(actions("Send answer")),
                ),
            ),
        )
}

fn freeform() -> Part<QuestionnaireProps> {
    questionnaire(vec![
        item("approach", &["incremental", "module", "rewrite"]).with_input(),
    ])
    .shortcuts(Some(QuestionnaireShortcuts::Letters))
    .on_submit(move |(_, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        toast(
            "Approach selected",
            format!("Approach: {}", value_of(&a, "approach", "None")),
        );
    })
}

fn freeform_demo(root: Part<QuestionnaireProps>) -> Part<QuestionnaireProps> {
    root.margin_x(Length::Auto)
        .max_width(448.0)
        .child(
            questionnaire_item("approach")
                .child(questionnaire_title("How should the agent approach this refactor?"))
                .child(questionnaire_description(
                    "Choose a strategy or write a more specific instruction.",
                ))
                .child(
                    choices(&[
                        ("incremental", "Make the smallest safe change"),
                        ("module", "Refactor one module at a time"),
                        ("rewrite", "Replace the implementation completely"),
                    ])
                    .child(questionnaire_input().placeholder("Describe another approach…")),
                )
                .child(questionnaire_error()),
        )
        .child(questionnaire_actions().child(questionnaire_submit("Use this approach")))
}

fn multiple() -> Part<QuestionnaireProps> {
    questionnaire(vec![
        item("context", &["source", "tests", "docs", "history"]).multiple(),
    ])
    .shortcuts(Some(QuestionnaireShortcuts::Letters))
    .on_submit(move |(_, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        toast("Context selected", format!("Context: {}", values_of(&a, "context")));
    })
}

fn multiple_demo(root: Part<QuestionnaireProps>) -> Part<QuestionnaireProps> {
    root.margin_x(Length::Auto)
        .max_width(448.0)
        .child(simple_item(
            "context",
            "What context should the agent inspect?",
            Some("Select every source that may affect the implementation."),
            &[
                ("source", "Relevant source files"),
                ("tests", "Existing tests"),
                ("docs", "Architecture documentation"),
                ("history", "Recent commit history"),
            ],
        ))
        .child(questionnaire_actions().child(questionnaire_submit("Share context")))
}

fn navigation_state() -> Part<QuestionnaireProps> {
    questionnaire(vec![
        item("permission", &["files", "tests", "config"]),
        item("verification", &["tests", "types", "all"]),
    ])
    .default_item("permission")
    .on_submit(move |(_, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        toast(
            "Permissions saved",
            format!(
                "Permission: {} · Verification: {}",
                value_of(&a, "permission", "None"),
                value_of(&a, "verification", "None")
            ),
        );
    })
}

fn navigation_state_demo(root: Part<QuestionnaireProps>) -> Part<QuestionnaireProps> {
    let unanswered = |s: Option<QuestionnaireStatus>| s == Some(QuestionnaireStatus::Unanswered);
    root.margin_x(Length::Auto)
        .max_width(448.0)
        .child(questionnaire_progress())
        .child(simple_item(
            "permission",
            "What may the agent modify?",
            Some("Next is intentionally disabled until an answer is selected."),
            &[
                ("files", "Project files"),
                ("tests", "Project files and tests"),
                ("config", "Files, tests, and configuration"),
            ],
        ))
        .child(simple_item(
            "verification",
            "What must pass before completion?",
            None,
            &[
                ("tests", "Tests"),
                ("types", "Tests and types"),
                ("all", "Tests, types, and visual QA"),
            ],
        ))
        .child(
            questionnaire_actions()
                .child(questionnaire_previous("Previous"))
                .child(
                    questionnaire_next("Next")
                        .variant(ButtonVariant::Secondary)
                        .disabled_when(unanswered),
                )
                .child(questionnaire_submit("Save permissions").disabled_when(unanswered)),
        )
}

fn progress() -> Part<QuestionnaireProps> {
    questionnaire(vec![
        item("scope", &["small", "medium", "large"]),
        item("strategy", &["single", "logical", "squash"]),
        item("tests", &["targeted", "package", "workspace"]),
        item("delivery", &["patch", "commit", "branch"]),
    ])
    .default_item("scope")
    .on_submit(move |(_, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        toast(
            "Pull request plan ready",
            format!(
                "Scope: {} · Commits: {} · Tests: {} · Delivery: {}",
                value_of(&a, "scope", "None"),
                value_of(&a, "strategy", "None"),
                value_of(&a, "tests", "None"),
                value_of(&a, "delivery", "None")
            ),
        );
    })
}

fn progress_demo(root: Part<QuestionnaireProps>) -> Part<QuestionnaireProps> {
    let progress = |p: QuestionnaireProgressState| {
        let t = theme();
        let bar = div()
            .margin_bottom(8.0)
            .display(Display::Flex)
            .gap(6.0)
            .children((1..=p.total).map(|step| {
                div()
                    .height(6.0)
                    .flex(Flex::grow(1.0))
                    .radius(9999.0)
                    .background(if step <= p.current { t.primary } else { t.muted })
            }));
        div()
            .child(bar)
            .child(text(format!("Checkpoint {} of {}", p.current, p.total)))
            .into_element()
    };
    root.margin_x(Length::Auto)
        .max_width(448.0)
        .child(questionnaire_progress_with(progress).width(pct(100.0)))
        .child(simple_item(
            "scope",
            "How large is the change?",
            None,
            &[
                ("small", "Small patch"),
                ("medium", "Feature-sized change"),
                ("large", "Cross-package change"),
            ],
        ))
        .child(simple_item(
            "strategy",
            "How should commits be organized?",
            None,
            &[
                ("single", "Single commit"),
                ("logical", "Logical commits"),
                ("squash", "Squash before review"),
            ],
        ))
        .child(simple_item(
            "tests",
            "Which tests should run?",
            None,
            &[
                ("targeted", "Targeted tests"),
                ("package", "Package suite"),
                ("workspace", "Full workspace"),
            ],
        ))
        .child(simple_item(
            "delivery",
            "How should the work be delivered?",
            None,
            &[
                ("patch", "Patch only"),
                ("commit", "Committed locally"),
                ("branch", "Push a review branch"),
            ],
        ))
        .child(actions("Finish plan"))
}

fn resume() -> Part<QuestionnaireProps> {
    questionnaire(vec![
        item("change", &["incremental", "cutover"]).checked(&["incremental"]),
        item("verification", &["tests", "typecheck", "manual"])
            .multiple()
            .checked(&["tests", "typecheck"]),
        QuestionnaireItem::new("notes", &[]).default_value("Keep the existing public API stable."),
    ])
    .default_item("verification")
    .on_submit(move |(_, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        toast(
            "Draft updated",
            format!(
                "Migration: {} · Verification: {} · Notes: {}",
                value_of(&a, "change", "None"),
                values_of(&a, "verification"),
                value_of(&a, "notes", "None")
            ),
        );
    })
}

fn resume_demo(root: Part<QuestionnaireProps>) -> Part<QuestionnaireProps> {
    root.on_reset(move |_: Questionnaire| {
        shadcn::toast("Saved answers restored");
    })
    .margin_x(Length::Auto)
    .max_width(448.0)
    .child(questionnaire_progress())
    .child(simple_item(
        "change",
        "What kind of migration is this?",
        Some("This answer was saved during the previous session."),
        &[("incremental", "Incremental migration"), ("cutover", "Single cutover")],
    ))
    .child(simple_item(
        "verification",
        "How should the migration be verified?",
        Some("These checks were selected during the previous session."),
        &[
            ("tests", "Run migration tests"),
            ("typecheck", "Run the typecheck"),
            ("manual", "Perform a manual smoke test"),
        ],
    ))
    .child(
        questionnaire_item("notes")
            .child(questionnaire_title("Anything else the agent should remember?"))
            .child(questionnaire_description("This note was saved with the draft."))
            .child(questionnaire_input()),
    )
    .child(
        questionnaire_actions()
            // `type="reset"`: no column of its own, so the grid puts it in the free middle one.
            .child(questionnaire_reset(
                button(ButtonVariant::Outline, ButtonSize::Default).child("Reset changes"),
            ))
            .child(questionnaire_previous("Previous"))
            .child(questionnaire_next("Next"))
            .child(questionnaire_submit("Update draft")),
    )
}

fn shortcuts() -> Part<QuestionnaireProps> {
    questionnaire(vec![item("action", &["inspect", "tests", "patch"])])
        .shortcuts(Some(QuestionnaireShortcuts::Letters))
        .on_submit(move |(_, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
            toast(
                "Next action selected",
                format!("Action: {}", value_of(&a, "action", "None")),
            );
        })
}

fn shortcuts_demo(root: Part<QuestionnaireProps>, mode: &Signal<String>, focus: FocusHandle) -> Div {
    let mode2 = *mode;
    let shortcuts = match mode.get().as_str() {
        "letters" => Some(QuestionnaireShortcuts::Letters),
        "numbers" => Some(QuestionnaireShortcuts::Numbers),
        _ => None,
    };
    let root = root.shortcuts(shortcuts);
    let select = native_select([
        native_select_option("none", "No shortcuts"),
        native_select_option("letters", "Letters"),
        native_select_option("numbers", "Numbers"),
    ])
    .value(mode.get())
    .on_change(move |v| mode2.set(v))
    .focus(focus)
    // The demo's `class` lands on the `<select>`, not its wrapper: the
    // wrapper keeps no width and the box hangs off its left edge, as in Vue.
    .class(|field, chevron| (field.position(Position::Absolute).right(0.0).top(0.0), chevron));
    div()
        .position(Position::Relative)
        .margin_x(Length::Auto)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .max_width(448.0)
        // The wrapper's `w-fit`: with the select out of flow it has no width.
        .child(div().align_self(Align::Start).child(select))
        .child(
            root.margin_top(Length::Auto)
                .child(simple_item(
                    "action",
                    "What should the agent do next?",
                    Some("Use the displayed shortcut or navigate with the keyboard."),
                    &[
                        ("inspect", "Inspect the implementation"),
                        ("tests", "Run the relevant tests"),
                        ("patch", "Prepare the patch"),
                    ],
                ))
                .child(questionnaire_actions().child(questionnaire_submit("Confirm action"))),
        )
}

fn skip() -> Part<QuestionnaireProps> {
    questionnaire(vec![
        item("task", &["feature", "fix", "refactor"]),
        QuestionnaireItem::new("constraints", &["no-dependencies", "no-migrations", "preserve-api"]).with_input(),
        item("review", &["tests", "diff", "both"]),
    ])
    .default_item("task")
    .on_submit(move |(q, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        let constraints = if q.status("constraints") == Some(QuestionnaireStatus::Skipped) {
            "Skipped".to_owned()
        } else {
            value_of(&a, "constraints", "None")
        };
        toast(
            "Agent brief submitted",
            format!(
                "Task: {} · Constraints: {constraints} · Review: {}",
                value_of(&a, "task", "None"),
                value_of(&a, "review", "None")
            ),
        );
    })
}

fn skip_demo(root: Part<QuestionnaireProps>) -> Part<QuestionnaireProps> {
    root.margin_x(Length::Auto)
        .max_width(448.0)
        .child(questionnaire_progress())
        .child(simple_item(
            "task",
            "What kind of change is this?",
            Some("Choose the category that best describes the work."),
            &[("feature", "New feature"), ("fix", "Bug fix"), ("refactor", "Refactor")],
        ))
        .child(
            questionnaire_item("constraints")
                .child(questionnaire_title("Are there any implementation constraints?"))
                .child(questionnaire_description(
                    "Answer if needed, or intentionally skip this question.",
                ))
                .child(
                    choices(&[
                        ("no-dependencies", "Do not add dependencies"),
                        ("no-migrations", "Do not change the database"),
                        ("preserve-api", "Preserve the public API"),
                    ])
                    .child(questionnaire_input().placeholder("Describe another constraint…")),
                ),
        )
        .child(simple_item(
            "review",
            "How should the work be reviewed?",
            Some("Choose the checks the agent should complete before handoff."),
            &[
                ("tests", "Run the test suite"),
                ("diff", "Review the final diff"),
                ("both", "Tests and diff review"),
            ],
        ))
        .child(
            questionnaire_actions()
                .child(questionnaire_previous("Previous"))
                .child(questionnaire_skip("Skip"))
                .child(questionnaire_next("Next"))
                .child(questionnaire_submit("Submit brief")),
        )
}

/// The message zod's `superRefine` adds to `detail`.
const PUBLIC_NEEDS_DETAIL: &str = "Public answers need enough context. Choose a complete answer.";

fn validation(error: &Signal<Option<String>>) -> Part<QuestionnaireProps> {
    let (error, cleared) = (*error, *error);
    questionnaire(vec![
        item("detail", &["summary", "complete"]),
        item("audience", &["team", "public"]),
    ])
    .default_item("detail")
    .on_submit(move |(q, a): (Questionnaire, Vec<QuestionnaireAnswer>)| {
        let (detail, audience) = (value_of(&a, "detail", ""), value_of(&a, "audience", ""));
        if audience == "public" && detail == "summary" {
            error.set(Some(PUBLIC_NEEDS_DETAIL.to_owned()));
            q.set_invalid("detail", true);
            q.set_item("detail");
            return;
        }
        toast(
            "Agent response configured",
            format!("Detail: {detail} · Audience: {audience}"),
        );
    })
    // `@change="clearError('detail')"`: only `detail` ever carries an error.
    .on_change(move |(q, name, _): (Questionnaire, String, String)| {
        if name == "detail" && cleared.get().is_some() {
            cleared.set(None);
            q.set_invalid("detail", false);
        }
    })
}

/// An item as a card's header and content, its title and description the
/// questionnaire's own (not `as-child`), its progress `{current} / {total}`.
fn validation_item(
    name: &str,
    title: &str,
    description: &str,
    entries: &[(&str, &str)],
    error: Option<String>,
) -> Part<QuestionnaireItemProps> {
    questionnaire_item(name)
        .child(
            card_header()
                .child(questionnaire_title(title.to_owned()).grid_column_start(1))
                // Not the item's legend here, so it keeps its place under the title.
                .child(
                    questionnaire_description(description.to_owned())
                        .margin_top(0.0)
                        .grid_column_start(1),
                )
                .child(
                    card_action().child(
                        questionnaire_progress_with(|p| text(format!("{} / {}", p.current, p.total)).into_element())
                            .min_width(0.0),
                    ),
                ),
        )
        .child(
            card_content()
                .child(choices(entries))
                .child(questionnaire_error_with(error.unwrap_or_default())),
        )
}

fn validation_demo(root: Part<QuestionnaireProps>, error: &Signal<Option<String>>) -> Part<QuestionnaireProps> {
    root.margin_x(Length::Auto).max_width(448.0).child(
        card()
            .width(pct(100.0))
            .child(validation_item(
                "detail",
                "How much detail should the answer include?",
                "Choose the response depth.",
                &[("summary", "Concise summary"), ("complete", "Complete answer")],
                error.get(),
            ))
            .child(validation_item(
                "audience",
                "Who will read the answer?",
                "Public answers require complete context.",
                &[("team", "My team"), ("public", "Public audience")],
                None,
            ))
            .child(card_footer().child(actions("Validate answers"))),
    )
}

pub fn render() -> Render {
    let saved = Signal::labeled("saved", String::new());
    let open = Signal::labeled("clarification open", false);
    let mode = Signal::labeled("shortcut mode", "letters".to_owned());
    let mode_focus = FocusHandle::new();
    let validation_error = Signal::labeled("validation error", None);
    let current = Signal::labeled("controlled item", "scope".to_owned());
    Box::new(move || {
        page(
            "Questionnaire",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo("QuestionnaireDemo.vue", questionnaire_demo(the_demo(&saved))))
                .child(text(saved.get()).text_size(14.0).line_height(20.0))
                .child(demo("QuestionnaireAnimatedDemo.vue", animated_demo(animated())))
                .child(demo("QuestionnaireCardDemo.vue", card_demo(card_questionnaire())))
                .child(demo(
                    "QuestionnaireConditionalDemo.vue",
                    conditional_demo(conditional()),
                ))
                .child(demo(
                    "QuestionnaireControlledDemo.vue",
                    controlled_demo(controlled().item(Model::bind(current)), current),
                ))
                .child(demo(
                    "QuestionnaireDialogDemo.vue",
                    dialog_demo(dialog_questionnaire(open), open),
                ))
                .child(demo("QuestionnaireFreeformDemo.vue", freeform_demo(freeform())))
                .child(demo("QuestionnaireMultipleDemo.vue", multiple_demo(multiple())))
                .child(demo(
                    "QuestionnaireNavigationStateDemo.vue",
                    navigation_state_demo(navigation_state()),
                ))
                .child(demo("QuestionnaireProgressDemo.vue", progress_demo(progress())))
                .child(demo("QuestionnaireResumeDemo.vue", resume_demo(resume())))
                .child(demo(
                    "QuestionnaireShortcutsDemo.vue",
                    shortcuts_demo(shortcuts(), &mode, mode_focus),
                ))
                .child(demo("QuestionnaireSkipDemo.vue", skip_demo(skip())))
                .child(demo(
                    "QuestionnaireValidationDemo.vue",
                    validation_demo(validation(&validation_error), &validation_error),
                ))
                .child(toaster()),
        )
    })
}

/// The centre of the `nth` node whose text is `label`, in the last frame.
fn nth_text(s: &Shot, label: &str, nth: usize) -> Result<(f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .filter(|n| matches!(&n.content, gui::layout::Content::Text { content, .. } if &**content == label))
        .nth(nth)
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound(label.to_owned()))
}

fn click_nth(s: &mut Shot, label: &str, nth: usize) -> Result<(), ScenarioError> {
    let (x, y) = nth_text(s, label, nth)?;
    s.click(x, y);
    Ok(())
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    // QuestionnaireDemo: Next without an answer shows the required error.
    click_nth(s, "Next", 0)?;
    s.frame("missing answer")?;
    s.click_text("Approval checkpoints")?;
    s.frame("chose approvals")?;
    // Arrow keys and letter shortcuts work once focus is inside.
    s.press(NamedKey::ArrowRight, gui::Modifiers::default());
    s.frame("arrow to signals")?;
    s.type_text("a");
    s.type_text("c");
    s.frame("shortcuts A and C")?;
    click_nth(s, "Next", 0)?;
    s.frame("timing")?;
    s.click_text("Start now")?;
    s.click_text("Save plan")?;
    s.frame("saved")?;

    // Conditional: choosing the cloud adds the environment question.
    s.click_text("Cloud workspace")?;
    s.frame("conditional cloud")?;
    if nth_text(s, "Question 1 of 3", 0).is_err() {
        return Err(ScenarioError::Unhandled(
            "choosing the cloud did not add the environment question".to_owned(),
        ));
    }

    // Navigation state: Next is disabled until an answer is chosen.
    s.click_text("Project files and tests")?;
    s.frame("navigation answered")?;

    // Resume: opens on the saved checks; reset brings them back.
    s.click_text("Run the typecheck")?;
    s.frame("resume unchecked typecheck")?;
    s.click_text("Reset changes")?;
    s.frame("resume reset")?;

    // Skip: skipping the optional constraints.
    s.click_text("Bug fix")?;
    s.frame("skip task answered")?;

    // Shortcuts: numbers instead of letters (the select takes ArrowDown once focused).
    let select = s
        .last()
        .nodes
        .iter()
        .find(|n| n.name == Some("native-select"))
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound("native-select".to_owned()))?;
    s.click(select.0, select.1);
    s.press(NamedKey::ArrowDown, gui::Modifiers::default());
    s.frame("shortcuts numbers")?;

    // Validation: a public summary is refused with the schema's message.
    s.click_text("Concise summary")?;
    let nexts = s
        .last()
        .nodes
        .iter()
        .filter(|n| matches!(&n.content, gui::layout::Content::Text { content, .. } if &**content == "Next"))
        .count();
    click_nth(s, "Next", nexts.saturating_sub(1))?;
    s.frame("validation audience")?;
    s.click_text("Public audience")?;
    s.click_text("Validate answers")?;
    s.frame("validation refused")?;

    // Dialog: open, answer both, send.
    s.click_text("Open clarification")?;
    s.frame("dialog open")?;
    s.click_text("Component only")?;
    s.frame("dialog chose component")?;
    let dialog_next = s
        .last()
        .nodes
        .iter()
        .filter(|n| {
            n.layer > 0 && matches!(&n.content, gui::layout::Content::Text { content, .. } if &**content == "Next")
        })
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .next()
        .ok_or_else(|| ScenarioError::TextNotFound("dialog Next".to_owned()))?;
    s.click(dialog_next.0, dialog_next.1);
    s.frame("dialog second")?;
    s.click_text("Package tests")?;
    s.click_text("Send answer")?;
    s.frame("dialog sent")?;
    Ok(())
}
