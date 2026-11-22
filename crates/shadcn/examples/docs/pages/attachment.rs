use gui::prelude::*;
use gui::{Div, Icon, ScenarioError, Signal, div, text};
use shadcn::{
    AttachmentMediaVariant, AttachmentOrientation, AttachmentProps, AttachmentSize, AttachmentSlots, AttachmentState,
    attachment, attachment_action, attachment_actions, attachment_content, attachment_description, attachment_group,
    attachment_icon, attachment_image, attachment_media, attachment_title, attachment_trigger, spinner,
};

use crate::{Render, Shot, demo, page, photo};

/// The demos' column: `mx-auto flex w-full max-w-sm flex-col py-12`.
fn column(gap: f32) -> Div {
    div().mx_auto().flex_col().w_full().max_w(96.0).gap(gap).py(12.0)
}

/// A horizontal file attachment, `class="w-full"`, with media, name, details and actions.
fn file(p: AttachmentProps, media: impl IntoElement, name: &str, meta: Option<&str>, actions: &[Icon]) -> Div {
    attachment(p, AttachmentSlots::default())
        .w_full()
        .child(attachment_media(p, AttachmentMediaVariant::Icon).child(media))
        .child(
            attachment_content(p)
                .child(attachment_title(name))
                .child(meta.map(|m| attachment_description(p, m))),
        )
        .when(!actions.is_empty(), |d| {
            d.child(attachment_actions(p).children(actions.iter().map(|&a| attachment_action(a))))
        })
}

fn with_state(state: AttachmentState) -> AttachmentProps {
    AttachmentProps {
        state,
        ..AttachmentProps::default()
    }
}

pub fn render() -> Render {
    let photos = [
        ("workspace.png", "PNG · 820 KB", "photo-1497366754035-f200968a6e72"),
        ("desk-reference.jpg", "JPG · 1.1 MB", "photo-1497215728101-856f4ea42174"),
        (
            "office-reference.jpg",
            "JPG · 940 KB",
            "photo-1497366811353-6870744d04b2",
        ),
    ]
    .map(|(name, meta, id)| (name, meta, photo(id)));
    let opened = Signal::labeled("opened", 0_u32);
    Box::new(move || {
        let plain = AttachmentProps::default();
        let vertical = AttachmentProps {
            orientation: AttachmentOrientation::Vertical,
            ..plain
        };
        let uploading = with_state(AttachmentState::Uploading);
        let o = opened;
        let trigger = demo(
            "AttachmentTriggerDemo.vue",
            div().mx_auto().w_full().max_w(96.0).py(12.0).child(
                attachment_trigger(file(
                    plain,
                    attachment_icon(plain, Icon::FileSearch),
                    "research-summary.pdf",
                    Some("Open preview dialog"),
                    &[Icon::Copy, Icon::X],
                ))
                .on_click(move |_| o.update(|n| *n = n.saturating_add(1))),
            ),
        );
        let gallery = demo(
            "AttachmentDemo.vue",
            column(3.0)
                .child(attachment_group().children(photos.iter().map(|(name, meta, img)| {
                    attachment(vertical, AttachmentSlots::default())
                        .flex_none()
                        .child(
                            attachment_media(vertical, AttachmentMediaVariant::Image)
                                .child(img.clone().map(attachment_image)),
                        )
                        .child(
                            attachment_content(vertical)
                                .child(attachment_title(*name))
                                .child(attachment_description(vertical, *meta)),
                        )
                })))
                .child(file(
                    uploading,
                    spinner(),
                    "sales-dashboard.pdf",
                    Some("Uploading · 64%"),
                    &[Icon::X],
                ))
                .child(file(
                    plain,
                    attachment_icon(plain, Icon::FileCode),
                    "message-renderer.vue",
                    Some("TypeScript · 12 KB"),
                    &[Icon::X],
                )),
        );
        let states = [
            (
                AttachmentState::Idle,
                Some(Icon::Clock),
                "selected-file.pdf",
                "Ready to upload",
            ),
            (AttachmentState::Uploading, None, "design-system.zip", "Uploading · 64%"),
            (
                AttachmentState::Processing,
                Some(Icon::FileText),
                "market-research.pdf",
                "Processing document",
            ),
            (
                AttachmentState::Error,
                Some(Icon::FileWarning),
                "financial-model.xlsx",
                "Upload failed. Try again.",
            ),
            (
                AttachmentState::Done,
                Some(Icon::Check),
                "uploaded-report.pdf",
                "Uploaded · 1.8 MB",
            ),
        ];
        let states = demo(
            "AttachmentStatesDemo.vue",
            column(2.0).children(states.map(|(s, which, name, meta)| {
                let p = with_state(s);
                let actions: &[Icon] = if s == AttachmentState::Error {
                    &[Icon::RefreshCw, Icon::X]
                } else {
                    &[Icon::X]
                };
                let media = which.map_or_else(|| spinner().into_element(), |i| attachment_icon(p, i).into_element());
                file(p, media, name, Some(meta), actions)
            })),
        );
        let sizes = [
            (AttachmentSize::Default, "Default attachment", Some("PDF · 2.4 MB")),
            (AttachmentSize::Sm, "Small attachment", Some("PDF · 2.4 MB")),
            (AttachmentSize::Xs, "Extra small attachment", None),
        ];
        let sizes = demo(
            "AttachmentSizesDemo.vue",
            column(3.0).children(sizes.map(|(size, name, meta)| {
                let p = AttachmentProps { size, ..plain };
                file(p, attachment_icon(p, Icon::FileText), name, meta, &[])
            })),
        );
        let workspace = photos.first().and_then(|(_, _, img)| img.clone());
        let files: [(&str, &str, Option<Icon>); 4] = [
            ("briefing-notes.pdf", "PDF · 1.4 MB", Some(Icon::FileText)),
            ("workspace.png", "PNG · 820 KB", None),
            ("customers.csv", "CSV · 18 KB", Some(Icon::Table)),
            ("renderer.vue", "Vue · 12 KB", Some(Icon::FileCode)),
        ];
        let group = demo(
            "AttachmentGroupDemo.vue",
            div()
                .mx_auto()
                .w_full()
                .max_w(96.0)
                .py(12.0)
                .child(attachment_group().w_full().children(files.map(|(name, meta, which)| {
                    let media = match which {
                        Some(i) => {
                            attachment_media(plain, AttachmentMediaVariant::Icon).child(attachment_icon(plain, i))
                        }
                        None => attachment_media(plain, AttachmentMediaVariant::Image)
                            .child(workspace.clone().map(attachment_image)),
                    };
                    attachment(plain, AttachmentSlots::default())
                        .flex_none()
                        .w(64.0)
                        .child(media)
                        .child(
                            attachment_content(plain)
                                .child(attachment_title(name))
                                .child(attachment_description(plain, meta)),
                        )
                        .child(attachment_actions(plain).child(attachment_action(Icon::X)))
                }))),
        );
        let o = opened;
        let images = demo(
            "AttachmentImageDemo.vue",
            div()
                .mx_auto()
                .w_full()
                .max_w(96.0)
                .py(12.0)
                .child(
                    attachment_group()
                        .w_full()
                        .children(photos.iter().map(|(name, meta, img)| {
                            attachment_trigger(
                                attachment(vertical, AttachmentSlots::default())
                                    .flex_none()
                                    .child(
                                        attachment_media(vertical, AttachmentMediaVariant::Image)
                                            .child(img.clone().map(attachment_image)),
                                    )
                                    .child(
                                        attachment_content(vertical)
                                            .child(attachment_title(*name))
                                            .child(attachment_description(vertical, *meta)),
                                    )
                                    .child(attachment_actions(vertical).child(attachment_action(Icon::X))),
                            )
                            .on_click(move |_| o.update(|n| *n = n.saturating_add(1)))
                        })),
                ),
        );
        page(
            "Attachment",
            div()
                .flex_col()
                .child(text(format!("Opened {} times", opened.get())).text_sm())
                .child(trigger)
                .child(gallery)
                .child(states)
                .child(sizes)
                .child(group)
                .child(images),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("research-summary.pdf")?;
    s.hover(x, y);
    s.frame("hover pressable")?;
    s.click(x, y);
    s.frame("opened")?;
    Ok(())
}
