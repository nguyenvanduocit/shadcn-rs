use std::rc::Rc;

use gui::prelude::*;
use gui::{Icon, ImageSource, Modifiers, NamedKey, ScenarioError, SideAlign, Signal, div, icon, text};
use shadcn::{
    ButtonSize, ButtonVariant, CommandItem, CommandSection, avatar, avatar_fallback, avatar_image, button, combobox,
    combobox_input, combobox_item_indicator, combobox_items, combobox_list, combobox_trigger, command, command_input,
    command_list, popover, popover_content, popover_trigger, theme,
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
    let framework = Signal::labeled("framework", None::<String>);
    let frameworks = Signal::labeled("frameworks", Vec::<String>::new());
    let timezone = Signal::labeled("timezone", Some(TIMEZONES[0].1[0].to_owned()));
    let user = Signal::labeled("user", None::<String>);
    let photos: Rc<Vec<(&str, Option<ImageSource>)>> = Rc::new(USERS.iter().map(|u| (*u, photo(u))).collect());
    let listbox = Signal::labeled("listbox open", false);
    let listbox_value = Signal::labeled("listbox framework", None::<String>);
    // The lists' data, once: the items are their values.
    let frameworks_list = Rc::new(vec![
        CommandSection::empty("No framework found."),
        CommandSection::group(None, FRAMEWORKS.map(CommandItem::new)),
    ]);
    let timezones_list: Rc<Vec<CommandSection>> = Rc::new(
        std::iter::once(CommandSection::empty("No timezone found."))
            .chain(TIMEZONES.map(|(region, zones)| CommandSection::group(Some(region), zones.map(CommandItem::new))))
            .collect(),
    );
    let create_timezone = Rc::new(vec![
        CommandSection::Separator,
        CommandSection::group(None, [CommandItem::null("Create timezone")]),
    ]);
    let users_list = Rc::new(vec![
        CommandSection::empty("No user found."),
        CommandSection::group(None, USERS.map(CommandItem::new)),
        CommandSection::Separator,
        CommandSection::group(None, [CommandItem::null("Create user")]),
    ]);
    let listbox_list = Rc::new(vec![
        CommandSection::empty("No framework found."),
        CommandSection::group(None, FRAMEWORK_VALUES.map(|(value, _)| CommandItem::new(value))),
    ]);
    Box::new(move || {
        let t = theme();
        // `Avatar class="size-5"` with the user's photo, or their initial.
        let face = {
            let photos = Rc::clone(&photos);
            move |login: &str| {
                let source = photos.iter().find(|(u, _)| *u == login).and_then(|(_, p)| p.as_ref());
                avatar()
                    .size(5.0)
                    .child(avatar_image(source))
                    .child(avatar_fallback(source, login.chars().take(1).collect::<String>()))
            }
        };
        let outline = || button(ButtonVariant::Outline, ButtonSize::Default);

        let chosen = framework.get();
        let demo_combobox = combobox()
            .model(framework)
            .child(combobox_trigger(
                outline()
                    .w_px(200.0)
                    .justify_between()
                    .child(chosen.unwrap_or_else(|| String::from("Select framework...")))
                    .child(icon(Icon::ChevronsUpDown).opacity(0.5)),
            ))
            .child(
                combobox_list()
                    .child(combobox_input("Search framework..."))
                    .child(combobox_items(Rc::clone(&frameworks_list), |item, row| {
                        row.child(item.value.clone())
                            .child(combobox_item_indicator(&item.value))
                    })),
            );

        let picked = frameworks.get();
        // The demo's checkbox look: `size-4 rounded-[4px] border`, filled when selected.
        let tick = move |on: bool| {
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
        let many_combobox = combobox()
            .model(frameworks)
            .child(combobox_trigger(
                outline()
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
                    .child(icon(Icon::ChevronsUpDown).opacity(0.5)),
            ))
            .child(
                combobox_list()
                    .align(SideAlign::Start)
                    .w_px(280.0)
                    .child(combobox_input("Search framework..."))
                    .child(combobox_items(Rc::clone(&frameworks_list), move |item, row| {
                        row.child(tick(frameworks.with(|p| p.contains(&item.value))))
                            .child(item.value.clone())
                    })),
            );

        let zone = timezone.get();
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
        let tz_combobox = combobox().model(timezone).child(combobox_trigger(tz_trigger)).child(
            combobox_list()
                .align(SideAlign::Start)
                .w(72.0)
                .child(combobox_input("Search timezone..."))
                .child(
                    combobox_items(Rc::clone(&timezones_list), move |item, row| {
                        if item.null {
                            return row
                                .child(icon(Icon::PlusCircle).text_color(t.muted_foreground))
                                .child(item.value.clone());
                        }
                        row.child(item.value.clone())
                            .child(combobox_item_indicator(&item.value))
                    })
                    .max_h_px(260.0)
                    .footer(Rc::clone(&create_timezone)),
                ),
        );

        let who = user.get();
        let user_trigger = outline()
            .w_px(200.0)
            .justify_between()
            .child(who.map_or_else(
                || div().child("Select user..."),
                |login| div().flex().items_center().gap(2.0).child(face(&login)).child(login),
            ))
            .child(icon(Icon::ChevronsUpDown).opacity(0.5));
        let user_combobox = combobox().model(user).child(combobox_trigger(user_trigger)).child(
            combobox_list()
                .child(combobox_input("Search user..."))
                .child(combobox_items(Rc::clone(&users_list), move |item, row| {
                    if item.null {
                        return row
                            .child(icon(Icon::PlusCircle).text_color(t.muted_foreground))
                            .child(item.value.clone());
                    }
                    row.child(face(&item.value))
                        .child(item.value.clone())
                        .child(combobox_item_indicator(&item.value))
                })),
        );

        let current = listbox_value.get();
        let listbox_label = FRAMEWORK_VALUES
            .iter()
            .find(|(v, _)| current.as_deref() == Some(*v))
            .map_or("Select framework...", |(_, l)| *l);
        let listbox_items = command_list(Rc::clone(&listbox_list), move |item, row| {
            let label = FRAMEWORK_VALUES
                .iter()
                .find(|(v, _)| *v == item.value)
                .map_or("", |(_, l)| *l);
            let on = listbox_value.with(|v| v.as_deref() == Some(item.value.as_str()));
            row.child(label)
                .child(icon(Icon::Check).ml_auto().opacity(if on { 1.0 } else { 0.0 }))
        })
        .on_select(move |item| {
            let value = item.value.clone();
            listbox_value.update(|v| *v = (v.as_deref() != Some(value.as_str())).then(|| value.clone()));
            listbox.set(false);
        });
        let with_listbox = popover()
            .open(listbox)
            .child(popover_trigger(
                outline()
                    .w_px(200.0)
                    .justify_between()
                    .child(listbox_label)
                    .child(icon(Icon::ChevronsUpDown).opacity(0.5)),
            ))
            .child(
                popover_content().w_px(200.0).p(0.0).child(
                    command()
                        .child(command_input("Search framework..."))
                        .child(listbox_items),
                ),
            );

        page(
            "Combobox",
            div()
                .flex_col()
                .gap(6.0)
                .child(demo("ComboboxDemo.vue", demo_combobox))
                .child(demo("ComboboxMultipleDemo.vue", many_combobox))
                .child(demo("ComboboxTimezoneDemo.vue", tz_combobox))
                .child(demo("ComboboxUserDemo.vue", user_combobox))
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
