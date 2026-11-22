use gui::prelude::*;
use gui::{Icon, Model, ScenarioError, Shadow, Signal, div, icon, text};
use shadcn::{
    ButtonSize, ButtonVariant, Orientation, Part, StepState, StepperItem, StepperProps, button, stepper,
    stepper_description, stepper_indicator, stepper_item, stepper_separator, stepper_title, stepper_trigger,
    stepper_trigger_as, theme,
};

use crate::{Render, Shot, demo, page};

const STEPS: [(usize, &str, &str); 3] = [
    (1, "Your details", "Provide your name and email"),
    (2, "Company details", "A few details about your company"),
    (3, "Invite your team", "Start collaborating with your team"),
];

/// `StepperVertical.vue`'s steps, whose descriptions show in full.
const VERTICAL_STEPS: [(usize, &str, &str); 3] = [
    (
        1,
        "Your details",
        "Provide your name and email address. We will use this information to create your account",
    ),
    (
        2,
        "Company details",
        "A few details about your company will help us personalize your experience",
    ),
    (
        3,
        "Invite your team",
        "Start collaborating with your team by inviting them to join your account. You can skip this step and invite them later",
    ),
];

const DEMO_STEPS: [(usize, &str, &str); 4] = [
    (1, "Address", "Add your address"),
    (2, "Shipping", "Set your preferred"),
    (3, "Payment", "Add any payment"),
    (4, "Checkout", "Confirm your order"),
];

/// The horizontal demos' line from step `step` to the next: `absolute
/// left-[calc(50%+20px)] right-[calc(-50%+10px)] top-5 h-0.5 rounded-full`.
pub(super) fn line(item: StepperItem) -> gui::Div {
    let t = theme();
    stepper_separator(item)
        .position(Position::Absolute)
        .left(Length::Fraction(0.5))
        .margin_left(20.0)
        .right(Length::Fraction(-0.5))
        .margin_right(10.0)
        .top(20.0)
        .height(2.0)
        .flex_shrink(0.0)
        .radius(9999.0)
        .when(item.state == StepState::Completed, |d| d.background(t.primary))
}

/// The demos' trigger: a round icon button, filled once reached, ringed
/// while active, marked with a check, a circle or a dot.
pub(super) fn round_trigger(item: StepperItem) -> Part<()> {
    let t = theme();
    let state = item.state;
    let mark = match state {
        StepState::Completed => icon(Icon::Check).size(20.0),
        StepState::Active => icon(Icon::Circle),
        StepState::Inactive => icon(Icon::Dot),
    };
    stepper_trigger_as(
        button(
            if state == StepState::Inactive {
                ButtonVariant::Outline
            } else {
                ButtonVariant::Default
            },
            ButtonSize::Icon,
        )
        .radius(9999.0)
        .flex_shrink(0.0)
        // `ring-2 ring-ring ring-offset-2 ring-offset-background`.
        .when(state == StepState::Active, |b| {
            // The default variant has no shadow of its own.
            b.shadow([
                Shadow {
                    x: 0.0,
                    y: 0.0,
                    blur: 0.0,
                    spread: 4.0,
                    color: t.ring,
                },
                Shadow {
                    x: 0.0,
                    y: 0.0,
                    blur: 0.0,
                    spread: 2.0,
                    color: t.background,
                },
            ])
        })
        .child(mark),
    )
}

/// `StepperVertical.vue`: steps down a column, a line from each trigger to the next.
fn vertical() -> Part<StepperProps> {
    let last = VERTICAL_STEPS.len();
    stepper()
        .orientation(Orientation::Vertical)
        .margin_x(Length::Auto)
        .width(pct(100.0))
        .max_width(448.0)
        .justify_content(Justify::Start)
        .gap(40.0)
        .children(VERTICAL_STEPS.map(|(step, title, description)| {
            stepper_item(step)
                .position(Position::Relative)
                .width(pct(100.0))
                .align_items(Align::Start)
                .gap(24.0)
                .fill(move |item, d| {
                    let t = theme();
                    let active = item.state == StepState::Active;
                    d.when(step != last, |d| {
                        // `absolute left-[18px] top-[38px] h-[105%] w-0.5 rounded-full`.
                        d.child(
                            stepper_separator(item)
                                .position(Position::Absolute)
                                .left(gui::px(18.0))
                                .top(gui::px(38.0))
                                .height(Length::Fraction(1.05))
                                .width(2.0)
                                .flex_shrink(0.0)
                                .radius(9999.0)
                                .when(item.state == StepState::Completed, |d| d.background(t.primary)),
                        )
                    })
                    .child(round_trigger(item))
                    .child(
                        div()
                            .display(Display::Flex)
                            .flex_direction(FlexDirection::Column)
                            .gap(4.0)
                            .child(
                                // `text-sm lg:text-base`: the reference renders at desktop width.
                                stepper_title(title)
                                    .text_size(16.0)
                                    .line_height(24.0)
                                    .font_weight(600)
                                    .when(active, |d| d.text_color(t.primary)),
                            )
                            // `text-xs lg:text-sm`; its `text-primary` loses to
                            // `text-muted-foreground` in the stylesheet's order.
                            .child(stepper_description(description).text_size(14.0).line_height(20.0)),
                    )
                })
        }))
}

pub fn render() -> Render {
    let horizontal = Signal::labeled("stepper step", 1_usize);
    let indicator = Signal::labeled("stepper step", 1_usize);
    Box::new(move || {
        let last = STEPS.len();
        let steps = stepper()
            .model(Model::bind(horizontal))
            .width(pct(100.0))
            .align_items(Align::Start)
            .gap(8.0)
            .children(STEPS.map(|(step, title, description)| {
                stepper_item(step)
                    .position(Position::Relative)
                    .display(Display::Flex)
                    .width(pct(100.0))
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .align_items(Align::Center)
                    .justify_content(Justify::Center)
                    .fill(move |item, d| {
                        let t = theme();
                        d.when(step != last, |d| d.child(line(item)))
                            .child(round_trigger(item))
                            .child(
                                div()
                                    .margin_top(20.0)
                                    .display(Display::Flex)
                                    .flex_direction(FlexDirection::Column)
                                    .align_items(Align::Center)
                                    .text_align(TextAlign::Center)
                                    .child(
                                        // `text-sm lg:text-base`: the reference renders at desktop width.
                                        stepper_title(title)
                                            .text_size(16.0)
                                            .line_height(24.0)
                                            .font_weight(600)
                                            .when(item.state == StepState::Active, |d| d.text_color(t.primary)),
                                    )
                                    .child(
                                        // `text-xs lg:text-sm`; its `text-primary` loses to
                                        // `text-muted-foreground` in the stylesheet's order.
                                        stepper_description(description).text_size(14.0).line_height(20.0),
                                    ),
                            )
                    })
            }));
        let demo_last = DEMO_STEPS.len();
        let indicators = stepper()
            .model(Model::bind(indicator))
            .width(640.0)
            .align_items(Align::Start)
            .gap(8.0)
            .children(DEMO_STEPS.map(|(step, title, description)| {
                stepper_item(step)
                    .position(Position::Relative)
                    .display(Display::Flex)
                    .width(pct(100.0))
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .align_items(Align::Center)
                    .justify_content(Justify::Center)
                    .fill(move |item, d| {
                        let t = theme();
                        d.child(
                            stepper_trigger().child(
                                stepper_indicator(item)
                                    .when(item.state == StepState::Inactive, |d| d.background(t.muted))
                                    .child(match step {
                                        1 => icon(Icon::BookUser).into_element(),
                                        2 => icon(Icon::Truck).into_element(),
                                        3 => icon(Icon::CreditCard).into_element(),
                                        _ => text(step.to_string()).into_element(),
                                    }),
                            ),
                        )
                        .when(step != demo_last, |d| d.child(line(item)))
                        .child(
                            div()
                                .display(Display::Flex)
                                .flex_direction(FlexDirection::Column)
                                .align_items(Align::Center)
                                .child(stepper_title(title))
                                .child(stepper_description(description)),
                        )
                    })
            }));
        page(
            "Stepper",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(40.0)
                .child(
                    text(format!("Steps: {} and {}", horizontal.get(), indicator.get()))
                        .text_size(14.0)
                        .line_height(20.0),
                )
                .child(demo("StepperHorizental.vue", steps))
                .child(demo("StepperDemo.vue", indicators))
                .child(demo("StepperVertical.vue", vertical())),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    // Each trigger sits 20px plus half its 36px above its title.
    let (x, y) = s.locate("Invite your team")?;
    s.click(x, y - 50.0);
    s.frame("step 3 unreachable from step 1")?;
    let (x, y) = s.locate("Company details")?;
    s.click(x, y - 50.0);
    s.frame("advanced to step 2")?;
    let (x, y) = s.locate("Invite your team")?;
    s.click(x, y - 50.0);
    s.frame("advanced to step 3")?;
    let (x, y) = s.locate("Shipping")?;
    s.click(x, y - 30.0);
    s.frame("indicators on step 2")?;
    Ok(())
}
