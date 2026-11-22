//! The stress scenes: each an app built to push one part of `gui` past what
//! a gallery page asks of it (many nodes, deep trees, many signals, a thread
//! flooding the UI, huge text, keyed churn), and one composite app that uses
//! many shadcn components together.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gui::list::{Columns, ListState, Selection, View, Width, virtual_list};
use gui::prelude::*;
use gui::testing::{RandomTree, Rng, Topology};
use gui::{Computed, Div, Effect, Element, Signal, TextField, Track, batch, component, div, key_of, mailbox, text};
use shadcn::{
    BadgeVariant, ButtonSize, ButtonVariant, badge, button, card, card_content, card_description, card_header,
    card_title, checkbox, dialog, dialog_close, dialog_content, dialog_description, dialog_footer, dialog_header,
    dialog_title, input, label, progress, switch, table, table_body, table_body_virtual, table_cell, table_head,
    table_header, table_row, table_row_on, tabs, tabs_content, tabs_list, tabs_trigger, textarea, theme, toast,
    toaster,
};

use crate::Render;
use shadcn::focus::StateLook;

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
    /// What its frames may allocate on the UI thread (ADR-0023).
    pub budget: Budget,
}

/// A scene's allocation budget, in allocations on the thread that builds
/// the frames (the layout pool, the GPU driver and the scene's own threads
/// aside): `--run` fails a scene over it, and CI runs `--run`. An action's
/// is about 1.25 times what `--run` measured; a steady frame's, which
/// allocates only the frame's fixed few since ADR-0013's per-node phases,
/// about 1.5 times (the GPU driver's calls on this thread vary more than
/// the UI's); lower it when a change allocates less.
pub struct Budget {
    /// A steady frame (nothing changed): the 95th percentile.
    pub steady: u64,
    /// One script step, every frame it draws: the most of any step.
    pub action: u64,
}

pub const ALL: &[Scene] = &[
    Scene {
        name: "list",
        about: "a virtual list of 10 000 000 rows of two heights: wheel, jump to the middle and the end",
        render: big_list,
        size: (1024, 768),
        script: &[
            "wheel 5000 #virtual-list",
            "click \"Go to middle\"",
            "wheel 3000 #virtual-list",
            "wheel -6000 #virtual-list",
            "click \"Go to end\"",
            "click \"Go to start\"",
        ],
        cycle: &[
            "click \"Go to middle\"",
            "wheel 3000 #virtual-list",
            "wheel -3000 #virtual-list",
            "click \"Go to start\"",
        ],
        budget: Budget {
            steady: 85,
            action: 7_500,
        },
    },
    Scene {
        name: "list1m",
        about: "a virtual list of 1 000 000 rows of two heights: wheel, jump to the middle and the end",
        render: million_list,
        size: (1024, 768),
        script: &[
            "wheel 5000 #virtual-list",
            "click \"Go to middle\"",
            "wheel -6000 #virtual-list",
            "click \"Go to end\"",
            "click \"Go to start\"",
        ],
        cycle: &[
            "click \"Go to middle\"",
            "wheel 3000 #virtual-list",
            "click \"Go to start\"",
        ],
        budget: Budget {
            steady: 85,
            action: 7_500,
        },
    },
    Scene {
        name: "random",
        about: "a seeded random tree of 3 000 nodes (gui::testing::RandomTree), edited 20 nodes at a time",
        render: random_tree,
        size: (1280, 800),
        script: &["click Edit", "click Edit", "wheel 2000 #random", "click Edit"],
        cycle: &["click Edit", "click Edit"],
        budget: Budget {
            steady: 95,
            action: 64_000,
        },
    },
    Scene {
        name: "bigtable",
        about: "a virtual table of 1 000 000 rows: sort and filter off the UI thread, select all, jump",
        render: big_table,
        size: (1280, 800),
        script: &[
            "wheel 4000 #table-body",
            "click \"Sort by amount\"",
            "wait 3000",
            "click \"Select all\"",
            "click \"Go to middle\"",
            "click \"Only failed\"",
            "wait 3000",
            "click \"Go to end\"",
            "wheel -3000 #table-body",
        ],
        cycle: &[
            "click \"Go to middle\"",
            "wheel 3000 #table-body",
            "click \"Go to start\"",
        ],
        budget: Budget {
            steady: 150,
            action: 33_000,
        },
    },
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
        budget: Budget {
            steady: 90,
            action: 1_230_000,
        },
    },
    Scene {
        name: "tree",
        about: "a file tree of 1 600 entries in folders four deep, and a chain of 120 nested boxes",
        render: file_tree,
        size: (1024, 768),
        script: &["click \"Expand all\"", "wheel 3000 #tree", "click \"Collapse all\""],
        cycle: &["click \"Expand all\"", "click \"Collapse all\""],
        budget: Budget {
            steady: 85,
            action: 22_500,
        },
    },
    Scene {
        name: "grid",
        about: "2 500 buttons with hover styles, each counting its own clicks",
        render: button_grid,
        size: (1280, 900),
        script: &["click \"#17\"", "click \"#17\"", "hover \"#900\"", "click Reset"],
        cycle: &["click \"#42\"", "hover \"#43\"", "click Reset"],
        budget: Budget {
            steady: 100,
            action: 28_500,
        },
    },
    Scene {
        name: "reactive",
        about: "5 000 signals, a chain of 2 000 computed values and 500 effects",
        render: reactive_graph,
        size: (960, 640),
        script: &["click \"Bump all\"", "click \"Bump first\"", "click \"Bump all\""],
        cycle: &["click \"Bump all\"", "click \"Bump first\""],
        budget: Budget {
            steady: 70,
            action: 600,
        },
    },
    Scene {
        name: "feed",
        about: "a thread sends 5 000 messages through a mailbox; the list keeps the last 200",
        render: feed,
        size: (960, 640),
        script: &["click Burst", "frame", "click Burst", "frame"],
        cycle: &["click Burst", "frame", "click Clear"],
        budget: Budget {
            steady: 90,
            action: 19_000,
        },
    },
    Scene {
        name: "toasts",
        about: "30 toasts at once, their timers, hover to expand, and their exits",
        render: toast_storm,
        size: (960, 640),
        script: &["click \"Burst 30\"", "hover \"Toast 30\"", "leave", "wait 6000", "rest"],
        cycle: &["click \"Burst 30\"", "wait 8000", "rest"],
        budget: Budget {
            steady: 75,
            action: 2_100,
        },
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
        budget: Budget {
            steady: 75,
            action: 3_600,
        },
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
        budget: Budget {
            steady: 90,
            action: 37_500,
        },
    },
    Scene {
        name: "churn",
        about: "300 cards mounted and unmounted, each with its own state",
        render: churn,
        size: (1280, 800),
        script: &["click Mount", "click Unmount", "click Mount"],
        cycle: &["click Unmount", "click Mount"],
        budget: Budget {
            steady: 90,
            action: 89_000,
        },
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
        budget: Budget {
            steady: 190,
            action: 25_000,
        },
    },
    Scene {
        name: "chart100m",
        about: "a line chart of 100 000 000 points at 4K: drag to pan, wheel to zoom, auto y (ADR-0018)",
        render: chart_100m,
        size: (3840, 2160),
        script: &[
            "rest",
            "wheel -240 #chart-plot",
            "drag 2000,1200 1900,1200",
            "drag 2000,1200 1900,1210",
            "drag 2000,1200 1880,1200",
            "drag 2000,1200 1900,1190",
            "drag 2000,1200 1950,1200",
            "drag 2000,1200 1900,1200",
            "wheel -240 #chart-plot",
            "drag 2000,1200 1900,1200",
            "drag 2000,1200 1900,1200",
            "wheel -480 #chart-plot",
            "drag 2000,1200 1900,1200",
            "drag 2000,1200 2100,1200",
            "wheel -480 #chart-plot",
            "drag 2000,1200 1900,1200",
            "wheel -960 #chart-plot",
            "drag 2000,1200 1900,1200",
            "drag 2000,1200 2300,1200",
            "wheel 960 #chart-plot",
            "drag 2000,1200 1900,1200",
            "wheel 960 #chart-plot",
            "drag 2000,1200 1700,1200",
            "wheel 480 #chart-plot",
            "wheel 480 #chart-plot",
            "drag 2000,1200 1900,1200",
            "wheel 240 #chart-plot",
            "drag 2000,1200 1500,1200",
            "drag 2000,1200 1900,1200",
            "click \"Zoom in\"",
            "click Pan",
            "click \"Zoom out\"",
            "hover #chart-plot",
        ],
        cycle: &[
            "drag 2000,1200 1900,1200",
            "wheel -240 #chart-plot",
            "drag 2000,1200 2100,1200",
            "wheel 240 #chart-plot",
        ],
        budget: Budget {
            steady: 130,
            action: 800,
        },
    },
    Scene {
        name: "stream",
        about: "a line chart of the last 10 000 000 points fed 1 000 000 a second, the oldest dropped (ADR-0018)",
        render: stream,
        size: (1920, 1080),
        script: &[
            "wait 16", "wait 16", "wait 16", "wait 16", "wait 16", "wait 16", "wait 16", "wait 16", "wait 16",
            "wait 16", "wait 16", "wait 16", "wait 16", "wait 16", "wait 16", "wait 16", "wait 16", "wait 16",
            "wait 16", "wait 16", "wait 16", "wait 16", "wait 16", "wait 16", "wait 16", "wait 16", "wait 16",
            "wait 16", "wait 16", "wait 16", "wait 16", "wait 16",
        ],
        cycle: &["wait 16", "wait 16", "wait 16", "wait 16"],
        budget: Budget {
            steady: 70,
            action: 800,
        },
    },
    Scene {
        name: "canvas",
        about: "a canvas of 100 000 paths: drag a card among them, pan, zoom in fast through tiles, zoom back out (ADR-0017)",
        render: canvas_100k,
        size: (1920, 1080),
        script: &[
            // Raster done first: the drag's frames are the drag's alone.
            "rest",
            // The card dragged live over the tiles: 60 frames of moves.
            "section drag",
            "down 985,555",
            "hover 991,557",
            "hover 997,559",
            "hover 1003,561",
            "hover 1009,563",
            "hover 1015,565",
            "hover 1021,567",
            "hover 1027,569",
            "hover 1033,571",
            "hover 1039,573",
            "hover 1045,575",
            "hover 1051,577",
            "hover 1057,579",
            "hover 1063,581",
            "hover 1069,583",
            "hover 1075,585",
            "hover 1081,587",
            "hover 1087,589",
            "hover 1093,591",
            "hover 1099,593",
            "hover 1105,595",
            "hover 1111,597",
            "hover 1117,599",
            "hover 1123,601",
            "hover 1129,603",
            "hover 1135,605",
            "hover 1141,607",
            "hover 1147,609",
            "hover 1153,611",
            "hover 1159,613",
            "hover 1165,615",
            "hover 1159,613",
            "hover 1153,611",
            "hover 1147,609",
            "hover 1141,607",
            "hover 1135,605",
            "hover 1129,603",
            "hover 1123,601",
            "hover 1117,599",
            "hover 1111,597",
            "hover 1105,595",
            "hover 1099,593",
            "hover 1093,591",
            "hover 1087,589",
            "hover 1081,587",
            "hover 1075,585",
            "hover 1069,583",
            "hover 1063,581",
            "hover 1057,579",
            "hover 1051,577",
            "hover 1045,575",
            "hover 1039,573",
            "hover 1033,571",
            "hover 1027,569",
            "hover 1021,567",
            "hover 1015,565",
            "hover 1009,563",
            "hover 1003,561",
            "hover 997,559",
            "hover 991,557",
            "hover 985,555",
            "up 985,555",
            "section end",
            "wait 200",
            "drag 900,500 700,400",
            "wheel -240 #canvas",
            "wheel -240 #canvas",
            "wheel -240 #canvas",
            "wheel -240 #canvas",
            "wheel -240 #canvas",
            "wheel -240 #canvas",
            "wheel -240 #canvas",
            "wheel -240 #canvas",
            "wheel -240 #canvas",
            "wheel -240 #canvas",
            "wheel -240 #canvas",
            "wheel -240 #canvas",
            "wait 200",
            "drag 900,500 1100,600",
            "wheel 480 #canvas",
            "wheel 480 #canvas",
            "wheel 480 #canvas",
            "wheel 480 #canvas",
            "wheel 480 #canvas",
            "wheel 480 #canvas",
            "wait 200",
        ],
        cycle: &[
            "wheel -480 #canvas",
            "wheel -480 #canvas",
            "wheel 960 #canvas",
            "wait 200",
        ],
        budget: Budget {
            steady: 70,
            action: 340_000,
        },
    },
];

// ── list ─────────────────────────────────────────────────────────────────────

const LIST_ROWS: u64 = 10_000_000;
const BIG_ROWS: u32 = 1_000_000;

/// Ten million rows.
fn big_list() -> Render {
    list_of(LIST_ROWS)
}

/// A million rows.
fn million_list() -> Render {
    list_of(1_000_000)
}

/// `count` rows, one in seven two lines tall, none stored: each built from
/// its number when it scrolls into view.
fn list_of(count: u64) -> Render {
    let state = ListState::new(36.0);
    Box::new(move || {
        let t = theme();
        let jump = |label: &'static str, to: f64| {
            let s = state.clone();
            button(ButtonVariant::Outline, ButtonSize::Sm)
                .on_click(move |_| s.scroll_to_fraction(to))
                .child(label)
        };
        let rows = virtual_list(
            &state,
            count,
            |i| i,
            |i| {
                let tall = i.is_multiple_of(7);
                div()
                    .padding_x(16.0)
                    .padding_y(8.0)
                    .border_bottom(1.0)
                    .border_color(t.border)
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .child(text(format!("Row {i}")).font_weight(500))
                    .when(tall, |d| {
                        d.child(
                            text(format!("{i} is a multiple of seven, so this row has a second line"))
                                .text_size(14.0)
                                .line_height(20.0)
                                .text_color(t.muted_foreground),
                        )
                    })
                    .into_element()
            },
        );
        div()
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(24.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(
                div()
                    .display(Display::Flex)
                    .gap(8.0)
                    .child(jump("Go to start", 0.0))
                    .child(jump("Go to middle", 0.5))
                    .child(jump("Go to end", 1.0)),
            )
            .child(
                rows.flex(Flex::grow(1.0))
                    .min_height(0.0)
                    .border(1.0)
                    .border_color(t.border)
                    .radius(8.0),
            )
            .into_element()
    })
}

/// One row of the million: what is sorted and filtered on.
#[derive(Clone, Copy)]
struct Pay {
    amount: u32,
    status: u8,
}

/// A million payments: a [`View`] of them sorted and filtered on a thread
/// of its own, a [`Selection`] in data rows, columns on a [`Columns`] model.
fn big_table() -> Render {
    let data = gui::Collection::new((0..BIG_ROWS).map(|i| {
        let (_, _, amount, status) = payment(usize::try_from(i).unwrap_or(0));
        Pay {
            amount,
            status: u8::try_from(STATUSES.iter().position(|s| *s == status).unwrap_or(0)).unwrap_or(0),
        }
    }));
    let view = Signal::labeled("table view", View::identity(BIG_ROWS));
    let sorting = Signal::labeled("sorting", false);
    let list = ListState::new(37.0);
    let to_top = list.clone();
    let deliver = mailbox(move |v: View| {
        view.set(v);
        sorting.set(false);
        to_top.scroll_to_row(0);
    });
    let selection = Signal::labeled("selection", Selection::new());
    let desc = Signal::labeled("amount sort", None::<bool>);
    let only_failed = Signal::labeled("only failed", false);
    let columns = Columns::new([
        Width::Px(40.0),
        Width::AUTO,
        Width::Fr(2.4),
        Width::Fr(1.0),
        Width::Fr(1.0),
    ]);
    let rebuild = Rc::new(move || {
        sorting.set(true);
        let failed = u8::try_from(STATUSES.iter().position(|s| *s == "failed").unwrap_or(0)).unwrap_or(0);
        let only = only_failed.get_untracked();
        let order = desc.get_untracked().map(|d| {
            move |a: &Pay, b: &Pay| {
                if d {
                    b.amount.cmp(&a.amount)
                } else {
                    a.amount.cmp(&b.amount)
                }
            }
        });
        View::spawn(
            data.snapshot(),
            move |p: &Pay| !only || p.status == failed,
            order,
            deliver.clone(),
        );
    });
    Box::new(move || {
        let t = theme();
        let shown = view.with(View::len);
        let sort = Rc::clone(&rebuild);
        let filter = Rc::clone(&rebuild);
        let jump = |label: &'static str, to: f64| {
            let s = list.clone();
            button(ButtonVariant::Outline, ButtonSize::Sm)
                .on_click(move |_| s.scroll_to_fraction(to))
                .child(label)
        };
        let picked = selection.with(|s| s.count(u64::from(BIG_ROWS)));
        let head = table_row_on(
            &columns,
            false,
            [
                table_head(Element::Empty),
                table_head("ID"),
                table_head("Email"),
                table_head("Amount"),
                table_head("Status"),
            ],
        );
        let cols = columns.clone();
        let body = table_body_virtual(
            &list,
            u64::try_from(shown).unwrap_or(0),
            move |p| {
                u64::from(
                    view.with(|v| v.row(usize::try_from(p).unwrap_or(usize::MAX)))
                        .unwrap_or(0),
                )
            },
            move |p| {
                let row = view
                    .with(|v| v.row(usize::try_from(p).unwrap_or(usize::MAX)))
                    .unwrap_or(0);
                let (id, email, amount, status) = payment(usize::try_from(row).unwrap_or(0));
                let on = selection.with(|s| s.contains(u64::from(row)));
                table_row_on(
                    &cols,
                    on,
                    [
                        table_cell(checkbox().checked(on).on_change(move |_| {
                            selection.update(|s| s.toggle(u64::from(row)));
                        })),
                        table_cell(id).font_family(FontFamily::GEIST_MONO),
                        table_cell(email),
                        table_cell(format!("${amount}.00")),
                        table_cell(status_badge(status)),
                    ],
                )
            },
        );
        div()
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(24.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(
                div()
                    .display(Display::Flex)
                    .align_items(Align::Center)
                    .gap(8.0)
                    .child(
                        button(ButtonVariant::Outline, ButtonSize::Sm)
                            .on_click(move |_| {
                                desc.update(|d| *d = Some(*d != Some(true)));
                                sort();
                            })
                            .child("Sort by amount"),
                    )
                    .child(
                        button(ButtonVariant::Outline, ButtonSize::Sm)
                            .on_click(move |_| {
                                only_failed.update(|o| *o = !*o);
                                filter();
                            })
                            .child("Only failed"),
                    )
                    .child(
                        button(ButtonVariant::Outline, ButtonSize::Sm)
                            .on_click(move |_| selection.update(Selection::select_all))
                            .child("Select all"),
                    )
                    .child(
                        button(ButtonVariant::Outline, ButtonSize::Sm)
                            .on_click(move |_| selection.update(Selection::clear))
                            .child("Clear"),
                    )
                    .child(jump("Go to start", 0.0))
                    .child(jump("Go to middle", 0.5))
                    .child(jump("Go to end", 1.0))
                    .child(
                        text(format!(
                            "{shown} shown, {picked} selected{}",
                            if sorting.get() { ", sorting…" } else { "" }
                        ))
                        .text_size(14.0)
                        .line_height(20.0)
                        .text_color(t.muted_foreground),
                    ),
            )
            .child(
                table()
                    .flex(Flex::grow(1.0))
                    .min_height(0.0)
                    .child(table_header([head]))
                    .child(body.flex(Flex::grow(1.0)).min_height(0.0)),
            )
            .into_element()
    })
}

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
        let sort = sort_desc;
        let head = table_row(&tracks, false)
            .child(table_head(Element::Empty))
            .child(table_head("ID"))
            .child(table_head("Email"))
            .child(table_head(
                button(ButtonVariant::Ghost, ButtonSize::Sm)
                    .on_click(move |_| sort.update(|s| *s = Some(*s != Some(true))))
                    .child("Amount"),
            ))
            .child(table_head("Status"));
        let body = rows.iter().filter_map(|&i| {
            let (id, email, amount, status) = data.get(i)?;
            let picked = selected;
            Some(
                table_row(&tracks, everything)
                    .key(key_of(id))
                    .child(table_cell(checkbox().checked(everything).on_change(move |on| {
                        picked.update(|n| *n = if on { n.saturating_add(1) } else { n.saturating_sub(1) });
                    })))
                    .child(table_cell(id.clone()).font_family(FontFamily::GEIST_MONO))
                    .child(table_cell(email.clone()))
                    .child(table_cell(format!("${amount}.00")))
                    .child(table_cell(status_badge(status))),
            )
        });
        let toggle = all;
        div()
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(24.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(
                div()
                    .display(Display::Flex)
                    .align_items(Align::Center)
                    .gap(16.0)
                    .child(input().field(filter).max_width(320.0).placeholder("Filter emails…"))
                    .child(
                        div()
                            .display(Display::Flex)
                            .align_items(Align::Center)
                            .gap(8.0)
                            .child(checkbox().checked(everything).on_change(move |on| toggle.set(on)))
                            .child(label("Select all")),
                    )
                    .child(
                        text(format!(
                            "{} of {} rows, {} picked",
                            rows.len(),
                            data.len(),
                            selected.get()
                        ))
                        .text_size(14.0)
                        .line_height(20.0)
                        .text_color(t.muted_foreground),
                    ),
            )
            .child(
                div()
                    .named("rows")
                    .flex(Flex::grow(1.0))
                    .min_height(0.0)
                    .overflow_y(Overflow::Scroll)
                    .overflow_x(Overflow::Hidden)
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
    every.push(open);
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
    let toggle = f.open;
    // 4 px per level.
    let indent = 4.0 * f32::from(u16::try_from(depth).unwrap_or(u16::MAX));
    let row = div()
        .display(Display::Flex)
        .align_items(Align::Center)
        .gap(8.0)
        .padding_left(indent)
        .padding_y(2.0)
        .radius(8.0)
        .cursor(Cursor::Pointer)
        .with_interaction(|d, i| d.background_while(move || i.hovered(), t.accent))
        .on_click(move |_| toggle.update(|o| *o = !*o))
        .child(text(if open { "▾" } else { "▸" }).text_size(12.0).line_height(16.0))
        .child(
            text(f.name.rsplit('/').next().unwrap_or(&f.name).to_owned())
                .text_size(14.0)
                .line_height(20.0),
        );
    let mut node = div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .child(row);
    if open {
        for sub in &f.folders {
            node = node.child(folder_view(sub, depth.saturating_add(1)));
        }
        for file in &f.files {
            node = node.child(
                text(file.rsplit('/').next().unwrap_or(file).to_owned())
                    .text_size(14.0)
                    .line_height(20.0)
                    .text_color(t.muted_foreground)
                    .padding_left(indent + 20.0),
            );
        }
    }
    node.into_element()
}

/// Boxes nested `depth` deep, each a pixel inside the last.
fn chain(depth: usize) -> Element {
    let t = theme();
    let mut inner = div().width(8.0).height(8.0).background(t.primary).into_element();
    for _ in 0..depth {
        inner = div()
            .padding(1.0)
            .border(1.0)
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
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(24.0)
            .display(Display::Flex)
            .gap(24.0)
            .child(
                div()
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .gap(8.0)
                    .width(420.0)
                    .child(
                        div()
                            .display(Display::Flex)
                            .gap(8.0)
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
                            .text_size(12.0)
                            .line_height(16.0)
                            .text_color(t.muted_foreground),
                    )
                    .child(
                        div()
                            .named("tree")
                            .flex(Flex::grow(1.0))
                            .min_height(0.0)
                            .overflow_y(Overflow::Scroll)
                            .overflow_x(Overflow::Hidden)
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
        let reset = (Rc::clone(&counts), total);
        // Each button is an instance: a click renders that one alone.
        let cells = counts.iter().enumerate().map(|(i, c)| grid_button(i, *c, total));
        div()
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(16.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(12.0)
            .child(
                div()
                    .display(Display::Flex)
                    .align_items(Align::Center)
                    .gap(12.0)
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
                    .child(component((), move |_| {
                        move || {
                            text(format!("{} clicks", total.get()))
                                .text_size(14.0)
                                .line_height(20.0)
                                .into_element()
                        }
                    })),
            )
            .child(
                div()
                    .named("grid")
                    .flex(Flex::grow(1.0))
                    .min_height(0.0)
                    .overflow_y(Overflow::Scroll)
                    .overflow_x(Overflow::Hidden)
                    .child(div().display(Display::Flex).flex_wrap(true).gap(4.0).children(cells)),
            )
            .into_element()
    })
}

fn grid_button(i: usize, count: Signal<u32>, all: Signal<u32>) -> Element {
    component((i, count), move |cx: gui::Ctx<(usize, Signal<u32>)>| {
        move || {
            let (i, count) = cx.props().get();
            let n = count.get();
            button(
                if n > 0 {
                    ButtonVariant::Default
                } else {
                    ButtonVariant::Outline
                },
                ButtonSize::Xs,
            )
            .width(44.0)
            .on_click(move |_| {
                count.update(|n| *n = n.saturating_add(1));
                all.update(|n| *n = n.saturating_add(1));
            })
            .child(if n > 0 { format!("{n}") } else { format!("#{i}") })
            .into_element()
        }
    })
    .into_element()
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
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(32.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(
                div()
                    .display(Display::Flex)
                    .gap(8.0)
                    .child(
                        button(ButtonVariant::Default, ButtonSize::Default)
                            .on_click(move |_| {
                                batch(|| all.signals.iter().for_each(|s| s.update(|v| *v = v.saturating_add(1))))
                            })
                            .child("Bump all"),
                    )
                    .child(
                        button(ButtonVariant::Outline, ButtonSize::Default)
                            .on_click(move |_| {
                                if let Some(s) = first.signals.first() {
                                    s.update(|v| *v = v.saturating_add(1));
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
    let (l, r) = (lines, received);
    let to_ui = mailbox(move |msg: String| {
        r.update(|n| *n = n.saturating_add(1));
        l.update(|all| {
            all.push(msg);
            if all.len() > KEEP {
                let over = all.len().saturating_sub(KEEP);
                all.drain(..over);
            }
        });
    });
    let burst = Rc::new(Cell::new(0_usize));
    Box::new(move || {
        let t = theme();
        let (send, n) = (to_ui.clone(), Rc::clone(&burst));
        let (clear_lines, clear_count) = (lines, received);
        let shown = lines.get();
        div()
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(24.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(12.0)
            .child(
                div()
                    .display(Display::Flex)
                    .gap(8.0)
                    .align_items(Align::Center)
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
                    .child(
                        text(format!("{} received", received.get()))
                            .text_size(14.0)
                            .line_height(20.0),
                    ),
            )
            .child(
                div()
                    .named("feed")
                    .flex(Flex::grow(1.0))
                    .min_height(0.0)
                    .overflow_y(Overflow::Scroll)
                    .overflow_x(Overflow::Hidden)
                    .font_family(FontFamily::GEIST_MONO)
                    .text_size(12.0)
                    .line_height(16.0)
                    .children(shown.into_iter().map(text)),
            )
            .into_element()
    })
}

/// How long a feed burst takes to arrive, at most, in a headless run.
pub const FEED_SETTLE: Duration = Duration::from_millis(500);

// ── toasts ───────────────────────────────────────────────────────────────────

fn toast_storm() -> Render {
    let next = Rc::new(Cell::new(0_u32));
    Box::new(move || {
        let t = theme();
        let n = Rc::clone(&next);
        div()
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(32.0)
            .child(
                button(ButtonVariant::Outline, ButtonSize::Default)
                    .on_click(move |_| {
                        for _ in 0..30 {
                            let i = n.get().saturating_add(1);
                            n.set(i);
                            match i.checked_rem(4).unwrap_or(0) {
                                0 => toast::success(format!("Toast {i}")),
                                1 => toast::error(format!("Toast {i}")),
                                2 => toast::info(format!("Toast {i}")),
                                _ => toast(format!("Toast {i}")),
                            };
                        }
                    })
                    .child("Burst 30"),
            )
            .child(toaster())
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
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(24.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(12.0)
            .child(input().field(field).placeholder("Big input"))
            .child(textarea().field(notes).height(120.0))
            .child(
                div()
                    .named("prose")
                    .flex(Flex::grow(1.0))
                    .min_height(0.0)
                    .overflow_y(Overflow::Scroll)
                    .overflow_x(Overflow::Hidden)
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .gap(12.0)
                    .child(text(word.clone()).text_size(14.0).line_height(20.0))
                    .children(prose.iter().map(|p| text(p.clone()).text_size(14.0).line_height(20.0))),
            )
            .into_element()
    })
}

// ── random ───────────────────────────────────────────────────────────────────

const RANDOM_SEED: u64 = 1;
const RANDOM_NODES: usize = 3_000;
const RANDOM_EDITS: usize = 20;

/// A seeded random element tree, the same every run (ADR-0023): "Edit"
/// makes 20 seeded edits (add, remove, restyle, retext).
fn random_tree() -> Render {
    let tree = Signal::new(RandomTree::new(RANDOM_SEED, RANDOM_NODES, Topology::Mixed));
    let rng = Rc::new(RefCell::new(Rng::new(RANDOM_SEED)));
    Box::new(move || {
        let t = theme();
        let r = Rc::clone(&rng);
        div()
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(24.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(12.0)
            .child(
                button(ButtonVariant::Outline, ButtonSize::Sm)
                    .on_click(move |_| {
                        tree.update(|tree| {
                            let mut rng = r.borrow_mut();
                            for _ in 0..RANDOM_EDITS {
                                tree.mutate(&mut rng);
                            }
                        });
                    })
                    .child("Edit"),
            )
            .child(
                div()
                    .named("random")
                    .flex(Flex::grow(1.0))
                    .min_height(0.0)
                    .overflow_y(Overflow::Scroll)
                    .overflow_x(Overflow::Hidden)
                    .child(tree.with(RandomTree::element)),
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
        let (sh, rev, ins, rem) = (items, items, items, items);
        let (n, s) = (Rc::clone(&next), Rc::clone(&seed));
        let rows = items.get().into_iter().map(|i| {
            div()
                .key(key_of(&format!("item-{i}")))
                .display(Display::Flex)
                .justify_content(Justify::Between)
                .padding_x(12.0)
                .padding_y(4.0)
                .border_bottom(1.0)
                .border_color(t.border)
                .child(text(format!("Item {i}")).text_size(14.0).line_height(20.0))
                .child(badge(BadgeVariant::Secondary).child(format!("{}", i.checked_rem(7).unwrap_or(0))))
        });
        let action = |name: &'static str, f: Box<dyn Fn()>| {
            button(ButtonVariant::Outline, ButtonSize::Sm)
                .on_click(move |_| f())
                .child(name)
        };
        div()
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(24.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(12.0)
            .child(
                div()
                    .display(Display::Flex)
                    .gap(8.0)
                    .child(action(
                        "Shuffle",
                        Box::new(move || {
                            sh.update(|v| {
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
                            });
                        }),
                    ))
                    .child(action(
                        "Reverse",
                        Box::new(move || {
                            rev.update(|v| {
                                v.reverse();
                            });
                        }),
                    ))
                    .child(action(
                        "Insert",
                        Box::new(move || {
                            let i = n.get();
                            n.set(i.saturating_add(1));
                            ins.update(|v| {
                                let at = v.len().checked_div(2).unwrap_or(0);
                                v.insert(at, i);
                            });
                        }),
                    ))
                    .child(action(
                        "Remove",
                        Box::new(move || {
                            rem.update(|v| {
                                if !v.is_empty() {
                                    v.remove(v.len().checked_div(2).unwrap_or(0));
                                }
                            });
                        }),
                    )),
            )
            .child(
                div()
                    .named("items")
                    .flex(Flex::grow(1.0))
                    .min_height(0.0)
                    .overflow_y(Overflow::Scroll)
                    .overflow_x(Overflow::Hidden)
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
        let (on, off) = (mounted, mounted);
        let cards = mounted.get().then(|| {
            div()
                .display(Display::Flex)
                .flex_wrap(true)
                .gap(8.0)
                .children((0..CARDS).map(churn_card))
        });
        div()
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(24.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(12.0)
            .child(
                div()
                    .display(Display::Flex)
                    .gap(8.0)
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
                    .flex(Flex::grow(1.0))
                    .min_height(0.0)
                    .overflow_y(Overflow::Scroll)
                    .overflow_x(Overflow::Hidden)
                    .child(cards),
            )
            .into_element()
    })
}

/// One card: an instance that makes its own state in setup when it mounts,
/// so a render (warm or cold) never makes a signal.
fn churn_card(i: usize) -> Element {
    component(i, move |cx: gui::Ctx<usize>| {
        let done = Signal::new(cx.props().get_untracked().checked_rem(3).unwrap_or(0) == 0);
        move || {
            let i = cx.props().get();
            card()
                .width(220.0)
                .child(
                    card_header()
                        .child(card_title(format!("Card {i}")))
                        .child(card_description("Mounted with its own state")),
                )
                .child(
                    card_content()
                        .display(Display::Flex)
                        .align_items(Align::Center)
                        .gap(8.0)
                        .child(checkbox().model(done))
                        .child(label("Done"))
                        .child(progress(f32::from(
                            u16::try_from(i.checked_rem(100).unwrap_or(0)).unwrap_or(0),
                        ))),
                )
                .into_element()
        }
    })
    .into_element()
}

// ── dashboard ────────────────────────────────────────────────────────────────

const CUSTOMERS: usize = 60;
const SECTIONS: [&str; 3] = ["Overview", "Customers", "Settings"];

struct Dashboard {
    section: Signal<&'static str>,
    search: TextField,
    edit: Signal<bool>,
    editing: Signal<usize>,
    name: TextField,
    email: TextField,
    vip: Signal<bool>,
    newsletter: Signal<bool>,
    email_reports: Signal<bool>,
    dark_charts: Signal<bool>,
    saved: Signal<u32>,
}

fn customer(i: usize) -> (String, String, u32) {
    let (_, email, amount, _) = payment(i);
    let name = email.split('@').next().unwrap_or("").to_owned();
    (name, email, amount.saturating_mul(12))
}

fn kpi(title: &str, value: String, change: &str, progress_value: f32) -> Div {
    let t = theme();
    card()
        .flex(Flex::grow(1.0))
        .child(
            card_header()
                .child(card_description(title.to_owned()))
                .child(card_title(value)),
        )
        .child(
            card_content()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(8.0)
                .child(
                    text(change.to_owned())
                        .text_size(12.0)
                        .line_height(16.0)
                        .text_color(t.muted_foreground),
                )
                .child(progress(progress_value)),
        )
}

fn overview(d: &Dashboard) -> Div {
    let t = theme();
    let bars = (0_u16..24).map(|h| {
        let v = f32::from(h.wrapping_mul(37).checked_rem(90).unwrap_or(0)) + 10.0;
        div()
            .flex(Flex::grow(1.0))
            .height(v * 2.0)
            .radius(6.0)
            .background(t.primary)
            .opacity(0.4 + v / 200.0)
    });
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(16.0)
        .child(
            tabs()
                .default_value("overview")
                .child(
                    tabs_list()
                        .child(tabs_trigger("overview").child("Overview"))
                        .child(tabs_trigger("analytics").child("Analytics"))
                        .child(tabs_trigger("reports").child("Reports")),
                )
                .child(
                    tabs_content("overview").child(
                        div()
                            .display(Display::Flex)
                            .gap(16.0)
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
                    ),
                )
                .child(
                    tabs_content("analytics").child(
                        card()
                            .child(card_header().child(card_title("Visitors per hour")))
                            .child(
                                card_content().child(
                                    div()
                                        .display(Display::Flex)
                                        .align_items(Align::End)
                                        .gap(4.0)
                                        .height(200.0)
                                        .children(bars),
                                ),
                            ),
                    ),
                )
                .child(tabs_content("reports").child(text("No reports yet.").text_size(14.0).line_height(20.0))),
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
        let (edit, editing, nf, ef) = (d.edit, d.editing, d.name, d.email);
        let (n2, e2) = (name.clone(), email.clone());
        Some(
            table_row(&tracks, false)
                .key(key_of(&email))
                .child(table_cell(name))
                .child(table_cell(email))
                .child(table_cell(format!("${spent}")))
                .child(table_cell(
                    button(ButtonVariant::Ghost, ButtonSize::Xs)
                        .on_click(move |_| {
                            editing.set(i);
                            nf.set_text(n2.clone());
                            ef.set_text(e2.clone());
                            edit.set(true);
                        })
                        .child("Edit"),
                )),
        )
    });
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(12.0)
        .child(
            input()
                .field(d.search)
                .max_width(320.0)
                .placeholder("Search customers…"),
        )
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
            .display(Display::Flex)
            .align_items(Align::Center)
            .justify_content(Justify::Between)
            .width(420.0)
            .padding_y(8.0)
            .child(label(name))
            .child(switch().model(*on))
    };
    card()
        .width(480.0)
        .child(
            card_header()
                .child(card_title("Settings"))
                .child(card_description("What the app sends you")),
        )
        .child(
            card_content()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .child(row("Email reports", &d.email_reports))
                .child(row("Dark charts", &d.dark_charts)),
        )
}

fn edit_dialog(d: &Dashboard) -> Element {
    let (saved, name, vip) = (d.saved, d.name, d.vip);
    // One dialog for every row: each row's button opens it through the model.
    dialog()
        .open(d.edit)
        .child(
            dialog_content()
                .max_width(460.0)
                .child(
                    dialog_header()
                        .child(dialog_title("Edit customer"))
                        .child(dialog_description("Change the details and save.")),
                )
                .child(
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(12.0)
                        .child(label("Customer name"))
                        .child(input().field(name).map(|i| i.aria_label("Customer name")))
                        .child(label("Email"))
                        .child(input().field(edit_email()).map(|i| i.aria_label("Email")))
                        .child(
                            div()
                                .display(Display::Flex)
                                .align_items(Align::Center)
                                .gap(8.0)
                                .child(checkbox().model(vip))
                                .child(label("VIP")),
                        ),
                )
                .child(
                    dialog_footer()
                        .child(dialog_close(
                            button(ButtonVariant::Outline, ButtonSize::Default).child("Cancel"),
                        ))
                        .child(dialog_close(
                            button(ButtonVariant::Default, ButtonSize::Default)
                                .on_click(move |_| {
                                    saved.update(|n| *n = n.saturating_add(1));
                                    toast::success(format!("Saved {}", name.text()));
                                })
                                .child("Save changes"),
                        )),
                ),
        )
        .into_element()
}

thread_local! {
    static EMAIL: TextField = gui::detached(|| TextField::new(""));
}

/// The dialog's email field (one per thread, as the dashboard is).
fn edit_email() -> TextField {
    EMAIL.with(Clone::clone)
}

fn dashboard() -> Render {
    let d = Dashboard {
        section: Signal::labeled("dashboard section", "Overview"),
        search: TextField::new(""),
        edit: Signal::labeled("edit dialog", false),
        editing: Signal::labeled("editing customer", 0),
        name: TextField::new(""),
        email: EMAIL.with(Clone::clone),
        vip: Signal::labeled("vip", false),
        newsletter: Signal::labeled("newsletter", true),
        email_reports: Signal::labeled("email reports", false),
        dark_charts: Signal::labeled("dark charts", false),
        saved: Signal::labeled("saves", 0),
    };
    Box::new(move || {
        let t = theme();
        let current = d.section.get();
        let nav = SECTIONS.iter().map(|&s| {
            let section = d.section;
            button(
                if s == current {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                },
                ButtonSize::Default,
            )
            .width(pct(100.0))
            .justify_content(Justify::Start)
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
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .display(Display::Flex)
            .child(
                div()
                    .named("nav")
                    .width(220.0)
                    .height(pct(100.0))
                    .border_right(1.0)
                    .border_color(t.border)
                    .padding(12.0)
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .gap(4.0)
                    .child(text("Acme Inc.").font_weight(600).padding_x(12.0).padding_y(8.0))
                    .children(nav),
            )
            .child(
                div()
                    .named("main")
                    .flex(Flex::grow(1.0))
                    .min_width(0.0)
                    .height(pct(100.0))
                    .overflow_y(Overflow::Scroll)
                    .overflow_x(Overflow::Hidden)
                    .padding(32.0)
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .gap(16.0)
                    .child(
                        text(current)
                            .text_size(24.0)
                            .line_height(32.0)
                            .font_weight(700)
                            .tracking(-0.025),
                    )
                    .child(body),
            )
            .child(edit_dialog(&d))
            .child(toaster())
            .into_element()
    })
}

// ── chart100m ────────────────────────────────────────────────────────────────

const CHART_POINTS: i64 = 100_000_000;

#[expect(
    clippy::as_conversions,
    reason = "a point's index as a position on its curve, exact below 2^53"
)]
fn index_f64(i: i64) -> f64 {
    i as f64
}

#[expect(
    clippy::as_conversions,
    reason = "a position on the series to a point's index, saturating"
)]
fn f64_index(v: f64) -> i64 {
    v.round() as i64
}

/// 100 000 000 points of a random walk over a slow sine, appended a page
/// at a time.
fn chart_100m() -> Render {
    use gui::chunk::plot::{YAuto, line, plot, series};
    let s = series();
    let mut rng = Rng::new(11);
    let mut y = 0.0_f64;
    let page = 1_i64 << 22;
    let mut at = 0_i64;
    while at < CHART_POINTS {
        let end = at.saturating_add(page).min(CHART_POINTS);
        let xs: Vec<i64> = (at..end).collect();
        let ys: Vec<f64> = xs
            .iter()
            .map(|x| {
                y += f64::from(rng.unit()) - 0.5;
                (index_f64(*x) * 1e-6).sin().mul_add(300.0, y)
            })
            .collect();
        s.append(&xs, &ys);
        at = end;
    }
    let view = Signal::labeled("chart view", (0_i64, CHART_POINTS));
    // Pan by a fiftieth of the view, back to the start past the end.
    let pan = move || {
        view.update(|(a, b)| {
            let span = b.saturating_sub(*a);
            let step = span / 50;
            if b.saturating_add(step) > CHART_POINTS {
                *a = 0;
            } else {
                *a = a.saturating_add(step);
            }
            *b = a.saturating_add(span);
        });
    };
    // Zoom by 4 about the middle, in or out, within the series.
    let zoom = move |out: bool| {
        view.update(|(a, b)| {
            let span = b.saturating_sub(*a);
            let half = if out { span.saturating_mul(2) } else { span / 8 }.clamp(1_000, CHART_POINTS / 2);
            let mid = (*a / 2).saturating_add(*b / 2);
            *a = mid
                .saturating_sub(half)
                .clamp(0, CHART_POINTS.saturating_sub(half.saturating_mul(2)));
            *b = a.saturating_add(half.saturating_mul(2));
        });
    };
    // Dragging pans by the pixels moved; the wheel zooms by 2 a notch
    // about the pointer, as a chart's own gestures do.
    let last = Rc::new(Cell::new(None::<f32>));
    let drag_to = Rc::new({
        move |x: f32, width: f32, phase: gui::DragPhase| {
            if let Some(from) = last.get() {
                let px = f64::from(from - x);
                view.update(|(a, b)| {
                    let span = b.saturating_sub(*a);
                    let k = index_f64(span) / f64::from(width.max(1.0));
                    let shift = f64_index(px * k);
                    *a = a.saturating_add(shift).clamp(0, CHART_POINTS.saturating_sub(span));
                    *b = a.saturating_add(span);
                });
            }
            last.set((phase != gui::DragPhase::End).then_some(x));
        }
    });
    let wheel = move |dy: f32, at: f32, w: f32| {
        view.update(|(a, b)| {
            let span = index_f64(b.saturating_sub(*a));
            let factor = (f64::from(dy) / 240.0).exp2();
            let new = (span * factor).clamp(1_000.0, index_f64(CHART_POINTS));
            let t = f64::from(at / w.max(1.0)).clamp(0.0, 1.0);
            let anchor = index_f64(*a) + span * t;
            let start = f64_index(anchor - new * t);
            let new = f64_index(new);
            *a = start.clamp(0, CHART_POINTS.saturating_sub(new));
            *b = a.saturating_add(new);
        });
    };
    Box::new(move || {
        let t = theme();
        div()
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(24.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(12.0)
            .child(
                div()
                    .display(Display::Flex)
                    .gap(8.0)
                    .child(
                        button(ButtonVariant::Outline, ButtonSize::Sm)
                            .on_click(move |_| pan())
                            .child("Pan"),
                    )
                    .child(
                        button(ButtonVariant::Outline, ButtonSize::Sm)
                            .on_click(move |_| zoom(false))
                            .child("Zoom in"),
                    )
                    .child(
                        button(ButtonVariant::Outline, ButtonSize::Sm)
                            .on_click(move |_| zoom(true))
                            .child("Zoom out"),
                    )
                    .child(text(format!("{} points", s.len())).text_size(14.0).line_height(20.0)),
            )
            .child(
                plot()
                    .x_with(move || view.get())
                    .y_auto(YAuto::default())
                    .mark(line(s).color(t.primary).width(1.5))
                    .build()
                    .on_drag({
                        let drag_to = Rc::clone(&drag_to);
                        move |e| drag_to(e.x, e.bounds.width, e.phase)
                    })
                    .on_wheel(move |e| wheel(e.dy, e.x - e.bounds.x, e.bounds.width))
                    .named("chart-plot")
                    .flex(Flex::grow(1.0))
                    .min_height(0.0),
            )
            .into_element()
    })
}

// ── stream ───────────────────────────────────────────────────────────────────

/// Points a second, and how many are kept.
const STREAM_RATE: f64 = 1_000_000.0;
const STREAM_KEEP: u64 = 10_000_000;
/// What the chart shows: the last two seconds.
const STREAM_SHOWN: i64 = 2_000_000;

#[expect(clippy::as_conversions, reason = "a count of points due, saturating at the ends")]
fn due_points(seconds: f64) -> i64 {
    (seconds * STREAM_RATE) as i64
}

/// A timer appending what a 1 000 000 points a second source made since
/// it last ran (headless, time is the scenario's own).
fn stream() -> Render {
    use gui::chunk::plot::{Series, YAuto, line, plot};
    fn tick(s: Series, made: Signal<i64>, started: f32, first: i64) {
        let due = first.saturating_add(due_points(f64::from(gui::clock::current_seconds() - started)));
        let from = made.get_untracked();
        if due > from {
            let xs: Vec<i64> = (from..due).collect();
            let ys: Vec<f64> = xs
                .iter()
                .map(|x| {
                    // A gap now and then: NaN breaks the line.
                    if x % 250_000 < 500 {
                        f64::NAN
                    } else {
                        let t = index_f64(*x) * 1e-5;
                        t.sin().mul_add(40.0, (t * 7.3).cos() * 9.0)
                    }
                })
                .collect();
            batch(|| {
                s.append(&xs, &ys);
                made.set(due);
            });
        }
        gui::clock::after(Duration::from_millis(4), move || tick(s, made, started, first));
    }
    let s = Series::keeping(STREAM_KEEP);
    // Full from the start: every append drops as many points as it adds.
    let full = i64::try_from(STREAM_KEEP).unwrap_or(i64::MAX);
    let mut at = 0_i64;
    while at < full {
        let end = at.saturating_add(1 << 22).min(full);
        let xs: Vec<i64> = (at..end).collect();
        let ys: Vec<f64> = xs.iter().map(|x| (index_f64(*x) * 1e-5).sin() * 40.0).collect();
        s.append(&xs, &ys);
        at = end;
    }
    let made = Signal::labeled("stream points", full);
    let running = Cell::new(false);
    Box::new(move || {
        if !running.replace(true) {
            tick(s, made, gui::clock::current_seconds(), made.get_untracked());
        }
        let t = theme();
        div()
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .padding(24.0)
            .child(
                plot()
                    .x_with(move || {
                        let last = made.get().max(1);
                        (last.saturating_sub(STREAM_SHOWN).max(0), last)
                    })
                    .y_auto(YAuto::default())
                    .mark(line(s).color(t.primary).width(1.5))
                    .build()
                    .named("stream-plot")
                    .size(pct(100.0)),
            )
            .into_element()
    })
}

// ── canvas ───────────────────────────────────────────────────────────────────

const CANVAS_PATHS: u32 = 100_000;

/// 100 000 paths, rectangles, circles and stroked zigzags, spread over a
/// 4 000 unit square, seeded, and a card above them that drags.
fn canvas_100k() -> Render {
    use gui::canvas::{Camera, WorldPath, canvas_editor, world};
    use gui::scene::Gradient;
    use gui::{Color, PathStyle};
    let w = world();
    let mut rng = Rng::new(17);
    let colors = [
        0x00e1_1d48_u32,
        0x0025_63eb,
        0x0016_a34a,
        0x00ca_8a04,
        0x0093_33ea,
        0x000f_172a,
    ];
    let mut shade = colors.iter().copied().cycle();
    w.extend((0..CANVAS_PATHS).map(|i| {
        let x = f64::from(rng.unit()) * 4000.0;
        let y = f64::from(rng.unit()) * 4000.0;
        let size = f64::from(rng.unit()).mul_add(18.0, 2.0);
        let color = Color::hex(shade.next().unwrap_or(0));
        match i % 3 {
            0 => (
                WorldPath::rect(x, y, size, size * 0.6),
                PathStyle::fill(Gradient::Solid(color)),
            ),
            1 => {
                // A circle as four cubics.
                let r = size / 2.0;
                let k = r * 0.552_284_75;
                let p = WorldPath::new()
                    .move_to(x + r, y)
                    .cubic_to(x + r, y + k, x + k, y + r, x, y + r)
                    .cubic_to(x - k, y + r, x - r, y + k, x - r, y)
                    .cubic_to(x - r, y - k, x - k, y - r, x, y - r)
                    .cubic_to(x + k, y - r, x + r, y - k, x + r, y)
                    .close();
                (p, PathStyle::fill(Gradient::Solid(color.opacity(0.8))))
            }
            _ => {
                let mut p = WorldPath::new().move_to(x, y);
                for j in 1..6_u8 {
                    let dy = if j % 2 == 0 { 0.0 } else { size * 0.5 };
                    p = p.line_to(f64::from(j).mul_add(size / 5.0, x), y + dy);
                }
                (p, PathStyle::stroke(Gradient::Solid(color), gui::Stroke::round(1.0)))
            }
        }
    }));
    // A card on top, the one object a drag moves (the rest are locked).
    let card = w.add(
        WorldPath::rect(1990.0, 1990.0, 200.0, 120.0),
        PathStyle::fill(Gradient::Solid(Color::hex(0x00f9_7316))),
    );
    let camera = Signal::labeled(
        "canvas camera",
        Camera {
            center: (2000.0, 2000.0),
            zoom: 0.25,
            rotation: 0.0,
            settled: true,
        },
    );
    Box::new(move || {
        let t = theme();
        div()
            .size(pct(100.0))
            .background(t.background)
            .child(
                canvas_editor(w, camera, move |id| id == card)
                    .named("canvas")
                    .size(pct(100.0)),
            )
            .into_element()
    })
}
