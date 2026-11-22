use gui::prelude::*;
use gui::{Icon, ImageSource, ScenarioError, SideAlign, div, icon, image, span, text};
use shadcn::{
    Button, ButtonSize, ButtonVariant, ItemMediaVariant, ItemSize, ItemVariant, MenuItem, avatar, avatar_fallback,
    avatar_image, button, dropdown_menu, dropdown_menu_content, dropdown_menu_item, dropdown_menu_trigger, item,
    item_actions, item_content, item_description, item_group, item_header, item_link, item_media, item_media_described,
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

fn open(label: &str) -> Button {
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
        .display(Display::Flex)
        .width(pct(100.0))
        .max_width(448.0)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(24.0)
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
                .child(item_media(ItemMediaVariant::Default).child(icon(Icon::BadgeCheck).size(20.0)))
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
    Box::new(move || {
        let t = theme();
        let image_of = |login: &str| photos.iter().find(|(l, _)| *l == login).and_then(|(_, p)| p.as_ref());

        let icon_demo = div()
            .display(Display::Flex)
            .width(pct(100.0))
            .max_width(512.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(24.0)
            .child(
                item(ItemVariant::Outline, ItemSize::Default)
                    .child(item_media_described(ItemMediaVariant::Icon).child(icon(Icon::ShieldAlert)))
                    .child(
                        item_content()
                            .child(item_title("Security Alert"))
                            .child(item_description("New login detected from unknown device.")),
                    )
                    .child(item_actions().child(open("Review"))),
            );
        let variants = div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(24.0)
            .children(
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
            .display(Display::Flex)
            .width(pct(100.0))
            .max_width(512.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(24.0)
            .child(
                item(ItemVariant::Outline, ItemSize::Default)
                    .child(item_media_described(ItemMediaVariant::Default).child(person(rabbit, "ER").size(40.0)))
                    .child(
                        item_content()
                            .child(item_title("Evil Rabbit"))
                            .child(item_description("Last seen 5 months ago")),
                    )
                    .child(
                        item_actions().child(
                            button(ButtonVariant::Outline, ButtonSize::IconSm)
                                .radius(9999.0)
                                .child(icon(Icon::Plus)),
                        ),
                    ),
            )
            .child(
                item(ItemVariant::Outline, ItemSize::Default)
                    .child(
                        item_media_described(ItemMediaVariant::Default).child(
                            // `-space-x-2`, and each avatar's `ring-2 ring-background grayscale`.
                            div().display(Display::Flex).children(
                                [("shadcn", "CN"), ("maxleiter", "LR"), ("evilrabbit", "ER")]
                                    .into_iter()
                                    .enumerate()
                                    .map(|(i, (login, initials))| {
                                        person(image_of(login), initials)
                                            .ring(Ring::new(2.0, t.background))
                                            .filters(Filter::Grayscale(1.0))
                                            .when(i > 0, |d| d.margin_left(-8.0))
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
            .display(Display::Flex)
            .width(pct(100.0))
            .max_width(448.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(24.0)
            .child(
                item_group().children(PEOPLE.iter().enumerate().flat_map(|(i, (login, email))| {
                    let row = item(ItemVariant::Default, ItemSize::Default)
                        .child(
                            item_media_described(ItemMediaVariant::Default)
                                .child(person(image_of(login), &initial(login)).filters(Filter::Grayscale(1.0))),
                        )
                        .child(
                            item_content()
                                .gap(4.0)
                                .child(item_title(*login))
                                .child(item_description(*email)),
                        )
                        .child(
                            item_actions().child(
                                button(ButtonVariant::Ghost, ButtonSize::Icon)
                                    .radius(9999.0)
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

        // `--radius: 0.65rem` on the content: its own corners (`rounded-md`),
        // its rows' (`rounded-sm`) and the items' (`rounded-md`) follow it.
        let r: f32 = 0.65 * 16.0;
        let menu = dropdown_menu_content()
            .align(SideAlign::End)
            .width(288.0)
            .radius(r * 0.8)
            .children(PEOPLE.iter().map(move |(login, email)| {
                dropdown_menu_item(login, MenuItem::default())
                    .padding(0.0)
                    .radius(r * 0.6)
                    .child(
                        item(ItemVariant::Default, ItemSize::Sm)
                            .width(pct(100.0))
                            .padding(8.0)
                            .radius(r * 0.8)
                            .child(
                                item_media_described(ItemMediaVariant::Default).child(
                                    avatar()
                                        .size(32.0)
                                        .child(avatar_image(image_of(login)).map(|e| e.filters(Filter::Grayscale(1.0))))
                                        .child(avatar_fallback(image_of(login), initial(login))),
                                ),
                            )
                            .child(
                                item_content()
                                    .gap(2.0)
                                    .child(item_title(*login))
                                    .child(item_description(*email)),
                            ),
                    )
            }));
        let dropdown = div()
            .display(Display::Flex)
            .min_height(256.0)
            .width(pct(100.0))
            .max_width(448.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .align_items(Align::Center)
            .gap(24.0)
            .child(
                dropdown_menu()
                    .child(dropdown_menu_trigger(
                        button(ButtonVariant::Outline, ButtonSize::Sm)
                            .width(Length::FitContent)
                            .child("Select")
                            .child(icon(Icon::ChevronDown)),
                    ))
                    .child(menu),
            );

        let header = div()
            .display(Display::Flex)
            .width(pct(100.0))
            .max_width(576.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(24.0)
            .child(
                item_group()
                    .display(Display::Grid)
                    .grid_columns(Tracks::repeat(3, Track::Fr(1.0)))
                    .gap(16.0)
                    .children(MODELS.iter().zip(&models).map(|((name, description, _), cover)| {
                        item_link(ItemVariant::Outline, ItemSize::Default)
                            .child(item_header().child(cover.clone().map(|c| {
                                image(c)
                                    .aspect_ratio(1.0)
                                    .width(pct(100.0))
                                    .radius(6.0)
                                    .filters(Filter::Grayscale(1.0))
                            })))
                            .child(
                                item_content()
                                    .child(item_title(*name))
                                    .child(item_description(*description)),
                            )
                    })),
            );

        let images = div()
            .display(Display::Flex)
            .width(pct(100.0))
            .max_width(448.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(24.0)
            .child(
                item_group().gap(16.0).children(MUSIC.iter().zip(&covers).map(
                    |((title, artist, album, duration), cover)| {
                        item_link(ItemVariant::Outline, ItemSize::Default)
                            .child(
                                item_media_described(ItemMediaVariant::Image).child(
                                    cover
                                        .clone()
                                        .map(|c| image(c).size(pct(100.0)).filters(Filter::Grayscale(1.0))),
                                ),
                            )
                            .child(
                                item_content()
                                    .child(
                                        item_title(
                                            text(format!("{title} - "))
                                                .span(span(*album).text_color(t.muted_foreground)),
                                        )
                                        .line_clamp(1),
                                    )
                                    .child(item_description(*artist)),
                            )
                            .child(
                                item_content()
                                    .flex(Flex::NONE)
                                    .text_align(TextAlign::Center)
                                    .child(item_description(*duration)),
                            )
                    },
                )),
            );

        let links = div()
            .display(Display::Flex)
            .width(pct(100.0))
            .max_width(448.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
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
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
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
