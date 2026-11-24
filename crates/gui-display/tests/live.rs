//! The real OS, on the main thread (no test harness: AppKit's screens are
//! read there only). On macOS: the outputs are read and are sane, and a
//! display link ticks predicted times at its output's refresh, stops when
//! nobody wants it and starts again. Elsewhere, and on a machine with no
//! display, it says so and passes.

fn main() {
    match run() {
        Ok(what) => println!("live: {what}"),
        Err(why) => {
            eprintln!("live: FAILED: {why}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn run() -> Result<String, String> {
    Ok("skipped (the live check runs on macOS; X11 needs a server, see ADR-0029)".to_owned())
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "a test's times, each checked to be ordered before it is subtracted"
)]
fn run() -> Result<String, String> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use gui_display::{Displays, IDLE, Stamp, native_links_made};
    use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};

    /// AppKit with no window: the displays need none on macOS.
    struct AppKit;
    impl HasDisplayHandle for AppKit {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(DisplayHandle::appkit())
        }
    }
    impl HasWindowHandle for AppKit {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Err(HandleError::Unavailable)
        }
    }
    fn check(ok: bool, what: impl FnOnce() -> String) -> Result<(), String> {
        if ok { Ok(()) } else { Err(what()) }
    }

    let app = Arc::new(AppKit);
    let woken = Arc::new(AtomicUsize::new(0));
    let theirs = Arc::clone(&woken);
    let mut displays = Displays::connect(&app, move || {
        theirs.fetch_add(1, Ordering::SeqCst);
    })
    .map_err(|e| e.to_string())?;
    check(native_links_made() == 0, || "a link made before any reader".to_owned())?;
    let outputs = displays.outputs().to_vec();
    if outputs.is_empty() {
        return Ok("skipped (no display attached)".to_owned());
    }
    for o in &outputs {
        println!(
            "live: output {:?} native {} {:?}: {:?} px, {:.3} ms, range {:?}, {:?}, headroom {:?}",
            o.id,
            o.native,
            o.name,
            o.geometry.pixels,
            o.refresh.interval.as_secs_f64() * 1000.0,
            o.refresh.range,
            o.colour.gamut,
            o.colour.headroom,
        );
        let i = o.refresh.interval;
        check(i >= Duration::from_millis(1) && i <= Duration::from_millis(100), || {
            format!("interval {i:?}")
        })?;
        check(o.geometry.pixels.0 > 0 && o.geometry.pixels.1 > 0, || {
            "no pixels".to_owned()
        })?;
        check(o.geometry.width > 0.0 && o.geometry.height > 0.0, || {
            "no size".to_owned()
        })?;
        let h = o.colour.headroom;
        check(h.current >= 1.0 && h.potential >= h.current, || {
            format!("headroom {h:?}")
        })?;
        check(o.key.0 != [0; 16], || "no key".to_owned())?;
        check(displays.by_native(o.native).map(|f| f.id) == Some(o.id), || {
            "not found by its handle".to_owned()
        })?;
        if let Some(r) = o.refresh.range {
            check(r.shortest < r.longest && r.shortest <= i.mul_f64(1.01), || {
                format!("range {r:?} around {i:?}")
            })?;
        }
    }
    check(displays.poll().is_empty(), || "a second poll found news".to_owned())?;
    let mut ids: Vec<_> = outputs.iter().map(|o| o.id).collect();
    ids.sort();
    ids.dedup();
    check(ids.len() == outputs.len(), || "two outputs share an id".to_owned())?;

    // The display link of the first output.
    let output = outputs.first().ok_or("no output")?;
    let ticks = displays.ticks(output.id).map_err(|e| e.to_string())?;
    let mut seen = Vec::new();
    let mut after = 0;
    while seen.len() < 12 {
        let t = ticks
            .wait(after, Duration::from_secs(3))
            .ok_or("no tick within 3 s of starting the display link")?;
        after = t.sequence;
        seen.push(t);
    }
    let now = Instant::now();
    for pair in seen.windows(2) {
        let [a, b] = pair else { continue };
        check(b.sequence > a.sequence, || "sequence went back".to_owned())?;
        check(matches!(b.target, Stamp::Predicted(_)), || {
            format!("a display link's time is predicted: {:?}", b.target)
        })?;
        check(b.target.at() > a.target.at(), || "target went back".to_owned())?;
        check(b.output == output.id, || "another output's tick".to_owned())?;
    }
    // A forecast of a vblank near now, not a time from another clock.
    let last = seen.last().ok_or("no tick")?;
    let off = if last.target.at() > now {
        last.target.at() - now
    } else {
        now - last.target.at()
    };
    check(off < Duration::from_millis(500), || {
        format!("the predicted time is {off:?} from now")
    })?;
    // The cadence: targets of ticks read one after another are whole
    // refreshes apart (one, unless a tick was missed between two waits).
    let interval = last.interval.unwrap_or(output.refresh.interval);
    let slowest = output.refresh.range.map_or(interval, |r| r.longest);
    let first = seen.first().ok_or("no tick")?;
    let span = last.target.at() - first.target.at();
    let steps = u32::try_from(last.sequence - first.sequence).map_err(|e| e.to_string())?;
    let mean = span / steps.max(1);
    println!(
        "live: {} ticks, mean step {:.3} ms, link interval {:?}, mode interval {:.3} ms",
        seen.len(),
        mean.as_secs_f64() * 1000.0,
        last.interval,
        output.refresh.interval.as_secs_f64() * 1000.0
    );
    check(mean >= interval.mul_f64(0.8) && mean <= slowest.mul_f64(1.2), || {
        format!("ticks {mean:?} apart on an output refreshing every {interval:?} (slowest {slowest:?})")
    })?;

    // Unwanted for the idle time, the link asks to be stopped, and is.
    ticks.run(false);
    let before = woken.load(Ordering::SeqCst);
    std::thread::sleep(IDLE + Duration::from_millis(200));
    check(woken.load(Ordering::SeqCst) > before, || {
        "an idle link did not ask to be stopped".to_owned()
    })?;
    displays.poll();
    std::thread::sleep(Duration::from_millis(100));
    let stopped = ticks.latest().map_or(0, |t| t.sequence);
    std::thread::sleep(Duration::from_millis(200));
    check(ticks.latest().map_or(0, |t| t.sequence) == stopped, || {
        "a stopped link still ticks".to_owned()
    })?;
    // Wanted again, it is started at the next poll.
    let before = woken.load(Ordering::SeqCst);
    ticks.run(true);
    check(woken.load(Ordering::SeqCst) > before, || {
        "wanting a stopped link woke nobody".to_owned()
    })?;
    displays.poll();
    check(ticks.wait(stopped, Duration::from_secs(3)).is_some(), || {
        "a link started again does not tick".to_owned()
    })?;
    // A second reader of the same display shares its link: one dropped,
    // the other still ticks; both dropped, the link stops and is kept.
    let second = displays.ticks(output.id).map_err(|e| e.to_string())?;
    check(second.wait(0, Duration::from_secs(3)).is_some(), || {
        "a second reader gets no tick".to_owned()
    })?;
    check(native_links_made() == 1, || {
        format!("two readers of one display made {} links", native_links_made())
    })?;
    drop(ticks);
    displays.poll();
    let at = second.latest().map_or(0, |t| t.sequence);
    check(second.wait(at, Duration::from_secs(3)).is_some(), || {
        "dropping one reader stopped the link under the other".to_owned()
    })?;
    drop(second);
    displays.poll();
    // Subscribed again after every reader left: the same link, started again.
    let third = displays.ticks(output.id).map_err(|e| e.to_string())?;
    check(third.wait(0, Duration::from_secs(3)).is_some(), || {
        "a link stopped and started again does not tick".to_owned()
    })?;
    drop(third);
    displays.poll();
    drop(displays);

    // Connected again and again, as an app that opens and closes windows:
    // each connection's reader ticks through the one link the process
    // made for the display, never a new one.
    let key = output.key;
    for round in 1..=3 {
        let mut displays = Displays::connect(&app, || {}).map_err(|e| e.to_string())?;
        let id = displays
            .outputs()
            .iter()
            .find(|o| o.key == key)
            .map(|o| o.id)
            .ok_or("the display is gone")?;
        let ticks = displays.ticks(id).map_err(|e| e.to_string())?;
        check(ticks.wait(0, Duration::from_secs(3)).is_some(), || {
            format!("connection {round}: no tick")
        })?;
        drop(ticks);
        displays.poll();
        drop(displays);
        check(native_links_made() == 1, || {
            format!("after {round} reconnections, {} links made", native_links_made())
        })?;
    }
    Ok(format!(
        "{} output(s), display link ticking and stopping, one link made for the display",
        outputs.len()
    ))
}
