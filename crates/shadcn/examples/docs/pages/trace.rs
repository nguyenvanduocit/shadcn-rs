mod chat;

use std::time::Duration;

use gui::prelude::*;
use gui::{Div, Element, Icon, Model, ScenarioError, Signal, div, icon, text};
use shadcn::{
    BadgeVariant, ButtonSize, ButtonVariant, CodeBlockContentProps, CodeBlockLine, CodeBlockLineVariant,
    CodeBlockProps, CodeBlockSize, CodeBlockVariant, Part, SheetSide, TraceStatus, badge, button, code_block,
    code_block_content, code_block_lines, dialog, dialog_content, dialog_description, dialog_header, dialog_title,
    dialog_trigger, sheet, sheet_content, sheet_description, sheet_header, sheet_title, spinner, trace, trace_action,
    trace_collapsible, trace_collapsible_content, trace_collapsible_trigger, trace_description, trace_end_icon,
    trace_icon, trace_line, trace_title,
};

use crate::{Render, Shot, demo, page};

fn step(status: Option<TraceStatus>, glyph: impl IntoElement, title: &str) -> Div {
    trace_line(status).child(trace_icon(glyph)).child(trace_title(title))
}

/// A command's output: `<CodeBlock size="sm" :variant><CodeBlockContent wrap class="max-h-64">`.
fn output(code: &str, variant: CodeBlockVariant) -> Div {
    let p = CodeBlockProps {
        size: CodeBlockSize::Sm,
        variant,
        ..CodeBlockProps::default()
    };
    let c = CodeBlockContentProps {
        wrap: true,
        ..CodeBlockContentProps::default()
    };
    code_block(p).child(code_block_content(p, c, code_block_lines(code)).max_h(64.0))
}

fn ran(open: &Signal<bool>, title: &str, code: &str) -> Element {
    trace_collapsible()
        .model(Model::bind(*open))
        .child(trace_collapsible_trigger(
            false,
            step(None, icon(Icon::Terminal), title),
        ))
        .child(trace_collapsible_content().map_root(|c| c.child(output(code, CodeBlockVariant::Default))))
        .into_element()
}

/// A step that links out: globe, title, arrow (Vue's `<a target="_blank">`
/// around a trace line). `gui` has no way to open a URL yet, so it takes
/// focus and hover but goes nowhere.
fn link(title: &str) -> Div {
    trace_action(
        false,
        step(None, icon(Icon::Globe), title).child(trace_end_icon(icon(Icon::ArrowUpRight))),
    )
}

/// A step that links to a page of the app: its glyph, title and a chevron.
fn link_to(glyph: Icon, title: &str) -> Div {
    trace_action(
        false,
        step(None, icon(glyph), title).child(trace_end_icon(icon(Icon::ChevronRight))),
    )
}

/// A step that opens on what it found, closed until pressed.
fn folding(open: &Signal<bool>, status: Option<TraceStatus>, line: Div, content: impl FnOnce(Div) -> Div) -> Element {
    trace_collapsible()
        .model(Model::bind(*open))
        .child(trace_collapsible_trigger(status == Some(TraceStatus::Error), line))
        .child(trace_collapsible_content().map_root(content))
        .into_element()
}

/// `TraceLinkDemo.vue`: a step that opens a page of the app, and one that
/// opens a site in a new tab.
fn links() -> Div {
    trace()
        .max_w(96.0)
        .child(link_to(Icon::FileText, "Read apps/v4/lib/docs.ts"))
        .child(link("Install Astro | docs.astro.build"))
}

/// Which steps of `TraceStepsDemo` and `TraceTurnDemo` are open.
#[derive(Clone)]
struct Folds {
    steps: Signal<bool>,
    searched: Signal<bool>,
    searching: Signal<bool>,
    turn: Signal<bool>,
    thought: Signal<bool>,
    commands: Signal<bool>,
    failed: Signal<bool>,
}

impl Folds {
    fn new() -> Self {
        Self {
            steps: Signal::labeled("steps open", true),
            searched: Signal::labeled("searched open", false),
            searching: Signal::labeled("searching open", false),
            turn: Signal::labeled("turn open", true),
            thought: Signal::labeled("thought open", false),
            commands: Signal::labeled("commands open", true),
            failed: Signal::labeled("pnpm failed open", false),
        }
    }

    /// `TraceStepsDemo.vue`: a running investigation and the steps under it.
    fn steps(&self) -> Div {
        let running = Some(TraceStatus::Running);
        div().w_full().max_w(96.0).child(folding(
            &self.steps,
            running,
            trace_line(running).child(trace_title("Looking into the slow deploy")),
            |c| {
                c.child(
                    trace()
                        .child(folding(
                            &self.searched,
                            None,
                            step(None, icon(Icon::Search), "Searched for the checkout build log"),
                            |c| {
                                c.flex().flex_wrap().gap(1.5).children(
                                    ["vercel.com", "github.com"].map(|site| badge(BadgeVariant::Secondary).child(site)),
                                )
                            },
                        ))
                        .child(link_to(Icon::FileText, "Read the last twelve deploys"))
                        .child(folding(
                            &self.searching,
                            running,
                            step(running, icon(Icon::Search), "Searching for the cache key"),
                            |c| c.child(output("cache key image optimization", CodeBlockVariant::Default)),
                        )),
                )
            },
        ))
    }

    /// `TraceTurnDemo.vue`: a finished turn with a failed command, its
    /// thinking, what it said, and the file it read.
    fn turn(&self) -> Div {
        let error = Some(TraceStatus::Error);
        div().w_full().max_w(96.0).child(folding(
            &self.turn,
            error,
            trace_line(error).child(trace_title("Worked for 2m 4s, 1 command failed")),
            |c| {
                c.flex_col()
                    .gap(2.0)
                    .child(
                        trace()
                            .child(folding(
                                &self.thought,
                                None,
                                step(None, icon(Icon::Brain), "Thought").child(trace_description("12s")),
                                |c| {
                                    c.child(text(
                                        "I should check which package managers are installed before I scaffold anything.",
                                    ))
                                },
                            ))
                            .child(folding(
                                &self.commands,
                                error,
                                step(error, icon(Icon::Terminal), "Ran 2 commands, 1 failed"),
                                |c| {
                                    c.child(
                                        trace()
                                            .child(step(None, icon(Icon::Terminal), "Ran node --version"))
                                            .child(folding(
                                                &self.failed,
                                                error,
                                                step(error, icon(Icon::Terminal), "Failed to run pnpm --version"),
                                                |c| {
                                                    c.child(output(
                                                        "command not found: pnpm",
                                                        CodeBlockVariant::Destructive,
                                                    ))
                                                },
                                            )),
                                    )
                                },
                            )),
                    )
                    .child(text("Node is installed and pnpm is not. Reading the docs config."))
                    .child(trace().child(link_to(Icon::FileText, "Read apps/v4/lib/docs.ts")))
            },
        ))
    }
}

/// The links a step used, one per line.
fn sources_trace(titles: &[&str]) -> Div {
    trace().children(titles.iter().map(|t| link(t)))
}

/// A step that opens a dialog or sheet: its line, and a chevron after the title.
fn opener(line: Div) -> Part<()> {
    dialog_trigger(trace_action(
        false,
        line.child(trace_end_icon(icon(Icon::ChevronRight))),
    ))
}

/// A file a commit changed, and its diff.
struct Changed {
    path: &'static str,
    added: bool,
    additions: u32,
    deletions: u32,
    diff: &'static str,
}

const CHANGED: [Changed; 2] = [
    Changed {
        path: "pages/login.vue",
        added: false,
        additions: 2,
        deletions: 2,
        diff: "@@ -1,6 +1,6 @@
 <script setup lang=\"ts\">
 const route = useRoute()
 const next = route.query.next as string | undefined
-const redirectTo =
-  next && next.startsWith('/') ? next : '/'
+const redirectTo = safeRedirect(next)
+
 </script>",
    },
    Changed {
        path: "utils/redirect.ts",
        added: true,
        additions: 7,
        deletions: 0,
        diff: "@@ -0,0 +1,7 @@
+export function safeRedirect(next: string | undefined) {
+  if (!next || !next.startsWith('/') || next.startsWith('//')) {
+    return '/'
+  }
+
+  return next
+}",
    },
];

/// A diff as code block lines: `+` added, `-` removed, the hunk header as is.
fn diff_lines(diff: &str) -> Vec<CodeBlockLine> {
    diff.split('\n')
        .map(|l| {
            if l.starts_with("@@") {
                // Vue also mutes the hunk header (`text-muted-foreground` on
                // its line); `CodeBlockLine` takes no class of its own here.
                return CodeBlockLine::new(l, CodeBlockLineVariant::Default);
            }
            let variant = match l.chars().next() {
                Some('+') => CodeBlockLineVariant::Added,
                Some('-') => CodeBlockLineVariant::Removed,
                _ => CodeBlockLineVariant::Default,
            };
            CodeBlockLine::new(l.get(1..).unwrap_or_default(), variant)
        })
        .collect()
}

/// `TraceCommitDemo.vue`: a commit that opens to the files it changed, each
/// opening its diff in a sheet.
fn commit(open: &Signal<bool>) -> Div {
    let files = CHANGED.iter().map(|f| {
        let glyph = if f.added { Icon::FilePlus } else { Icon::FilePen };
        let title = format!("{} {}", if f.added { "Added" } else { "Edited" }, f.path);
        let line = step(None, icon(glyph), &title)
            .child(trace_description(format!("+{} \u{2212}{}", f.additions, f.deletions)).font_mono());
        let p = CodeBlockProps::default();
        sheet().child(opener(line)).child(
            sheet_content(SheetSide::Right)
                .child(
                    sheet_header()
                        .child(sheet_title(f.path).font_mono())
                        .child(sheet_description(format!(
                            "{} additions, {} deletions",
                            f.additions, f.deletions
                        ))),
                )
                .child(code_block(p).mx(6.0).mb(6.0).w_auto().child(code_block_content(
                    p,
                    CodeBlockContentProps::default(),
                    diff_lines(f.diff),
                ))),
        )
    });
    trace().max_w(96.0).child(
        trace_collapsible()
            .model(Model::bind(*open))
            .child(trace_collapsible_trigger(
                false,
                step(
                    None,
                    icon(Icon::GitCommitHorizontal),
                    "Committed a1b2c3: Fix the login redirect",
                ),
            ))
            .child(trace_collapsible_content().map_root(|c| c.child(trace().children(files)))),
    )
}

/// `TraceDialogDemo.vue`: a web search that opens its results in a dialog.
fn searched() -> Div {
    const TITLE: &str = "Searched the web for cache keys";
    trace().max_w(96.0).child(
        dialog().child(opener(step(None, icon(Icon::Globe), TITLE))).child(
            dialog_content()
                .child(
                    dialog_header()
                        .child(dialog_title(TITLE))
                        .child(dialog_description("Three results were used in the response.")),
                )
                .child(sources_trace(&[
                    "Caching build outputs",
                    "Image optimization reference",
                    "Deploy hooks and cache keys",
                ])),
        ),
    )
}

/// `TraceSheetDemo.vue`: explored files listed in a sheet.
fn explored() -> Div {
    const TITLE: &str = "Explored 4 files";
    trace().max_w(96.0).child(
        sheet()
            .child(opener(step(None, icon(Icon::FolderSearch), TITLE)))
            .child(
                sheet_content(SheetSide::Right)
                    .child(
                        sheet_header()
                            .child(sheet_title(TITLE))
                            .child(sheet_description("Files read while preparing the response.")),
                    )
                    .child(
                        div().flex_col().gap(2.0).px(6.0).font_mono().text_xs().children(
                            [
                                "apps/v4/lib/docs.ts",
                                "apps/v4/lib/components.ts",
                                "apps/v4/nuxt.config.ts",
                                "apps/v4/package.json",
                            ]
                            .map(text),
                        ),
                    ),
            ),
    )
}

/// `TraceSourcesDemo.vue`: the sources an answer used, open.
fn sources(open: &Signal<bool>) -> Div {
    let titles = [
        "Install Astro | docs.astro.build",
        "create-astro | npmjs.com",
        "Settings: minimumReleaseAge | pnpm.io",
    ];
    div().w_full().max_w(96.0).child(
        trace_collapsible()
            .model(Model::bind(*open))
            .child(trace_collapsible_trigger(
                false,
                step(None, icon(Icon::Globe), &format!("Used {} sources", titles.len())),
            ))
            .child(trace_collapsible_content().map_root(|c| c.child(sources_trace(&titles)))),
    )
}

pub fn render() -> Render {
    let commit_open = Signal::labeled("commit open", true);
    let sources_open = Signal::labeled("sources open", true);
    let conversation = chat::TraceDemo::new();
    let thinking = chat::ThinkingDemo::new();
    let tool_calls = chat::ToolCallsDemo::new();
    let node = Signal::labeled("node open", true);
    let pnpm = Signal::labeled("pnpm open", false);
    let lone = Signal::labeled("lone open", false);
    let folds = Folds::new();
    Box::new(move || {
        let toggle = pnpm;
        let opened = demo(
            "TraceOpenDemo.vue",
            div()
                .flex_col()
                .w_full()
                .max_w(96.0)
                .items_start()
                .gap(4.0)
                .child(ran(&node, "Ran node --version", "$ node --version\nv24.18.0"))
                .child(ran(&pnpm, "Ran pnpm --version", "$ pnpm --version\n10.33.4"))
                .child(
                    button(ButtonVariant::Outline, ButtonSize::Sm)
                        .child(if pnpm.get() {
                            "Hide the pnpm output"
                        } else {
                            "Show the pnpm output"
                        })
                        .on_click(move |_| toggle.update(|o| *o = !*o)),
                ),
        );
        let collapsible = demo(
            "TraceCollapsibleDemo.vue",
            div()
                .w_full()
                .max_w(96.0)
                .child(ran(&lone, "Ran node --version", "$ node --version\nv24.18.0")),
        );
        let status = demo(
            "TraceStatusDemo.vue",
            trace()
                .max_w(96.0)
                .child(step(None, icon(Icon::CircleCheck), "Ran pnpm install"))
                .child(step(
                    Some(TraceStatus::Error),
                    icon(Icon::CircleAlert),
                    "Failed to run pnpm lint",
                ))
                .child(step(Some(TraceStatus::Running), spinner(), "Running pnpm build"))
                .child(step(Some(TraceStatus::Pending), icon(Icon::Circle), "Run pnpm test")),
        );
        let described = demo(
            "TraceDescriptionDemo.vue",
            trace()
                .max_w(96.0)
                .child(
                    step(
                        None,
                        icon(Icon::FilePen),
                        "Edited components/settings/NotificationPreferencesForm.vue",
                    )
                    .child(trace_description("+48 \u{2212}12").font_mono()),
                )
                .child(step(None, icon(Icon::Terminal), "Ran pnpm test").child(trace_description("14s")))
                .child(step(None, icon(Icon::Search), "Searched for useChat").child(trace_description("8 results"))),
        );
        let single = demo(
            "TraceStaticDemo.vue",
            trace()
                .max_w(96.0)
                .child(step(None, icon(Icon::GitBranch), "Created branch fix/frozen-lockfile")),
        );
        let group = demo(
            "TraceGroupDemo.vue",
            trace()
                .max_w(96.0)
                .child(step(None, icon(Icon::Search), "Searched for minimumReleaseAge"))
                .child(step(None, icon(Icon::FileText), "Read pnpm-workspace.yaml"))
                .child(step(None, icon(Icon::Terminal), "Ran pnpm install")),
        );
        page(
            "Trace",
            div()
                .flex_col()
                .gap(8.0)
                .child(opened)
                .child(collapsible)
                .child(status)
                .child(described)
                .child(single)
                .child(group)
                .child(demo("TraceCommitDemo.vue", commit(&commit_open)))
                .child(demo("TraceDemo.vue", conversation.render()))
                .child(demo("TraceDialogDemo.vue", searched()))
                .child(demo("TraceSheetDemo.vue", explored()))
                .child(demo("TraceSourcesDemo.vue", sources(&sources_open)))
                .child(demo("TraceThinkingDemo.vue", thinking.render()))
                .child(demo("TraceToolCallsDemo.vue", tool_calls.render()))
                .child(demo("TraceLinkDemo.vue", links()))
                .child(demo("TraceStepsDemo.vue", folds.steps()))
                .child(demo("TraceTurnDemo.vue", folds.turn())),
        )
    })
}

/// Let `ms` pass, running the timers a stream schedules as they fall due.
fn wait(s: &mut Shot, ms: u64) {
    s.wait(Duration::from_millis(ms));
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.click_text("Show the pnpm output")?;
    s.frame("pnpm shown")?;
    let (x, y) = s.locate("Ran node --version")?;
    s.hover(x, y);
    s.frame("hover node step")?;
    s.click(x, y);
    s.frame("node closed")?;

    s.click_text("Edited pages/login.vue")?;
    s.frame("commit diff sheet")?;
    s.press(gui::NamedKey::Escape, gui::Modifiers::default());
    s.settle()?;
    s.click_text("Searched the web for cache keys")?;
    s.frame("search results dialog")?;
    s.press(gui::NamedKey::Escape, gui::Modifiers::default());
    s.settle()?;
    s.click_text("Explored 4 files")?;
    s.frame("explored files sheet")?;
    s.press(gui::NamedKey::Escape, gui::Modifiers::default());
    s.settle()?;

    s.click_text("Replay")?;
    wait(s, 600);
    s.frame("replay thinking")?;
    wait(s, 2500);
    s.frame("replay running tools")?;
    wait(s, 4000);
    s.frame("replay answered")?;

    s.click_text("Send message")?;
    wait(s, 800);
    s.frame("thinking streams open")?;
    wait(s, 3500);
    s.frame("thought folded, answered")?;
    s.click_text("Thought")?;
    s.frame("thought opened")?;

    // The thinking demo now offers Reset, so this is the tool calls' button.
    s.click_text("Send message")?;
    wait(s, 1200);
    s.frame("tool calls running")?;
    wait(s, 2500);
    s.frame("tool calls done")?;
    s.click_text("Failed to run pnpm --version")?;
    s.frame("failed call opened")?;
    s.click_text("Reset")?;
    s.frame("thinking reset")?;
    Ok(())
}
