//! What the agent demos share (chat, reasoning, plan, tool, subagent,
//! terminal, stack trace, test results, the streamed code blocks): the
//! column a conversation sits in, its turns, a command's output, and the
//! script that replays one in place of a model (Vue's `sleep` and
//! `streamInto` over a `run` counter).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gui::prelude::*;
use gui::{Div, Element, Signal, div, text};
use shadcn::{
    BubbleAlign, BubbleProps, BubbleVariant, ButtonSize, ButtonVariant, CodeBlockContentProps, CodeBlockProps,
    CodeBlockSize, MessageProps, bubble, bubble_content, button, code_block, code_block_content, code_block_lines,
    message, message_content,
};

/// One step of a script: wait `ms`, then act.
pub struct Step {
    ms: u64,
    act: Rc<dyn Fn()>,
}

impl Step {
    /// Wait `ms` more before acting.
    #[must_use]
    pub fn lead(mut self, ms: u64) -> Self {
        self.ms = self.ms.saturating_add(ms);
        self
    }
}

/// Wait `ms`, then run `f`.
pub fn after(ms: u64, f: impl Fn() + 'static) -> Step {
    Step { ms, act: Rc::new(f) }
}

/// Show each of `frames` in turn with `show`, each its wait in ms after the
/// last (a turn replayed as snapshots: a command printing, then the answer).
pub fn frames<T: 'static>(frames: Vec<(u64, T)>, show: impl Fn(&T) + 'static) -> Vec<Step> {
    let show = Rc::new(show);
    frames
        .into_iter()
        .map(|(ms, frame)| {
            let show = Rc::clone(&show);
            after(ms, move || show(&frame))
        })
        .collect()
}

/// Vue's `text.match(/\S+\s*/g)`: each word with the spaces after it.
pub fn chunks(text: &str) -> Vec<&str> {
    let text = text.trim_start();
    // A word starts where a non-space follows a space.
    let mut starts: Vec<usize> = text
        .char_indices()
        .zip(text.chars().skip(1))
        .filter(|((_, a), b)| a.is_whitespace() && !b.is_whitespace())
        .map(|((i, a), _)| i.saturating_add(a.len_utf8()))
        .collect();
    starts.insert(0, 0);
    starts.push(text.len());
    starts
        .windows(2)
        .filter_map(|w| text.get(*w.first()?..*w.get(1)?))
        .filter(|c| !c.is_empty())
        .collect()
}

/// Vue's `streamInto(into, text, ms)`: append a word, wait `ms`, and so on;
/// the wait after the last word is the next step's lead.
pub fn stream(into: &Signal<String>, text: &'static str, ms: u64) -> Vec<Step> {
    chunks(text)
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| {
            let into = *into;
            after(if i == 0 { 0 } else { ms }, move || into.update(|t| t.push_str(chunk)))
        })
        .collect()
}

/// A replaying conversation: whether it is writing, and which run is current,
/// so the steps of a run that was replaced stop.
#[derive(Clone)]
pub struct Script {
    busy: Signal<bool>,
    run: Rc<Cell<u64>>,
}

impl Script {
    pub fn new(label: &'static str) -> Self {
        Self {
            busy: Signal::labeled(label, false),
            run: Rc::new(Cell::new(0)),
        }
    }

    /// Whether a run is writing (Vue's `busy`).
    pub fn busy(&self) -> bool {
        self.busy.get()
    }

    /// Start a run: `steps` one after another, then not busy.
    pub fn play(&self, steps: Vec<Step>) {
        let id = self.run.get().wrapping_add(1);
        self.run.set(id);
        self.busy.set(true);
        next(self.clone(), id, Rc::new(steps), 0);
    }

    /// Stop the run, if any.
    pub fn stop(&self) {
        self.run.set(self.run.get().wrapping_add(1));
        self.busy.set(false);
    }
}

/// Run step `i` and the rest after it. A step with no wait runs at once, as
/// Vue's async run goes on without yielding until its first `sleep`.
fn next(script: Script, id: u64, steps: Rc<Vec<Step>>, i: usize) {
    let Some(step) = steps.get(i) else {
        script.busy.set(false);
        return;
    };
    let (act, ms) = (step.act.clone(), step.ms);
    let run = move || {
        if script.run.get() != id {
            return;
        }
        act();
        next(script.clone(), id, steps.clone(), i.saturating_add(1));
    };
    if ms == 0 {
        run();
    } else {
        gui::clock::after(Duration::from_millis(ms), run);
    }
}

/// A collapsible row that opens and closes with its step (Vue's
/// `openState[id] ?? running`) until the reader opens or closes it.
#[derive(Debug, Clone)]
pub struct Follow {
    pub open: Signal<bool>,
    /// What [`Follow::follow`] last set; `open` differing from it means the
    /// reader has chosen.
    set: Rc<Cell<bool>>,
}

impl Follow {
    pub fn new(open: bool) -> Self {
        Self {
            open: Signal::new(open),
            set: Rc::new(Cell::new(open)),
        }
    }

    /// Open or close with the step, unless the reader has chosen.
    pub fn follow(&self, open: bool) {
        if self.open.get() == self.set.get() {
            self.open.set(open);
            self.set.set(open);
        }
    }

    /// Forget the reader's choice (a new run).
    pub fn reset(&self, open: bool) {
        self.open.set(open);
        self.set.set(open);
    }
}

/// The column a conversation sits in: `flex min-h-72 w-full max-w-sm
/// flex-col gap-4 py-6 text-sm`.
pub fn thread() -> Div {
    div()
        .flex_col()
        .min_h(72.0)
        .w_full()
        .max_w(96.0)
        .gap(4.0)
        .py(6.0)
        .text_sm()
}

/// The reader's turn: a muted bubble on the right.
pub fn user_turn(words: &str) -> Div {
    let p = MessageProps {
        align: BubbleAlign::End,
        ..MessageProps::default()
    };
    let b = BubbleProps {
        variant: BubbleVariant::Muted,
        align: BubbleAlign::End,
    };
    message(p).child(message_content(p).child(bubble(b).child(bubble_content(b).child(text(words)))))
}

/// The agent's turn: `<MessageContent class="gap-4">` around `body`.
pub fn assistant_turn(body: impl IntoIterator<Item = Element>) -> Div {
    let p = MessageProps::default();
    message(p).child(message_content(p).gap(4.0).children(body))
}

/// The agent's prose: a paragraph, nothing while it is empty.
pub fn said(words: &str) -> Element {
    if words.is_empty() {
        Element::Empty
    } else {
        text(words).into_element()
    }
}

/// A command's output: `<CodeBlock size="sm"><CodeBlockContent wrap class="max-h-64">`.
pub fn output(code: &str) -> Div {
    let p = CodeBlockProps {
        size: CodeBlockSize::Sm,
        ..CodeBlockProps::default()
    };
    let c = CodeBlockContentProps {
        wrap: true,
        ..CodeBlockContentProps::default()
    };
    code_block(p).child(code_block_content(p, c, code_block_lines(code)).max_h(64.0))
}

/// The outline button under the thread (`mt-auto self-start`), hidden while
/// the script writes.
pub fn replay(script: &Script, f: impl Fn() + 'static) -> Option<Div> {
    (!script.busy()).then(|| {
        button(ButtonVariant::Outline, ButtonSize::Sm)
            .mt_auto()
            .self_start()
            .child("Replay")
            .on_click(move |_| f())
    })
}

/// Let `ms` pass, running timers as they come due, as a window would.
pub fn wait(s: &mut crate::Shot, ms: u64) {
    s.wait(Duration::from_millis(ms));
}

/// The box of the demo `file` on the last frame.
fn demo_bounds(s: &crate::Shot, file: &str) -> Option<gui::Bounds> {
    s.last().nodes.iter().find(|n| n.name == Some(file)).map(|n| n.bounds)
}

/// Click the last node named `name` inside the demo `file` (an icon button
/// has no text to find it by).
pub fn click_named(s: &mut crate::Shot, file: &str, name: &str) -> Result<(), gui::ScenarioError> {
    // The last frame may predate timers that enabled it.
    s.settle()?;
    let inside = demo_bounds(s, file).ok_or_else(|| gui::ScenarioError::TextNotFound(file.to_owned()))?;
    let at = s
        .last()
        .nodes
        .iter()
        .rfind(|n| n.name == Some(name) && inside.contains(n.bounds.x + 1.0, n.bounds.y + 1.0))
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| gui::ScenarioError::TextNotFound(name.to_owned()))?;
    s.click(at.0, at.1);
    s.settle()
}
