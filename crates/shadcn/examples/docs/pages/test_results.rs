//! A test run as an agent's step shows it: how many passed, failed and were
//! skipped, a step per suite, and each failing test's error.

use gui::prelude::*;
use gui::{Div, Element, Icon, Model, ScenarioError, Signal, div, icon};
use shadcn::{
    TraceStatus, trace, trace_collapsible, trace_collapsible_content, trace_collapsible_trigger, trace_icon,
    trace_line, trace_title,
};

use super::thread::{Script, assistant_turn, chunks, frames, output, replay, said, thread, user_turn};
use crate::{Render, Shot, demo, page};

const QUESTION: &str = "Run the checkout tests.";
const ANSWER: &str =
    "One test failed. The payment client does not retry a declined card, so the second charge attempt never happens.";
const COMMAND: &str = "pnpm test checkout";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Passed,
    /// Failed, with the error it printed.
    Failed(&'static str),
    Skipped,
}

struct Test {
    name: &'static str,
    status: Status,
}

struct Suite {
    name: &'static str,
    tests: &'static [Test],
}

const fn passed(name: &'static str) -> Test {
    Test {
        name,
        status: Status::Passed,
    }
}

const CHECKOUT: &[Suite] = &[
    Suite {
        name: "cart.test.ts",
        tests: &[
            passed("adds an item"),
            passed("removes an item"),
            passed("applies a discount code"),
        ],
    },
    Suite {
        name: "payment.test.ts",
        tests: &[
            passed("charges the saved card"),
            Test {
                name: "retries a declined card",
                status: Status::Failed(
                    "Expected 2 charge attempts, received 1\n\n  at Object.<anonymous> (src/payment.test.ts:58:22)",
                ),
            },
        ],
    },
];

const SUITES: &[Suite] = &[
    Suite {
        name: "auth.test.ts",
        tests: &[
            passed("logs in with valid credentials"),
            passed("rejects an invalid password"),
            passed("refreshes an expired token"),
        ],
    },
    Suite {
        name: "user-api.test.ts",
        tests: &[
            passed("creates a user"),
            Test {
                name: "updates a user profile",
                status: Status::Failed(
                    "Expected status 200, received 500\n\n  at Object.<anonymous> (src/user-api.test.ts:45:12)",
                ),
            },
            Test {
                name: "deletes a user",
                status: Status::Skipped,
            },
        ],
    },
    Suite {
        name: "db.test.ts",
        tests: &[
            passed("connects to the database"),
            Test {
                name: "times out a slow query",
                status: Status::Failed(
                    "Exceeded timeout of 5000 ms for a test.\n\n  at Object.<anonymous> (src/db.test.ts:22:3)",
                ),
            },
        ],
    },
];

/// How many tests ran, failed and were skipped.
#[derive(Clone, Copy, Default)]
struct Counts {
    total: usize,
    failed: usize,
    skipped: usize,
}

impl Counts {
    fn of<'a>(tests: impl IntoIterator<Item = &'a Test>) -> Self {
        tests.into_iter().fold(Self::default(), |c, t| Self {
            total: c.total.saturating_add(1),
            failed: c
                .failed
                .saturating_add(usize::from(matches!(t.status, Status::Failed(_)))),
            skipped: c.skipped.saturating_add(usize::from(t.status == Status::Skipped)),
        })
    }

    /// `3 tests, 1 failed, 1 skipped`, leaving out what is zero.
    fn label(self) -> String {
        let mut parts = vec![format!("{} tests", self.total)];
        if self.failed > 0 {
            parts.push(format!("{} failed", self.failed));
        }
        if self.skipped > 0 {
            parts.push(format!("{} skipped", self.skipped));
        }
        parts.join(", ")
    }

    fn status(self) -> Option<TraceStatus> {
        (self.failed > 0).then_some(TraceStatus::Error)
    }
}

/// The open state of a run's foldable steps: the run, each suite, each test.
#[derive(Clone)]
struct Folds {
    run: Signal<bool>,
    suites: Vec<Signal<bool>>,
    tests: Vec<Vec<Signal<bool>>>,
}

impl Folds {
    fn new(suites: &[Suite]) -> Self {
        Self {
            run: Signal::labeled("run open", false),
            suites: suites.iter().map(|_| Signal::labeled("suite open", false)).collect(),
            tests: suites
                .iter()
                .map(|s| s.tests.iter().map(|_| Signal::labeled("test open", false)).collect())
                .collect(),
        }
    }

    /// Open the run and the suites that failed (Vue's `default-open`), and
    /// the failed tests when `tests_open`.
    fn set_default(&self, suites: &[Suite], tests_open: bool) {
        self.run.set(Counts::of(suites.iter().flat_map(|s| s.tests)).failed > 0);
        for ((suite, open), tests) in suites.iter().zip(&self.suites).zip(&self.tests) {
            open.set(Counts::of(suite.tests).failed > 0);
            for t in tests {
                t.set(tests_open);
            }
        }
    }
}

fn line(status: Option<TraceStatus>, glyph: Icon, title: impl Into<String>) -> Div {
    trace_line(status)
        .child(trace_icon(icon(glyph)))
        .child(trace_title(title))
}

fn test_row(test: &Test, open: &Signal<bool>) -> Element {
    match test.status {
        Status::Failed(error) => trace_collapsible()
            .model(Model::bind(*open))
            .child(trace_collapsible_trigger(
                true,
                line(Some(TraceStatus::Error), Icon::X, test.name),
            ))
            .child(trace_collapsible_content().map_root(|c| c.child(output(error))))
            .into_element(),
        Status::Skipped => line(None, Icon::CircleDashed, format!("Skipped {}", test.name)).into_element(),
        Status::Passed => line(None, Icon::Check, test.name).into_element(),
    }
}

/// The run's step: `Ran …` over a step per suite, each over its tests.
fn results(suites: &[Suite], folds: &Folds) -> Element {
    let summary = Counts::of(suites.iter().flat_map(|s| s.tests));
    trace_collapsible()
        .model(Model::bind(folds.run))
        .child(trace_collapsible_trigger(
            summary.failed > 0,
            line(summary.status(), Icon::FlaskConical, format!("Ran {}", summary.label())),
        ))
        .child(trace_collapsible_content().map_root(|c| {
            c.child(
                trace().children(
                    suites
                        .iter()
                        .zip(&folds.suites)
                        .zip(&folds.tests)
                        .map(|((suite, open), tests)| {
                            let counts = Counts::of(suite.tests);
                            trace_collapsible()
                                .model(Model::bind(*open))
                                .child(trace_collapsible_trigger(
                                    counts.failed > 0,
                                    line(
                                        counts.status(),
                                        Icon::FileCode,
                                        format!("{}, {}", suite.name, counts.label()),
                                    ),
                                ))
                                .child(trace_collapsible_content().map_root(|c| {
                                    c.child(
                                        trace().children(suite.tests.iter().zip(tests).map(|(t, o)| test_row(t, o))),
                                    )
                                }))
                        }),
                ),
            )
        }))
        .into_element()
}

#[derive(Clone, Debug, PartialEq)]
struct Turn {
    /// The tests have run (else they are running, or failed to run once the turn stops).
    ran: bool,
    text: String,
}

impl Turn {
    fn finished() -> Self {
        Self {
            ran: true,
            text: ANSWER.to_owned(),
        }
    }

    /// The tests run for 1500 ms, then the answer streams a word every 20 ms.
    fn replay() -> Vec<(u64, Self)> {
        let mut steps = vec![
            (
                0,
                Self {
                    ran: false,
                    text: String::new(),
                },
            ),
            (
                1500,
                Self {
                    ran: true,
                    text: String::new(),
                },
            ),
        ];
        let mut text = String::new();
        for word in chunks(ANSWER) {
            text.push_str(word);
            steps.push((
                20,
                Self {
                    ran: true,
                    text: text.clone(),
                },
            ));
        }
        steps
    }
}

fn test_results_demo(turn: &Signal<Turn>, folds: &Folds, script: &Script) -> Div {
    let now = turn.get();
    let busy = script.busy();
    let step = if now.ran {
        results(CHECKOUT, folds)
    } else {
        let status = if busy { TraceStatus::Running } else { TraceStatus::Error };
        let verb = if busy { "Running" } else { "Failed to run" };
        line(Some(status), Icon::FlaskConical, format!("{verb} {COMMAND}")).into_element()
    };
    let (turn, folds, play) = (*turn, folds.clone(), script.clone());
    thread()
        .child(user_turn(QUESTION))
        .child(assistant_turn([trace().child(step).into_element(), said(&now.text)]))
        .child(replay(script, move || {
            let (turn, folds) = (turn, folds.clone());
            play.play(frames(Turn::replay(), move |step: &Turn| {
                // The results mount anew once the tests have run.
                if step.ran && !turn.get().ran {
                    folds.set_default(CHECKOUT, true);
                }
                turn.set(step.clone());
            }));
        }))
}

pub fn render() -> Render {
    let turn = Signal::labeled("tests turn", Turn::finished());
    let checkout = Folds::new(CHECKOUT);
    checkout.set_default(CHECKOUT, true);
    let script = Script::new("tests replay");
    let suites = Folds::new(SUITES);
    suites.set_default(SUITES, false);
    Box::new(move || {
        page(
            "Test results",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "TestResultsDemo.vue",
                    test_results_demo(&turn, &checkout, &script),
                ))
                .child(demo(
                    "TestResultsSuitesDemo.vue",
                    trace().max_width(384.0).child(results(SUITES, &suites)),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.click_text("cart.test.ts, 3 tests")?;
    s.frame("cart opened")?;
    s.click_text("updates a user profile")?;
    s.frame("failure opened")?;
    s.click_text("Replay")?;
    s.frame("replay started")?;
    Ok(())
}
