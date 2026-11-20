//! The stress scenes: each an app built to push one part of `gui` past what
//! a gallery page asks of it (many nodes, deep trees, many signals, a thread
//! flooding the UI, huge text, keyed churn), and one composite app that uses
//! many shadcn components together.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gui::prelude::*;
use gui::{Computed, Div, Effect, Element, Signal, TextField, Track, batch, div, key_of, mailbox, text};
use shadcn::{
    BadgeVariant, ButtonSize, ButtonVariant, DialogState, TabsState, Toaster, badge, button, card, card_content,
    card_description, card_header, card_title, checkbox, checkbox_state, dialog_close, dialog_content,
    dialog_description, dialog_footer, dialog_header, dialog_title, dialog_trigger, input, label, progress, switch,
    table, table_body, table_cell, table_head, table_header, table_row, tabs, tabs_content, tabs_list, tabs_trigger,
    textarea, theme, toaster,
};

use crate::Render;

/// One stress scene: an app, what to do with it, and what to repeat.
pub struct Scene {
    pub name: &'static str,
    pub about: &'static str,
    pub render: fn() -> Render,
    /// Its window, in logical pixels.
    pub size: (u32, u32),
    /// Devtools commands run once, in order, after the first frame.
    pub script: &'static [&'static str],
    /// Devtools commands repeated: after the first round and after the last,
    /// at rest, the heap must hold the same (what one round takes, it gives back).
    pub cycle: &'static [&'static str],
}

pub const ALL: &[Scene] = &[
    Scene {
        name: "table",
        about: "a data table of 5 000 rows, all laid out: filter, sort, select",
        render: data_table,
        size: (1280, 800),
        script: &[
            "click \"Filter emails…\"",
            "type 77",
            "press mod+a",
            "press backspace",
            "activate Amount",
            "wheel 4000 #rows",
            "click \"Select all\"",
        ],
        cycle: &[
            "click \"Filter emails…\"",
            "type 9",
            "press backspace",
            "activate Amount",
        ],
    },
    Scene {
        name: "tree",
        about: "a file tree of 1 600 entries in folders four deep, and a chain of 120 nested boxes",
        render: file_tree,
        size: (1024, 768),
        script: &["click \"Expand all\"", "wheel 3000 #tree", "click \"Collapse all\""],
        cycle: &["click \"Expand all\"", "click \"Collapse all\""],
    },
    Scene {
        name: "grid",
        about: "2 500 buttons with hover styles, each counting its own clicks",
        render: button_grid,
        size: (1280, 900),
        script: &["click \"#17\"", "click \"#17\"", "hover \"#900\"", "click Reset"],
        cycle: &["click \"#42\"", "hover \"#43\"", "click Reset"],
    },
    Scene {
        name: "reactive",
        about: "5 000 signals, a chain of 2 000 computed values and 500 effects",
        render: reactive_graph,
        size: (960, 640),
        script: &["click \"Bump all\"", "click \"Bump first\"", "click \"Bump all\""],
        cycle: &["click \"Bump all\"", "click \"Bump first\""],
    },
    Scene {
        name: "feed",
        about: "a thread sends 5 000 messages through a mailbox; the list keeps the last 200",
        render: feed,
        size: (960, 640),
        script: &["click Burst", "frame", "click Burst", "frame"],
        cycle: &["click Burst", "frame", "click Clear"],
    },
    Scene {
        name: "toasts",
        about: "30 toasts at once, their timers, hover to expand, and their exits",
        render: toast_storm,
        size: (960, 640),
        script: &["click \"Burst 30\"", "hover \"Toast 30\"", "leave", "wait 6000", "rest"],
        cycle: &["click \"Burst 30\"", "wait 8000", "rest"],
    },
    Scene {
        name: "text",
        about: "60 KB of text in five scripts, a 2 000-letter word, a 5 000-letter input",
        render: big_text,
        size: (1024, 768),
        script: &[
            "click @0/0",
            "click \"Big input\"",
            "press end",
            "type \" và thêm chữ\"",
            "press mod+a",
            "wheel 6000 #prose",
        ],
        cycle: &[
            "click \"Big input\"",
            "type xyz",
            "press backspace",
            "press backspace",
            "press backspace",
        ],
    },
    Scene {
        name: "shuffle",
        about: "400 keyed rows shuffled, reversed, inserted into and removed from",
        render: shuffle,
        size: (960, 800),
        script: &[
            "click Shuffle",
            "click Reverse",
            "click Insert",
            "click Remove",
            "click Shuffle",
        ],
        cycle: &["click Shuffle", "click Insert", "click Remove"],
    },
    Scene {
        name: "churn",
        about: "300 cards mounted and unmounted, each with its own state",
        render: churn,
        size: (1280, 800),
        script: &["click Mount", "click Unmount", "click Mount"],
        cycle: &["click Unmount", "click Mount"],
    },
    Scene {
        name: "dashboard",
        about: "an admin app: navigation, tabs, KPI cards, a customer table, an edit dialog, toasts",
        render: dashboard,
        size: (1440, 900),
        script: &[
            "click Customers",
            "click \"Search customers…\"",
            "type an",
            "click Edit",
            "click @0",
            "setvalue \"Customer name\" \"Ana Nguyễn\"",
            "click \"Save changes\"",
            "click Settings",
            "click \"Email reports\"",
            "click Overview",
            "click Analytics",
        ],
        cycle: &[
            "click Customers",
            "click Edit",
            "click \"Save changes\"",
            "rest",
            "click Overview",
        ],
    },
];

// ── table ────────────────────────────────────────────────────────────────────

const ROWS: usize = 5_000;
const STATUSES: [&str; 4] = ["success", "processing", "failed", "pending"];

/// A row of made-up data, the same for the same `i`.
fn payment(i: usize) -> (String, String, u32, &'static str) {
    let names = [
        "ken",
        "abe",
        "monserrat",
        "silas",
        "carmella",
        "an",
        "linh",
        "minh",
        "olga",
        "yuki",
    ];
    let hosts = ["yahoo.com", "gmail.com", "hotmail.com", "proton.me"];
    let name = names
        .get(i.checked_rem(names.len()).unwrap_or(0))
        .copied()
        .unwrap_or("x");
    let host = hosts
        .get(i.checked_rem(hosts.len()).unwrap_or(0))
        .copied()
        .unwrap_or("x");
    let amount = u32::try_from(i.wrapping_mul(7_919).checked_rem(1_000).unwrap_or(0)).unwrap_or(0);
    let status = STATUSES
        .get(i.checked_rem(STATUSES.len()).unwrap_or(0))
        .copied()
        .unwrap_or("pending");
    (format!("{i:05x}"), format!("{name}{i}@{host}"), amount, status)
}

fn status_badge(status: &str) -> Div {
    let variant = match status {
        "success" => BadgeVariant::Default,
        "failed" => BadgeVariant::Destructive,
        "processing" => BadgeVariant::Secondary,
        _ => BadgeVariant::Outline,
    };
    badge(variant).child(status.to_owned())
}

fn data_table() -> Render {
    let data: Rc<Vec<(String, String, u32, &'static str)>> = Rc::new((0..ROWS).map(payment).collect());
    let filter = TextField::new("");
    let sort_desc = Signal::labeled("amount sort", None::<bool>);
    let selected = Signal::labeled("selected rows", 0_usize);
    let all = Signal::labeled("all selected", false);
    Box::new(move || {
        let t = theme();
        let needle = filter.text().to_lowercase();
        let mut rows: Vec<usize> = (0..data.len())
            .filter(|&i| data.get(i).is_some_and(|p| p.1.contains(&needle)))
            .collect();
        if let Some(desc) = sort_desc.get() {
            rows.sort_by_key(|&i| data.get(i).map_or(0, |p| p.2));
            if desc {
                rows.reverse();
            }
        }
        let tracks = [
            Track::Fr(0.4),
            Track::Fr(1.0),
            Track::Fr(2.4),
            Track::Fr(1.0),
            Track::Fr(1.0),
        ];
        let everything = all.get();
        let sort = sort_desc.clone();
        let head = table_row(&tracks, false)
            .child(table_head(Element::Empty))
            .child(table_head("ID"))
            .child(table_head("Email"))
            .child(table_head(
                button(ButtonVariant::Ghost, ButtonSize::Sm)
                    .on_click(move |_| sort.update(|s| Some(s != Some(true))))
                    .child("Amount"),
            ))
            .child(table_head("Status"));
        let body = rows.iter().filter_map(|&i| {
            let (id, email, amount, status) = data.get(i)?;
            let picked = selected.clone();
            Some(
                table_row(&tracks, everything)
                    .key(key_of(id))
                    .child(table_cell(checkbox_state(everything.into(), move |on| {
                        picked.update(|n| if on { n.saturating_add(1) } else { n.saturating_sub(1) });
                    })))
                    .child(table_cell(id.clone()).font_mono())
                    .child(table_cell(email.clone()))
                    .child(table_cell(format!("${amount}.00")))
                    .child(table_cell(status_badge(status))),
            )
        });
        let toggle = all.clone();
        div()
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .p(6.0)
            .flex_col()
            .gap(4.0)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(4.0)
                    .child(input(&filter).max_w_px(320.0).placeholder("Filter emails…"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(2.0)
                            .child(checkbox_state(everything.into(), move |on| toggle.set(on)))
                            .child(label("Select all")),
                    )
                    .child(
                        text(format!(
                            "{} of {} rows, {} picked",
                            rows.len(),
                            data.len(),
                            selected.get()
                        ))
                        .text_sm()
                        .text_color(t.muted_foreground),
                    ),
            )
            .child(
                div()
                    .named("rows")
                    .flex_1()
                    .min_h(0.0)
                    .overflow_y_scroll()
                    .child(table().child(table_header([head])).child(table_body(body))),
            )
            .into_element()
    })
}

// ── tree ─────────────────────────────────────────────────────────────────────

/// Folders per level and files per folder: 4 levels of 4 folders, 6 files each.
const FANOUT: usize = 4;
const FILES: usize = 6;
const LEVELS: usize = 4;

struct Folder {
    name: String,
    open: Signal<bool>,
    folders: Vec<Folder>,
    files: Vec<String>,
}

fn folder(name: String, level: usize, every: &mut Vec<Signal<bool>>) -> Folder {
    let open = Signal::new(level == 0);
    every.push(open.clone());
    let folders = if level.saturating_add(1) < LEVELS {
        (0..FANOUT)
            .map(|i| folder(format!("{name}/d{i}"), level.saturating_add(1), every))
            .collect()
    } else {
        Vec::new()
    };
    let files = (0..FILES).map(|i| format!("{name}/file{i}.rs")).collect();
    Folder {
        name,
        open,
        folders,
        files,
    }
}

fn folder_view(f: &Folder, depth: usize) -> Element {
    let t = theme();
    let open = f.open.get();
    let toggle = f.open.clone();
    // One Tailwind unit (4px) per level.
    let indent = f32::from(u16::try_from(depth).unwrap_or(u16::MAX));
    let row = div()
        .flex()
        .items_center()
        .gap(2.0)
        .pl(indent)
        .py(0.5)
        .rounded_md()
        .cursor_pointer()
        .hover(move |s| s.bg(t.accent))
        .on_click(move |_| toggle.update(|o| !o))
        .child(text(if open { "▾" } else { "▸" }).text_xs())
        .child(text(f.name.rsplit('/').next().unwrap_or(&f.name).to_owned()).text_sm());
    let mut node = div().flex_col().child(row);
    if open {
        for sub in &f.folders {
            node = node.child(folder_view(sub, depth.saturating_add(1)));
        }
        for file in &f.files {
            node = node.child(
                text(file.rsplit('/').next().unwrap_or(file).to_owned())
                    .text_sm()
                    .text_color(t.muted_foreground)
                    .pl(indent + 5.0),
            );
        }
    }
    node.into_element()
}

/// Boxes nested `depth` deep, each a pixel inside the last.
fn chain(depth: usize) -> Element {
    let t = theme();
    let mut inner = div().w_px(8.0).h_px(8.0).bg(t.primary).into_element();
    for _ in 0..depth {
        inner = div()
            .p(0.25)
            .border_w(1.0)
            .border_color(t.border)
            .child(inner)
            .into_element();
    }
    inner
}

fn file_tree() -> Render {
    let mut every = Vec::new();
    let root = folder("src".to_owned(), 0, &mut every);
    let every = Rc::new(every);
    Box::new(move || {
        let t = theme();
        let (open_all, close_all) = (Rc::clone(&every), Rc::clone(&every));
        let shown = every.iter().filter(|o| o.get()).count();
        div()
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .p(6.0)
            .flex()
            .gap(6.0)
            .child(
                div()
                    .flex_col()
                    .gap(2.0)
                    .w_px(420.0)
                    .child(
                        div()
                            .flex()
                            .gap(2.0)
                            .child(
                                button(ButtonVariant::Outline, ButtonSize::Sm)
                                    .on_click(move |_| batch(|| open_all.iter().for_each(|o| o.set(true))))
                                    .child("Expand all"),
                            )
                            .child(
                                button(ButtonVariant::Outline, ButtonSize::Sm)
                                    .on_click(move |_| batch(|| close_all.iter().for_each(|o| o.set(false))))
                                    .child("Collapse all"),
                            ),
                    )
                    .child(
                        text(format!("{shown} of {} folders open", every.len()))
                            .text_xs()
                            .text_color(t.muted_foreground),
                    )
                    .child(
                        div()
                            .named("tree")
                            .flex_1()
                            .min_h(0.0)
                            .overflow_y_scroll()
                            .child(folder_view(&root, 0)),
                    ),
            )
            .child(div().named("chain").child(chain(120)))
            .into_element()
    })
}

// ── grid ─────────────────────────────────────────────────────────────────────

const BUTTONS: usize = 2_500;

fn button_grid() -> Render {
    let counts: Rc<Vec<Signal<u32>>> = Rc::new((0..BUTTONS).map(|_| Signal::new(0)).collect());
    let total = Signal::labeled("grid clicks", 0_u32);
    Box::new(move || {
        let t = theme();
        let reset = (Rc::clone(&counts), total.clone());
        let cells = counts.iter().enumerate().map(|(i, c)| {
            let (count, all) = (c.clone(), total.clone());
            let n = c.get();
            button(
                if n > 0 {
                    ButtonVariant::Default
                } else {
                    ButtonVariant::Outline
                },
                ButtonSize::Xs,
            )
            .w_px(44.0)
            .on_click(move |_| {
                count.update(|n| n.saturating_add(1));
                all.update(|n| n.saturating_add(1));
            })
            .child(if n > 0 { format!("{n}") } else { format!("#{i}") })
        });
        div()
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .p(4.0)
            .flex_col()
            .gap(3.0)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(3.0)
                    .child(
                        button(ButtonVariant::Secondary, ButtonSize::Sm)
                            .on_click(move |_| {
                                batch(|| {
                                    reset.0.iter().for_each(|c| c.set(0));
                                    reset.1.set(0);
                                });
                            })
                            .child("Reset"),
                    )
                    .child(text(format!("{} clicks", total.get())).text_sm()),
            )
            .child(
                div()
                    .named("grid")
                    .flex_1()
                    .min_h(0.0)
                    .overflow_y_scroll()
                    .child(div().flex().flex_wrap().gap(1.0).children(cells)),
            )
            .into_element()
    })
}

// ── reactive ─────────────────────────────────────────────────────────────────

const SIGNALS: usize = 5_000;
const CHAIN: usize = 2_000;
const EFFECTS: usize = 500;

struct Graph {
    signals: Vec<Signal<i64>>,
    chain: Vec<Computed<i64>>,
    _effects: Vec<Effect>,
    runs: Rc<Cell<u64>>,
}

fn reactive_graph() -> Render {
    let signals: Vec<Signal<i64>> = (0..SIGNALS).map(|_| Signal::new(1)).collect();
    // c[0] = s[0]; c[i] = c[i-1] + s[i]: a write to s[0] runs the whole chain.
    let mut chain: Vec<Computed<i64>> = Vec::with_capacity(CHAIN);
    for i in 0..CHAIN {
        let s = signals.get(i).cloned();
        let before = chain.last().cloned();
        chain.push(Computed::new(move || {
            before
                .as_ref()
                .map_or(0, Computed::get)
                .saturating_add(s.as_ref().map_or(0, Signal::get))
        }));
    }
    let runs = Rc::new(Cell::new(0_u64));
    let effects = (0..EFFECTS)
        .map(|e| {
            // Each effect reads ten signals spread over all of them.
            let read: Vec<Signal<i64>> = (0..10)
                .filter_map(|k| signals.get(e.wrapping_mul(10).wrapping_add(k).checked_rem(SIGNALS).unwrap_or(0)))
                .cloned()
                .collect();
            let r = Rc::clone(&runs);
            Effect::new(move || {
                let _sum: i64 = read.iter().map(Signal::get).sum();
                r.set(r.get().saturating_add(1));
            })
        })
        .collect();
    let g = Rc::new(Graph {
        signals,
        chain,
        _effects: effects,
        runs,
    });
    Box::new(move || {
        let t = theme();
        let (all, first) = (Rc::clone(&g), Rc::clone(&g));
        let end = g.chain.last().map_or(0, Computed::get);
        div()
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .p(8.0)
            .flex_col()
            .gap(4.0)
            .child(
                div()
                    .flex()
                    .gap(2.0)
                    .child(
                        button(ButtonVariant::Default, ButtonSize::Default)
                            .on_click(move |_| {
                                batch(|| all.signals.iter().for_each(|s| s.update(|v| v.saturating_add(1))))
                            })
                            .child("Bump all"),
                    )
                    .child(
                        button(ButtonVariant::Outline, ButtonSize::Default)
                            .on_click(move |_| {
                                if let Some(s) = first.signals.first() {
                                    s.update(|v| v.saturating_add(1));
                                }
                            })
                            .child("Bump first"),
                    ),
            )
            .child(text(format!("end of the chain: {end}")).named("chain-end"))
            .child(text(format!("effect runs: {}", g.runs.get())).named("effect-runs"))
            .into_element()
    })
}

// ── feed ─────────────────────────────────────────────────────────────────────

const MESSAGES: usize = 5_000;
const KEEP: usize = 200;

fn feed() -> Render {
    let lines = Signal::labeled("feed lines", Vec::<String>::new());
    let received = Signal::labeled("feed received", 0_usize);
    let (l, r) = (lines.clone(), received.clone());
    let to_ui = mailbox(move |msg: String| {
        r.update(|n| n.saturating_add(1));
        l.update(|mut all| {
            all.push(msg);
            if all.len() > KEEP {
                let over = all.len().saturating_sub(KEEP);
                all.drain(..over);
            }
            all
        });
    });
    let burst = Rc::new(Cell::new(0_usize));
    Box::new(move || {
        let t = theme();
        let (send, n) = (to_ui.clone(), Rc::clone(&burst));
        let (clear_lines, clear_count) = (lines.clone(), received.clone());
        let shown = lines.get();
        div()
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .p(6.0)
            .flex_col()
            .gap(3.0)
            .child(
                div()
                    .flex()
                    .gap(2.0)
                    .items_center()
                    .child(
                        button(ButtonVariant::Default, ButtonSize::Sm)
                            .on_click(move |_| {
                                let round = n.get().saturating_add(1);
                                n.set(round);
                                let send = send.clone();
                                std::thread::spawn(move || {
                                    for i in 0..MESSAGES {
                                        if !send.send(format!("burst {round} message {i}: lorem ipsum dolor")) {
                                            break;
                                        }
                                    }
                                });
                            })
                            .child("Burst"),
                    )
                    .child(
                        button(ButtonVariant::Outline, ButtonSize::Sm)
                            .on_click(move |_| {
                                batch(|| {
                                    clear_lines.set(Vec::new());
                                    clear_count.set(0);
                                });
                            })
                            .child("Clear"),
                    )
                    .child(text(format!("{} received", received.get())).text_sm()),
            )
            .child(
                div()
                    .named("feed")
                    .flex_1()
                    .min_h(0.0)
                    .overflow_y_scroll()
                    .font_mono()
                    .text_xs()
                    .children(shown.into_iter().map(text)),
            )
            .into_element()
    })
}

/// How long a feed burst takes to arrive, at most, in a headless run.
pub const FEED_SETTLE: Duration = Duration::from_millis(500);

// ── toasts ───────────────────────────────────────────────────────────────────

fn toast_storm() -> Render {
    let toasts = Toaster::new();
    let next = Rc::new(Cell::new(0_u32));
    Box::new(move || {
        let t = theme();
        let (burst, n) = (toasts.clone(), Rc::clone(&next));
        div()
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .p(8.0)
            .child(
                button(ButtonVariant::Outline, ButtonSize::Default)
                    .on_click(move |_| {
                        for _ in 0..30 {
                            let i = n.get().saturating_add(1);
                            n.set(i);
                            match i.checked_rem(4).unwrap_or(0) {
                                0 => burst.success(format!("Toast {i}")),
                                1 => burst.error(format!("Toast {i}")),
                                2 => burst.info(format!("Toast {i}")),
                                _ => burst.toast(format!("Toast {i}")),
                            };
                        }
                    })
                    .child("Burst 30"),
            )
            .child(toaster(&toasts))
            .into_element()
    })
}

// ── text ─────────────────────────────────────────────────────────────────────

const PROSE: &str = "The quick brown fox jumps over the lazy dog. \
Tiếng Việt có dấu: Những con chim én bay lượn trên bầu trời xanh thẳm. \
العربية: الثعلب البني السريع يقفز فوق الكلب الكسول. \
日本語：素早い茶色の狐がのろまな犬を飛び越える。 \
Emoji: 🦊🐶🚀✨ and numbers 1234567890. ";

fn big_text() -> Render {
    let prose: Rc<Vec<String>> = Rc::new(
        (0..3)
            .map(|p| format!("Paragraph {p}. {}", PROSE.repeat(100)))
            .collect(),
    );
    let word = "a".repeat(2_000);
    let field = TextField::new("x".repeat(5_000));
    let notes = TextField::new(PROSE.repeat(20));
    Box::new(move || {
        let t = theme();
        div()
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .p(6.0)
            .flex_col()
            .gap(3.0)
            .child(input(&field).placeholder("Big input"))
            .child(textarea(&notes).h_px(120.0))
            .child(
                div()
                    .named("prose")
                    .flex_1()
                    .min_h(0.0)
                    .overflow_y_scroll()
                    .flex_col()
                    .gap(3.0)
                    .child(text(word.clone()).text_sm())
                    .children(prose.iter().map(|p| text(p.clone()).text_sm())),
            )
            .into_element()
    })
}

// ── shuffle ──────────────────────────────────────────────────────────────────

const ITEMS: u32 = 400;

fn shuffle() -> Render {
    let items = Signal::labeled("shuffled items", (0..ITEMS).collect::<Vec<u32>>());
    let next = Rc::new(Cell::new(ITEMS));
    let seed = Rc::new(Cell::new(0x9e37_79b9_u64));
    Box::new(move || {
        let t = theme();
        let (sh, rev, ins, rem) = (items.clone(), items.clone(), items.clone(), items.clone());
        let (n, s) = (Rc::clone(&next), Rc::clone(&seed));
        let rows = items.get().into_iter().map(|i| {
            div()
                .key(key_of(&format!("item-{i}")))
                .flex()
                .justify_between()
                .px(3.0)
                .py(1.0)
                .border_b()
                .border_color(t.border)
                .child(text(format!("Item {i}")).text_sm())
                .child(badge(BadgeVariant::Secondary).child(format!("{}", i.checked_rem(7).unwrap_or(0))))
        });
        let action = |name: &'static str, f: Box<dyn Fn()>| {
            button(ButtonVariant::Outline, ButtonSize::Sm)
                .on_click(move |_| f())
                .child(name)
        };
        div()
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .p(6.0)
            .flex_col()
            .gap(3.0)
            .child(
                div()
                    .flex()
                    .gap(2.0)
                    .child(action(
                        "Shuffle",
                        Box::new(move || {
                            sh.update(|mut v| {
                                // Fisher–Yates with a xorshift: the same every run.
                                for i in (1..v.len()).rev() {
                                    let mut x = s.get();
                                    x ^= x << 13;
                                    x ^= x >> 7;
                                    x ^= x << 17;
                                    s.set(x);
                                    let j = usize::try_from(
                                        x.checked_rem(u64::try_from(i.saturating_add(1)).unwrap_or(1))
                                            .unwrap_or(0),
                                    )
                                    .unwrap_or(0);
                                    v.swap(i, j.min(i));
                                }
                                v
                            });
                        }),
                    ))
                    .child(action(
                        "Reverse",
                        Box::new(move || {
                            rev.update(|mut v| {
                                v.reverse();
                                v
                            });
                        }),
                    ))
                    .child(action(
                        "Insert",
                        Box::new(move || {
                            let i = n.get();
                            n.set(i.saturating_add(1));
                            ins.update(|mut v| {
                                let at = v.len().checked_div(2).unwrap_or(0);
                                v.insert(at, i);
                                v
                            });
                        }),
                    ))
                    .child(action(
                        "Remove",
                        Box::new(move || {
                            rem.update(|mut v| {
                                if !v.is_empty() {
                                    v.remove(v.len().checked_div(2).unwrap_or(0));
                                }
                                v
                            });
                        }),
                    )),
            )
            .child(
                div()
                    .named("items")
                    .flex_1()
                    .min_h(0.0)
                    .overflow_y_scroll()
                    .children(rows),
            )
            .into_element()
    })
}

// ── churn ────────────────────────────────────────────────────────────────────

const CARDS: usize = 300;

fn churn() -> Render {
    let mounted = Signal::labeled("cards mounted", false);
    Box::new(move || {
        let t = theme();
        let (on, off) = (mounted.clone(), mounted.clone());
        let cards = mounted.get().then(|| {
            div().flex().flex_wrap().gap(2.0).children((0..CARDS).map(|i| {
                // Each card makes its own state when it mounts, as a component does.
                let done = Signal::new(i.checked_rem(3).unwrap_or(0) == 0);
                card()
                    .w_px(220.0)
                    .child(
                        card_header()
                            .child(card_title(format!("Card {i}")))
                            .child(card_description("Mounted with its own state")),
                    )
                    .child(
                        card_content()
                            .flex()
                            .items_center()
                            .gap(2.0)
                            .child(checkbox(&done))
                            .child(label("Done"))
                            .child(progress(f32::from(
                                u16::try_from(i.checked_rem(100).unwrap_or(0)).unwrap_or(0),
                            ))),
                    )
            }))
        });
        div()
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .p(6.0)
            .flex_col()
            .gap(3.0)
            .child(
                div()
                    .flex()
                    .gap(2.0)
                    .child(
                        button(ButtonVariant::Default, ButtonSize::Sm)
                            .on_click(move |_| on.set(true))
                            .child("Mount"),
                    )
                    .child(
                        button(ButtonVariant::Outline, ButtonSize::Sm)
                            .on_click(move |_| off.set(false))
                            .child("Unmount"),
                    ),
            )
            .child(
                div()
                    .named("cards")
                    .flex_1()
                    .min_h(0.0)
                    .overflow_y_scroll()
                    .child(cards),
            )
            .into_element()
    })
}

// ── dashboard ────────────────────────────────────────────────────────────────

const CUSTOMERS: usize = 60;
const SECTIONS: [&str; 3] = ["Overview", "Customers", "Settings"];

struct Dashboard {
    section: Signal<&'static str>,
    tabs: TabsState,
    search: TextField,
    edit: DialogState,
    editing: Signal<usize>,
    name: TextField,
    email: TextField,
    vip: Signal<bool>,
    newsletter: Signal<bool>,
    email_reports: Signal<bool>,
    dark_charts: Signal<bool>,
    saved: Signal<u32>,
    toasts: Toaster,
}

fn customer(i: usize) -> (String, String, u32) {
    let (_, email, amount, _) = payment(i);
    let name = email.split('@').next().unwrap_or("").to_owned();
    (name, email, amount.saturating_mul(12))
}

fn kpi(title: &str, value: String, change: &str, progress_value: f32) -> Div {
    let t = theme();
    card()
        .flex_1()
        .child(
            card_header()
                .child(card_description(title.to_owned()))
                .child(card_title(value)),
        )
        .child(
            card_content()
                .flex_col()
                .gap(2.0)
                .child(text(change.to_owned()).text_xs().text_color(t.muted_foreground))
                .child(progress(progress_value)),
        )
}

fn overview(d: &Dashboard) -> Div {
    let t = theme();
    let bars = (0_u16..24).map(|h| {
        let v = f32::from(h.wrapping_mul(37).checked_rem(90).unwrap_or(0)) + 10.0;
        div()
            .flex_1()
            .h_px(v * 2.0)
            .rounded_sm()
            .bg(t.primary)
            .opacity(0.4 + v / 200.0)
    });
    div().flex_col().gap(4.0).child(
        tabs()
            .child(
                tabs_list()
                    .child(tabs_trigger(&d.tabs, "overview").child("Overview"))
                    .child(tabs_trigger(&d.tabs, "analytics").child("Analytics"))
                    .child(tabs_trigger(&d.tabs, "reports").child("Reports")),
            )
            .child(tabs_content(&d.tabs, "overview").map(|c| {
                c.child(
                    div()
                        .flex()
                        .gap(4.0)
                        .child(kpi(
                            "Total Revenue",
                            "$45,231.89".to_owned(),
                            "+20.1% from last month",
                            72.0,
                        ))
                        .child(kpi(
                            "Subscriptions",
                            "+2,350".to_owned(),
                            "+180.1% from last month",
                            55.0,
                        ))
                        .child(kpi("Sales", "+12,234".to_owned(), "+19% from last month", 64.0))
                        .child(kpi(
                            "Active Now",
                            format!("+{}", 573_u32.saturating_add(d.saved.get())),
                            "+201 since last hour",
                            31.0,
                        )),
                )
            }))
            .child(tabs_content(&d.tabs, "analytics").map(|c| {
                c.child(
                    card()
                        .child(card_header().child(card_title("Visitors per hour")))
                        .child(card_content().child(div().flex().items_end().gap(1.0).h_px(200.0).children(bars))),
                )
            }))
            .child(tabs_content(&d.tabs, "reports").map(|c| c.child(text("No reports yet.").text_sm()))),
    )
}

fn customers(d: &Dashboard) -> Div {
    let needle = d.search.text().to_lowercase();
    let tracks = [Track::Fr(1.2), Track::Fr(2.0), Track::Fr(1.0), Track::Fr(0.8)];
    let rows = (0..CUSTOMERS).filter_map(|i| {
        let (name, email, spent) = customer(i);
        if !name.contains(&needle) && !email.contains(&needle) {
            return None;
        }
        let (edit, editing, nf, ef) = (d.edit.clone(), d.editing.clone(), d.name.clone(), d.email.clone());
        let (n2, e2) = (name.clone(), email.clone());
        Some(
            table_row(&tracks, false)
                .key(key_of(&email))
                .child(table_cell(name))
                .child(table_cell(email))
                .child(table_cell(format!("${spent}")))
                .child(table_cell(
                    dialog_trigger(
                        &edit,
                        button(ButtonVariant::Ghost, ButtonSize::Xs).on_click(move |_| {
                            editing.set(i);
                            nf.set_text(n2.clone());
                            ef.set_text(e2.clone());
                        }),
                    )
                    .child("Edit"),
                )),
        )
    });
    div()
        .flex_col()
        .gap(3.0)
        .child(input(&d.search).max_w_px(320.0).placeholder("Search customers…"))
        .child(
            table()
                .child(table_header([table_row(&tracks, false)
                    .child(table_head("Name"))
                    .child(table_head("Email"))
                    .child(table_head("Spent"))
                    .child(table_head(Element::Empty))]))
                .child(table_body(rows)),
        )
}

fn settings(d: &Dashboard) -> Div {
    let row = |name: &'static str, on: &Signal<bool>| {
        div()
            .flex()
            .items_center()
            .justify_between()
            .w_px(420.0)
            .py(2.0)
            .child(label(name))
            .child(switch(on))
    };
    card()
        .w_px(480.0)
        .child(
            card_header()
                .child(card_title("Settings"))
                .child(card_description("What the app sends you")),
        )
        .child(
            card_content()
                .flex_col()
                .child(row("Email reports", &d.email_reports))
                .child(row("Dark charts", &d.dark_charts)),
        )
}

fn edit_dialog(d: &Dashboard) -> Element {
    let (save_toasts, saved, name, vip) = (d.toasts.clone(), d.saved.clone(), d.name.clone(), d.vip.clone());
    let edit = d.edit.clone();
    dialog_content(&d.edit, move |c| {
        c.max_w_px(460.0)
            .child(
                dialog_header()
                    .child(dialog_title("Edit customer"))
                    .child(dialog_description("Change the details and save.")),
            )
            .child(
                div()
                    .flex_col()
                    .gap(3.0)
                    .child(label("Customer name"))
                    .child(input(&name).aria_label("Customer name"))
                    .child(label("Email"))
                    .child(input(&edit_email(&edit)).aria_label("Email"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(2.0)
                            .child(checkbox(&vip))
                            .child(label("VIP")),
                    ),
            )
            .child(
                dialog_footer()
                    .child(dialog_close(
                        &edit,
                        button(ButtonVariant::Outline, ButtonSize::Default).child("Cancel"),
                    ))
                    .child(dialog_close(
                        &edit,
                        button(ButtonVariant::Default, ButtonSize::Default)
                            .on_click(move |_| {
                                saved.update(|n| n.saturating_add(1));
                                save_toasts.success(format!("Saved {}", name.text()));
                            })
                            .child("Save changes"),
                    )),
            )
    })
}

thread_local! {
    static EMAIL: TextField = TextField::new("");
}

/// The dialog's email field (one per thread, as the dashboard is).
fn edit_email(_state: &DialogState) -> TextField {
    EMAIL.with(Clone::clone)
}

fn dashboard() -> Render {
    let d = Dashboard {
        section: Signal::labeled("dashboard section", "Overview"),
        tabs: TabsState::new("overview"),
        search: TextField::new(""),
        edit: DialogState::new(),
        editing: Signal::labeled("editing customer", 0),
        name: TextField::new(""),
        email: EMAIL.with(Clone::clone),
        vip: Signal::labeled("vip", false),
        newsletter: Signal::labeled("newsletter", true),
        email_reports: Signal::labeled("email reports", false),
        dark_charts: Signal::labeled("dark charts", false),
        saved: Signal::labeled("saves", 0),
        toasts: Toaster::new(),
    };
    Box::new(move || {
        let t = theme();
        let current = d.section.get();
        let nav = SECTIONS.iter().map(|&s| {
            let section = d.section.clone();
            button(
                if s == current {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                },
                ButtonSize::Default,
            )
            .w_full()
            .justify_start()
            .on_click(move |_| section.set(s))
            .child(s)
        });
        let body = match current {
            "Customers" => customers(&d),
            "Settings" => settings(&d),
            _ => overview(&d),
        };
        let _ = d.newsletter.get();
        div()
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .flex()
            .child(
                div()
                    .named("nav")
                    .w_px(220.0)
                    .h_full()
                    .border_r()
                    .border_color(t.border)
                    .p(3.0)
                    .flex_col()
                    .gap(1.0)
                    .child(text("Acme Inc.").font_semibold().px(3.0).py(2.0))
                    .children(nav),
            )
            .child(
                div()
                    .named("main")
                    .flex_1()
                    .min_w(0.0)
                    .h_full()
                    .overflow_y_scroll()
                    .p(8.0)
                    .flex_col()
                    .gap(4.0)
                    .child(text(current).text_2xl().font_bold().tracking_tight())
                    .child(body),
            )
            .child(edit_dialog(&d))
            .child(toaster(&d.toasts))
            .into_element()
    })
}
