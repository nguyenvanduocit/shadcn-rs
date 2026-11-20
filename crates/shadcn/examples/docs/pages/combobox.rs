use gui::prelude::*;
use gui::{Icon, ImageSource, Modifiers, NamedKey, ScenarioError, SideAlign, Signal, div, icon, text};
use shadcn::{
    ButtonSize, ButtonVariant, ComboboxState, CommandState, PopoverState, avatar, avatar_fallback, avatar_image,
    button, combobox_empty, combobox_group, combobox_input, combobox_item, combobox_item_indicator, combobox_item_null,
    combobox_list, combobox_separator, combobox_trigger, combobox_trigger_aligned, combobox_viewport, command,
    command_empty, command_group, command_input, command_item, command_list, popover, popover_content, popover_trigger,
    theme,
};

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page, photo};

const FRAMEWORKS: [&str; 5] = ["Next.js", "SvelteKit", "Nuxt.js", "Remix", "Astro"];

/// ComboboxWithListboxDemo's frameworks: the value the search matches, the label shown.
const FRAMEWORK_VALUES: [(&str, &str); 5] = [
    ("next.js", "Next.js"),
    ("sveltekit", "SvelteKit"),
    ("nuxt.js", "Nuxt.js"),
    ("remix", "Remix"),
    ("astro", "Astro"),
];

/// ComboboxTimezoneDemo's regions and their zones' labels (`by="label"`).
const TIMEZONES: [(&str, [&str; 6]); 3] = [
    (
        "Americas",
        [
            "(GMT-5) New York",
            "(GMT-8) Los Angeles",
            "(GMT-6) Chicago",
            "(GMT-5) Toronto",
            "(GMT-8) Vancouver",
            "(GMT-3) São Paulo",
        ],
    ),
    (
        "Europe",
        [
            "(GMT+0) London",
            "(GMT+1) Paris",
            "(GMT+1) Berlin",
            "(GMT+1) Rome",
            "(GMT+1) Madrid",
            "(GMT+1) Amsterdam",
        ],
    ),
    (
        "Asia/Pacific",
        [
            "(GMT+9) Tokyo",
            "(GMT+8) Shanghai",
            "(GMT+8) Singapore",
            "(GMT+4) Dubai",
            "(GMT+11) Sydney",
            "(GMT+9) Seoul",
        ],
    ),
];

/// ComboboxUserDemo's users, by GitHub login.
const USERS: [&str; 3] = ["shadcn", "leerob", "evilrabbit"];

pub fn render() -> Render {
    let framework = ComboboxState::new(None);
    let frameworks = ComboboxState::multiple(&[]);
    let timezone = ComboboxState::new(Some(TIMEZONES[0].1[0]));
    let user = ComboboxState::new(None);
    let photos: Vec<(&str, Option<ImageSource>)> = USERS.iter().map(|u| (*u, photo(u))).collect();
    let listbox = PopoverState::new();
    let listbox_search = CommandState::new();
    let listbox_value = Signal::labeled("listbox framework", None::<String>);
    Box::new(move || {
        let t = theme();
        // `Avatar class="size-5"` with the user's photo, or their initial.
        let face = |login: &str| {
            let source = photos.iter().find(|(u, _)| *u == login).and_then(|(_, p)| p.as_ref());
            avatar()
                .size(5.0)
                .child(avatar_image(source))
                .child(avatar_fallback(source, login.chars().take(1).collect::<String>()))
        };
        let outline = || button(ButtonVariant::Outline, ButtonSize::Default);

        let cb = &framework;
        let list = combobox_list(cb)
            .child(combobox_input(cb, "Search framework..."))
            .child(combobox_empty(cb, "No framework found."))
            .child(combobox_group(
                None,
                FRAMEWORKS.map(|label| {
                    combobox_item(cb, label).map(|i| i.child(label).child(combobox_item_indicator(cb, label)))
                }),
            ));
        let chosen = cb.value.get();
        let trigger = outline()
            .w_px(200.0)
            .justify_between()
            .child(chosen.unwrap_or_else(|| String::from("Select framework...")))
            .child(icon(Icon::ChevronsUpDown).opacity(0.5));

        let many = &frameworks;
        let picked = many.value.get();
        // The demo's checkbox look: `size-4 rounded-[4px] border`, filled when selected.
        let tick = |on: bool| {
            div()
                .size(4.0)
                .shrink_0()
                .rounded(4.0)
                .border()
                .border_color(if on { t.primary } else { t.input })
                .when(on, |d| {
                    d.bg(t.primary)
                        .text_color(t.primary_foreground)
                        .child(icon(Icon::Check).size(3.5))
                })
        };
        let many_list = combobox_list(many)
            .w_px(280.0)
            .child(combobox_input(many, "Search framework..."))
            .child(combobox_empty(many, "No framework found."))
            .child(combobox_group(
                None,
                FRAMEWORKS.map(|label| {
                    combobox_item(many, label).map(|i| i.child(tick(picked.iter().any(|p| p == label))).child(label))
                }),
            ));
        let many_trigger = outline()
            .w_px(280.0)
            .justify_between()
            .child(
                text(if picked.is_empty() {
                    String::from("Select frameworks...")
                } else {
                    picked.join(", ")
                })
                .truncate(),
            )
            .child(icon(Icon::ChevronsUpDown).opacity(0.5));

        let tz = &timezone;
        let zone = tz.value.get();
        let region = zone
            .as_deref()
            .and_then(|z| TIMEZONES.iter().find(|(_, zones)| zones.contains(&z)))
            .map(|(region, _)| *region);
        let tz_trigger = outline()
            .h(12.0)
            .w_px(200.0)
            .justify_between()
            .px(2.5)
            .child(zone.map_or_else(
                || div().child("Select timezone"),
                |z| {
                    div()
                        .flex_col()
                        .items_start()
                        .gap(0.5)
                        .child(
                            text(region.unwrap_or_default())
                                .text_color(t.muted_foreground)
                                .text_xs()
                                .font_normal(),
                        )
                        .child(text(z))
                },
            ))
            .child(icon(Icon::ChevronDown).text_color(t.muted_foreground));
        let tz_list = combobox_list(tz)
            .w(72.0)
            .child(combobox_input(tz, "Search timezone..."))
            .child(
                combobox_viewport()
                    .max_h_px(260.0)
                    .child(combobox_empty(tz, "No timezone found."))
                    .children(TIMEZONES.map(|(region, zones)| {
                        combobox_group(
                            Some(region),
                            zones.map(|z| {
                                combobox_item(tz, z).map(|i| i.child(z).child(combobox_item_indicator(tz, z)))
                            }),
                        )
                    })),
            )
            .child(combobox_separator())
            .child(
                combobox_group(
                    None,
                    [combobox_item_null(tz, "Create timezone").map(|i| {
                        i.child(icon(Icon::PlusCircle).text_color(t.muted_foreground))
                            .child("Create timezone")
                    })],
                )
                .map(|g| g.bg(t.popover)),
            );

        let u = &user;
        let who = u.value.get();
        let user_trigger = outline()
            .w_px(200.0)
            .justify_between()
            .child(who.map_or_else(
                || div().child("Select user..."),
                |login| div().flex().items_center().gap(2.0).child(face(&login)).child(login),
            ))
            .child(icon(Icon::ChevronsUpDown).opacity(0.5));
        let user_list = combobox_list(u)
            .child(combobox_input(u, "Search user..."))
            .child(combobox_empty(u, "No user found."))
            .child(combobox_group(
                None,
                USERS.map(|login| {
                    combobox_item(u, login).map(|i| {
                        i.child(face(login))
                            .child(login)
                            .child(combobox_item_indicator(u, login))
                    })
                }),
            ))
            .child(combobox_separator())
            .child(combobox_group(
                None,
                [combobox_item_null(u, "Create user").map(|i| {
                    i.child(icon(Icon::PlusCircle).text_color(t.muted_foreground))
                        .child("Create user")
                })],
            ));

        let current = listbox_value.get();
        let listbox_label = FRAMEWORK_VALUES
            .iter()
            .find(|(v, _)| current.as_deref() == Some(*v))
            .map_or("Select framework...", |(_, l)| *l);
        // The command's root first: it starts the frame its items register in.
        let listbox_root = command(&listbox_search);
        let listbox_items = FRAMEWORK_VALUES.map(|(value, label)| {
            let (pick, close) = (listbox_value.clone(), listbox.clone());
            command_item(&listbox_search, value, false, move || {
                pick.update(|v| (v.as_deref() != Some(value)).then(|| value.to_owned()));
                close.close();
            })
            .map(|i| {
                i.child(label).child(
                    icon(Icon::Check)
                        .ml_auto()
                        .opacity(if current.as_deref() == Some(value) { 1.0 } else { 0.0 }),
                )
            })
        });
        let with_listbox = popover()
            .child(popover_trigger(
                &listbox,
                outline()
                    .w_px(200.0)
                    .justify_between()
                    .child(listbox_label)
                    .child(icon(Icon::ChevronsUpDown).opacity(0.5)),
            ))
            .child(popover_content(&listbox, |c| {
                c.w_px(200.0).p(0.0).child(
                    listbox_root
                        .child(command_input(&listbox_search, "Search framework..."))
                        .child(
                            command_list()
                                .child(command_empty(&listbox_search, "No framework found."))
                                .child(command_group(None, listbox_items)),
                        ),
                )
            }));

        page(
            "Combobox",
            div()
                .flex_col()
                .gap(6.0)
                .child(demo("ComboboxDemo.vue", combobox_trigger(cb, trigger, list)))
                .child(demo(
                    "ComboboxMultipleDemo.vue",
                    combobox_trigger_aligned(many, many_trigger, SideAlign::Start, many_list),
                ))
                .child(demo(
                    "ComboboxTimezoneDemo.vue",
                    combobox_trigger_aligned(tz, tz_trigger, SideAlign::Start, tz_list),
                ))
                .child(demo(
                    "ComboboxUserDemo.vue",
                    combobox_trigger(u, user_trigger, user_list),
                ))
                .child(demo("ComboboxWithListboxDemo.vue", with_listbox)),
        )
    })
}

fn key(s: &mut Shot, k: NamedKey) {
    s.press(k, Modifiers::default());
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    motion_frames(s, "opening", &MOTION_MS, |s| s.click_text("Select framework..."))?;
    motion_frames(s, "closing", &MOTION_MS, |s| {
        s.press(NamedKey::Escape, Modifiers::default());
        Ok(())
    })?;
    s.click_text("Select framework...")?;
    s.frame("open, first highlighted")?;
    let (x, y) = s.locate("Remix")?;
    s.hover(x, y);
    s.frame("hover remix")?;
    s.type_text("n");
    s.frame("typed n")?;
    key(s, NamedKey::ArrowDown);
    s.frame("arrow down")?;
    key(s, NamedKey::Enter);
    s.frame("enter picks")?;
    s.click_text("Nuxt.js")?;
    s.settle()?;
    s.frame("reopen shows the check")?;
    s.type_text("zz");
    s.frame("no framework found")?;
    key(s, NamedKey::Escape);
    s.frame("escape closes")?;
    // `multiple`: each pick toggles its box and the list stays open.
    s.click_text("Select frameworks...")?;
    s.settle()?;
    s.click_text("SvelteKit")?;
    s.settle()?;
    s.click_text("Astro")?;
    s.frame("multiple: two ticked, still open")?;
    key(s, NamedKey::Escape);
    s.frame("multiple trigger lists both")?;
    s.click_text("(GMT-5) New York")?;
    s.frame("timezones open on new york")?;
    s.type_text("tok");
    s.settle()?;
    s.click_text("(GMT+9) Tokyo")?;
    s.frame("tokyo under asia/pacific")?;
    s.click_text("Select user...")?;
    s.frame("users with their photos")?;
    s.click_text("leerob")?;
    s.frame("leerob chosen")?;
    s.click_text("leerob")?;
    s.settle()?;
    s.click_text("Create user")?;
    s.frame("create user clears the choice")?;
    s.click_text("Select framework...")?;
    s.frame("listbox popover open")?;
    s.click_text("Remix")?;
    s.frame("listbox picks remix")?;
    Ok(())
}
