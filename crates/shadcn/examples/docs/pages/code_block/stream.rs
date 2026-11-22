//! The two code-block demos that write themselves out: `CodeBlockStreamingDemo`
//! (five blocks typed three characters at a time) and `CodeBlockMarkdownDemo`
//! (a chat answer in Markdown, a word at a time, its fence a code block).

use gui::prelude::*;
use gui::{Div, Element, Icon, Signal, Span, div, icon, span, text};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use shadcn::{
    ButtonSize, ButtonVariant, CodeBlockContentProps, CodeBlockLineVariant, CodeBlockProps, MessageProps, button,
    code_block, code_block_actions, code_block_content, code_block_copy_button, code_block_header, code_block_title,
    highlighted_lines, message, message_content, theme,
};

use super::super::thread::{Script, frames, replay, user_turn};

struct Block {
    title: Option<&'static str>,
    language: &'static str,
    code: &'static str,
    line_numbers: bool,
    highlight: &'static [usize],
    /// Each row starts with `+`, `-` or a space, which is not part of the code.
    diff: bool,
}

const BLOCKS: [Block; 5] = [
    Block {
        title: Some("composables/useDebounce.ts"),
        language: "ts",
        line_numbers: true,
        highlight: &[],
        diff: false,
        code: "export function useDebounce<T>(value: Ref<T>, delay = 300) {
  const debounced = shallowRef(value.value)

  watch(value, (next, _, onCleanup) => {
    const timeout = setTimeout(() => (debounced.value = next), delay)
    onCleanup(() => clearTimeout(timeout))
  })

  return debounced
}",
    },
    Block {
        title: None,
        language: "bash",
        line_numbers: false,
        highlight: &[],
        diff: false,
        code: "pnpm dlx shadcn-vue@latest add code-block",
    },
    Block {
        title: Some("components/Search.vue"),
        language: "vue",
        line_numbers: true,
        highlight: &[],
        diff: true,
        code: r#" <script setup lang="ts">
 const query = ref("")
-const results = useSearch(query)
+const debounced = useDebounce(query, 300)
+const results = useSearch(debounced)
 </script>

 <template>
   <SearchResults :results="results" />
 </template>"#,
    },
    Block {
        title: Some("server/api/chat.post.ts"),
        language: "ts",
        line_numbers: true,
        highlight: &[5, 6, 7, 8],
        diff: false,
        code: r#"import { streamText } from "ai"

export default defineEventHandler(async (event) => {
  const { messages } = await readBody(event)

  const result = streamText({
    model: "anthropic/claude-sonnet-4.5",
    messages,
  })

  return result.toUIMessageStreamResponse()
})"#,
    },
    Block {
        title: Some(".env"),
        language: "bash",
        line_numbers: false,
        highlight: &[],
        diff: false,
        code: "AI_GATEWAY_API_KEY=your-api-key",
    },
];

/// How many characters the stream adds each tick, and how often (ms).
const STEP: usize = 3;
const TICK: u64 = 16;

/// `CodeBlockStreamingDemo`: how much of the blocks has been written.
#[derive(Clone)]
pub struct Streaming {
    length: Signal<usize>,
    script: Script,
}

fn total() -> usize {
    BLOCKS.iter().map(|b| b.code.len()).sum()
}

impl Streaming {
    /// Written out in full, as the demo opens.
    pub fn new() -> Self {
        Self {
            length: Signal::labeled("streamed length", total()),
            script: Script::new("streaming"),
        }
    }

    fn replay(&self) {
        let total = total();
        let steps = std::iter::successors(Some(0_usize), |n| (*n < total).then(|| n.saturating_add(STEP)))
            .map(|n| (if n == 0 { 0 } else { TICK }, n.min(total)))
            .collect();
        let length = self.length;
        self.script.play(frames(steps, move |n| length.set(*n)));
    }

    pub fn view(&self) -> Div {
        let length = self.length.get();
        let mut offset = 0_usize;
        let mut blocks = Vec::new();
        for block in &BLOCKS {
            let start = offset;
            offset = offset.saturating_add(block.code.len());
            if length > start {
                blocks.push(streamed(block, length.saturating_sub(start)));
            }
        }
        let me = self.clone();
        div().flex_col().w_full().max_w(112.0).gap(4.0).children(blocks).child(
            button(ButtonVariant::Outline, ButtonSize::Sm)
                .self_start()
                .disabled(length < total())
                .child(icon(Icon::RotateCcw))
                .child("Replay")
                .on_click(move |_| me.replay()),
        )
    }
}

/// `block` with its first `written` bytes.
fn streamed(block: &Block, written: usize) -> Div {
    let code = block.code.get(..written).unwrap_or(block.code);
    let rows: Vec<&str> = code.split('\n').collect();
    let source = if block.diff {
        rows.iter()
            .map(|r| r.get(1..).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        code.to_owned()
    };
    let lines = highlighted_lines(&source, block.language)
        .into_iter()
        .enumerate()
        .map(|(i, mut line)| {
            line.variant = if block.diff {
                match rows.get(i).and_then(|r| r.chars().next()) {
                    Some('+') => CodeBlockLineVariant::Added,
                    Some('-') => CodeBlockLineVariant::Removed,
                    _ => CodeBlockLineVariant::Default,
                }
            } else if block.highlight.contains(&i) {
                CodeBlockLineVariant::Highlighted
            } else {
                CodeBlockLineVariant::Default
            };
            line
        });
    let p = CodeBlockProps {
        header: block.title.is_some(),
        ..CodeBlockProps::default()
    };
    let c = CodeBlockContentProps {
        line_numbers: block.line_numbers,
        ..CodeBlockContentProps::default()
    };
    code_block(p)
        .child(block.title.map(|title| {
            code_block_header(p).child(code_block_title(p, None, title)).child(
                code_block_actions()
                    .child(code_block_copy_button(source.clone()).disabled(code.len() < block.code.len())),
            )
        }))
        .child(code_block_content(p, c, lines))
}

const QUESTION: &str = "How do I debounce a search input in Vue?";
const RESPONSE: &str = "Wrap the value in a `useDebounce` composable. It returns the latest value once it stops changing for `delay` milliseconds.

```ts
export function useDebounce<T>(value: Ref<T>, delay = 300) {
  const debounced = shallowRef(value.value)

  watch(value, (next, _, onCleanup) => {
    const timeout = setTimeout(() => (debounced.value = next), delay)
    onCleanup(() => clearTimeout(timeout))
  })

  return debounced
}
```

Pass the debounced value to your fetch instead of the raw input.";

/// The demo streams the answer a word at a time, every 30 ms.
const WORD: u64 = 30;
/// typeset's `--typeset-leading` and `--typeset-flow` (in ems), at the
/// message's `text-sm`.
const SIZE: f32 = 14.0;
const LEADING: f32 = 1.75;
const FLOW: f32 = 1.25;

/// `CodeBlockMarkdownDemo`: the answer written so far (`None` before its
/// first word), while a replay runs.
#[derive(Clone)]
pub struct MarkdownChat {
    answer: Signal<Option<String>>,
    script: Script,
}

impl MarkdownChat {
    /// The whole exchange, as the demo opens.
    pub fn new() -> Self {
        Self {
            answer: Signal::labeled("markdown answer", Some(RESPONSE.to_owned())),
            script: Script::new("markdown replay"),
        }
    }

    fn replay(&self) {
        // JavaScript's `split(/(?<=\s)/)`: each chunk keeps the white space after it.
        let chunks: Vec<&'static str> = RESPONSE.split_inclusive(char::is_whitespace).collect();
        let count = chunks.len();
        // The demo's interval runs once more after the last word to stop itself.
        let steps = std::iter::once((0, 0))
            .chain((1..=count).map(|k| (WORD, k)))
            .chain(std::iter::once((WORD, count)))
            .collect();
        let answer = self.answer;
        self.script.play(frames(steps, move |k: &usize| {
            answer.set((*k > 0).then(|| chunks.get(..*k).unwrap_or_default().concat()));
        }));
    }

    pub fn view(&self) -> Div {
        let busy = self.script.busy();
        let me = self.clone();
        div()
            .flex_col()
            .min_h(112.0)
            .w_full()
            .max_w(112.0)
            .gap(4.0)
            .py(6.0)
            .text_sm()
            .child(user_turn(QUESTION))
            .child(self.answer.get().map(|answer| {
                let p = MessageProps::default();
                message(p).child(message_content(p).child(typeset(&answer, busy)))
            }))
            .child(replay(&self.script, move || me.replay()).map(|b| b.named("markdown-replay")))
    }
}

/// `source` as the demo's comark renders it inside `.typeset`: paragraphs
/// with inline code, and each fence as a code block whose copy button waits
/// for the closing fence while the answer streams.
fn typeset(source: &str, streaming: bool) -> Div {
    // The trailing fence is still open when an odd number of lines start with ```.
    let fences = source.trim().lines().filter(|l| l.starts_with("```")).count();
    let open_fence = streaming && fences % 2 == 1;
    let mut blocks: Vec<Element> = Vec::new();
    let mut runs: Vec<Span> = Vec::new();
    let mut fence: Option<(String, String)> = None;
    for event in Parser::new(source) {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                fence = Some((language, String::new()));
            }
            Event::Text(t) => match &mut fence {
                Some((_, code)) => code.push_str(&t),
                None => runs.push(span(t.to_string())),
            },
            Event::Code(c) => runs.push(inline_code(&c)),
            Event::SoftBreak | Event::HardBreak => runs.push(span(" ")),
            Event::End(TagEnd::Paragraph) => {
                blocks.push(text("").spans(std::mem::take(&mut runs)).into_element());
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((language, code)) = fence.take() {
                    blocks.push(code_block_of(&language, &code, false).into_element());
                }
            }
            _ => {}
        }
    }
    // A stream cut mid-block shows what has arrived.
    if let Some((language, code)) = fence.take() {
        blocks.push(code_block_of(&language, &code, open_fence).into_element());
    }
    if !runs.is_empty() {
        blocks.push(text("").spans(runs).into_element());
    }
    let gap = FLOW * SIZE / gui::styled::UNIT;
    div()
        .flex_col()
        .w_full()
        .min_w(0.0)
        .text_size(SIZE, SIZE * LEADING)
        .text_color(theme().foreground)
        .children(
            blocks
                .into_iter()
                .enumerate()
                .map(|(i, b)| div().min_w(0.0).when(i > 0, |d| d.mt(gap)).child(b)),
        )
}

/// The demo's `MarkdownCode`: `rounded-sm bg-muted px-1 py-0.5 font-mono text-[0.9em]`.
fn inline_code(code: &str) -> Span {
    let em = 0.9 * SIZE;
    span(code.to_owned())
        .font_mono()
        .text_size(em, em * LEADING)
        .bg(theme().muted)
        .rounded_sm()
        .px(1.0)
        .py(0.5)
}

/// The demo's `MarkdownCodeBlock`: the language as its title, a copy button
/// that waits for the closing fence (`incomplete`), the highlighted lines.
fn code_block_of(language: &str, code: &str, incomplete: bool) -> Div {
    let p = CodeBlockProps {
        header: true,
        ..CodeBlockProps::default()
    };
    let code = code.strip_suffix('\n').unwrap_or(code);
    let title = if language.is_empty() { "text" } else { language };
    code_block(p)
        .child(
            code_block_header(p)
                .child(code_block_title(p, None, title))
                .child(code_block_actions().child(code_block_copy_button(code).disabled(incomplete))),
        )
        .child(code_block_content(
            p,
            CodeBlockContentProps::default(),
            highlighted_lines(code, language),
        ))
}
