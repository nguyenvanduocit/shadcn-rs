use gui::prelude::*;
use gui::{ImageSource, ScenarioError, div};
use shadcn::{avatar, avatar_fallback, avatar_image, theme};

use crate::{Render, Shot, demo, page, photo, section};

fn person(image: Option<&ImageSource>, initials: &str) -> gui::Div {
    avatar()
        .child(avatar_image(image))
        .child(avatar_fallback(image, initials))
}

pub fn render() -> Render {
    let [shadcn, leerob, evilrabbit] = ["shadcn", "leerob", "evilrabbit"].map(photo);
    Box::new(move || {
        let t = theme();
        page(
            "Avatar",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "AvatarDemo.vue",
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap(12.0)
                        .child(person(shadcn.as_ref(), "CN"))
                        .child(person(evilrabbit.as_ref(), "ER"))
                        // `-space-x-2`, and each avatar's `ring-2 ring-background grayscale`.
                        .child(
                            div().flex().children(
                                [
                                    (shadcn.as_ref(), "CN"),
                                    (leerob.as_ref(), "LR"),
                                    (evilrabbit.as_ref(), "ER"),
                                ]
                                .into_iter()
                                .enumerate()
                                .map(|(i, (image, initials))| {
                                    person(image, initials)
                                        .ring(2.0, t.background)
                                        .grayscale()
                                        .when(i > 0, |d| d.ml(-2.0))
                                }),
                            ),
                        ),
                ))
                .child(section(
                    "Fallback, when there is no picture",
                    div()
                        .flex()
                        .items_center()
                        .gap(4.0)
                        .child(person(None, "CN"))
                        .child(person(None, "ER").size(12.0).text_lg()),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    crate::still(s)
}
