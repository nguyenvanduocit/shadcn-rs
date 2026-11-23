use gui::prelude::*;
use gui::{Div, Element, Icon, ScenarioError, SideAlign, Signal, TextField, Track, div, icon};
use shadcn::{
    ButtonSize, ButtonVariant, CheckedState, MenuItem, button, checkbox, dropdown_menu, dropdown_menu_checkbox_item,
    dropdown_menu_content, dropdown_menu_item, dropdown_menu_label, dropdown_menu_separator, dropdown_menu_trigger,
    input, table, table_body, table_caption, table_cell, table_empty, table_footer, table_head, table_header,
    table_row, theme,
};

use crate::{Render, Shot, demo, page, section};

const INVOICES: [(&str, &str, &str, &str); 7] = [
    ("INV001", "Paid", "Credit Card", "$250.00"),
    ("INV002", "Pending", "PayPal", "$150.00"),
    ("INV003", "Unpaid", "Bank Transfer", "$350.00"),
    ("INV004", "Paid", "Credit Card", "$450.00"),
    ("INV005", "Paid", "PayPal", "$550.00"),
    ("INV006", "Pending", "Bank Transfer", "$200.00"),
    ("INV007", "Unpaid", "Credit Card", "$300.00"),
];

/// `DataTableDemo.vue`'s payments: id, amount in dollars, status, email.
const PAYMENTS: [(&str, u32, &str, &str); 5] = [
    ("m5gr84i9", 316, "success", "ken99@yahoo.com"),
    ("3u1reuv4", 242, "success", "Abe45@gmail.com"),
    ("derv1ws0", 837, "processing", "Monserrat44@gmail.com"),
    ("5kma53ae", 874, "success", "Silas22@gmail.com"),
    ("bhqecj4p", 721, "failed", "carmella@hotmail.com"),
];

/// TanStack's default `pageSize`.
const PAGE_SIZE: usize = 10;

/// The columns, in order, and whether the "Columns" menu can hide each.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Column {
    Select,
    Status,
    Email,
    Amount,
    Actions,
}

impl Column {
    const ALL: [Self; 5] = [Self::Select, Self::Status, Self::Email, Self::Amount, Self::Actions];

    /// Its share of the row. HTML's auto table layout gives each column its
    /// widest cell and shares the rest by those widths, which gui's grid
    /// cannot measure; these are the widths Chrome lays the demo out at.
    fn track(self) -> Track {
        Track::Fr(match self {
            Self::Select => 44.0,
            Self::Status => 164.1,
            Self::Email => 335.8,
            Self::Amount => 134.1,
            Self::Actions => 88.0,
        })
    }
}

/// What the data table keeps: TanStack's filter, sorting, selection,
/// visibility and page state, and each row's actions menu.
struct DataTable {
    filter: TextField,
    /// `Some(true)` sorts emails descending.
    sorted_desc: Signal<Option<bool>>,
    selected: Signal<Vec<&'static str>>,
    status: Signal<bool>,
    email: Signal<bool>,
    amount: Signal<bool>,
    page: Signal<usize>,
}

impl DataTable {
    fn new() -> Self {
        Self {
            filter: TextField::new(""),
            sorted_desc: Signal::labeled("email sorting", None),
            selected: Signal::labeled("selected payments", Vec::new()),
            status: Signal::labeled("status visible", true),
            email: Signal::labeled("email visible", true),
            amount: Signal::labeled("amount visible", true),
            page: Signal::labeled("page", 0),
        }
    }

    fn visible(&self, c: Column) -> bool {
        match c {
            Column::Status => self.status.get(),
            Column::Email => self.email.get(),
            Column::Amount => self.amount.get(),
            Column::Select | Column::Actions => true,
        }
    }

    /// The rows the filter keeps (`includesString`: any case), sorted.
    fn filtered(&self) -> Vec<usize> {
        let needle = self.filter.text().to_lowercase();
        let mut rows: Vec<usize> = (0..PAYMENTS.len())
            .filter(|&i| PAYMENTS.get(i).is_some_and(|p| p.3.to_lowercase().contains(&needle)))
            .collect();
        if let Some(desc) = self.sorted_desc.get() {
            let email = |i: &usize| PAYMENTS.get(*i).map(|p| p.3.to_lowercase());
            rows.sort_by(|a, b| {
                let o = email(a).cmp(&email(b));
                if desc { o.reverse() } else { o }
            });
        }
        rows
    }

    fn is_selected(&self, id: &str) -> bool {
        self.selected.get().contains(&id)
    }
}

/// Select (or clear) every payment in `ids` (TanStack's `toggleSelected`).
fn select(selected: &Signal<Vec<&'static str>>, ids: &[&'static str], on: bool) {
    selected.update(|all| {
        all.retain(|id| !ids.contains(id));
        if on {
            all.extend_from_slice(ids);
        }
    });
}

/// `$316.00`: `Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' })`.
fn dollars(amount: u32) -> String {
    format!("${amount}.00")
}

/// CSS `capitalize`: the first letter upper case.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn data_table(dt: &DataTable) -> Div {
    let t = theme();
    let columns: Vec<Column> = Column::ALL.into_iter().filter(|&c| dt.visible(c)).collect();
    let tracks: Vec<Track> = columns.iter().map(|c| c.track()).collect();
    let rows = dt.filtered();
    let start = dt.page.get().saturating_mul(PAGE_SIZE);
    let page: Vec<usize> = rows.iter().copied().skip(start).take(PAGE_SIZE).collect();
    let page_ids: Vec<&'static str> = page.iter().filter_map(|&i| PAYMENTS.get(i).map(|p| p.0)).collect();
    let picked = page_ids.iter().filter(|id| dt.is_selected(id)).count();
    let all_state = if picked > 0 && picked == page_ids.len() {
        CheckedState::Checked
    } else if picked > 0 {
        CheckedState::Indeterminate
    } else {
        CheckedState::Unchecked
    };

    let head = |c: Column| -> Element {
        match c {
            Column::Select => {
                let set = dt.selected;
                let ids = page_ids.clone();
                table_head(
                    checkbox()
                        .checked(all_state)
                        .on_change(move |on| select(&set, &ids, on)),
                )
                .padding_right(0.0)
                .into_element()
            }
            Column::Status => table_head("Status").into_element(),
            Column::Email => {
                let sorted = dt.sorted_desc;
                table_head(
                    button(ButtonVariant::Ghost, ButtonSize::Default)
                        .on_click(move |_| sorted.update(|s| *s = Some(*s == Some(false))))
                        .child("Email")
                        .child(icon(Icon::ArrowUpDown).margin_left(8.0).size(16.0)),
                )
                .align_items(Align::Start)
                .into_element()
            }
            Column::Amount => table_head(div().text_align(TextAlign::Right).child("Amount")).into_element(),
            Column::Actions => table_head(Element::Empty).into_element(),
        }
    };
    let cell = |c: Column, i: usize| -> Element {
        let Some(&(id, amount, status, email)) = PAYMENTS.get(i) else {
            return Element::Empty;
        };
        match c {
            Column::Select => {
                let set = dt.selected;
                table_cell(
                    checkbox()
                        .checked(dt.is_selected(id))
                        .on_change(move |on| select(&set, &[id], on)),
                )
                .padding_right(0.0)
                .align_items(Align::Start)
                .into_element()
            }
            Column::Status => table_cell(capitalize(status)).into_element(),
            Column::Email => table_cell(email.to_lowercase()).into_element(),
            Column::Amount => table_cell(
                div()
                    .text_align(TextAlign::Right)
                    .font_weight(500)
                    .child(dollars(amount)),
            )
            .into_element(),
            Column::Actions => {
                let menu_for = |name: &'static str| dropdown_menu_item(name, MenuItem::default()).child(name);
                table_cell(
                    dropdown_menu()
                        .child(dropdown_menu_trigger(
                            button(ButtonVariant::Ghost, ButtonSize::Default)
                                .height(32.0)
                                .width(32.0)
                                .padding(0.0)
                                .child(icon(Icon::Ellipsis).size(16.0)),
                        ))
                        .child(
                            dropdown_menu_content()
                                .align(SideAlign::End)
                                .child(dropdown_menu_label("Actions", false))
                                .child(
                                    dropdown_menu_item("Copy payment ID", MenuItem::default())
                                        .on_select(move |_| {
                                            gui::clipboard::copy(id);
                                        })
                                        .child("Copy payment ID"),
                                )
                                .child(dropdown_menu_separator())
                                .child(menu_for("View customer"))
                                .child(menu_for("View payment details")),
                        ),
                )
                .align_items(Align::Start)
                .into_element()
            }
        }
    };

    let body = if page.is_empty() {
        // A row and a cell across every column (`colspan`, `h-24 text-center`), not `TableEmpty`.
        let span = u16::try_from(tracks.len()).unwrap_or(u16::MAX);
        vec![
            table_row(&tracks, false).child(
                table_cell("No results.")
                    .grid_column_span(span)
                    .height(96.0)
                    .text_align(TextAlign::Center),
            ),
        ]
    } else {
        page.iter()
            .map(|&i| {
                let selected = PAYMENTS.get(i).is_some_and(|p| dt.is_selected(p.0));
                table_row(&tracks, selected).children(columns.iter().map(|&c| cell(c, i)))
            })
            .collect()
    };
    let hideable = [("status", dt.status), ("email", dt.email), ("amount", dt.amount)];
    let can_prev = dt.page.get() > 0;
    let can_next = start.saturating_add(PAGE_SIZE) < rows.len();
    let (prev, next) = (dt.page, dt.page);
    div()
        .width(pct(100.0))
        .child(
            div()
                .display(Display::Flex)
                .align_items(Align::Center)
                .padding_y(16.0)
                .child(
                    input()
                        .field(dt.filter)
                        .max_width(384.0)
                        .placeholder("Filter emails..."),
                )
                .child(
                    dropdown_menu()
                        .margin_left(Length::Auto)
                        .child(dropdown_menu_trigger(
                            button(ButtonVariant::Outline, ButtonSize::Default)
                                .child("Columns")
                                .child(icon(Icon::ChevronDown).margin_left(8.0).size(16.0)),
                        ))
                        .child(dropdown_menu_content().align(SideAlign::End).children(
                            hideable.map(|(name, shown)| {
                                dropdown_menu_checkbox_item(&capitalize(name), false).model(shown)
                            }),
                        )),
                ),
        )
        .child(
            div().radius(8.0).border(1.0).border_color(t.border).child(
                table()
                    .child(table_header([
                        table_row(&tracks, false).children(columns.iter().map(|&c| head(c)))
                    ]))
                    .child(table_body(body)),
            ),
        )
        .child(
            div()
                .display(Display::Flex)
                .align_items(Align::Center)
                .justify_content(Justify::End)
                .gap(8.0)
                .padding_y(16.0)
                .child(
                    div()
                        .flex(Flex::grow(1.0))
                        .text_size(14.0)
                        .line_height(20.0)
                        .text_color(t.muted_foreground)
                        .child(format!(
                            "{} of {} row(s) selected.",
                            rows.iter()
                                .filter(|&&i| PAYMENTS.get(i).is_some_and(|p| dt.is_selected(p.0)))
                                .count(),
                            rows.len()
                        )),
                )
                .child(
                    div()
                        .display(Display::Flex)
                        .gap(8.0)
                        .child(
                            button(ButtonVariant::Outline, ButtonSize::Sm)
                                .disabled(!can_prev)
                                .on_click(move |_| prev.update(|p| *p = p.saturating_sub(1)))
                                .child("Previous"),
                        )
                        .child(
                            button(ButtonVariant::Outline, ButtonSize::Sm)
                                .disabled(!can_next)
                                .on_click(move |_| next.update(|p| *p = p.saturating_add(1)))
                                .child("Next"),
                        ),
                ),
        )
}

pub fn render() -> Render {
    let payments = DataTable::new();
    Box::new(move || {
        // `TableHead class="w-[100px]"`; HTML's auto table layout shares the
        // rest by each column's widest cell ("Pending", "Bank Transfer",
        // "$2,500.00" plus padding), which gui's grid cannot measure.
        let cols = [Track::Px(100.0), Track::Fr(70.0), Track::Fr(108.0), Track::Fr(86.0)];
        let header = || {
            table_header([table_row(&cols, false)
                .child(table_head("Invoice"))
                .child(table_head("Status"))
                .child(table_head("Method"))
                .child(table_head("Amount").text_align(TextAlign::Right))])
        };
        let invoices = |selected: Option<&str>| {
            table()
                .child(header())
                .child(table_body(INVOICES.map(|(invoice, status, method, amount)| {
                    table_row(&cols, selected == Some(invoice))
                        .child(table_cell(invoice).font_weight(500))
                        .child(table_cell(status))
                        .child(table_cell(method))
                        .child(table_cell(amount).text_align(TextAlign::Right))
                })))
                .child(table_footer([table_row(&cols, false)
                    .child(table_cell("Total").grid_column_span(3))
                    .child(table_cell("$2,500.00").text_align(TextAlign::Right))]))
                .child(table_caption("A list of your recent invoices."))
        };
        let empty = table()
            .child(header())
            .child(table_body([table_empty(&cols, "No results.")]));
        page(
            "Table",
            gui::div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo("DataTableDemo.vue", data_table(&payments)))
                .child(demo("TableDemo.vue", invoices(None)))
                .child(
                    gui::div()
                        .display(Display::Flex)
                        .align_items(Align::Start)
                        .gap(32.0)
                        .child(
                            gui::div()
                                .flex(Flex::grow(1.0))
                                .child(section("INV003 selected", invoices(Some("INV003")))),
                        )
                        .child(gui::div().flex(Flex::grow(1.0)).child(section("Empty", empty))),
                ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("monserrat44@gmail.com")?;
    s.hover(x, y);
    s.frame("hover row")?;
    let (_, y) = s.locate("ken99@yahoo.com")?;
    let (cx, cy) = s
        .last()
        .nodes
        .iter()
        .find(|n| n.name == Some("checkbox") && (n.bounds.y + n.bounds.height / 2.0 - y).abs() < 10.0)
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound("ken99's checkbox".to_owned()))?;
    s.click(cx, cy);
    s.frame("one row selected, header indeterminate")?;
    s.click_text("Email")?;
    s.frame("sorted by email ascending")?;
    s.click_text("Email")?;
    s.frame("sorted by email descending")?;
    let (fx, fy) = s.locate("Filter emails...")?;
    s.click(fx, fy);
    s.type_text("gmail");
    s.frame("filtered to gmail")?;
    s.type_text("zzz");
    s.frame("no results")?;
    s.click_text("Columns")?;
    s.frame("columns menu")?;
    // The menu's first item, "Status" (the tables' own headers say it too).
    let (mx, my) = s
        .last()
        .nodes
        .iter()
        .find(|n| n.name == Some("dropdown-menu-checkbox-item"))
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound("Status item".to_owned()))?;
    s.click(mx, my);
    s.frame("status hidden")?;
    Ok(())
}
