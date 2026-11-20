use gui::prelude::*;
use gui::{Icon, ImageSource, ScenarioError, TextField, div, icon};
use shadcn::{
    ButtonSize, ButtonVariant, EmptyMediaVariant, InputGroupAlign, avatar, avatar_fallback, avatar_image, button,
    empty, empty_content, empty_description, empty_header, empty_link, empty_media, empty_media_icon, empty_title,
    input_group, input_group_addon, input_group_input, kbd, theme,
};

use crate::{Render, Shot, demo, page, photo};

/// An avatar with its picture, or its initials until there is one.
fn person(image: Option<&ImageSource>, initials: &str) -> gui::Div {
    avatar()
        .child(avatar_image(image))
        .child(avatar_fallback(image, initials))
}

pub fn render() -> Render {
    let zernonia = photo("zernonia");
    let team = [("ace-of-aces", "AA"), ("sadeghbarati", "SB"), ("zernonia", "ZN")]
        .map(|(login, initials)| (photo(login), initials));
    let search = TextField::new("");
    Box::new(move || {
        let t = theme();
        page(
            "Empty",
            div()
                .flex_col()
                .gap(6.0)
                .child(demo(
                    "EmptyDemo.vue",
                    empty()
                        .child(
                            empty_header()
                                .child(empty_media_icon(Icon::FolderCode))
                                .child(empty_title("No Projects Yet"))
                                .child(empty_description(
                                    "You haven't created any projects yet. Get started by creating your first project.",
                                )),
                        )
                        .child(
                            empty_content().child(
                                div()
                                    .flex()
                                    .gap(2.0)
                                    .child(button(ButtonVariant::Default, ButtonSize::Default).child("Create Project"))
                                    .child(button(ButtonVariant::Outline, ButtonSize::Default).child("Import Project")),
                            ),
                        )
                        .child(
                            button(ButtonVariant::Link, ButtonSize::Sm)
                                .text_color(t.muted_foreground)
                                .child("Learn More")
                                .child(icon(Icon::ArrowUpRight)),
                        ),
                ))
                .child(demo(
                    "EmptyOutlineDemo.vue",
                    empty()
                        .border()
                        .child(
                            empty_header()
                                .child(empty_media_icon(Icon::Cloud))
                                .child(empty_title("Cloud Storage Empty"))
                                .child(empty_description(
                                    "Upload files to your cloud storage to access them anywhere.",
                                )),
                        )
                        .child(
                            empty_content()
                                .child(button(ButtonVariant::Outline, ButtonSize::Sm).child("Upload Files")),
                        ),
                ))
                .child(demo(
                    "EmptyAvatarDemo.vue",
                    empty()
                        .child(
                            empty_header()
                                .child(
                                    empty_media(EmptyMediaVariant::Default).child(
                                        avatar()
                                            .size(12.0)
                                            .child(avatar_image(zernonia.as_ref()).map(Styled::grayscale))
                                            .child(avatar_fallback(zernonia.as_ref(), "ZN")),
                                    ),
                                )
                                .child(empty_title("User Offline"))
                                .child(empty_description(
                                    "This user is currently offline. You can leave a message to notify them or try again later.",
                                )),
                        )
                        .child(
                            empty_content().child(button(ButtonVariant::Default, ButtonSize::Sm).child("Leave Message")),
                        ),
                ))
                .child(demo(
                    "EmptyBackgroundDemo.vue",
                    // `from-muted/50 to-background h-full bg-gradient-to-b from-30%`.
                    empty()
                        .h_full()
                        .bg_linear_to_b()
                        .from(t.muted.opacity(0.5))
                        .gradient_from_at(0.3)
                        .to(t.background)
                        .child(
                            empty_header()
                                .child(empty_media_icon(Icon::Bell))
                                .child(empty_title("No Notifications"))
                                .child(empty_description(
                                    "You're all caught up. New notifications will appear here.",
                                )),
                        )
                        .child(
                            empty_content().child(
                                button(ButtonVariant::Outline, ButtonSize::Sm)
                                    .child(icon(Icon::RefreshCcw))
                                    .child("Refresh"),
                            ),
                        ),
                ))
                .child(demo(
                    "EmptyInputGroupDemo.vue",
                    empty()
                        .child(
                            empty_header().child(empty_title("404 - Not Found")).child(empty_description(
                                "The page you're looking for doesn't exist. Try searching for what you need below.",
                            )),
                        )
                        .child(
                            empty_content()
                                .child(
                                    input_group(input_group_input(&search).placeholder("Try searching for pages..."))
                                        .w_len(gui::relative(0.75))
                                        .addon(input_group_addon(InputGroupAlign::InlineStart).child(icon(Icon::Search)))
                                        .addon(input_group_addon(InputGroupAlign::InlineEnd).kbd(kbd().child("/"))),
                                )
                                .child(empty_description("Need help? ").span(empty_link("Contact support"))),
                        ),
                ))
                .child(demo(
                    "EmptyAvatarGroupDemo.vue",
                    empty()
                        .child(
                            empty_header()
                                .child(
                                    empty_media(EmptyMediaVariant::Default).child(
                                        // `-space-x-2`, and each avatar's `size-12 ring-2 ring-background grayscale`.
                                        div().flex().children(team.iter().enumerate().map(|(i, (image, initials))| {
                                            person(image.as_ref(), initials)
                                                .size(12.0)
                                                .ring(2.0, t.background)
                                                .grayscale()
                                                .when(i > 0, |d| d.ml(-2.0))
                                        })),
                                    ),
                                )
                                .child(empty_title("No Team Members"))
                                .child(empty_description("Invite your team to collaborate on this project.")),
                        )
                        .child(
                            empty_content().child(
                                button(ButtonVariant::Default, ButtonSize::Sm)
                                    .child(icon(Icon::Plus))
                                    .child("Invite Members"),
                            ),
                        ),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    crate::still(s)
}
