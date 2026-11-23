use gui::prelude::*;
use gui::{ImageSource, ScenarioError, div};
use shadcn::{avatar, avatar_fallback, avatar_image, theme};

use crate::{Render, Shot, demo, page, photo, section};

fn person(image: Option<&ImageSource>, initials: &str) -> shadcn::Part<()> {
    avatar().child(avatar_image(image)).child(avatar_fallback(initials))
}

pub fn render() -> Render {
    let [shadcn, leerob, evilrabbit] = ["shadcn", "leerob", "evilrabbit"].map(photo);
    Box::new(move || {
        let t = theme();
        page(
            "Avatar",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "AvatarDemo.vue",
                    div()
                        .display(Display::Flex)
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Row)
                        .flex_wrap(true)
                        .align_items(Align::Center)
                        .gap(48.0)
                        .child(person(shadcn.as_ref(), "CN"))
                        .child(person(evilrabbit.as_ref(), "ER"))
                        // `-space-x-2`, and each avatar's `ring-2 ring-background grayscale`.
                        .child(
                            div().display(Display::Flex).children(
                                [
                                    (shadcn.as_ref(), "CN"),
                                    (leerob.as_ref(), "LR"),
                                    (evilrabbit.as_ref(), "ER"),
                                ]
                                .into_iter()
                                .enumerate()
                                .map(|(i, (image, initials))| {
                                    person(image, initials)
                                        .ring(Ring::new(2.0, t.background))
                                        .filters(Filter::Grayscale(1.0))
                                        .when(i > 0, |d| d.margin_left(-8.0))
                                }),
                            ),
                        ),
                ))
                .child(section(
                    "Fallback, when there is no picture",
                    div()
                        .display(Display::Flex)
                        .align_items(Align::Center)
                        .gap(16.0)
                        .child(person(None, "CN"))
                        .child(person(None, "ER").size(48.0).text_size(18.0).line_height(28.0)),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    crate::still(s)
}
