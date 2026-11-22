use gui::prelude::*;
use gui::{Icon, Model, ScenarioError, Shadow, Signal, div, icon, relative, text};
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
        .absolute()
        .left_len(relative(0.5))
        .ml(5.0)
        .right_len(relative(-0.5))
        .mr(2.5)
        .top(5.0)
        .h(0.5)
        .shrink_0()
        .rounded_full()
        .when(item.state == StepState::Completed, |d| d.bg(t.primary))
}

/// The demos' trigger: a round icon button, filled once reached, ringed
/// while active, marked with a check, a circle or a dot.
pub(super) fn round_trigger(item: StepperItem) -> Part<()> {
    let t = theme();
    let state = item.state;
    let mark = match state {
        StepState::Completed => icon(Icon::Check).size(5.0),
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
        .rounded_full()
        .shrink_0()
        // `ring-2 ring-ring ring-offset-2 ring-offset-background`.
        .when(state == StepState::Active, |b| {
            b.shadow(Shadow {
                x: 0.0,
                y: 0.0,
                blur: 0.0,
                spread: 4.0,
                color: t.ring,
            })
            .shadow(Shadow {
                x: 0.0,
                y: 0.0,
                blur: 0.0,
                spread: 2.0,
                color: t.background,
            })
        })
        .child(mark),
    )
}

/// `StepperVertical.vue`: steps down a column, a line from each trigger to the next.
fn vertical() -> Part<StepperProps> {
    let last = VERTICAL_STEPS.len();
    stepper()
        .orientation(Orientation::Vertical)
        .mx_auto()
        .w_full()
        .max_w_px(448.0)
        .justify_start()
        .gap(10.0)
        .children(VERTICAL_STEPS.map(|(step, title, description)| {
            stepper_item(step)
                .relative()
                .w_full()
                .items_start()
                .gap(6.0)
                .fill(move |item, d| {
                    let t = theme();
                    let active = item.state == StepState::Active;
                    d.when(step != last, |d| {
                        // `absolute left-[18px] top-[38px] h-[105%] w-0.5 rounded-full`.
                        d.child(
                            stepper_separator(item)
                                .absolute()
                                .left_len(gui::px(18.0))
                                .top_len(gui::px(38.0))
                                .h_len(relative(1.05))
                                .w(0.5)
                                .shrink_0()
                                .rounded_full()
                                .when(item.state == StepState::Completed, |d| d.bg(t.primary)),
                        )
                    })
                    .child(round_trigger(item))
                    .child(
                        div()
                            .flex_col()
                            .gap(1.0)
                            .child(
                                // `text-sm lg:text-base`: the reference renders at desktop width.
                                stepper_title(title)
                                    .text_base()
                                    .font_semibold()
                                    .when(active, |d| d.text_color(t.primary)),
                            )
                            // `text-xs lg:text-sm`; its `text-primary` loses to
                            // `text-muted-foreground` in the stylesheet's order.
                            .child(stepper_description(description).text_sm()),
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
            .w_full()
            .items_start()
            .gap(2.0)
            .children(STEPS.map(|(step, title, description)| {
                stepper_item(step)
                    .relative()
                    .flex()
                    .w_full()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .fill(move |item, d| {
                        let t = theme();
                        d.when(step != last, |d| d.child(line(item)))
                            .child(round_trigger(item))
                            .child(
                                div()
                                    .mt(5.0)
                                    .flex_col()
                                    .items_center()
                                    .text_center()
                                    .child(
                                        // `text-sm lg:text-base`: the reference renders at desktop width.
                                        stepper_title(title)
                                            .text_base()
                                            .font_semibold()
                                            .when(item.state == StepState::Active, |d| d.text_color(t.primary)),
                                    )
                                    .child(
                                        // `text-xs lg:text-sm`; its `text-primary` loses to
                                        // `text-muted-foreground` in the stylesheet's order.
                                        stepper_description(description).text_sm(),
                                    ),
                            )
                    })
            }));
        let demo_last = DEMO_STEPS.len();
        let indicators = stepper()
            .model(Model::bind(indicator))
            .w_px(640.0)
            .items_start()
            .gap(2.0)
            .children(DEMO_STEPS.map(|(step, title, description)| {
                stepper_item(step)
                    .relative()
                    .flex()
                    .w_full()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .fill(move |item, d| {
                        let t = theme();
                        d.child(
                            stepper_trigger().child(
                                stepper_indicator(item)
                                    .when(item.state == StepState::Inactive, |d| d.bg(t.muted))
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
                                .flex_col()
                                .items_center()
                                .child(stepper_title(title))
                                .child(stepper_description(description)),
                        )
                    })
            }));
        page(
            "Stepper",
            div()
                .flex_col()
                .gap(10.0)
                .child(text(format!("Steps: {} and {}", horizontal.get(), indicator.get())).text_sm())
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
