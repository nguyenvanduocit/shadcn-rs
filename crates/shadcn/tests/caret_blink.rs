//! `input-otp`'s caret blinks as a text input's does: only while the
//! window has the keyboard focus, for a while from when it began, and
//! afresh when the window has the focus again. While it blinks, the render
//! thread draws every vblank; while it does not, nothing is drawn for it.

use gui::events::{Modifiers, NamedKey};
use gui::prelude::*;
use gui::{Scenario, clock};
use shadcn::{input_otp, input_otp_group, input_otp_slot};

#[test]
fn an_otp_caret_blinks_only_in_the_active_window_and_afresh_when_it_is_again() {
    let mut s = Scenario::measuring(
        || {
            input_otp()
                .child(input_otp_group((0..4).map(input_otp_slot)))
                .into_element()
        },
        400,
        100,
        1.0,
    )
    .expect("a scenario");
    s.play_motion();
    s.settle().expect("settled");
    s.press(NamedKey::Tab, Modifiers::default());
    s.settle().expect("settled");
    let blinking = |s: &mut Scenario| {
        let now = clock::frame_time();
        s.compositor().animating(now)
    };
    assert!(blinking(&mut s), "focused: the caret blinks");
    s.at(1.0);
    s.set_window_active(false);
    s.settle().expect("settled");
    assert!(
        !blinking(&mut s),
        "the window lost focus: no caret, nothing drawn for it"
    );
    // Long past the blink's end, had it run on.
    s.at(30.0);
    s.set_window_active(true);
    s.settle().expect("settled");
    assert!(blinking(&mut s), "the window has it again: the caret blinks afresh");
    s.at(60.0);
    s.settle().expect("settled");
    assert!(!blinking(&mut s), "and ends once it blinked long enough");
}
