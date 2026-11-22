//! The nine cards Formisch, TanStack Form and vee-validate each have a demo
//! of (`Formisch*.vue`, `TanStackForm*.vue`, `VeeValidate*Demo.vue`): the same
//! fields and words, checked when each library checks them. A card holds
//! its fields ([`FormField`]) and draws itself with `view`.

use gui::prelude::*;
use gui::{Color, Div, Icon, Radius, ScenarioError, Signal, TextField, div, icon, text};

use crate::{Shot, demo};
use shadcn::{
    ButtonSize, ButtonVariant, Checks, FieldLegendVariant, FieldOrientation, FieldStack, FormEntry, FormField,
    FormValue, InputGroupAlign, InputGroupButtonSize, OnChange, SelectPosition, Toast, ToastKind, button, card,
    card_content, card_description, card_footer, card_header, card_title, checkbox, field, field_checkbox_group,
    field_choices, field_content, field_description, field_error, field_group, field_label, field_label_card,
    field_legend, field_separator, field_set, field_title, form_control, form_reset, form_submit, input, input_group,
    input_group_addon, input_group_button, input_group_input, input_group_text, input_group_textarea, radio_group,
    radio_group_item, select, select_content, select_item, select_separator, select_value, switch, textarea, theme,
};

const V: FieldOrientation = FieldOrientation::Vertical;
const H: FieldOrientation = FieldOrientation::Horizontal;

/// The form library a demo is written with.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Library {
    Formisch,
    TanStack,
    Vee,
}

impl Library {
    /// When its demos check a field besides a submit: Formisch validates on
    /// submit and revalidates on input; TanStack's `onSubmit` schema errors
    /// go at the field's next change; vee-validate checks on blur and on
    /// every change.
    pub const fn checks(self) -> Checks {
        match self {
            Self::Formisch => Checks {
                on_blur: false,
                on_change: OnChange::Recheck,
            },
            Self::TanStack => Checks {
                on_blur: false,
                on_change: OnChange::Clear,
            },
            Self::Vee => Checks {
                on_blur: true,
                on_change: OnChange::Check,
            },
        }
    }
}

/// A submitted value, as the toast shows it with `JSON.stringify(v, null, 2)`.
pub enum Json {
    Str(String),
    Bool(bool),
    Num(f64),
    List(Vec<Json>),
    Object(Vec<(&'static str, Json)>),
}

impl Json {
    fn write(&self, depth: usize, out: &mut String) {
        let pad = |d: usize| "  ".repeat(d);
        let inner = depth.saturating_add(1);
        match self {
            Self::Str(s) => out.push_str(&format!("{s:?}")),
            Self::Bool(b) => out.push_str(&b.to_string()),
            Self::Num(n) => out.push_str(&n.to_string()),
            Self::List(items) if items.is_empty() => out.push_str("[]"),
            Self::List(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    out.push_str(if i == 0 { "\n" } else { ",\n" });
                    out.push_str(&pad(inner));
                    item.write(inner, out);
                }
                out.push_str(&format!("\n{}]", pad(depth)));
            }
            Self::Object(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    out.push_str(if i == 0 { "\n" } else { ",\n" });
                    out.push_str(&format!("{}\"{key}\": ", pad(inner)));
                    value.write(inner, out);
                }
                out.push_str(&format!("\n{}}}", pad(depth)));
            }
        }
    }
}

/// The `<pre>` a demo's submit toast shows its values in
/// (`description: h('pre', …, h('code', JSON.stringify(values, null, 2)))`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pre {
    /// `bg-code text-code-foreground mt-2 w-[320px] overflow-x-auto
    /// rounded-md p-4`, in a toast of `class: 'flex flex-col gap-2'` and
    /// `--border-radius: calc(var(--radius) + 4px)` (Formisch, TanStack Form, vee-validate).
    Code,
    /// `mt-2 w-[320px] rounded-md bg-neutral-950 p-4`, its code `text-white`.
    Neutral950,
    /// `mt-2 rounded-md bg-neutral-900 p-4 w-full`, its code `text-code-foreground`.
    Neutral900,
}

/// The toast a demo shows for a valid submit: `values` as JSON in `pre`.
pub fn submitted(values: &Json, pre: Pre) {
    let mut json = String::new();
    values.write(0, &mut json);
    let toast = Toast::new(ToastKind::Default, "You submitted the following values:").description_view(move || {
        let t = theme();
        let block = div().mt(2.0).rounded_md().p(4.0).font_mono();
        match pre {
            Pre::Code => block
                .bg(t.code)
                .text_color(t.code_foreground)
                .w_px(320.0)
                .overflow_x_scroll(),
            Pre::Neutral950 => block.bg(Color::hex(0x000a_0a0a)).text_color(Color::WHITE).w_px(320.0),
            Pre::Neutral900 => block.bg(Color::hex(0x0017_1717)).text_color(t.code_foreground).w_full(),
        }
        .child(text(json.clone()).whitespace_nowrap())
        .into_element()
    });
    shadcn::toast(if pre == Pre::Code {
        toast.class(|d| d.flex_col().gap(2.0).rounded(Radius::var(4.0)))
    } else {
        toast
    });
}

/// The messages of the checks that fail.
pub fn failing(checks: &[(bool, &str)]) -> Vec<String> {
    checks
        .iter()
        .filter(|(ok, _)| !ok)
        .map(|(_, message)| (*message).to_owned())
        .collect()
}

/// An address zod's and valibot's `email()` accept: a dot-free-edged local
/// part, `@`, and dotted labels ending in a TLD of two letters or more.
pub fn is_email(v: &str) -> bool {
    let Some((local, domain)) = v.rsplit_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local.chars().all(|c| c.is_ascii_alphanumeric() || "_'+-.".contains(c));
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = labels.len() >= 2
        && labels.iter().all(|l| {
            l.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        && labels
            .last()
            .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()));
    local_ok && domain_ok
}

/// `^\w+$`: letters, digits and underscores, at least one.
fn is_word(v: &str) -> bool {
    !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The card around a demo's form; `divided` draws the rules under the
/// header and over the footer (`border-b`, `border-t`).
fn shell(
    width: f32,
    header: Option<(&str, &str)>,
    divided: bool,
    content: impl IntoElement,
    footer: impl IntoElement,
) -> Div {
    let t = theme();
    card()
        .w_full()
        .max_w_px(width)
        .when_some(header, |c, (title, description)| {
            c.child(
                card_header()
                    .when(divided, |h| h.border_b().border_color(t.border).pb(6.0))
                    .child(card_title(title.to_owned()))
                    .child(card_description(description.to_owned())),
            )
        })
        .child(card_content().child(content))
        .child(
            card_footer()
                .when(divided, |f| f.border_t().border_color(t.border).pt(6.0))
                .child(footer),
        )
}

fn outline(label: &str) -> Div {
    button(ButtonVariant::Outline, ButtonSize::Default).child(label.to_owned())
}

fn primary(label: &str) -> Div {
    button(ButtonVariant::Default, ButtonSize::Default).child(label.to_owned())
}

/// The footer most demos have: Reset, then the submit button.
fn reset_then(save: &str, reset: impl Fn() + 'static, submit: impl Fn() + 'static) -> FieldStack {
    field(H, false)
        .child(outline("Reset").on_click(move |_| reset()))
        .child(primary(save).on_click(move |_| submit()))
}

/// A label that flips its checkbox or switch, as `<label for>` does.
fn label_for(name: &str, checked: &Signal<bool>) -> Div {
    let flip = *checked;
    field_label(name.to_owned()).on_click(move |_| flip.update(|c| *c = !*c))
}

// --- Profile Settings (`*Input`) ---

pub struct InputCard {
    username: FormField<TextField>,
}

impl InputCard {
    pub fn new(library: Library) -> Self {
        Self {
            username: FormField::new(TextField::new(""), library.checks(), |v: &String| {
                let n = v.chars().count();
                failing(&[
                    (n >= 3, "Username must be at least 3 characters."),
                    (n <= 10, "Username must be at most 10 characters."),
                    (
                        is_word(v),
                        "Username can only contain letters, numbers, and underscores.",
                    ),
                ])
            }),
        }
    }

    pub fn view(&self) -> Div {
        let u = &self.username;
        let invalid = u.invalid();
        let (reset, submit) = (*u, *u);
        shell(
            448.0,
            Some(("Profile Settings", "Update your profile information below.")),
            false,
            field_group().child(
                field(V, invalid)
                    .child(field_label("Username"))
                    .child(form_control(input().field(u.control()).placeholder("shadcn"), invalid))
                    .child(field_description(
                        "This is your public display name. Must be between 3 and 10 characters. Must only contain letters, numbers, and underscores.",
                    ))
                    .child(field_error(u.errors())),
            ),
            reset_then(
                "Save",
                move || reset.reset(),
                move || {
                    if form_submit(&[&submit]) {
                        submitted(&Json::Object(vec![("username", Json::Str(submit.control().text()))]), Pre::Code);
                    }
                },
            ),
        )
    }
}

// --- Bug Report (`*Demo`) ---

pub struct BugCard {
    title: FormField<TextField>,
    description: FormField<TextField>,
}

impl BugCard {
    pub fn new(library: Library) -> Self {
        Self {
            title: FormField::new(TextField::new(""), library.checks(), |v: &String| {
                let n = v.chars().count();
                failing(&[
                    (n >= 5, "Bug title must be at least 5 characters."),
                    (n <= 32, "Bug title must be at most 32 characters."),
                ])
            }),
            description: FormField::new(TextField::new(""), library.checks(), |v: &String| {
                let n = v.chars().count();
                failing(&[
                    (n >= 20, "Description must be at least 20 characters."),
                    (n <= 100, "Description must be at most 100 characters."),
                ])
            }),
        }
    }

    pub fn view(&self) -> Div {
        let (title, description) = (&self.title, &self.description);
        let (title_invalid, description_invalid) = (title.invalid(), description.invalid());
        let typed = description.control().text().chars().count();
        let (reset, submit) = ((*title, *description), (*title, *description));
        shell(
            448.0,
            Some(("Bug Report", "Help us improve by reporting bugs you encounter.")),
            false,
            field_group()
                .child(
                    field(V, title_invalid)
                        .child(field_label("Bug Title"))
                        .child(form_control(
                            input()
                                .field(title.control())
                                .placeholder("Login button not working on mobile"),
                            title_invalid,
                        ))
                        .child(field_error(title.errors())),
                )
                .child(
                    field(V, description_invalid)
                        .child(field_label("Description"))
                        .child(
                            input_group(
                                input_group_textarea(&description.control())
                                    .placeholder("I'm having an issue with the login button on mobile.")
                                    .min_h(24.0)
                                    .invalid(description_invalid),
                            )
                            .addon(
                                input_group_addon(InputGroupAlign::BlockEnd)
                                    .child(input_group_text().child(format!("{typed}/100 characters"))),
                            ),
                        )
                        .child(field_description(
                            "Include steps to reproduce, expected behavior, and what actually happened.",
                        ))
                        .child(field_error(description.errors())),
                ),
            reset_then(
                "Submit",
                move || form_reset(&[&reset.0, &reset.1]),
                move || {
                    if form_submit(&[&submit.0, &submit.1]) {
                        submitted(
                            &Json::Object(vec![
                                ("title", Json::Str(submit.0.control().text())),
                                ("description", Json::Str(submit.1.control().text())),
                            ]),
                            Pre::Code,
                        );
                    }
                },
            ),
        )
    }
}

// --- Personalization (`*Textarea`) ---

pub struct TextareaCard {
    about: FormField<TextField>,
}

impl TextareaCard {
    pub fn new(library: Library) -> Self {
        Self {
            about: FormField::new(TextField::new(""), library.checks(), |v: &String| {
                let n = v.chars().count();
                failing(&[
                    (n >= 10, "Please provide at least 10 characters."),
                    (n <= 200, "Please keep it under 200 characters."),
                ])
            }),
        }
    }

    pub fn view(&self) -> Div {
        let about = &self.about;
        let invalid = about.invalid();
        let (reset, submit) = (*about, *about);
        shell(
            448.0,
            Some((
                "Personalization",
                "Customize your experience by telling us more about yourself.",
            )),
            false,
            field_group().child(
                field(V, invalid)
                    .child(field_label("More about you"))
                    .child(form_control(
                        textarea()
                            .field(about.control())
                            .placeholder("I'm a software engineer...")
                            .min_h_px(120.0),
                        invalid,
                    ))
                    .child(field_description(
                        "Tell us more about yourself. This will be used to help us personalize your experience.",
                    ))
                    .child(field_error(about.errors())),
            ),
            reset_then(
                "Save",
                move || reset.reset(),
                move || {
                    if form_submit(&[&submit]) {
                        submitted(
                            &Json::Object(vec![("about", Json::Str(submit.control().text()))]),
                            Pre::Code,
                        );
                    }
                },
            ),
        )
    }
}

// --- Security Settings (`*Switch`) ---

pub struct SwitchCard {
    two_factor: FormField<Signal<bool>>,
}

impl SwitchCard {
    pub fn new(library: Library) -> Self {
        Self {
            two_factor: FormField::new(Signal::new(false), library.checks(), |on: &bool| {
                failing(&[(*on, "It is highly recommended to enable two-factor authentication.")])
            }),
        }
    }

    pub fn view(&self) -> Div {
        let f = &self.two_factor;
        let invalid = f.invalid();
        let (reset, submit) = (*f, *f);
        shell(
            448.0,
            Some(("Security Settings", "Manage your account security preferences.")),
            false,
            field_group().child(
                field(H, invalid)
                    .child(
                        field_content()
                            .child(label_for("Multi-factor authentication", &f.control()))
                            .child(field_description(
                                "Enable multi-factor authentication to secure your account.",
                            ))
                            .child(field_error(f.errors())),
                    )
                    .child(switch().model(f.control())),
            ),
            reset_then(
                "Save",
                move || reset.reset(),
                move || {
                    if form_submit(&[&submit]) {
                        submitted(
                            &Json::Object(vec![("twoFactor", Json::Bool(submit.control().get()))]),
                            Pre::Code,
                        );
                    }
                },
            ),
        )
    }
}

// --- Language Preferences (`*Select`) ---

const LANGUAGES: [(&str, &str); 7] = [
    ("en", "English"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("zh", "Chinese"),
    ("ja", "Japanese"),
];

pub struct SelectCard {
    language: FormField<Signal<Option<String>>>,
}

impl SelectCard {
    pub fn new(library: Library) -> Self {
        Self {
            language: FormField::new(Signal::new(None), library.checks(), |v: &Option<String>| {
                let v = v.as_deref().unwrap_or_default();
                failing(&[
                    (!v.is_empty(), "Please select your spoken language."),
                    (
                        v != "auto",
                        "Auto-detection is not allowed. Please select a specific language.",
                    ),
                ])
            }),
        }
    }

    pub fn view(&self) -> Div {
        let f = &self.language;
        let invalid = f.invalid();
        let content = LANGUAGES.iter().fold(
            select_content()
                .position(SelectPosition::ItemAligned)
                .child(select_item("auto", "Auto"))
                .child(select_separator()),
            |c, (value, label)| c.child(select_item(*value, *label)),
        );
        let (reset, submit) = (*f, *f);
        // `orientation="responsive"` lays the field out in a row once its
        // group is `@md` (28rem) wide; this card's content is 29rem.
        shell(
            512.0,
            Some(("Language Preferences", "Select your preferred spoken language.")),
            false,
            field_group().child(
                field(H, invalid)
                    .child(
                        field_content()
                            .child(field_label("Spoken Language"))
                            .child(field_description("For best results, select the language you speak."))
                            .child(field_error(f.errors())),
                    )
                    .child(form_control(
                        select()
                            .model(f.control())
                            .min_w_px(120.0)
                            .child(select_value("Select"))
                            .child(content),
                        invalid,
                    )),
            ),
            reset_then(
                "Save",
                move || reset.reset(),
                move || {
                    if form_submit(&[&submit]) {
                        let language = submit.control().get().unwrap_or_default();
                        submitted(&Json::Object(vec![("language", Json::Str(language))]), Pre::Code);
                    }
                },
            ),
        )
    }
}

// --- Subscription Plan (`*RadioGroup`) ---

const PLANS: [(&str, &str, &str); 3] = [
    (
        "starter",
        "Starter (100K tokens/month)",
        "For everyday use with basic features.",
    ),
    (
        "pro",
        "Pro (1M tokens/month)",
        "For advanced AI usage with more features.",
    ),
    (
        "enterprise",
        "Enterprise (Unlimited tokens)",
        "For large teams and heavy usage.",
    ),
];

/// A choice card: a labelled radio, the whole card choosing it.
fn choice(plan: Signal<Option<String>>, value: &'static str, title: &str, description: &str, invalid: bool) -> Div {
    let choose = plan;
    let checked = plan.get().as_deref() == Some(value);
    field_label_card(
        field(H, invalid)
            .child(
                field_content()
                    .child(field_title(title.to_owned()))
                    .child(field_description(description.to_owned())),
            )
            .child(form_control(radio_group_item(value), invalid)),
        checked,
    )
    .on_click(move |_| choose.set(Some(value.to_owned())))
}

pub struct RadioCard {
    library: Library,
    plan: FormField<Signal<Option<String>>>,
}

impl RadioCard {
    pub fn new(library: Library) -> Self {
        Self {
            plan: FormField::new(Signal::new(None), library.checks(), |v: &Option<String>| {
                failing(&[(
                    v.as_deref().is_some_and(|p| !p.is_empty()),
                    "You must select a subscription plan to continue.",
                )])
            }),
            library,
        }
    }

    pub fn view(&self) -> Div {
        let f = &self.plan;
        let g = f.control();
        let invalid = f.invalid();
        let (reset, submit) = (*f, *f);
        shell(
            448.0,
            Some(("Subscription Plan", "See pricing and features for each plan.")),
            false,
            field_group().child(
                field_set()
                    .child(field_legend("Plan", None))
                    .child(field_description("You can upgrade or downgrade your plan at any time."))
                    .child(field_choices(PLANS.iter().fold(
                        radio_group().model(g),
                        |r, (id, title, description)| {
                            // TanStack's demo words its starter plan as the Complex demo's Basic.
                            let description = if *id == "starter" && self.library == Library::TanStack {
                                "For individuals and small teams"
                            } else {
                                description
                            };
                            r.child(choice(g, id, title, description, invalid))
                        },
                    )))
                    .child(field_error(f.errors())),
            ),
            reset_then(
                "Save",
                move || reset.reset(),
                move || {
                    if form_submit(&[&submit]) {
                        let plan = submit.control().get().unwrap_or_default();
                        submitted(&Json::Object(vec![("plan", Json::Str(plan))]), Pre::Code);
                    }
                },
            ),
        )
    }
}

// --- Notifications (`*Checkbox`) ---

/// Checkboxes whose value is the ids of the checked ones, in their order.
#[derive(Clone, Copy)]
struct Checklist(Signal<Vec<(&'static str, Signal<bool>)>>);

impl Checklist {
    fn new(ids: &[&'static str]) -> Self {
        Self(Signal::new(ids.iter().map(|id| (*id, Signal::new(false))).collect()))
    }

    fn of(&self, id: &str) -> Option<Signal<bool>> {
        self.0
            .with_untracked(|all| all.iter().find(|(i, _)| *i == id).map(|(_, s)| *s))
    }

    fn json(&self) -> Json {
        Json::List(self.value().into_iter().map(Json::Str).collect())
    }
}

impl FormValue for Checklist {
    type Value = Vec<String>;

    fn value(&self) -> Vec<String> {
        self.0.with_untracked(|all| {
            all.iter()
                .filter(|(_, on)| on.get())
                .map(|(id, _)| (*id).to_owned())
                .collect()
        })
    }

    fn set_value(&self, value: Vec<String>) {
        self.0.with_untracked(|all| {
            for (id, on) in all {
                on.set(value.iter().any(|v| v == id));
            }
        });
    }
}

/// A checkbox beside its label, in a horizontal field.
fn check(name: &str, checked: &Signal<bool>, invalid: bool) -> FieldStack {
    field(H, invalid)
        .child(form_control(checkbox().model(*checked), invalid))
        .child(label_for(name, checked).font_normal())
}

pub struct CheckboxCard {
    library: Library,
    responses: Signal<bool>,
    tasks: FormField<Checklist>,
}

impl CheckboxCard {
    pub fn new(library: Library) -> Self {
        Self {
            library,
            responses: Signal::new(true),
            tasks: FormField::new(
                Checklist::new(&["push", "email"]),
                library.checks(),
                |v: &Vec<String>| failing(&[(!v.is_empty(), "Please select at least one notification type.")]),
            ),
        }
    }

    pub fn view(&self) -> Div {
        let f = &self.tasks;
        let invalid = f.invalid();
        let list = f.control();
        let task = |id: &str, name: &str| list.of(id).map(|on| check(name, &on, invalid));
        let tasks = field_set()
            .child(field_legend("Tasks", Some(FieldLegendVariant::Label)))
            .child(field_description(
                "Get notified when tasks you've created have updates.",
            ))
            .child(
                field_checkbox_group()
                    .child(task("push", "Push notifications"))
                    .child(task("email", "Email notifications")),
            );
        // Formisch's demo puts the error under the set, in a group of its
        // own (a group in a group is `gap-4`); the others put it in the set.
        let tasks = if self.library == Library::Formisch {
            field_group()
                .gap(4.0)
                .child(tasks)
                .child(field_error(f.errors()))
                .into_element()
        } else {
            tasks.child(field_error(f.errors())).into_element()
        };
        let (responses, reset, submit) = (self.responses, *f, *f);
        shell(
            448.0,
            Some(("Notifications", "Manage your notification preferences.")),
            false,
            field_group()
                .child(
                    field_set()
                        .child(field_legend("Responses", Some(FieldLegendVariant::Label)))
                        .child(field_description(
                            "Get notified for requests that take time, like research or image generation.",
                        ))
                        .child(
                            field_checkbox_group().child(
                                field(H, false)
                                    .child(checkbox().model(self.responses).disabled(true))
                                    .child(field_label("Push notifications").font_normal()),
                            ),
                        ),
                )
                .child(field_separator(None))
                .child(tasks),
            reset_then(
                "Save",
                move || reset.reset(),
                move || {
                    if form_submit(&[&submit]) {
                        submitted(
                            &Json::Object(vec![
                                ("responses", Json::Bool(responses.get())),
                                ("tasks", submit.control().json()),
                            ]),
                            Pre::Code,
                        );
                    }
                },
            ),
        )
    }
}

// --- Contact Emails (`*Array`) ---

pub struct ArrayCard {
    library: Library,
    emails: Signal<Vec<FormField<TextField>>>,
}

impl ArrayCard {
    pub fn new(library: Library) -> Self {
        Self {
            library,
            emails: Signal::new(Self::initial(library)),
        }
    }

    /// The addresses the form opens with: TanStack's demo has one, the others two.
    fn initial(library: Library) -> Vec<FormField<TextField>> {
        let count = if library == Library::TanStack { 1 } else { 2 };
        (0..count).map(|_| Self::address(library)).collect()
    }

    fn address(library: Library) -> FormField<TextField> {
        // TanStack's demo validates its schema on blur.
        let checks = if library == Library::TanStack {
            Checks {
                on_blur: true,
                on_change: OnChange::Keep,
            }
        } else {
            library.checks()
        };
        FormField::new(TextField::new(""), checks, move |v: &String| {
            // Formisch's (valibot) schema also says an empty one is missing.
            let required = library != Library::Formisch || !v.is_empty();
            failing(&[
                (required, "Enter an email address."),
                (is_email(v), "Enter a valid email address."),
            ])
        })
    }

    pub fn view(&self) -> Div {
        let library = self.library;
        let emails = self.emails.get();
        let count = emails.len();
        let rows = emails.iter().enumerate().map(|(i, f)| {
            let invalid = f.invalid();
            let list = self.emails;
            let remove = input_group_addon(InputGroupAlign::InlineEnd).button(
                input_group_button(ButtonVariant::Ghost, InputGroupButtonSize::IconXs)
                    .child(icon(Icon::X))
                    .on_click(move |_| {
                        list.update(|all| {
                            if i < all.len() {
                                all.remove(i);
                            }
                        });
                    }),
            );
            let group = input_group(
                input_group_input(&f.control())
                    .placeholder("name@example.com")
                    .invalid(invalid),
            );
            let group = if count > 1 { group.addon(remove) } else { group };
            field(H, invalid).child(field_content().child(group).child(field_error(f.errors())))
        });
        let add = {
            let list = self.emails;
            button(ButtonVariant::Outline, ButtonSize::Sm)
                .child("Add Email Address")
                .disabled(count >= 5)
                .on_click(move |_| {
                    list.update(|all| {
                        if all.len() < 5 {
                            all.push(Self::address(library));
                        }
                    });
                })
        };
        let (reset, submit) = (self.emails, self.emails);
        shell(
            448.0,
            Some(("Contact Emails", "Manage your contact email addresses.")),
            true,
            field_set()
                .gap(4.0)
                .child(field_legend("Email Addresses", Some(FieldLegendVariant::Label)))
                .child(field_description(
                    "Add up to 5 email addresses where we can contact you.",
                ))
                .child(field_group().gap(4.0).children(rows).child(add)),
            reset_then(
                "Save",
                move || reset.replace(Self::initial(library)),
                move || {
                    let all = submit.get();
                    let entries: Vec<&dyn FormEntry> = all.iter().map(|f| -> &dyn FormEntry { f }).collect();
                    if form_submit(&entries) {
                        let addresses = all
                            .iter()
                            .map(|f| Json::Object(vec![("address", Json::Str(f.control().text()))]))
                            .collect();
                        submitted(&Json::Object(vec![("emails", Json::List(addresses))]), Pre::Code);
                    }
                },
            ),
        )
    }
}

// --- You're almost there! (`*Complex`) ---

const ADDONS: [(&str, &str, &str); 3] = [
    ("analytics", "Analytics", "Advanced analytics and reporting"),
    ("backup", "Backup", "Automated daily backups"),
    ("support", "Priority Support", "24/7 premium customer support"),
];

pub struct ComplexCard {
    library: Library,
    plan: Signal<Option<String>>,
    billing: FormField<Signal<Option<String>>>,
    addons: FormField<Checklist>,
    email_notifications: Signal<bool>,
}

impl ComplexCard {
    pub fn new(library: Library) -> Self {
        // TanStack's demo opens billed monthly; the others with no period.
        let billing_select = Signal::new((library == Library::TanStack).then(|| "monthly".to_owned()));
        let ids: Vec<&'static str> = ADDONS.iter().map(|(id, _, _)| *id).collect();
        Self {
            library,
            plan: Signal::new(Some("basic".to_owned())),
            billing: FormField::new(billing_select, library.checks(), |v: &Option<String>| {
                failing(&[(
                    v.as_deref().is_some_and(|p| !p.is_empty()),
                    "Please select a billing period",
                )])
            }),
            addons: FormField::new(Checklist::new(&ids), library.checks(), |v: &Vec<String>| {
                failing(&[(!v.is_empty(), "Please select at least one add-on")])
            }),
            email_notifications: Signal::new(false),
        }
    }

    pub fn view(&self) -> Div {
        let tanstack = self.library == Library::TanStack;
        let (billing, addons) = (self.billing, self.addons);
        let (billing_invalid, addons_invalid) = (billing.invalid(), addons.invalid());
        let periods = select_content()
            .child(select_item("monthly", "Monthly"))
            .child(select_item("yearly", "Yearly"));
        let list = addons.control();
        let addon_rows = ADDONS.iter().filter_map(|(id, title, description)| {
            list.of(id).map(|on| {
                field(H, addons_invalid)
                    .child(form_control(checkbox().model(on), addons_invalid))
                    .child(
                        field_content()
                            .child(label_for(title, &on))
                            .child(field_description((*description).to_owned())),
                    )
            })
        });
        let legend_variant = (!tanstack).then_some(FieldLegendVariant::Label);
        let content = field_group()
            .child(
                field_set()
                    .child(field_legend("Subscription Plan", legend_variant))
                    .child(field_description("Choose your subscription plan."))
                    .child(field_choices(
                        radio_group()
                            .model(self.plan)
                            .child(choice(
                                self.plan,
                                "basic",
                                "Basic",
                                "For individuals and small teams",
                                false,
                            ))
                            .child(choice(
                                self.plan,
                                "pro",
                                "Pro",
                                "For businesses with higher demands",
                                false,
                            )),
                    )),
            )
            .child(field_separator(None))
            .child(
                field(V, billing_invalid)
                    .child(field_label("Billing Period"))
                    // TanStack's demo marks the select's root, not its trigger.
                    .child(form_control(
                        select()
                            .model(billing.control())
                            .child(select_value("Select"))
                            .child(periods),
                        billing_invalid && !tanstack,
                    ))
                    .child(field_description("Choose how often you want to be billed."))
                    .child(field_error(billing.errors())),
            )
            .child(field_separator(None))
            .child(
                field_set()
                    .child(field_legend("Add-ons", None))
                    .child(field_description("Select additional features you'd like to include."))
                    .child(addon_rows.fold(field_checkbox_group(), |g, row| g.child(row)))
                    .child(field_error(addons.errors())),
            )
            .child(field_separator(None))
            .child(
                field(H, false)
                    .child(
                        field_content()
                            .child(label_for("Email Notifications", &self.email_notifications))
                            .child(field_description("Receive email updates about your subscription")),
                    )
                    .child(switch().model(self.email_notifications)),
            );
        let submit = {
            let (plan, billing, addons, email) = (self.plan, billing, addons, self.email_notifications);
            move || {
                if form_submit(&[&billing, &addons]) {
                    submitted(
                        &Json::Object(vec![
                            ("plan", Json::Str(plan.get().unwrap_or_default())),
                            ("billingPeriod", Json::Str(billing.control().get().unwrap_or_default())),
                            ("addons", addons.control().json()),
                            ("emailNotifications", Json::Bool(email.get())),
                        ]),
                        Pre::Code,
                    );
                }
            }
        };
        let save = primary("Save Preferences").on_click(move |_| submit());
        if tanstack {
            shell(384.0, None, false, content, field(H, false).justify_end().child(save))
        } else {
            let (plan, billing, addons, email) = (self.plan, billing, addons, self.email_notifications);
            let reset = outline("Reset").on_click(move |_| {
                plan.set(Some("basic".to_owned()));
                email.set(false);
                form_reset(&[&billing, &addons]);
            });
            shell(
                384.0,
                Some((
                    "You're almost there!",
                    "Choose your subscription plan and billing period.",
                )),
                true,
                content,
                field(V, false).child(save).child(reset),
            )
        }
    }
}

// --- A library's page ---

/// The demo file of each card, as one library names them.
pub struct Files {
    pub demo: &'static str,
    pub input: &'static str,
    pub textarea: &'static str,
    pub select: &'static str,
    pub switch: &'static str,
    pub radio_group: &'static str,
    pub checkbox: &'static str,
    pub array: &'static str,
    pub complex: &'static str,
}

/// The nine cards of one library, sharing the page's toasts.
pub struct Cards {
    bug: BugCard,
    input: InputCard,
    textarea: TextareaCard,
    select: SelectCard,
    switch: SwitchCard,
    radio: RadioCard,
    checkbox: CheckboxCard,
    array: ArrayCard,
    complex: ComplexCard,
}

impl Cards {
    pub fn new(library: Library) -> Self {
        Self {
            bug: BugCard::new(library),
            input: InputCard::new(library),
            textarea: TextareaCard::new(library),
            select: SelectCard::new(library),
            switch: SwitchCard::new(library),
            radio: RadioCard::new(library),
            checkbox: CheckboxCard::new(library),
            array: ArrayCard::new(library),
            complex: ComplexCard::new(library),
        }
    }

    /// Each card in its demo, one under the other.
    pub fn view(&self, files: &Files) -> Div {
        gui::div()
            .flex_col()
            .gap(8.0)
            .child(demo(files.demo, self.bug.view()))
            .child(demo(files.input, self.input.view()))
            .child(demo(files.textarea, self.textarea.view()))
            .child(demo(files.select, self.select.view()))
            .child(demo(files.switch, self.switch.view()))
            .child(demo(files.radio_group, self.radio.view()))
            .child(demo(files.checkbox, self.checkbox.view()))
            .child(demo(files.array, self.array.view()))
            .child(demo(files.complex, self.complex.view()))
    }
}

/// The script every library's page runs on its first card, Bug Report:
/// leave the title empty, submit empty, fix both fields and submit again,
/// so the frames show when each library's errors come and go.
pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let title = s.locate("Login button not working on mobile")?;
    s.click(title.0, title.1);
    let (x, y) = s.locate("Bug Report")?;
    s.click(x, y);
    s.frame("title left empty")?;
    s.click_text("Submit")?;
    s.frame("submitted empty")?;
    s.click(title.0, title.1);
    s.type_text("Lo");
    s.frame("title typed short")?;
    s.type_text("gin fails");
    let (x, y) = s.locate("I'm having an issue with the login button on mobile.")?;
    s.click(x, y);
    s.type_text("The login button does nothing on iOS.");
    s.frame("both fixed")?;
    s.click_text("Submit")?;
    s.frame("submitted valid")?;
    Ok(())
}
