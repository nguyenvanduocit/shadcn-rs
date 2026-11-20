//! The composer a user talks to an agent through: a textarea, files to
//! send with it, and the model and effort the agent runs on.

use gui::prelude::*;
use gui::{Div, Icon, Modifiers, NamedKey, ScenarioError, Side, SideAlign, Signal, TextField, div, icon, text};
use shadcn::{
    AttachmentProps, AttachmentSize, AttachmentSlots, ButtonVariant, InputGroupAlign, InputGroupButtonSize, MenuState,
    attachment, attachment_action, attachment_actions, attachment_content, attachment_media, attachment_title,
    dropdown_menu_content, dropdown_menu_group, dropdown_menu_label, dropdown_menu_radio_group,
    dropdown_menu_radio_item, dropdown_menu_separator, dropdown_menu_trigger, input_group, input_group_addon,
    input_group_button, input_group_textarea, theme,
};

use crate::{Render, Shot, demo, page};

/// The models on offer, by provider: id and name.
const PROVIDERS: [(&str, [(&str, &str); 2]); 2] = [
    ("OpenAI", [("gpt-5", "GPT-5"), ("gpt-5-mini", "GPT-5 mini")]),
    (
        "Anthropic",
        [
            ("claude-sonnet-4-5", "Claude Sonnet 4.5"),
            ("claude-haiku-4-5", "Claude Haiku 4.5"),
        ],
    ),
];
const EFFORTS: [(&str, &str); 3] = [("low", "Low"), ("medium", "Medium"), ("high", "High")];

/// A composer: the draft, its files, and the model and effort picked.
#[derive(Clone)]
struct Composer {
    draft: TextField,
    files: Signal<Vec<String>>,
    model: Signal<String>,
    effort: Signal<String>,
    models: MenuState,
    efforts: MenuState,
}

impl Composer {
    fn new() -> Self {
        Self {
            draft: TextField::new(""),
            files: Signal::new(Vec::new()),
            model: Signal::new("gpt-5".to_owned()),
            effort: Signal::new("medium".to_owned()),
            models: MenuState::new(),
            efforts: MenuState::new(),
        }
    }

    /// Vue's `onSubmit`: send, then clear the draft and the files.
    fn submit(&self) {
        if self.draft.text().trim().is_empty() {
            return;
        }
        self.draft.set_text("");
        self.files.set(Vec::new());
    }

    fn model_menu(&self) -> Div {
        let m = &self.models;
        let model = self.model.get();
        let name = PROVIDERS
            .iter()
            .flat_map(|(_, ms)| ms.iter())
            .find(|(id, _)| *id == model)
            .map_or("GPT-5", |(_, n)| *n);
        let content = dropdown_menu_content(m)
            .w(48.0)
            .child(dropdown_menu_radio_group().children(PROVIDERS.iter().enumerate().map(
                |(i, (provider, models))| {
                    div().when(i > 0, |d| d.child(dropdown_menu_separator())).child(
                        dropdown_menu_group()
                            .child(dropdown_menu_label(provider, false))
                            .children(
                                models
                                    .iter()
                                    .map(|(id, n)| dropdown_menu_radio_item(m, n, id, &self.model, false)),
                            ),
                    )
                },
            )));
        dropdown_menu_trigger(
            m,
            input_group_button(ButtonVariant::Outline, InputGroupButtonSize::Sm)
                .ml_auto()
                .child(name)
                .child(icon(Icon::ChevronDown)),
            Side::Bottom,
            SideAlign::Start,
            content,
        )
    }

    fn effort_menu(&self) -> Div {
        let m = &self.efforts;
        let effort = self.effort.get();
        let name = EFFORTS
            .iter()
            .find(|(id, _)| *id == effort)
            .map_or("Medium", |(_, n)| *n);
        let content = dropdown_menu_content(m).w(36.0).child(
            dropdown_menu_group().child(dropdown_menu_label("Effort", false)).child(
                dropdown_menu_radio_group().children(
                    EFFORTS
                        .iter()
                        .map(|(id, n)| dropdown_menu_radio_item(m, n, id, &self.effort, false)),
                ),
            ),
        );
        dropdown_menu_trigger(
            m,
            input_group_button(ButtonVariant::Outline, InputGroupButtonSize::Sm)
                .child(text("Effort").text_color(theme().muted_foreground))
                .child(name)
                .child(icon(Icon::ChevronDown)),
            Side::Bottom,
            SideAlign::Start,
            content,
        )
    }

    /// The files picked, above the textarea; `None` while there are none.
    fn attachments(&self) -> Option<shadcn::InputGroupAddon> {
        let files = self.files.get();
        if files.is_empty() {
            return None;
        }
        let p = AttachmentProps {
            size: AttachmentSize::Sm,
            ..AttachmentProps::default()
        };
        let row = input_group_addon(InputGroupAlign::BlockStart).flex_wrap();
        Some(files.into_iter().enumerate().fold(row, |row, (i, name)| {
            let files = self.files.clone();
            row.child(
                attachment(p, AttachmentSlots::default())
                    .child(attachment_media(p, shadcn::AttachmentMediaVariant::Icon).child(icon(Icon::FileText)))
                    .child(attachment_content(p).child(attachment_title(name)))
                    .child(
                        attachment_actions(p).child(attachment_action(Icon::X).on_click(move |_| {
                            files.update(|mut f| {
                                if i < f.len() {
                                    f.remove(i);
                                }
                                f
                            });
                        })),
                    ),
            )
        }))
    }

    /// The composer; with `files`, the row of picked files above it.
    fn view(&self, files: bool) -> Div {
        let can_send = !self.draft.text().trim().is_empty();
        let send = self.clone();
        let group = input_group(
            input_group_textarea(&self.draft)
                .placeholder("Ask the agent to change your code...")
                .p(3.0)
                .on_submit({
                    let send = send.clone();
                    move || send.submit()
                }),
        );
        let group = match self.attachments().filter(|_| files) {
            Some(row) => group.addon(row),
            None => group,
        };
        // `gui` opens no file dialog, so the attach button picks nothing.
        let group = group.addon(
            input_group_addon(InputGroupAlign::BlockEnd)
                .button(
                    input_group_button(ButtonVariant::Outline, InputGroupButtonSize::IconSm)
                        .rounded_full()
                        .child(icon(Icon::Plus)),
                )
                .button(self.model_menu())
                .button(self.effort_menu())
                .button(
                    input_group_button(ButtonVariant::Default, InputGroupButtonSize::IconSm)
                        .rounded_full()
                        .disabled(!can_send)
                        .child(icon(Icon::ArrowUp))
                        .on_click(move |_| send.submit()),
                ),
        );
        div().mx_auto().w_full().max_w(128.0).py(6.0).child(group)
    }
}

pub fn render() -> Render {
    let prompt = Composer::new();
    let selector = Composer::new();
    Box::new(move || {
        page(
            "Prompt input",
            div()
                .flex_col()
                .gap(10.0)
                .child(demo("PromptInputDemo.vue", prompt.view(true)))
                .child(demo("ModelSelectorDemo.vue", selector.view(false))),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.click_text("GPT-5")?;
    s.frame("models open")?;
    s.click_text("Claude Sonnet 4.5")?;
    s.frame("model picked")?;
    // Shift+Enter breaks the line; Enter sends, which clears the draft.
    let (x, y) = s.locate("Ask the agent to change your code...")?;
    s.click(x, y);
    s.type_text("Fix the tests");
    s.press(
        NamedKey::Enter,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    s.type_text("please");
    s.frame("two lines typed")?;
    s.press(NamedKey::Enter, Modifiers::default());
    s.frame("enter sent, draft cleared")?;
    Ok(())
}
