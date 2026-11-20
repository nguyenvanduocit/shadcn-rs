//! What a screen reader is told about a frame: an AccessKit tree of the laid
//! out nodes, each with the role, name and state its [`Semantics`] or its
//! content give it, the window as its root and each layer a child of it.
//! Pure: the window sends it while assistive technology listens, and turns
//! the actions it asks for back into input ([`target`]).

use std::hash::{DefaultHasher, Hash, Hasher};

use accesskit::{Action, Affine, Node, NodeId, Rect, Role, TreeId, TreeInfo, TreeUpdate};

use crate::element::{InputType, Semantics};
use crate::focus::FocusId;
use crate::geometry::{Bounds, NodePath};
use crate::layout::{Behaviour, Content, Frame, FrameNode};

/// The window, the root every layer hangs from.
const WINDOW: NodeId = NodeId(0);

/// A node's identity across frames: its path, so a screen reader's place
/// survives a frame that rebuilds everything.
pub(crate) fn id_of(path: &NodePath) -> NodeId {
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    NodeId(h.finish().max(1))
}

/// The node an action from assistive technology names, in `frame`.
#[must_use]
pub fn target(frame: &Frame, id: NodeId) -> Option<usize> {
    frame.nodes.iter().position(|n| id_of(&n.path) == id)
}

/// The whole tree for `frame`, laid out at `scale` physical pixels per
/// logical one, with focus on the node `focused` names.
#[must_use]
pub fn tree(frame: &Frame, focused: Option<FocusId>, scale: f32, title: &str) -> TreeUpdate {
    let mut nodes: Vec<Option<(NodeId, Node)>> = Vec::with_capacity(frame.nodes.len());
    let mut window_children = Vec::new();
    let mut focus = WINDOW;
    for (i, n) in frame.nodes.iter().enumerate() {
        let Some(b) = frame.behaviour(i) else {
            nodes.push(None);
            continue;
        };
        let shown_parent = n.parent.map(|p| nodes.get(p).is_some_and(Option::is_some));
        let node = (shown_parent != Some(false) && !n.look.invisible && !b.semantics.hidden)
            .then(|| node_of(frame, i, b))
            .flatten();
        let Some(node) = node else {
            nodes.push(None);
            continue;
        };
        let id = id_of(&n.path);
        if let (Some(f), Some(handle)) = (focused, b.focus)
            && handle.id() == f
        {
            focus = id;
        }
        match n.parent.and_then(|p| nodes.get_mut(p)).and_then(Option::as_mut) {
            Some((_, parent)) => parent.push_child(id),
            None => window_children.push(id),
        }
        nodes.push(Some((id, node)));
    }
    let mut window = Node::new(Role::Window);
    window.set_label(title);
    window.set_transform(Affine::scale(f64::from(scale)));
    window.set_bounds(rect(frame.viewport));
    window.set_children(window_children);
    let mut all = vec![(WINDOW, window)];
    all.extend(nodes.into_iter().flatten());
    let mut info = TreeInfo::new(WINDOW);
    info.toolkit_name = Some("gui".to_owned());
    TreeUpdate {
        nodes: all,
        tree: Some(info),
        tree_id: TreeId::ROOT,
        focus,
    }
}

/// One node as assistive technology sees it; `None` leaves it out (an icon
/// or a drawing with no name says nothing).
fn node_of(frame: &Frame, i: usize, b: &Behaviour) -> Option<Node> {
    let n = frame.nodes.get(i)?;
    let s = &b.semantics;
    let role = s.role.or_else(|| implied_role(n, b, s))?;
    let mut node = Node::new(role);
    node.set_bounds(rect(n.bounds));
    match &s.label {
        Some(label) => node.set_label(label.as_str()),
        // AccessKit names a button from the text inside it, but not a tab.
        None if role == Role::Tab => node.set_label(text_within(frame, i)),
        None => {}
    }
    match &n.content {
        Content::Text { content, .. } => node.set_value(content.as_str()),
        Content::Input {
            edit,
            placeholder,
            mode,
            ..
        } => {
            node.set_value(edit.text.as_str());
            if !placeholder.is_empty() {
                node.set_placeholder(placeholder.as_str());
            }
            if !mode.read_only {
                node.add_action(Action::SetValue);
            } else {
                node.set_read_only();
            }
        }
        _ => {}
    }
    if b.focus.is_some() {
        node.add_action(Action::Focus);
    }
    if b.handlers.click.is_some() {
        node.add_action(Action::Click);
    }
    if b.disabled {
        node.set_disabled();
    }
    if let Some(checked) = s.checked {
        node.set_toggled(checked);
    }
    if let Some(selected) = s.selected {
        node.set_selected(selected);
    }
    if let Some(expanded) = s.expanded {
        node.set_expanded(expanded);
    }
    Some(node)
}

/// The text of the nodes inside the node at `i`, in reading order: they
/// follow it in the frame until the first that is not inside it.
fn text_within(frame: &Frame, i: usize) -> String {
    let inside = |j: &usize| frame.ancestors(*j).any(|a| a == i);
    let words: Vec<&str> = (i.saturating_add(1)..frame.nodes.len())
        .take_while(inside)
        .filter_map(|j| match &frame.nodes.get(j)?.content {
            Content::Text { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    words.join(" ")
}

/// The role a node's content implies when it sets none: text is a label, a
/// text box an input of its kind, a box that takes clicks a button.
fn implied_role(n: &FrameNode, b: &Behaviour, s: &Semantics) -> Option<Role> {
    match &n.content {
        Content::Text { .. } => Some(Role::Label),
        Content::Input { mode, multiline, .. } => Some(match (mode.input_type, multiline) {
            (InputType::Password, _) => Role::PasswordInput,
            (InputType::Text, true) => Role::MultilineTextInput,
            (InputType::Text, false) => Role::TextInput,
        }),
        Content::Image { .. } => Some(Role::Image),
        Content::Icon { .. } | Content::Shapes(_) => s.label.as_ref().map(|_| Role::Image),
        Content::None if b.handlers.click.is_some() => Some(Role::Button),
        Content::None => Some(Role::GenericContainer),
    }
}

fn rect(b: Bounds) -> Rect {
    Rect {
        x0: f64::from(b.x),
        y0: f64::from(b.y),
        x1: f64::from(b.right()),
        y1: f64::from(b.bottom()),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use accesskit::{ActionData, ActionRequest};

    use super::*;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, icon, text, text_input};
    use crate::icon::Icon;
    use crate::interact::Interaction;
    use crate::layout::layout;
    use crate::styled::Styled;
    use crate::text::TextSystem;

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
        )
        .unwrap();
        let update = tree(&frame, crate::focus::focused(), 2.0, "Remit");
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
        ui.act(&frame, &ask(Action::Click, *button_id, None));
        assert_eq!(pressed.get(), 1, "pressing from a screen reader clicks");
        ui.act(
            &frame,
            &ask(Action::SetValue, *input_id, Some(ActionData::Value("hello".into()))),
        );
        assert_eq!(field.text(), "hello");
    }
}
