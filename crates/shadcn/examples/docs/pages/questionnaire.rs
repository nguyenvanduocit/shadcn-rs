use gui::prelude::*;
use gui::{Div, FocusHandle, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{
    ButtonSize, ButtonVariant, DialogState, NativeSelectLook, Questionnaire, QuestionnaireAnswer, QuestionnaireItem,
    QuestionnaireShortcuts, QuestionnaireStatus, Toast, ToastKind, Toaster, button, card, card_action, card_content,
    card_description, card_footer, card_header, dialog_close, dialog_content, dialog_description, dialog_footer,
    dialog_header, dialog_title, dialog_trigger, native_select_option, native_select_with, questionnaire,
    questionnaire_actions, questionnaire_choice, questionnaire_choices, questionnaire_description, questionnaire_error,
    questionnaire_error_with, questionnaire_input, questionnaire_item, questionnaire_next, questionnaire_next_with,
    questionnaire_previous, questionnaire_progress, questionnaire_progress_with, questionnaire_skip,
    questionnaire_submit, questionnaire_submit_with, questionnaire_title, theme, toaster,
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
fn toast(toasts: &Toaster, title: &str, description: String) {
    toasts.show(Toast::new(ToastKind::Default, title).description(description));
}

/// A question's choices, each `(value, label)`.
fn choices(q: &Questionnaire, name: &str, entries: &[(&str, &str)]) -> Div {
    questionnaire_choices().children(
        entries
            .iter()
            .map(|(value, label)| questionnaire_choice(q, name, value, text((*label).to_owned()), None)),
    )
}

/// The common item: title, an optional description, choices and the error.
fn simple_item(
    q: &Questionnaire,
    name: &str,
    title: &str,
    description: Option<&str>,
    entries: &[(&str, &str)],
) -> Option<Div> {
    questionnaire_item(q, name).map(|item| {
        item.child(questionnaire_title(title.to_owned()))
            .child(description.map(|d| questionnaire_description(d.to_owned())))
            .child(choices(q, name, entries))
            .child(questionnaire_error(q, name))
    })
}

/// Previous, next and submit.
fn actions(q: &Questionnaire, submit: &str) -> Div {
    questionnaire_actions()
        .child(questionnaire_previous(q, "Previous"))
        .child(questionnaire_next(q, "Next"))
        .child(questionnaire_submit(q, submit))
}

fn item(name: &str, choices: &[&str]) -> QuestionnaireItem {
    QuestionnaireItem::new(name, choices).required()
}

fn the_demo(saved: &Signal<String>) -> Questionnaire {
    let s = saved.clone();
    Questionnaire::new(
        vec![
            QuestionnaireItem::new("direction", &["tool-calls", "approvals", "handoffs"])
                .required()
                .with_input(),
            QuestionnaireItem::new("signals", &["progress", "decisions", "risks", "next-step"]).multiple(),
            QuestionnaireItem::new("timing", &["now", "next-cycle", "backlog"]).required(),
        ],
        Some("direction"),
        Some(QuestionnaireShortcuts::Letters),
        move |_, answers| {
            let summary: Vec<String> = answers
                .iter()
                .map(|a| format!("{}: {}", a.name, values_of(&answers, &a.name)))
                .collect();
            s.set(format!("Agent plan saved · {}", summary.join(" · ")));
        },
    )
}

fn questionnaire_demo(q: &Questionnaire) -> Div {
    let choice = |name: &str, value: &str, label: &str, description: Option<&str>| {
        questionnaire_choice(
            q,
            name,
            value,
            text(label.to_owned()).when(description.is_some(), |t| t.font_medium()),
            description,
        )
    };
    questionnaire(q)
        .mx_auto()
        .max_w(112.0)
        .child(questionnaire_progress(q))
        .child(questionnaire_item(q, "direction").map(|item| {
            item.child(questionnaire_title("What should the agent build next?"))
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
                        .child(questionnaire_input(q, "direction").map(|i| i.placeholder("Describe another feature…"))),
                )
                .child(questionnaire_error(q, "direction"))
        }))
        .child(questionnaire_item(q, "signals").map(|item| {
            item.child(questionnaire_title("What should every progress update include?"))
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
                .child(questionnaire_error(q, "signals"))
        }))
        .child(questionnaire_item(q, "timing").map(|item| {
            item.child(questionnaire_title("When should work begin?"))
                .child(questionnaire_description(
                    "Choose when the agent should begin the work.",
                ))
                .child(
                    questionnaire_choices()
                        .child(choice("timing", "now", "Start now", None))
                        .child(choice("timing", "next-cycle", "Next development cycle", None))
                        .child(choice("timing", "backlog", "Add it to the backlog", None)),
                )
                .child(questionnaire_error(q, "timing"))
        }))
        .child(
            questionnaire_actions()
                .child(questionnaire_previous(q, "Previous"))
                .child(questionnaire_skip(q, "Skip"))
                .child(questionnaire_next(q, "Next"))
                .child(questionnaire_submit(q, "Save plan")),
        )
}

fn animated(toasts: &Toaster) -> Questionnaire {
    let toasts = toasts.clone();
    Questionnaire::new(
        vec![
            item("task", &["implement", "debug", "review"]),
            item("review", &["targeted", "complete", "manual"]),
            item("delivery", &["summary", "diff", "handoff"]),
        ],
        Some("task"),
        None,
        move |_, a| {
            toast(
                &toasts,
                "Agent workflow saved",
                format!(
                    "Task: {} · Review: {} · Delivery: {}",
                    value_of(&a, "task", "None"),
                    value_of(&a, "review", "None"),
                    value_of(&a, "delivery", "None")
                ),
            );
        },
    )
}

/// Vue fades and slides each item in as it opens; `gui` draws no entrance
/// animations, so an item shows at once, as Vue does under `motion-reduce`.
fn animated_demo(q: &Questionnaire) -> Div {
    questionnaire(q)
        .mx_auto()
        .max_w(112.0)
        .child(questionnaire_progress(q))
        .child(simple_item(
            q,
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
            q,
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
            q,
            "delivery",
            "How should the result be delivered?",
            Some("Choose the final handoff format."),
            &[
                ("summary", "Concise summary"),
                ("diff", "Summary and changed files"),
                ("handoff", "Detailed review handoff"),
            ],
        ))
        .child(actions(q, "Save workflow"))
}

fn card_questionnaire(toasts: &Toaster) -> Questionnaire {
    let toasts = toasts.clone();
    Questionnaire::new(
        vec![
            item("task", &["fix", "refactor", "docs"]),
            item("output", &["summary", "files", "review"]),
        ],
        Some("task"),
        Some(QuestionnaireShortcuts::Numbers),
        move |_, a| {
            toast(
                &toasts,
                "Agent task created",
                format!(
                    "Task: {} · Handoff: {}",
                    value_of(&a, "task", "None"),
                    value_of(&a, "output", "None")
                ),
            );
        },
    )
}

/// An item laid out as a card's header and content (`QuestionnaireTitle
/// as-child` on `CardTitle`, and so on).
fn card_item(q: &Questionnaire, name: &str, title: &str, description: &str, entries: &[(&str, &str)]) -> Option<Div> {
    questionnaire_item(q, name).map(|item| {
        item.child(
            card_header()
                // `as-child` merges the title's `text-base` into `CardTitle`'s
                // `leading-none`, and `text-base`'s 24px line wins (Chrome).
                .child(questionnaire_title(title.to_owned()).col_start(1))
                .child(card_description(description.to_owned()))
                .child(card_action().child(questionnaire_progress(q))),
        )
        .child(
            card_content()
                .child(choices(q, name, entries))
                .child(questionnaire_error(q, name)),
        )
    })
}

fn card_demo(q: &Questionnaire) -> Div {
    questionnaire(q).mx_auto().max_w(112.0).child(
        card()
            .child(card_item(
                q,
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
                q,
                "output",
                "What should the final handoff include?",
                "Pick the level of detail needed for review.",
                &[
                    ("summary", "Summary only"),
                    ("files", "Summary and changed files"),
                    ("review", "Full review handoff"),
                ],
            ))
            .child(card_footer().child(actions(q, "Create task").w_full())),
    )
}

fn conditional(toasts: &Toaster) -> Questionnaire {
    let toasts = toasts.clone();
    Questionnaire::new(
        vec![
            item("runtime", &["local", "cloud"]).checked(&["local"]),
            item("environment", &["preview", "staging", "isolated"]).disabled(),
            item("approval", &["writes", "commands", "sensitive"]),
        ],
        Some("runtime"),
        None,
        move |_, a| {
            toast(
                &toasts,
                "Execution plan saved",
                format!(
                    "Runtime: {} · Environment: {} · Approval: {}",
                    value_of(&a, "runtime", "None"),
                    value_of(&a, "environment", "Not applicable"),
                    value_of(&a, "approval", "None")
                ),
            );
        },
    )
    // `@change="runtime = …"` and `:disabled="runtime !== 'cloud'"`.
    .on_change(|q, name, value| {
        if name == "runtime" {
            q.set_disabled("environment", value != "cloud");
        }
    })
}

fn conditional_demo(q: &Questionnaire) -> Div {
    questionnaire(q)
        .mx_auto()
        .max_w(112.0)
        .child(questionnaire_progress(q))
        .child(simple_item(
            q,
            "runtime",
            "Where should the agent run?",
            Some("Cloud runs add an environment question to this flow."),
            &[("local", "Local workspace"), ("cloud", "Cloud workspace")],
        ))
        .child(simple_item(
            q,
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
            q,
            "approval",
            "When should the agent request approval?",
            None,
            &[
                ("writes", "Before writing files"),
                ("commands", "Before running commands"),
                ("sensitive", "Only for sensitive actions"),
            ],
        ))
        .child(actions(q, "Save execution plan"))
}

fn controlled(toasts: &Toaster) -> Questionnaire {
    let toasts = toasts.clone();
    Questionnaire::new(
        vec![
            item("scope", &["component", "tests", "feature"]),
            item("checks", &["targeted", "package", "full"]),
            item("output", &["summary", "diff", "handoff"]),
        ],
        Some("scope"),
        None,
        move |_, a| {
            toast(
                &toasts,
                "Agent workflow configured",
                format!(
                    "Scope: {} · Verification: {} · Output: {}",
                    value_of(&a, "scope", "None"),
                    value_of(&a, "checks", "None"),
                    value_of(&a, "output", "None")
                ),
            );
        },
    )
}

fn controlled_demo(q: &Questionnaire) -> Div {
    let t = theme();
    let label = match q.item().as_deref() {
        Some("scope") => "Change scope",
        Some("checks") => "Verification",
        Some("output") => "Final output",
        _ => "",
    };
    div()
        .relative()
        .mx_auto()
        .flex_col()
        .w_full()
        .max_w(112.0)
        .child(
            text(format!("Current checkpoint: {label}"))
                .absolute()
                .right(0.0)
                .top(0.0)
                .text_sm()
                .text_color(t.muted_foreground),
        )
        .child(
            questionnaire(q)
                .mt_auto()
                .child(questionnaire_progress(q))
                .child(simple_item(
                    q,
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
                    q,
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
                    q,
                    "output",
                    "What should the agent return when finished?",
                    None,
                    &[
                        ("summary", "Concise summary"),
                        ("diff", "Summary with changed files"),
                        ("handoff", "Detailed implementation handoff"),
                    ],
                ))
                .child(actions(q, "Save workflow")),
        )
}

fn dialog_questionnaire(toasts: &Toaster, open: &DialogState) -> Questionnaire {
    let (toasts, open) = (toasts.clone(), open.clone());
    Questionnaire::new(
        vec![
            item("scope", &["component", "feature", "workspace"]),
            item("tests", &["targeted", "package", "full"]),
        ],
        Some("scope"),
        None,
        move |_, a| {
            open.close();
            toast(
                &toasts,
                "Clarification sent",
                format!(
                    "Scope: {} · Verification: {}",
                    value_of(&a, "scope", "None"),
                    value_of(&a, "tests", "None")
                ),
            );
        },
    )
}

/// An item whose title and description are the dialog's.
fn dialog_item(q: &Questionnaire, name: &str, title: &str, description: &str, entries: &[(&str, &str)]) -> Option<Div> {
    questionnaire_item(q, name).map(|item| {
        item.child(
            dialog_header()
                .child(questionnaire_progress(q))
                .child(dialog_title(title.to_owned()))
                .child(dialog_description(description.to_owned())),
        )
        .child(choices(q, name, entries))
        .child(questionnaire_error(q, name))
    })
}

fn dialog_demo(q: &Questionnaire, open: &DialogState) -> Div {
    let outline = |label: &str| button(ButtonVariant::Outline, ButtonSize::Default).child(label.to_owned());
    div()
        .child(dialog_trigger(open, outline("Open clarification")))
        .child(dialog_content(open, |c| {
            c.child(
                questionnaire(q)
                    .child(dialog_item(
                        q,
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
                        q,
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
                            .child(dialog_close(open, outline("Cancel")))
                            .child(actions(q, "Send answer")),
                    ),
            )
        }))
}

fn freeform(toasts: &Toaster) -> Questionnaire {
    let toasts = toasts.clone();
    Questionnaire::new(
        vec![item("approach", &["incremental", "module", "rewrite"]).with_input()],
        None,
        Some(QuestionnaireShortcuts::Letters),
        move |_, a| {
            toast(
                &toasts,
                "Approach selected",
                format!("Approach: {}", value_of(&a, "approach", "None")),
            );
        },
    )
}

fn freeform_demo(q: &Questionnaire) -> Div {
    questionnaire(q)
        .mx_auto()
        .max_w(112.0)
        .child(questionnaire_item(q, "approach").map(|item| {
            item.child(questionnaire_title("How should the agent approach this refactor?"))
                .child(questionnaire_description(
                    "Choose a strategy or write a more specific instruction.",
                ))
                .child(
                    choices(
                        q,
                        "approach",
                        &[
                            ("incremental", "Make the smallest safe change"),
                            ("module", "Refactor one module at a time"),
                            ("rewrite", "Replace the implementation completely"),
                        ],
                    )
                    .child(questionnaire_input(q, "approach").map(|i| i.placeholder("Describe another approach…"))),
                )
                .child(questionnaire_error(q, "approach"))
        }))
        .child(questionnaire_actions().child(questionnaire_submit(q, "Use this approach")))
}

fn multiple(toasts: &Toaster) -> Questionnaire {
    let toasts = toasts.clone();
    Questionnaire::new(
        vec![item("context", &["source", "tests", "docs", "history"]).multiple()],
        None,
        Some(QuestionnaireShortcuts::Letters),
        move |_, a| {
            toast(
                &toasts,
                "Context selected",
                format!("Context: {}", values_of(&a, "context")),
            );
        },
    )
}

fn multiple_demo(q: &Questionnaire) -> Div {
    questionnaire(q)
        .mx_auto()
        .max_w(112.0)
        .child(simple_item(
            q,
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
        .child(questionnaire_actions().child(questionnaire_submit(q, "Share context")))
}

fn navigation_state(toasts: &Toaster) -> Questionnaire {
    let toasts = toasts.clone();
    Questionnaire::new(
        vec![
            item("permission", &["files", "tests", "config"]),
            item("verification", &["tests", "types", "all"]),
        ],
        Some("permission"),
        None,
        move |_, a| {
            toast(
                &toasts,
                "Permissions saved",
                format!(
                    "Permission: {} · Verification: {}",
                    value_of(&a, "permission", "None"),
                    value_of(&a, "verification", "None")
                ),
            );
        },
    )
}

fn navigation_state_demo(q: &Questionnaire) -> Div {
    let unanswered = q.active_status() == Some(QuestionnaireStatus::Unanswered);
    questionnaire(q)
        .mx_auto()
        .max_w(112.0)
        .child(questionnaire_progress(q))
        .child(simple_item(
            q,
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
            q,
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
                .child(questionnaire_previous(q, "Previous"))
                .child(questionnaire_next_with(q, "Next", ButtonVariant::Secondary, unanswered))
                .child(questionnaire_submit_with(
                    q,
                    "Save permissions",
                    ButtonVariant::Default,
                    unanswered,
                )),
        )
}

fn progress(toasts: &Toaster) -> Questionnaire {
    let toasts = toasts.clone();
    Questionnaire::new(
        vec![
            item("scope", &["small", "medium", "large"]),
            item("strategy", &["single", "logical", "squash"]),
            item("tests", &["targeted", "package", "workspace"]),
            item("delivery", &["patch", "commit", "branch"]),
        ],
        Some("scope"),
        None,
        move |_, a| {
            toast(
                &toasts,
                "Pull request plan ready",
                format!(
                    "Scope: {} · Commits: {} · Tests: {} · Delivery: {}",
                    value_of(&a, "scope", "None"),
                    value_of(&a, "strategy", "None"),
                    value_of(&a, "tests", "None"),
                    value_of(&a, "delivery", "None")
                ),
            );
        },
    )
}

fn progress_demo(q: &Questionnaire) -> Div {
    let t = theme();
    let (current, total) = (q.current(), q.total());
    let bar = div().mb(2.0).flex().gap(1.5).children((1..=total).map(|step| {
        div()
            .h(1.5)
            .flex_1()
            .rounded_full()
            .bg(if step <= current { t.primary } else { t.muted })
    }));
    questionnaire(q)
        .mx_auto()
        .max_w(112.0)
        .child(
            questionnaire_progress_with(div().child(bar).child(text(format!("Checkpoint {current} of {total}"))))
                .w_full(),
        )
        .child(simple_item(
            q,
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
            q,
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
            q,
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
            q,
            "delivery",
            "How should the work be delivered?",
            None,
            &[
                ("patch", "Patch only"),
                ("commit", "Committed locally"),
                ("branch", "Push a review branch"),
            ],
        ))
        .child(actions(q, "Finish plan"))
}

fn resume(toasts: &Toaster) -> Questionnaire {
    let toasts = toasts.clone();
    Questionnaire::new(
        vec![
            item("change", &["incremental", "cutover"]).checked(&["incremental"]),
            item("verification", &["tests", "typecheck", "manual"])
                .multiple()
                .checked(&["tests", "typecheck"]),
            QuestionnaireItem::new("notes", &[]).default_value("Keep the existing public API stable."),
        ],
        Some("verification"),
        None,
        move |_, a| {
            toast(
                &toasts,
                "Draft updated",
                format!(
                    "Migration: {} · Verification: {} · Notes: {}",
                    value_of(&a, "change", "None"),
                    values_of(&a, "verification"),
                    value_of(&a, "notes", "None")
                ),
            );
        },
    )
}

fn resume_demo(q: &Questionnaire, toasts: &Toaster) -> Div {
    let (reset, toasts) = (q.clone(), toasts.clone());
    questionnaire(q)
        .mx_auto()
        .max_w(112.0)
        .child(questionnaire_progress(q))
        .child(simple_item(
            q,
            "change",
            "What kind of migration is this?",
            Some("This answer was saved during the previous session."),
            &[("incremental", "Incremental migration"), ("cutover", "Single cutover")],
        ))
        .child(simple_item(
            q,
            "verification",
            "How should the migration be verified?",
            Some("These checks were selected during the previous session."),
            &[
                ("tests", "Run migration tests"),
                ("typecheck", "Run the typecheck"),
                ("manual", "Perform a manual smoke test"),
            ],
        ))
        .child(questionnaire_item(q, "notes").map(|item| {
            item.child(questionnaire_title("Anything else the agent should remember?"))
                .child(questionnaire_description("This note was saved with the draft."))
                .child(questionnaire_input(q, "notes"))
        }))
        .child(
            questionnaire_actions()
                // `type="reset"`: no column of its own, so the grid puts it in the free middle one.
                .child(
                    button(ButtonVariant::Outline, ButtonSize::Default)
                        .child("Reset changes")
                        .on_click(move |_| {
                            reset.reset();
                            toasts.toast("Saved answers restored");
                        }),
                )
                .child(questionnaire_previous(q, "Previous"))
                .child(questionnaire_next(q, "Next"))
                .child(questionnaire_submit(q, "Update draft")),
        )
}

fn shortcuts(toasts: &Toaster) -> Questionnaire {
    let toasts = toasts.clone();
    Questionnaire::new(
        vec![item("action", &["inspect", "tests", "patch"])],
        None,
        Some(QuestionnaireShortcuts::Letters),
        move |_, a| {
            toast(
                &toasts,
                "Next action selected",
                format!("Action: {}", value_of(&a, "action", "None")),
            );
        },
    )
}

fn shortcuts_demo(q: &Questionnaire, mode: &Signal<String>, focus: FocusHandle) -> Div {
    let (q2, mode2) = (q.clone(), mode.clone());
    let select = native_select_with(
        focus,
        &mode.get(),
        move |v| {
            q2.set_shortcuts(match v.as_str() {
                "letters" => Some(QuestionnaireShortcuts::Letters),
                "numbers" => Some(QuestionnaireShortcuts::Numbers),
                _ => None,
            });
            mode2.set(v);
        },
        &[
            native_select_option("none", "No shortcuts"),
            native_select_option("letters", "Letters"),
            native_select_option("numbers", "Numbers"),
        ],
        NativeSelectLook::default(),
        // The demo's `class` lands on the `<select>`, not its wrapper: the
        // wrapper keeps no width and the box hangs off its left edge, as in Vue.
        |field, chevron| (field.absolute().right(0.0).top(0.0), chevron),
    );
    div()
        .relative()
        .mx_auto()
        .flex_col()
        .w_full()
        .max_w(112.0)
        // The wrapper's `w-fit`: with the select out of flow it has no width.
        .child(select.self_start())
        .child(
            questionnaire(q)
                .mt_auto()
                .child(simple_item(
                    q,
                    "action",
                    "What should the agent do next?",
                    Some("Use the displayed shortcut or navigate with the keyboard."),
                    &[
                        ("inspect", "Inspect the implementation"),
                        ("tests", "Run the relevant tests"),
                        ("patch", "Prepare the patch"),
                    ],
                ))
                .child(questionnaire_actions().child(questionnaire_submit(q, "Confirm action"))),
        )
}

fn skip(toasts: &Toaster) -> Questionnaire {
    let toasts = toasts.clone();
    Questionnaire::new(
        vec![
            item("task", &["feature", "fix", "refactor"]),
            QuestionnaireItem::new("constraints", &["no-dependencies", "no-migrations", "preserve-api"]).with_input(),
            item("review", &["tests", "diff", "both"]),
        ],
        Some("task"),
        None,
        move |q, a| {
            let constraints = if q.status("constraints") == Some(QuestionnaireStatus::Skipped) {
                "Skipped".to_owned()
            } else {
                value_of(&a, "constraints", "None")
            };
            toast(
                &toasts,
                "Agent brief submitted",
                format!(
                    "Task: {} · Constraints: {constraints} · Review: {}",
                    value_of(&a, "task", "None"),
                    value_of(&a, "review", "None")
                ),
            );
        },
    )
}

fn skip_demo(q: &Questionnaire) -> Div {
    questionnaire(q)
        .mx_auto()
        .max_w(112.0)
        .child(questionnaire_progress(q))
        .child(simple_item(
            q,
            "task",
            "What kind of change is this?",
            Some("Choose the category that best describes the work."),
            &[("feature", "New feature"), ("fix", "Bug fix"), ("refactor", "Refactor")],
        ))
        .child(questionnaire_item(q, "constraints").map(|item| {
            item.child(questionnaire_title("Are there any implementation constraints?"))
                .child(questionnaire_description(
                    "Answer if needed, or intentionally skip this question.",
                ))
                .child(
                    choices(
                        q,
                        "constraints",
                        &[
                            ("no-dependencies", "Do not add dependencies"),
                            ("no-migrations", "Do not change the database"),
                            ("preserve-api", "Preserve the public API"),
                        ],
                    )
                    .child(
                        questionnaire_input(q, "constraints").map(|i| i.placeholder("Describe another constraint…")),
                    ),
                )
        }))
        .child(simple_item(
            q,
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
                .child(questionnaire_previous(q, "Previous"))
                .child(questionnaire_skip(q, "Skip"))
                .child(questionnaire_next(q, "Next"))
                .child(questionnaire_submit(q, "Submit brief")),
        )
}

/// The message zod's `superRefine` adds to `detail`.
const PUBLIC_NEEDS_DETAIL: &str = "Public answers need enough context. Choose a complete answer.";

fn validation(toasts: &Toaster, error: &Signal<Option<String>>) -> Questionnaire {
    let (toasts, error, cleared) = (toasts.clone(), error.clone(), error.clone());
    Questionnaire::new(
        vec![
            item("detail", &["summary", "complete"]),
            item("audience", &["team", "public"]),
        ],
        Some("detail"),
        None,
        move |q, a| {
            let (detail, audience) = (value_of(&a, "detail", ""), value_of(&a, "audience", ""));
            if audience == "public" && detail == "summary" {
                error.set(Some(PUBLIC_NEEDS_DETAIL.to_owned()));
                q.set_invalid("detail", true);
                q.set_item("detail");
                return;
            }
            toast(
                &toasts,
                "Agent response configured",
                format!("Detail: {detail} · Audience: {audience}"),
            );
        },
    )
    // `@change="clearError('detail')"`: only `detail` ever carries an error.
    .on_change(move |q, name, _| {
        if name == "detail" && cleared.get().is_some() {
            cleared.set(None);
            q.set_invalid("detail", false);
        }
    })
}

/// An item as a card's header and content, its title and description the
/// questionnaire's own (not `as-child`), its progress `{current} / {total}`.
fn validation_item(
    q: &Questionnaire,
    name: &str,
    title: &str,
    description: &str,
    entries: &[(&str, &str)],
    error: Option<String>,
) -> Option<Div> {
    questionnaire_item(q, name).map(|item| {
        item.child(
            card_header()
                .child(questionnaire_title(title.to_owned()).col_start(1))
                // Not the item's legend here, so it keeps its place under the title.
                .child(questionnaire_description(description.to_owned()).mt(0.0).col_start(1))
                .child(
                    card_action()
                        .child(questionnaire_progress_with(format!("{} / {}", q.current(), q.total())).min_w(0.0)),
                ),
        )
        .child(
            card_content()
                .child(choices(q, name, entries))
                .child(questionnaire_error_with(q, name, error.unwrap_or_default())),
        )
    })
}

fn validation_demo(q: &Questionnaire, error: &Signal<Option<String>>) -> Div {
    questionnaire(q).mx_auto().max_w(112.0).child(
        card()
            .w_full()
            .child(validation_item(
                q,
                "detail",
                "How much detail should the answer include?",
                "Choose the response depth.",
                &[("summary", "Concise summary"), ("complete", "Complete answer")],
                error.get(),
            ))
            .child(validation_item(
                q,
                "audience",
                "Who will read the answer?",
                "Public answers require complete context.",
                &[("team", "My team"), ("public", "Public audience")],
                None,
            ))
            .child(card_footer().child(actions(q, "Validate answers"))),
    )
}

pub fn render() -> Render {
    let saved = Signal::labeled("saved", String::new());
    let toasts = Toaster::new();
    let open = DialogState::new();
    let mode = Signal::labeled("shortcut mode", "letters".to_owned());
    let mode_focus = FocusHandle::new();
    let validation_error = Signal::labeled("validation error", None);
    let q = the_demo(&saved);
    let animated = animated(&toasts);
    let card = card_questionnaire(&toasts);
    let conditional = conditional(&toasts);
    let controlled = controlled(&toasts);
    let dialog = dialog_questionnaire(&toasts, &open);
    let freeform = freeform(&toasts);
    let multiple = multiple(&toasts);
    let navigation = navigation_state(&toasts);
    let progress = progress(&toasts);
    let resume = resume(&toasts);
    let shortcuts = shortcuts(&toasts);
    let skip = skip(&toasts);
    let validation = validation(&toasts, &validation_error);
    Box::new(move || {
        page(
            "Questionnaire",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo("QuestionnaireDemo.vue", questionnaire_demo(&q)))
                .child(text(saved.get()).text_sm())
                .child(demo("QuestionnaireAnimatedDemo.vue", animated_demo(&animated)))
                .child(demo("QuestionnaireCardDemo.vue", card_demo(&card)))
                .child(demo("QuestionnaireConditionalDemo.vue", conditional_demo(&conditional)))
                .child(demo("QuestionnaireControlledDemo.vue", controlled_demo(&controlled)))
                .child(demo("QuestionnaireDialogDemo.vue", dialog_demo(&dialog, &open)))
                .child(demo("QuestionnaireFreeformDemo.vue", freeform_demo(&freeform)))
                .child(demo("QuestionnaireMultipleDemo.vue", multiple_demo(&multiple)))
                .child(demo(
                    "QuestionnaireNavigationStateDemo.vue",
                    navigation_state_demo(&navigation),
                ))
                .child(demo("QuestionnaireProgressDemo.vue", progress_demo(&progress)))
                .child(demo("QuestionnaireResumeDemo.vue", resume_demo(&resume, &toasts)))
                .child(demo(
                    "QuestionnaireShortcutsDemo.vue",
                    shortcuts_demo(&shortcuts, &mode, mode_focus),
                ))
                .child(demo("QuestionnaireSkipDemo.vue", skip_demo(&skip)))
                .child(demo(
                    "QuestionnaireValidationDemo.vue",
                    validation_demo(&validation, &validation_error),
                ))
                .child(toaster(&toasts)),
        )
    })
}

/// The centre of the `nth` node whose text is `label`, in the last frame.
fn nth_text(s: &Shot, label: &str, nth: usize) -> Result<(f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .filter(|n| matches!(&n.content, gui::layout::Content::Text { content, .. } if content == label))
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
        .filter(|n| matches!(&n.content, gui::layout::Content::Text { content, .. } if content == "Next"))
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
            n.layer > 0 && matches!(&n.content, gui::layout::Content::Text { content, .. } if content == "Next")
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
