use gui::prelude::*;
use gui::{ScenarioError, div, text};
use shadcn::{
    PaginationEntry, PaginationState, pagination, pagination_content, pagination_ellipsis, pagination_first,
    pagination_item, pagination_last, pagination_next, pagination_previous,
};

use crate::{Render, Shot, demo, page, section};

fn list(state: &PaginationState) -> gui::Div {
    pagination_content().children(state.entries().into_iter().map(|e| match e {
        PaginationEntry::Page(n) => pagination_item(state, n),
        PaginationEntry::Ellipsis => pagination_ellipsis(),
    }))
}

pub fn render() -> Render {
    // The demo: 30 items, 10 a page, opening on page 2.
    let demo_state = PaginationState::new(30, 10, 2);
    let mut long = PaginationState::new(200, 10, 10);
    long.show_edges = true;
    Box::new(move || {
        page(
            "Pagination",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "PaginationDemo.vue",
                    div().flex_col().gap(6.0).child(
                        pagination().child(
                            pagination_content()
                                .child(pagination_previous(&demo_state))
                                .child(list(&demo_state))
                                .child(pagination_ellipsis())
                                .child(pagination_next(&demo_state)),
                        ),
                    ),
                ))
                .child(section(
                    "20 pages, edges shown",
                    pagination().child(
                        pagination_content()
                            .child(pagination_first(&long))
                            .child(pagination_previous(&long))
                            .child(list(&long))
                            .child(pagination_next(&long))
                            .child(pagination_last(&long)),
                    ),
                ))
                .child(text(format!("Page {} of {}", demo_state.page.get(), demo_state.page_count())).text_sm()),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Next")?;
    s.hover(x, y);
    s.frame("hover next")?;
    s.click_text("3")?;
    s.frame("page 3 next disabled")?;
    s.click_text("1")?;
    s.frame("page 1 previous disabled")?;
    s.click_text("Last")?;
    s.frame("last of 20")?;
    s.click_text("First")?;
    s.frame("first of 20")?;
    Ok(())
}
