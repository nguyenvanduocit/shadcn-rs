use gui::prelude::*;
use gui::{Icon, ImageSource, ScenarioError, Side, SideAlign, div, icon, image, span, text};
use shadcn::{
    ButtonSize, ButtonVariant, ItemMediaVariant, ItemSize, ItemVariant, MenuItem, MenuState, avatar, avatar_fallback,
    avatar_image, button, dropdown_menu_content, dropdown_menu_item, dropdown_menu_trigger, item, item_actions,
    item_content, item_description, item_group, item_header, item_link, item_media, item_media_described,
    item_separator, item_title, theme,
};

use crate::{Render, Shot, demo, page, photo};

/// `ItemGroupDemo` and `ItemDropdownDemo`'s people: login, email.
const PEOPLE: [(&str, &str); 3] = [
    ("shadcn", "shadcn@vercel.com"),
    ("maxleiter", "maxleiter@vercel.com"),
    ("evilrabbit", "evilrabbit@vercel.com"),
];

/// `ItemHeaderDemo`'s models: name, description, Unsplash photo.
const MODELS: [(&str, &str, &str); 3] = [
    (
        "v0-1.5-sm",
        "Everyday tasks and UI generation.",
        "photo-1650804068570-7fb2e3dbf888",
    ),
    (
        "v0-1.5-lg",
        "Advanced thinking or reasoning.",
        "photo-1610280777472-54133d004c8c",
    ),
    (
        "v0-2.0-mini",
        "Open Source model for everyone.",
        "photo-1602146057681-08560aee8cde",
    ),
];

/// `ItemImageDemo`'s songs: title, artist, album, duration.
const MUSIC: [(&str, &str, &str, &str); 3] = [
    ("Midnight City Lights", "Neon Dreams", "Electric Nights", "3:45"),
    ("Coffee Shop Conversations", "The Morning Brew", "Urban Stories", "4:05"),
    ("Digital Rain", "Cyber Symphony", "Binary Beats", "3:30"),
];

fn open(label: &str) -> gui::Div {
    button(ButtonVariant::Outline, ButtonSize::Sm).child(label.to_owned())
}

/// An avatar with its picture, or its initials until there is one.
fn person(image: Option<&ImageSource>, initials: &str) -> gui::Div {
    avatar()
        .child(avatar_image(image))
        .child(avatar_fallback(image, initials))
}

/// The first letter of a login, as the demos' fallback (`username.charAt(0)`).
fn initial(login: &str) -> String {
    login.chars().take(1).collect()
}

/// `ItemDemo` and `ItemSizeDemo`: a basic item and a small link item.
fn sizes() -> gui::Div {
    div()
        .flex()
        .w_full()
        .max_w_px(448.0)
        .flex_col()
        .gap(6.0)
        .child(
            item(ItemVariant::Outline, ItemSize::Default)
                .child(
                    item_content()
                        .child(item_title("Basic Item"))
                        .child(item_description("A simple item with title and description.")),
                )
                .child(item_actions().child(open("Action"))),
        )
        .child(
            item_link(ItemVariant::Outline, ItemSize::Sm)
                .child(item_media(ItemMediaVariant::Default).child(icon(Icon::BadgeCheck).size(5.0)))
                .child(item_content().child(item_title("Your profile has been verified.")))
                .child(item_actions().child(icon(Icon::ChevronRight))),
        )
}

pub fn render() -> Render {
    let photos: Vec<(&str, Option<ImageSource>)> = ["shadcn", "maxleiter", "evilrabbit"]
        .into_iter()
        .map(|login| (login, photo(login)))
        .collect();
    let models = MODELS.map(|(_, _, id)| photo(id));
    let covers = MUSIC.map(|(title, ..)| photo(title));
    let select = MenuState::new();
    Box::new(move || {
        let t = theme();
        let image_of = |login: &str| photos.iter().find(|(l, _)| *l == login).and_then(|(_, p)| p.as_ref());

        let icon_demo = div().flex().w_full().max_w_px(512.0).flex_col().gap(6.0).child(
            item(ItemVariant::Outline, ItemSize::Default)
                .child(item_media_described(ItemMediaVariant::Icon).child(icon(Icon::ShieldAlert)))
                .child(
                    item_content()
                        .child(item_title("Security Alert"))
                        .child(item_description("New login detected from unknown device.")),
                )
                .child(item_actions().child(open("Review"))),
        );
        let variants = div().flex_col().gap(6.0).children(
            [
                (
                    ItemVariant::Default,
                    "Default Variant",
                    "Standard styling with subtle background and borders.",
                ),
                (
                    ItemVariant::Outline,
                    "Outline Variant",
                    "Outlined style with clear borders and transparent background.",
                ),
                (
                    ItemVariant::Muted,
                    "Muted Variant",
                    "Subdued appearance with muted colors for secondary content.",
                ),
            ]
            .map(|(v, title, description)| {
                item(v, ItemSize::Default)
                    .child(
                        item_content()
                            .child(item_title(title))
                            .child(item_description(description)),
                    )
                    .child(item_actions().child(open("Open")))
            }),
        );

        let rabbit = image_of("evilrabbit");
        let avatar_demo = div()
            .flex()
            .w_full()
            .max_w_px(512.0)
            .flex_col()
            .gap(6.0)
            .child(
                item(ItemVariant::Outline, ItemSize::Default)
                    .child(item_media_described(ItemMediaVariant::Default).child(person(rabbit, "ER").size(10.0)))
                    .child(
                        item_content()
                            .child(item_title("Evil Rabbit"))
                            .child(item_description("Last seen 5 months ago")),
                    )
                    .child(
                        item_actions().child(
                            button(ButtonVariant::Outline, ButtonSize::IconSm)
                                .rounded_full()
                                .child(icon(Icon::Plus)),
                        ),
                    ),
            )
            .child(
                item(ItemVariant::Outline, ItemSize::Default)
                    .child(
                        item_media_described(ItemMediaVariant::Default).child(
                            // `-space-x-2`, and each avatar's `ring-2 ring-background grayscale`.
                            div().flex().children(
                                [("shadcn", "CN"), ("maxleiter", "LR"), ("evilrabbit", "ER")]
                                    .into_iter()
                                    .enumerate()
                                    .map(|(i, (login, initials))| {
                                        person(image_of(login), initials)
                                            .ring(2.0, t.background)
                                            .grayscale()
                                            .when(i > 0, |d| d.ml(-2.0))
                                    }),
                            ),
                        ),
                    )
                    .child(
                        item_content()
                            .child(item_title("No Team Members"))
                            .child(item_description("Invite your team to collaborate on this project.")),
                    )
                    .child(item_actions().child(open("Invite"))),
            );

        let last = PEOPLE.len().saturating_sub(1);
        let group = div()
            .flex()
            .w_full()
            .max_w_px(448.0)
            .flex_col()
            .gap(6.0)
            .child(
                item_group().children(PEOPLE.iter().enumerate().flat_map(|(i, (login, email))| {
                    let row = item(ItemVariant::Default, ItemSize::Default)
                        .child(
                            item_media_described(ItemMediaVariant::Default)
                                .child(person(image_of(login), &initial(login)).grayscale()),
                        )
                        .child(
                            item_content()
                                .gap(1.0)
                                .child(item_title(*login))
                                .child(item_description(*email)),
                        )
                        .child(
                            item_actions().child(
                                button(ButtonVariant::Ghost, ButtonSize::Icon)
                                    .rounded_full()
                                    .child(icon(Icon::Plus)),
                            ),
                        );
                    [
                        Some(row.into_element()),
                        (i != last).then(|| item_separator().into_element()),
                    ]
                    .into_iter()
                    .flatten()
                })),
            );

        let menu = dropdown_menu_content(&select)
            .w(72.0)
            .radius_var(0.65 * 16.0)
            .children(PEOPLE.iter().map(|(login, email)| {
                dropdown_menu_item(&select, login, MenuItem::default(), || {})
                    .p(0.0)
                    .child(
                        item(ItemVariant::Default, ItemSize::Sm)
                            .w_full()
                            .p(2.0)
                            .child(
                                item_media_described(ItemMediaVariant::Default).child(
                                    avatar()
                                        .size(8.0)
                                        .child(avatar_image(image_of(login)).map(Styled::grayscale))
                                        .child(avatar_fallback(image_of(login), initial(login))),
                                ),
                            )
                            .child(
                                item_content()
                                    .gap(0.5)
                                    .child(item_title(*login))
                                    .child(item_description(*email)),
                            ),
                    )
            }));
        let dropdown = div()
            .flex()
            .min_h(64.0)
            .w_full()
            .max_w_px(448.0)
            .flex_col()
            .items_center()
            .gap(6.0)
            .child(dropdown_menu_trigger(
                &select,
                button(ButtonVariant::Outline, ButtonSize::Sm)
                    .w_fit()
                    .child("Select")
                    .child(icon(Icon::ChevronDown)),
                Side::Bottom,
                SideAlign::End,
                menu,
            ));

        let header = div().flex().w_full().max_w_px(576.0).flex_col().gap(6.0).child(
            item_group()
                .grid()
                .grid_cols(3)
                .gap(4.0)
                .children(MODELS.iter().zip(&models).map(|((name, description, _), cover)| {
                    item_link(ItemVariant::Outline, ItemSize::Default)
                        .child(
                            item_header().child(
                                cover
                                    .clone()
                                    .map(|c| image(c).aspect_square().w_full().rounded_sm().grayscale()),
                            ),
                        )
                        .child(
                            item_content()
                                .child(item_title(*name))
                                .child(item_description(*description)),
                        )
                })),
        );

        let images = div().flex().w_full().max_w_px(448.0).flex_col().gap(6.0).child(
            item_group().gap(4.0).children(MUSIC.iter().zip(&covers).map(
                |((title, artist, album, duration), cover)| {
                    item_link(ItemVariant::Outline, ItemSize::Default)
                        .child(
                            item_media_described(ItemMediaVariant::Image)
                                .child(cover.clone().map(|c| image(c).size_full().grayscale())),
                        )
                        .child(
                            item_content()
                                .child(
                                    item_title(
                                        text(format!("{title} - ")).span(span(*album).text_color(t.muted_foreground)),
                                    )
                                    .line_clamp(1),
                                )
                                .child(item_description(*artist)),
                        )
                        .child(
                            item_content()
                                .flex_none()
                                .text_center()
                                .child(item_description(*duration)),
                        )
                },
            )),
        );

        let links = div()
            .flex()
            .w_full()
            .max_w_px(448.0)
            .flex_col()
            .gap(4.0)
            .child(
                item_link(ItemVariant::Default, ItemSize::Default)
                    .child(
                        item_content()
                            .child(item_title("Visit our documentation"))
                            .child(item_description("Learn how to get started with our components.")),
                    )
                    .child(item_actions().child(icon(Icon::ChevronRight))),
            )
            .child(
                item_link(ItemVariant::Outline, ItemSize::Default)
                    .child(
                        item_content()
                            .child(item_title("External resource"))
                            .child(item_description("Opens in a new tab with security attributes.")),
                    )
                    .child(item_actions().child(icon(Icon::ExternalLink))),
            );

        page(
            "Item",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo("ItemDropdownDemo.vue", dropdown))
                .child(demo("ItemDemo.vue", sizes()))
                .child(demo("ItemIconDemo.vue", icon_demo))
                .child(demo("ItemVariantDemo.vue", variants))
                .child(demo("ItemAvatarDemo.vue", avatar_demo))
                .child(demo("ItemGroupDemo.vue", group))
                .child(demo("ItemHeaderDemo.vue", header))
                .child(demo("ItemImageDemo.vue", images))
                .child(demo("ItemLinkDemo.vue", links))
                .child(demo("ItemSizeDemo.vue", sizes())),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.click_text("Select")?;
    s.frame("people menu open")?;
    // The menu's second row; the page shows the same people further down.
    let (x, y) = s
        .last()
        .nodes
        .iter()
        .filter(|n| n.name == Some("dropdown-menu-item"))
        .nth(1)
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound("dropdown-menu-item".to_owned()))?;
    s.hover(x, y);
    s.frame("hover maxleiter")?;
    s.click(x, y);
    s.frame("menu closed after pick")?;
    let (x, y) = s.locate("Your profile has been verified.")?;
    s.hover(x, y);
    s.frame("hover link item")?;
    Ok(())
}
