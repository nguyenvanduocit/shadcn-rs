//! ADR-0021's checks: an update holds only what changed, keeps AccessKit's
//! contract and equals the whole tree; text fields expose their runs and
//! take a selection; virtual lists expose rows and keep pinned ones.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Role, TextPosition, TextSelection, TreeId};
use accesskit_consumer::{NodeRef, Tree, TreeChangeHandler};

use super::*;
use crate::app::Scenario;
use crate::edit::TextField;
use crate::element::{Element, IntoElement, div, icon, text, text_input};
use crate::icon::Icon;
use crate::interact::Interaction;
use crate::layout::layout;
use crate::list::{ListState, virtual_list};
use crate::reactive::Signal;
use crate::styled::Styled;
use crate::window::Input;

#[test]
fn a_screen_reader_reads_names_roles_and_focus_and_can_press_and_type() {
    let pressed = Rc::new(Cell::new(0));
    let field = TextField::new("");
    let p = Rc::clone(&pressed);
    let root = div()
        .child(
            div()
                .on_click(move |_| p.set(p.get() + 1))
                .child(icon(Icon::Plus))
                .child(text("New agent")),
        )
        .child(div().aria_label("Close").role(Role::Button).child(icon(Icon::X)))
        .child(div().aria_hidden().child(text("decoration")))
        .child(text_input(&field).placeholder("Ask the assistant").w_px(200.0))
        .into_element();
    let mut text_system = TextSystem::new();
    let mut ui = Interaction::default();
    field.focus.focus();
    ui.sync_focus();
    let frame = layout(
        &root,
        400.0,
        200.0,
        ui.state(),
        &mut crate::motion::Motion::default(),
        &mut text_system,
    );
    let update = tree(&frame, crate::focus::focused(), 2.0, "Remit", &mut text_system);
    let find = |role: Role| update.nodes.iter().filter(move |(_, n)| n.role() == role);

    let (button_id, button) = find(Role::Button).next().unwrap();
    let labels: Vec<_> = button
        .children()
        .iter()
        .filter_map(|c| update.nodes.iter().find(|(id, _)| id == c))
        .map(|(_, n)| (n.role(), n.value().map(str::to_owned)))
        .collect();
    assert_eq!(
        labels,
        [(Role::Label, Some("New agent".to_owned()))],
        "a nameless icon says nothing"
    );
    assert!(button.supports_action(Action::Click));
    assert_eq!(find(Role::Button).nth(1).and_then(|(_, n)| n.label()), Some("Close"));
    assert!(
        update.nodes.iter().all(|(_, n)| n.value() != Some("decoration")),
        "what is hidden is not read"
    );
    let (input_id, input) = find(Role::TextInput).next().unwrap();
    assert_eq!(input.placeholder(), Some("Ask the assistant"));
    assert_eq!(update.focus, *input_id, "focus is where the caret is");

    let ask = |action, target_node, data| ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node,
        data,
    };
    ui.act(&frame, &mut text_system, &ask(Action::Click, *button_id, None));
    assert_eq!(pressed.get(), 1, "pressing from a screen reader clicks");
    ui.act(
        &frame,
        &mut text_system,
        &ask(Action::SetValue, *input_id, Some(ActionData::Value("hello".into()))),
    );
    assert_eq!(field.text(), "hello");
}

// ── A1: only what changed ────────────────────────────────────────────────────

/// A label and `items` clickable rows under one parent.
fn page(label: Signal<String>, items: Signal<usize>) -> impl FnMut() -> Element {
    move || {
        div()
            .flex_col()
            .child(text(label.get()))
            .child(div().flex_col().children((0..items.get()).map(|i| {
                div()
                    .key(u64::try_from(i).unwrap())
                    .on_click(|_| {})
                    .child(text(format!("item {i}")))
            })))
            .into_element()
    }
}

fn update(s: &mut Scenario) -> TreeUpdate {
    s.settle().unwrap();
    s.pipeline_mut().access_update(1.0, "page")
}

fn full(s: &mut Scenario) -> TreeUpdate {
    s.pipeline_mut().access_tree(1.0, "page")
}

fn by_id(u: &TreeUpdate) -> HashMap<NodeId, Node> {
    u.nodes.iter().cloned().collect()
}

#[test]
fn an_update_holds_the_nodes_that_changed_and_parents_whose_children_did() {
    let label = Signal::labeled("label", "one".to_owned());
    let items = Signal::labeled("items", 3_usize);
    let mut s = Scenario::measuring(page(label, items), 300, 300, 1.0).unwrap();

    let first = update(&mut s);
    assert!(first.tree.is_some(), "the first update is the whole tree");
    let whole = full(&mut s);
    assert_eq!(by_id(&first), by_id(&whole));
    let stats = s.pipeline().access_stats();
    assert!(stats.full && stats.sent == stats.nodes && stats.nodes == whole.nodes.len());

    let still = update(&mut s);
    assert!(
        still.nodes.is_empty(),
        "nothing changed, nothing sent: {:?}",
        still.nodes
    );
    assert!(still.tree.is_none());

    s.act("relabel", || label.set("two".to_owned()));
    let relabelled = update(&mut s);
    assert_eq!(relabelled.nodes.len(), 1, "{:?}", relabelled.nodes);
    assert_eq!(relabelled.nodes[0].1.value(), Some("two"));

    s.act("grow", || items.set(4));
    let grown = update(&mut s);
    let roles: Vec<Role> = grown.nodes.iter().map(|(_, n)| n.role()).collect();
    assert_eq!(
        grown.nodes.len(),
        3,
        "the new row, its text and their parent: {roles:?}"
    );
    let parent = grown
        .nodes
        .iter()
        .find(|(_, n)| n.children().len() == 4)
        .expect("the parent");
    assert_eq!(parent.1.role(), Role::GenericContainer);

    s.act("shrink", || items.set(1));
    let shrunk = update(&mut s);
    assert_eq!(shrunk.nodes.len(), 1, "the parent alone removes three rows");
    assert_eq!(shrunk.nodes[0].1.children().len(), 1);

    s.pipeline_mut().access_reset();
    let again = update(&mut s);
    assert!(
        again.tree.is_some(),
        "a screen reader connecting again gets the whole tree"
    );
    assert_eq!(by_id(&again), by_id(&full(&mut s)));
}

struct Quiet;

impl TreeChangeHandler for Quiet {
    fn node_added(&mut self, _: &NodeRef<'_>) {}
    fn node_updated(&mut self, _: &NodeRef<'_>, _: &NodeRef<'_>) {}
    fn focus_moved(&mut self, _: Option<&NodeRef<'_>>, _: Option<&NodeRef<'_>>) {}
    fn node_removed(&mut self, _: &NodeRef<'_>) {}
}

/// Every node of `tree` reachable from its root.
fn held(tree: &Tree) -> HashMap<NodeId, Node> {
    let state = tree.state();
    let mut out = HashMap::new();
    let mut stack = vec![NodeId(0)];
    while let Some(id) = stack.pop() {
        let node = state
            .node_by_tree_local_id(id, TreeId::ROOT)
            .expect("a child the tree holds");
        stack.extend(node.data().children().iter().copied());
        out.insert(id, node.data().clone());
    }
    out
}

#[test]
fn incremental_updates_keep_accesskits_contract_and_equal_the_whole_tree() {
    let label = Signal::labeled("label", "one".to_owned());
    let items = Signal::labeled("items", 3_usize);
    let field = TextField::new("type here");
    let f = field;
    let mut page = page(label, items);
    let mut s = Scenario::measuring(
        move || {
            div()
                .flex_col()
                .child(page())
                .child(text_input(&f).w_px(200.0))
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .unwrap();
    let mut tree = Tree::new(update(&mut s), true);
    let steps: Vec<Box<dyn Fn()>> = vec![
        Box::new(move || label.set("two".to_owned())),
        Box::new(move || items.set(6)),
        Box::new(move || items.set(2)),
        Box::new(move || field.focus.focus()),
        Box::new(move || field.set_text("typed\nover two lines")),
        Box::new(move || items.set(0)),
        Box::new(move || label.set("three".to_owned())),
    ];
    for (k, step) in steps.iter().enumerate() {
        s.act(format!("step {k}"), step);
        let u = update(&mut s);
        let focus = u.focus;
        tree.update_and_process_changes(u, &mut Quiet);
        let whole = full(&mut s);
        assert_eq!(focus, whole.focus, "step {k}: focus is in every update");
        assert_eq!(held(&tree), by_id(&whole), "step {k}: the tree held is the whole tree");
    }
}

// ── A2: text runs ────────────────────────────────────────────────────────────

fn field_scenario(field: &TextField, multiline: bool) -> Scenario {
    let f = *field;
    let mut s = Scenario::measuring(
        move || {
            let input = text_input(&f).w_px(240.0);
            let input = if multiline { input.multiline().h_px(80.0) } else { input };
            div().child(input).into_element()
        },
        300,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s
}

/// The input node of `u` and its runs.
fn input_and_runs(u: &TreeUpdate, role: Role) -> ((NodeId, Node), Vec<(NodeId, Node)>) {
    let nodes = by_id(u);
    let (id, input) = u
        .nodes
        .iter()
        .find(|(_, n)| n.role() == role)
        .cloned()
        .expect("the input");
    let runs = input
        .children()
        .iter()
        .map(|c| (*c, nodes.get(c).cloned().expect("a run sent with its field")))
        .collect();
    ((id, input), runs)
}

#[test]
fn a_text_field_exposes_its_characters_words_and_caret_as_runs() {
    let field = TextField::new("hello world");
    let mut s = field_scenario(&field, false);
    s.act("focus", || field.focus.focus());
    let u = update(&mut s);
    let ((input_id, input), runs) = input_and_runs(&u, Role::TextInput);
    assert_eq!(runs.len(), 1);
    let (run_id, run) = &runs[0];
    assert_eq!(run.role(), Role::TextRun);
    assert_eq!(run.value(), Some("hello world"));
    assert_eq!(run.character_lengths(), &[1_u8; 11]);
    assert_eq!(run.word_starts(), &[0, 6]);
    let positions = run.character_positions().unwrap();
    assert_eq!(positions.len(), 11);
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{positions:?}");
    assert!(run.character_widths().unwrap().iter().all(|w| *w > 0.0));
    let b = run.bounds().unwrap();
    let ib = input.bounds().unwrap();
    assert!(
        b.x0 >= ib.x0 && b.x1 <= ib.x1 && b.y0 >= ib.y0 && b.y1 <= ib.y1,
        "{b:?} in {ib:?}"
    );
    let caret = TextPosition {
        node: *run_id,
        character_index: 11,
    };
    assert_eq!(
        input.text_selection(),
        Some(&TextSelection {
            anchor: caret,
            focus: caret
        }),
        "the caret after the last character"
    );
    assert!(input.supports_action(Action::SetTextSelection));
    assert_eq!(u.focus, input_id);

    // A screen reader selects "hello": the field's selection, and only the
    // field is sent again (its run did not change).
    let at = |character_index| TextPosition {
        node: *run_id,
        character_index,
    };
    let out = s.pipeline_mut().input(Input::Act(ActionRequest {
        action: Action::SetTextSelection,
        target_tree: TreeId::ROOT,
        target_node: input_id,
        data: Some(ActionData::SetTextSelection(TextSelection {
            anchor: at(0),
            focus: at(5),
        })),
    }));
    assert!(out.handled_by.is_some());
    let e = field.edit.get_untracked();
    assert_eq!((e.anchor, e.caret), (0, 5));
    let selected = update(&mut s);
    assert_eq!(selected.nodes.len(), 1, "{:?}", selected.nodes);
    assert_eq!(
        selected.nodes[0].1.text_selection(),
        Some(&TextSelection {
            anchor: at(0),
            focus: at(5)
        })
    );

    // Typing changes the field and its run, nothing else.
    s.type_text("!");
    let typed = update(&mut s);
    assert_eq!(typed.nodes.len(), 2, "{:?}", typed.nodes);
    assert!(typed.nodes.iter().any(|(_, n)| n.value() == Some("! world")));
}

#[test]
fn a_multiline_field_has_a_run_per_line_and_its_break_ends_the_first() {
    let field = TextField::new("ab\ncd");
    field.edit.update(|e| {
        e.caret = 3;
        e.anchor = 3;
    });
    let mut s = field_scenario(&field, true);
    let u = update(&mut s);
    let ((_, input), runs) = input_and_runs(&u, Role::MultilineTextInput);
    let values: Vec<_> = runs.iter().map(|(_, n)| n.value().unwrap().to_owned()).collect();
    assert_eq!(values, ["ab\n", "cd"]);
    assert_eq!(runs[0].1.character_lengths(), &[1, 1, 1]);
    assert!(runs[1].1.bounds().unwrap().y0 > runs[0].1.bounds().unwrap().y0);
    let sel = input.text_selection().unwrap();
    assert_eq!(
        sel.focus,
        TextPosition {
            node: runs[1].0,
            character_index: 0
        },
        "the caret at the start of the second line"
    );
    assert_ne!(runs[0].0, runs[1].0);
}

#[test]
fn an_empty_field_has_one_empty_run_for_the_caret() {
    let field = TextField::new("");
    let mut s = field_scenario(&field, false);
    let u = update(&mut s);
    let ((_, input), runs) = input_and_runs(&u, Role::TextInput);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].1.value(), Some(""));
    assert_eq!(input.text_selection().unwrap().focus.character_index, 0);
}

// ── A3: virtual lists ────────────────────────────────────────────────────────

const ROWS: u64 = 1000;

fn list_scenario(state: &ListState, rows: impl Fn(u64) -> Element + 'static, table: bool) -> Scenario {
    let st = state.clone();
    let rows = Rc::new(rows);
    let mut s = Scenario::measuring(
        move || {
            let r = Rc::clone(&rows);
            let list = virtual_list(&st, ROWS, |i| i, move |i| r(i)).h_px(200.0);
            let list = if table { list.role(Role::Table) } else { list };
            div().w_px(300.0).child(list).into_element()
        },
        300,
        300,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s
}

#[test]
fn a_virtual_list_says_how_many_rows_it_has_and_where_each_is() {
    let state = ListState::new(24.0);
    let mut s = list_scenario(
        &state,
        |i| div().h_px(24.0).child(text(format!("row {i}"))).into_element(),
        false,
    );
    let u = full(&mut s);
    let list = u.nodes.iter().find(|(_, n)| n.role() == Role::List).expect("the list");
    assert_eq!(list.1.row_count(), Some(usize::try_from(ROWS).unwrap()));
    let items: Vec<&Node> = u
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::ListItem)
        .map(|(_, n)| n)
        .collect();
    assert!(!items.is_empty() && items.len() < 100, "{} rows built", items.len());
    for item in &items {
        assert_eq!(item.size_of_set(), Some(usize::try_from(ROWS).unwrap()));
        let at = item.position_in_set().unwrap();
        assert_eq!(item.row_index(), Some(at - 1));
    }
    assert_eq!(items[0].position_in_set(), Some(1));
}

#[test]
fn a_row_that_names_its_role_carries_its_index_itself() {
    let state = ListState::new(24.0);
    let mut s = list_scenario(
        &state,
        |i| {
            div()
                .role(Role::Row)
                .h_px(24.0)
                .child(text(format!("row {i}")))
                .into_element()
        },
        true,
    );
    let u = full(&mut s);
    assert!(u.nodes.iter().all(|(_, n)| n.role() != Role::ListItem));
    let rows: Vec<&Node> = u
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::Row)
        .map(|(_, n)| n)
        .collect();
    assert!(!rows.is_empty());
    assert_eq!(rows[2].row_index(), Some(2));
    assert_eq!(rows[2].position_in_set(), Some(3));
    assert_eq!(rows[2].size_of_set(), Some(usize::try_from(ROWS).unwrap()));
    let table = u.nodes.iter().find(|(_, n)| n.role() == Role::Table).unwrap();
    assert_eq!(table.1.row_count(), Some(usize::try_from(ROWS).unwrap()));
}

#[test]
fn the_focused_and_the_active_rows_stay_when_scrolled_out_of_view() {
    let state = ListState::new(24.0);
    let mut s = list_scenario(
        &state,
        |i| {
            div()
                .focusable()
                .h_px(24.0)
                .child(text(format!("row {i}")))
                .into_element()
        },
        false,
    );
    let (x, y) = s.locate("row 3").unwrap();
    s.click(x, y);
    s.settle().unwrap();
    assert_eq!(state.focused_row(), Some(3));
    let focused_id = full(&mut s).focus;
    assert_ne!(focused_id, NodeId(0));

    s.act("far", || {
        state.scroll_to_row(500);
        state.set_active(Some(800));
    });
    s.settle().unwrap();
    s.settle().unwrap();
    assert_eq!(state.pinned(), [3, 800], "focus and the active row stay built");
    let u = full(&mut s);
    assert_eq!(u.focus, focused_id, "focus is still on the row");
    let positions: Vec<usize> = u
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::ListItem)
        .filter_map(|(_, n)| n.position_in_set())
        .collect();
    assert!(positions.contains(&4) && positions.contains(&801), "{positions:?}");
    assert_eq!(
        positions.first(),
        Some(&4),
        "a pinned row above is read before the window"
    );
    assert_eq!(positions.last(), Some(&801), "one below, after it");
    let top = s.locate("row 500").unwrap();
    assert!(top.1 > 0.0, "the window shows row 500");
    assert!(s.locate("row 3").unwrap().1 < 0.0, "the pinned row is not shown");

    // A screen reader asks for the focused row: the list brings it back.
    let row3 = u
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::ListItem && n.position_in_set() == Some(4))
        .map(|(id, _)| *id)
        .unwrap();
    let out = s.pipeline_mut().input(Input::Act(ActionRequest {
        action: Action::ScrollIntoView,
        target_tree: TreeId::ROOT,
        target_node: row3,
        data: None,
    }));
    assert!(out.handled_by.is_some());
    s.act("no active row", || state.set_active(None));
    s.settle().unwrap();
    s.settle().unwrap();
    assert!(state.pinned().is_empty(), "{:?}", state.pinned());
    assert!(s.locate("row 3").unwrap().1 >= 0.0, "row 3 is in view again");
}

#[test]
fn a_scroll_sends_the_container_alone() {
    let mut s =
        Scenario::measuring(
            || {
                div()
                    .child(
                        div().w_px(200.0).h_px(100.0).overflow_y_scroll().flex_col().children(
                            (0..20).map(|i| div().h_px(30.0).on_click(|_| {}).child(text(format!("row {i}")))),
                        ),
                    )
                    .into_element()
            },
            300,
            300,
            1.0,
        )
        .unwrap();
    let mut tree = Tree::new(update(&mut s), true);
    s.hover(50.0, 50.0);
    s.wheel(45.0);
    let u = update(&mut s);
    assert_eq!(u.nodes.len(), 1, "{:?}", u.nodes);
    let t = u.nodes[0].1.transform().expect("the container moves its content");
    assert_eq!(t.as_coeffs()[5], -45.0);
    tree.update_and_process_changes(u, &mut Quiet);
    assert_eq!(held(&tree), by_id(&full(&mut s)));
}
