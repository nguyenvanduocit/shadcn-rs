//! A project's files as a tree: folders that fold open and closed
//! (`collapsible`) and ghost-button rows, one file selected.

use std::collections::HashMap;

use gui::prelude::*;
use gui::{Div, Icon, ScenarioError, Signal, div, icon};
use shadcn::{ButtonSize, ButtonVariant, button, collapsible, collapsible_content, collapsible_toggle, theme};

use crate::{Render, Shot, demo, page};

/// A file or a folder of them.
enum Node {
    File {
        name: &'static str,
        path: &'static str,
    },
    Folder {
        name: &'static str,
        path: &'static str,
        children: &'static [Node],
    },
}

const FILES: &[Node] = &[
    Node::Folder {
        name: "app",
        path: "app",
        children: &[
            Node::File {
                name: "app.vue",
                path: "app/app.vue",
            },
            Node::File {
                name: "error.vue",
                path: "app/error.vue",
            },
        ],
    },
    Node::Folder {
        name: "components",
        path: "components",
        children: &[
            Node::Folder {
                name: "ui",
                path: "components/ui",
                children: &[Node::File {
                    name: "Button.vue",
                    path: "components/ui/Button.vue",
                }],
            },
            Node::File {
                name: "LoginForm.vue",
                path: "components/LoginForm.vue",
            },
        ],
    },
];

/// Every folder under `nodes`, by path.
fn folders(nodes: &[Node], into: &mut Vec<&'static str>) {
    for n in nodes {
        if let Node::Folder { path, children, .. } = n {
            into.push(path);
            folders(children, into);
        }
    }
}

/// The tree's state: which folders are open and which file is selected.
struct Tree {
    open: HashMap<&'static str, Signal<bool>>,
    selected: Signal<&'static str>,
}

/// A row: `<Button variant="ghost" size="sm" class="w-full justify-start font-normal">`.
fn row() -> Div {
    button(ButtonVariant::Ghost, ButtonSize::Sm)
        .w_full()
        .justify_start()
        .font_normal()
}

fn node(tree: &Tree, n: &Node) -> Div {
    let t = theme();
    match n {
        Node::Folder { name, path, children } => {
            let Some(open) = tree.open.get(path) else {
                return div();
            };
            let expanded = open.get();
            collapsible()
                .child(
                    row()
                        .when(expanded, |b| {
                            // `aria-expanded:bg-transparent aria-expanded:hover:bg-muted`.
                            let hover = if t.dark { t.muted.opacity(0.5) } else { t.muted };
                            b.hover(move |s| s.bg(hover))
                        })
                        .on_click(collapsible_toggle(open))
                        .child(icon(Icon::ChevronRight).rotate(if expanded { 90.0 } else { 0.0 }))
                        .child(icon(Icon::Folder))
                        .child(*name),
                )
                .child(collapsible_content(open).map(|c| {
                    c.child(
                        div()
                            .ml(4.0)
                            .flex_col()
                            .children(children.iter().map(|c| node(tree, c))),
                    )
                }))
        }
        Node::File { name, path } => {
            let current = tree.selected.get() == *path;
            let selected = tree.selected.clone();
            let path = *path;
            row()
                .when(current, |b| b.bg(t.accent).text_color(t.accent_foreground))
                .on_click(move |_| selected.set(path))
                .child(icon(Icon::File))
                .child(*name)
        }
    }
}

pub fn render() -> Render {
    let mut paths = Vec::new();
    folders(FILES, &mut paths);
    let tree = Tree {
        // `<Collapsible default-open>`: every folder starts open.
        open: paths
            .into_iter()
            .map(|p| (p, Signal::labeled("folder open", true)))
            .collect(),
        selected: Signal::labeled("selected file", "components/ui/Button.vue"),
    };
    Box::new(move || {
        page(
            "File tree",
            demo(
                "FileTreeDemo.vue",
                div()
                    .flex_col()
                    .w_full()
                    .max_w(80.0)
                    .py(6.0)
                    .text_sm()
                    .children(FILES.iter().map(|n| node(&tree, n))),
            ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("app.vue")?;
    s.hover(x, y);
    s.frame("hover app.vue")?;
    s.click(x, y);
    s.frame("app.vue selected")?;
    s.click_text("components")?;
    s.frame("components closed")?;
    Ok(())
}
