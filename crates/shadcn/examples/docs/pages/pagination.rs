use gui::prelude::*;
use gui::{Model, ScenarioError, Signal, div, text};
use shadcn::{
    pagination, pagination_content, pagination_ellipsis, pagination_first, pagination_last, pagination_list,
    pagination_next, pagination_previous,
};

use crate::{Render, Shot, demo, page, section};

pub fn render() -> Render {
    // The demo: 30 items, 10 a page, opening on page 2; the page below it reads it.
    let shown = Signal::labeled("pagination page", 2_usize);
    Box::new(move || {
        page(
            "Pagination",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "PaginationDemo.vue",
                    div().flex_col().gap(6.0).child(
                        pagination().total(30, 10).model(Model::bind(shown)).child(
                            pagination_content()
                                .child(pagination_previous())
                                .child(pagination_list())
                                .child(pagination_ellipsis())
                                .child(pagination_next()),
                        ),
                    ),
                ))
                .child(section(
                    "20 pages, edges shown",
                    pagination().total(200, 10).default_page(10).show_edges(true).child(
                        pagination_content()
                            .child(pagination_first())
                            .child(pagination_previous())
                            .child(pagination_list())
                            .child(pagination_next())
                            .child(pagination_last()),
                    ),
                ))
                .child(text(format!("Page {} of 3", shown.get())).text_sm()),
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
