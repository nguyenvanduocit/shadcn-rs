//! What the message scroller demos share: shadcn-vue's
//! `lib/message-scroller-demo.ts` (a scripted conversation that shows a user
//! turn at once and streams the reply after it, standing in for `useChat`)
//! and `components/MessageAnimated.vue` (one turn as a scroller item).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gui::prelude::*;
use gui::{Div, Signal, clock, text};
use shadcn::{BubbleAlign, BubbleProps, BubbleVariant, MessageProps, bubble, bubble_content, message, message_content};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DemoMessage {
    pub id: String,
    pub role: Role,
    pub text: String,
}

/// A conversation's turns, in order.
pub type Turns = &'static [(Role, &'static str)];

/// `useDemoChat`: the turns shown so far, and whether a reply is streaming.
#[derive(Clone)]
pub struct DemoChat {
    turns: Turns,
    /// Turns shown before the first send.
    initial: usize,
    /// Delay between streamed chunks.
    chunk_delay: Duration,
    pub messages: Signal<Vec<DemoMessage>>,
    pub streaming: Signal<bool>,
    /// Bumped by `reset`, so the stream it cut stops.
    run: Rc<Cell<u64>>,
}

/// Characters revealed per chunk.
const CHUNK_SIZE: usize = 3;

fn shown(turns: Turns, count: usize) -> Vec<DemoMessage> {
    turns
        .iter()
        .take(count)
        .enumerate()
        .map(|(i, (role, text))| DemoMessage {
            id: format!("demo-{i}"),
            role: *role,
            text: (*text).to_owned(),
        })
        .collect()
}

impl DemoChat {
    pub fn new(turns: Turns, initial: usize, chunk_delay_ms: u64) -> Self {
        Self {
            turns,
            initial,
            chunk_delay: Duration::from_millis(chunk_delay_ms),
            messages: Signal::labeled("demo chat messages", shown(turns, initial)),
            streaming: Signal::labeled("demo chat streaming", false),
            run: Rc::new(Cell::new(0)),
        }
    }

    /// The next user turn to send, if the script has one.
    pub fn next_message(&self) -> Option<&'static str> {
        match self.turns.get(self.messages.get().len()) {
            Some((Role::User, text)) => Some(text),
            _ => None,
        }
    }

    /// Show the next user turn and stream the reply after it.
    pub fn send(&self) {
        if self.streaming.get() {
            return;
        }
        let index = self.messages.get().len();
        let Some((Role::User, words)) = self.turns.get(index) else {
            return;
        };
        self.messages.update(|m| {
            m.push(DemoMessage {
                id: format!("demo-{index}"),
                role: Role::User,
                text: (*words).to_owned(),
            });
        });
        let next = index.saturating_add(1);
        if let Some((Role::Assistant, reply)) = self.turns.get(next) {
            self.stream(reply, next);
        }
    }

    fn stream(&self, reply: &'static str, index: usize) {
        self.messages.update(|m| {
            m.push(DemoMessage {
                id: format!("demo-{index}"),
                role: Role::Assistant,
                text: String::new(),
            });
        });
        self.streaming.set(true);
        self.tick(reply, 0, self.run.get());
    }

    /// One `setInterval` tick: `CHUNK_SIZE` more characters of `reply`.
    fn tick(&self, reply: &'static str, shown: usize, run: u64) {
        let me = self.clone();
        clock::after(self.chunk_delay, move || {
            if me.run.get() != run {
                return;
            }
            let count = shown.saturating_add(CHUNK_SIZE);
            let end = reply.char_indices().nth(count).map_or(reply.len(), |(i, _)| i);
            let part = reply.get(..end).unwrap_or(reply).to_owned();
            me.messages.update(|m| {
                if let Some(last) = m.last_mut() {
                    last.text = part;
                }
            });
            if end >= reply.len() {
                me.streaming.set(false);
            } else {
                me.tick(reply, count, run);
            }
        });
    }

    pub fn reset(&self) {
        self.run.set(self.run.get().wrapping_add(1));
        self.streaming.set(false);
        self.messages.set(shown(self.turns, self.initial));
    }
}

/// `MessageAnimated.vue`: `m` as a scroller message, its paragraphs (split
/// on blank lines) in one bubble, a user turn at the end. Vue names an
/// entrance animation per preset in a class no stylesheet defines, so
/// nothing animates there or here.
pub fn animated(m: &DemoMessage, user: BubbleVariant, assistant: BubbleVariant) -> Div {
    let is_user = m.role == Role::User;
    let (align, variant) = if is_user {
        (BubbleAlign::End, user)
    } else {
        (BubbleAlign::Start, assistant)
    };
    let p = MessageProps {
        align,
        bubble: variant,
        ..MessageProps::default()
    };
    let b = BubbleProps { variant, align };
    let paragraphs = m.text.split("\n\n").map(str::trim).filter(|s| !s.is_empty());
    message(p).child(
        message_content(p).child(
            bubble(b).child(
                bubble_content(b).children(
                    paragraphs
                        .enumerate()
                        .map(|(i, para)| text(para).when(i > 0, |t| t.margin_top(8.0))),
                ),
            ),
        ),
    )
}
