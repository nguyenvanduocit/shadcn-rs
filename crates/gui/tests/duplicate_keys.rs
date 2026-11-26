//! Duplicate keys reach the frame's report (ADR-0026, K4g): a commit that
//! finds equal keys says so in `FrameReport::duplicate_keys`, once while the
//! key stays duplicated, and the report's text names it.

use gui::app::Scenario;
use gui::component::each;
use gui::prelude::*;
use gui::{Signal, div, text};

#[test]
fn a_duplicate_key_reaches_the_frame_report_once() {
    let items = Signal::new(vec![1_u32, 2, 3]);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(each(move || items.get(), |i| *i, |r| text(r.key().to_string())))
                .into_element()
        },
        200,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let clean = s.take_reports();
    assert!(
        clean.iter().all(|r| r.duplicate_keys.is_empty()),
        "no duplicate yet: {clean:?}"
    );

    s.act("duplicate", || items.set(vec![1, 1, 2]));
    let r = s.frame("duplicated").unwrap();
    assert_eq!(r.duplicate_keys.len(), 1, "{:?}", r.duplicate_keys);
    assert_eq!((r.duplicate_keys[0].key.as_str(), r.duplicate_keys[0].count), ("1", 2));
    assert!(r.to_string().contains("DUP KEY"), "{r}");

    // Still duplicated: not reported again.
    s.act("still", || items.set(vec![1, 1, 3]));
    let r = s.frame("still duplicated").unwrap();
    assert!(r.duplicate_keys.is_empty(), "{:?}", r.duplicate_keys);
}
