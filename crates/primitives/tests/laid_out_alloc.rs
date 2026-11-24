//! A layout that lays out the same members as the last allocates nothing in
//! what watches them: the tabs' own prune and a part's callback.

use std::cell::Cell;
use std::rc::Rc;

use gui::app::Scenario;
use gui::prelude::*;
use gui::{Bind, Color, Direction, FocusHandle, Interaction, component, div, text, use_model};
use primitives::Availability;
use primitives::tabs::{Tabs, TabsOptions};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

#[test]
fn unchanged_layouts_of_tabs_allocate_nothing_in_their_watchers() {
    gui::testing::both_passes(tabs);
}

#[expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]
fn tabs(pass: gui::testing::Pass) {
    let told = Rc::new(Cell::new(0_u32));
    let t = Rc::clone(&told);
    let mut s = Scenario::measuring(
        move || {
            let t = Rc::clone(&t);
            component(Bind::default_value(0_u32), move |cx| {
                let tabs = Tabs::new(
                    use_model(cx.props(), |b: &Bind<u32>| b),
                    TabsOptions::default,
                    Direction::Ltr,
                );
                let t = Rc::clone(&t);
                tabs.roving()
                    .items()
                    .on_laid_out(move |_| t.set(t.get().saturating_add(1)));
                let focus: Vec<FocusHandle> = (0..5).map(|_| FocusHandle::new()).collect();
                move || {
                    tabs.list(div().display(Display::Flex))
                        .children(focus.iter().zip(0_u32..).map(|(f, i)| {
                            let hover = Interaction::new();
                            tabs.trigger(
                                div()
                                    .interaction(hover)
                                    .width(80.0)
                                    .height(24.0)
                                    .background(move || if hover.hovered() { Color::BLACK } else { Color::WHITE })
                                    .child(text(format!("tab {i}"))),
                                i,
                                Availability::Enabled,
                                *f,
                            )
                        }))
                        .into_element()
                }
            })
            .into_element()
        },
        600,
        200,
        1.0,
    )
    .expect("a scenario");
    gui::reactive::stop_trace();
    pass.scenario(&mut s);
    s.settle().expect("settled");
    for tab in ["tab 1", "tab 2", "tab 1"] {
        let (x, y) = s.locate(tab).expect("a tab");
        s.hover(x, y);
        s.settle().expect("settled");
        s.frame("warm").expect("a frame");
    }
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    let before = told.get();
    let mut taken = 0;
    for step in 0..100 {
        let tab = if step % 2 == 0 { "tab 3" } else { "tab 4" };
        let (x, y) = s.locate(tab).expect("a tab");
        s.hover(x, y);
        let r = s.frame("hover").expect("a frame");
        let bytes = |t: Option<gui::heap::ThreadHeap>| t.map_or(u64::MAX, |t| t.allocated);
        taken = bytes(r.footprint.thread)
            .saturating_sub(bytes(r.footprint.committed))
            .saturating_add(taken);
    }
    let laid_out = told.get().saturating_sub(before);
    assert!(laid_out >= 100, "each frame laid the tabs out: {laid_out}");
    pass.measured(|| assert_eq!(taken, 0, "{taken} bytes allocated on the UI thread over 100 layouts"));
}
