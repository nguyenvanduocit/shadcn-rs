//! A gallery page's element tree split for its docs page: each demo
//! (a node `demo()` named after its shadcn-vue file) lifted out in page order,
//! and the rest of the page with every box the lifting emptied removed.

use gui::Element;

/// A page's demos and what is left of it.
pub struct Lifted {
    /// Each demo's file and its subtree, in page order.
    pub demos: Vec<(&'static str, Element)>,
    /// The page without its demos; `None` when the demos were all it held.
    pub rest: Option<Element>,
}

/// Split `page`'s tree into its demos and the rest.
pub fn lift(mut page: Element) -> Lifted {
    let mut demos = Vec::new();
    let emptied = take_demos(&mut page, &mut demos);
    Lifted {
        demos,
        rest: (!emptied).then_some(page),
    }
}

/// Lift the demos under `el` into `out`; whether `el` held nothing else (it is
/// then replaced by nothing, so its gap, padding or border leaves no trace).
/// A box that never held anything (a spacer, a skeleton bar) is kept.
fn take_demos(el: &mut Element, out: &mut Vec<(&'static str, Element)>) -> bool {
    if let Some(file) = el.name().filter(|n| n.ends_with(".vue")) {
        out.push((file, std::mem::take(el)));
        return true;
    }
    let section = el.name() == Some(crate::SECTION);
    let Some(children) = el.children_mut() else {
        return false;
    };
    let mut lifted_any = false;
    let mut kept_any = false;
    for child in children.iter_mut() {
        if matches!(child, Element::Empty) {
            continue;
        }
        if take_demos(child, out) {
            lifted_any = true;
        } else {
            kept_any = true;
        }
    }
    // A section is its heading over its content: emptied with its content.
    let content_gone = section && children.last().is_some_and(|c| matches!(c, Element::Empty));
    let emptied = lifted_any && (!kept_any || content_gone);
    if emptied {
        *el = Element::Empty;
    }
    emptied
}
