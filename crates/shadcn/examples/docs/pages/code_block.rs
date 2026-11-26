mod stream;

use gui::prelude::*;
use gui::{Div, Element, Icon, Model, ScenarioError, Signal, div, icon, span, text};
use shadcn::focus::StateLook;
use shadcn::{
    Button, ButtonSize, ButtonVariant, CodeBlockContentProps, CodeBlockLine, CodeBlockLineVariant, CodeBlockProps,
    CodeBlockSize, CodeBlockVariant, MessageProps, TraceStatus, button, code_block, code_block_actions,
    code_block_content, code_block_copy_button, code_block_floating_actions, code_block_footer, code_block_header,
    code_block_lines, code_block_title, highlighted_lines, message, message_content, tabs, tabs_content, tabs_list,
    tabs_trigger, theme, trace, trace_collapsible, trace_collapsible_content, trace_collapsible_trigger, trace_icon,
    trace_line, trace_title,
};

use super::thread::{user_turn, wait};
use crate::{Render, Shot, demo, page};

const COMMAND: &str = "pnpm add ai @ai-sdk/vue zod";
const INSTALL: &str = "$ pnpm add ai @ai-sdk/vue zod
Packages: +14
Progress: resolved 412, reused 398, downloaded 14, added 14, done
Done in 2.8s using pnpm v10.18.0";
const BUILD: &str = "$ pnpm build
> nuxt build

 ✔ Client built in 4215ms
 ✔ Server built in 1803ms

Route                      Size  Gzip
┌ ○ /                    1.9 kB  0.9 kB
├ ○ /about               993 B   0.5 kB
└ ƒ /api/chat            141 B   0.1 kB";
const PACKAGE: &str = r#"{
  "name": "chat",
  "private": true,
  "scripts": {
    "dev": "nuxt dev",
    "build": "nuxt build"
  }
}"#;
const PATCH: &str = r#" <template>
-  <p>Hello, {{ name }}</p>
+  <p class="text-sm text-muted-foreground">
+    Hello, {{ name }}!
+  </p>
 </template>"#;
const DEPENDENCIES: &str = "$ pnpm add ai @ai-sdk/vue zod
Progress: resolved 412, reused 398, downloaded 14, added 14, done
dependencies:
+ @ai-sdk/vue 3.0.12
+ ai 6.0.12
+ zod 4.1.12
Done in 2.8s using pnpm v10.18.0";
const ERROR: &str = "TypeError: Cannot read properties of undefined (reading 'map')
    at setup (components/MessageList.vue:18:22)
    at callWithErrorHandling (runtime-core.esm-bundler.js:199:19)";
const OUTPUT: &str = "$ pnpm add ai @ai-sdk/vue zod
Progress: resolved 412, reused 398, downloaded 14, added 14, done
Done in 2.8s using pnpm v10.18.0";
const TOO_LARGE: &str = "Error: The model \"anthropic/claude-sonnet-4.5\" returned an error: Request too large. The prompt is 212,431 tokens and the context window is 200,000 tokens. Remove some messages or summarize the conversation and try again.";
const NUXT: &str = r#"export default defineNuxtConfig({
  experimental: {
    viewTransition: true,
    typedPages: true,
  },
  image: {
    domains: ["avatars.githubusercontent.com"],
  },
})"#;
const PROJECTS: &str = r#"  if (!session) {
    throw createError({ statusCode: 401 })
  }

  const projects = await db.project.findMany({
    where: { ownerId: session.user.id },
    orderBy: { updatedAt: "desc" },
  })

  return projects
})"#;
const CHAT_ROUTE: &str = r#"import { streamText } from "ai"

export default defineEventHandler(async (event) => {
  const { messages } = await readBody(event)

  const result = streamText({
    model: "anthropic/claude-sonnet-4.5",
    system: "You are a helpful assistant. Answer in one short paragraph.",
    messages,
  })

  return result.toUIMessageStreamResponse()
})"#;
const CHAT_ROUTE_SHORT: &str = r#"import { streamText } from "ai"

export default defineEventHandler(async (event) => {
  const { messages } = await readBody(event)

  const result = streamText({
    model: "anthropic/claude-sonnet-4.5",
    messages,
  })

  return result.toUIMessageStreamResponse()
})"#;
const CHAT_ROUTE_ONE_LINE: &str = r#"import { streamText } from "ai"

export default defineEventHandler(async (event) => {
  const { messages } = await readBody(event)
  const result = streamText({ model: "anthropic/claude-sonnet-4.5", messages })

  return result.toUIMessageStreamResponse()
})"#;
const LINT: &str = "$ pnpm lint
app/components/MessageList.vue
  18:22  error    'messages' is possibly 'undefined'. Add a guard before calling map  ts/no-unsafe-call
  42:7   warning  'scrollToBottom' is assigned a value but never used  unused-imports/no-unused-vars

\u{2716} 2 problems (1 error, 1 warning)";
const TESTS: &str = "$ pnpm test
\x1B[32m\u{2713}\x1B[39m src/format.test.ts \x1B[2m(4 tests)\x1B[22m
\x1B[32m\u{2713}\x1B[39m src/parse.test.ts \x1B[2m(7 tests)\x1B[22m
\x1B[31m\u{2717}\x1B[39m src/stream.test.ts \x1B[2m(3 tests | 1 failed)\x1B[22m

\x1B[31mAssertionError: expected 'hello' to be 'hello world'\x1B[39m

Tests  \x1B[31m1 failed\x1B[39m | \x1B[32m13 passed\x1B[39m (14)";
const WEATHER: &str = r#"{
  "location": "San Francisco",
  "temperature": 18,
  "unit": "celsius",
  "conditions": ["fog", "wind"]
}"#;
const SAMPLES: [(&str, &str, &str); 3] = [
    (
        "css",
        "assets/css/main.css",
        r#"@import "tailwindcss";

:root {
  --radius: 0.625rem;
}"#,
    ),
    (
        "python",
        "main.py",
        r#"def greet(name: str) -> str:
    return f"Hello, {name}!""#,
    ),
    (
        "text",
        "notes.txt",
        "A language that is not in the list renders as plain text.",
    ),
];
const CHAT_COMPONENT: &str = r#"<script setup lang="ts">
import { Chat } from "@ai-sdk/vue"
import { ref } from "vue"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"

const input = ref("")
const chat = new Chat({})

function onSubmit() {
  chat.sendMessage({ text: input.value })
  input.value = ""
}
</script>

<template>
  <div class="flex flex-col gap-4">
    <div v-for="message in chat.messages" :key="message.id">
      <template v-for="(part, index) in message.parts" :key="index">
        <p v-if="part.type === 'text'">
          {{ part.text }}
        </p>
      </template>
    </div>
    <form @submit.prevent="onSubmit">
      <Input v-model="input" />
      <Button :disabled="chat.status !== 'ready'">
        Send
      </Button>
    </form>
  </div>
</template>"#;
/// `CodeBlockTabsDemo`'s files: value, title, language, code.
const FILES: [(&str, &str, &str, &str); 3] = [
    ("route", "server/api/chat.post.ts", "ts", CHAT_ROUTE_ONE_LINE),
    (
        "chat",
        "components/Chat.vue",
        "vue",
        r#"<script setup lang="ts">
import { Chat } from "@ai-sdk/vue"

const chat = new Chat({})
</script>

<template>
  <MessageList :messages="chat.messages" />
</template>"#,
    ),
    ("env", ".env", "bash", "AI_GATEWAY_API_KEY=your-api-key"),
];
const INSTALL_AND_RUN: &str = "pnpm add ai @ai-sdk/vue zod
pnpm dev";
const NODE_VERSION: &str = "$ node --version
v24.18.0";
const ADDRESS_IN_USE: &str = "$ pnpm dev
Error: listen EADDRINUSE: address already in use :::3000
    at Server.setupListenHandle [as _listen2] (node:net:1908:16)";

/// `w-full max-w-md`.
fn frame() -> Div {
    div().width(pct(100.0)).max_width(448.0)
}

fn with_header(p: CodeBlockProps) -> CodeBlockProps {
    CodeBlockProps { header: true, ..p }
}

fn plain() -> CodeBlockContentProps {
    CodeBlockContentProps::default()
}

fn wrapped() -> CodeBlockContentProps {
    CodeBlockContentProps { wrap: true, ..plain() }
}

fn numbered() -> CodeBlockContentProps {
    CodeBlockContentProps {
        line_numbers: true,
        ..plain()
    }
}

/// A block with a header: title (and icon), a copy button, then the code.
fn titled(
    p: CodeBlockProps,
    glyph: Option<Icon>,
    title: &str,
    copy: Option<&str>,
    c: CodeBlockContentProps,
    lines: Vec<CodeBlockLine>,
) -> Div {
    let p = with_header(p);
    code_block(p)
        .child(
            code_block_header(p)
                .child(code_block_title(p, glyph, title))
                .child(copy.map(|value| code_block_actions().child(code_block_copy_button(value)))),
        )
        .child(code_block_content(p, c, lines))
}

/// `code` highlighted in `language`, each line marked by `variant(index)`.
fn marked(code: &str, language: &str, variant: impl Fn(usize) -> CodeBlockLineVariant) -> Vec<CodeBlockLine> {
    highlighted_lines(code, language)
        .into_iter()
        .enumerate()
        .map(|(i, mut line)| {
            line.variant = variant(i);
            line
        })
        .collect()
}

/// A header holding a title and `actions`.
fn header(p: CodeBlockProps, glyph: Option<Icon>, title: &str, actions: Vec<Element>) -> Div {
    code_block_header(p)
        .child(code_block_title(p, glyph, title))
        .child(code_block_actions().children(actions))
}

/// The wrap toggle: `aria-pressed:text-foreground`.
fn wrap_button(wrap: &Signal<bool>) -> Button {
    let toggle = *wrap;
    button(ButtonVariant::Ghost, ButtonSize::IconXs)
        .named("wrap-lines")
        .when(wrap.get(), |b| {
            // Over the ghost button's rest: its hover text stays over it.
            let i = b.interaction();
            b.text_color(theme().foreground)
                .text_color_while(move || i.hovered(), theme().accent_foreground)
        })
        .child(icon(Icon::WrapText).size(12.0))
        .on_click(move |_| toggle.update(|w| *w = !*w))
}

/// The download button. Vue saves the code as a file; `gui` has no way to
/// save one yet, so it is drawn and takes focus but saves nothing.
fn download_button() -> Button {
    button(ButtonVariant::Ghost, ButtonSize::IconXs).child(icon(Icon::Download).size(12.0))
}

/// `output` without its ANSI escapes (`/\x1B\[[\d;]*m/g`), as the copy button copies it.
fn ansi_plain(output: &str) -> String {
    let mut plain = String::with_capacity(output.len());
    let mut rest = output;
    while let Some(at) = rest.find("\x1B[") {
        plain.push_str(rest.get(..at).unwrap_or_default());
        let after = rest.get(at.saturating_add(2)..).unwrap_or_default();
        let params = after
            .find(|c: char| !(c.is_ascii_digit() || c == ';'))
            .unwrap_or(after.len());
        rest = match after.get(params..) {
            Some(tail) if tail.starts_with('m') => tail.get(1..).unwrap_or_default(),
            _ => {
                plain.push_str("\x1B[");
                after
            }
        };
    }
    plain.push_str(rest);
    plain
}

/// `[01] GET /api/chat 200 in 12ms`, forty of them.
fn logs() -> String {
    (1..=40_usize)
        .map(|n| {
            let ms = 12_usize.saturating_add(n.saturating_sub(1).saturating_mul(7) % 40);
            format!("[{n:02}] GET /api/chat 200 in {ms}ms")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `CodeBlockShowMoreDemo` (`scroll: false`) and `CodeBlockShowMoreScrollDemo`:
/// the Vue file folded to `max-h-64`; expanded it shows in full, or scrolls
/// inside `max-h-96`.
fn show_more(expanded: &Signal<bool>, scroll: bool) -> Div {
    let p = CodeBlockProps {
        header: true,
        footer: true,
        ..CodeBlockProps::default()
    };
    let open = expanded.get();
    let content = code_block_content(p, numbered(), highlighted_lines(CHAT_COMPONENT, "vue"));
    let content = match (open, scroll) {
        (false, _) => {
            let mut c = content.max_height(256.0).mask(ScrollFade {
                axis: Axis::Vertical,
                start: true,
                end: true,
                size: None,
            });
            c.style_mut().overflow_y = gui::style::Overflow::Hidden;
            c
        }
        (true, true) => content.max_height(384.0).mask(ScrollFade {
            axis: Axis::Vertical,
            start: true,
            end: true,
            size: None,
        }),
        (true, false) => content,
    };
    let toggle = *expanded;
    frame().child(
        code_block(p)
            .child(header(
                p,
                None,
                "components/Chat.vue",
                vec![code_block_copy_button(CHAT_COMPONENT).into_element()],
            ))
            .child(content)
            .child(
                code_block_footer(p).justify_content(Justify::Center).child(
                    button(ButtonVariant::Ghost, ButtonSize::Xs)
                        .child(icon(Icon::ChevronDown).rotate(if open { 180.0 } else { 0.0 }))
                        .child(if open { "Show less" } else { "Show more" })
                        .on_click(move |_| toggle.update(|o| *o = !*o)),
                ),
            ),
    )
}

/// One step of `CodeBlockTraceDemo`: a command, open on its output.
fn ran(open: &Signal<bool>, failed: bool, title: &str, output: &str) -> gui::Element {
    let p = CodeBlockProps {
        size: CodeBlockSize::Sm,
        variant: if failed {
            CodeBlockVariant::Destructive
        } else {
            CodeBlockVariant::Default
        },
        ..CodeBlockProps::default()
    };
    let line = trace_line(failed.then_some(TraceStatus::Error))
        .child(trace_icon(icon(Icon::Terminal)))
        .child(trace_title(title));
    trace_collapsible()
        .model(Model::bind(*open))
        .child(trace_collapsible_trigger(failed, line))
        .child(trace_collapsible_content().map_root(|c| {
            c.child(code_block(p).child(code_block_content(p, wrapped(), code_block_lines(output)).max_height(256.0)))
        }))
        .into_element()
}

/// typeset's inline code at `size` px: `0.85em`, `bg-muted`,
/// `padding: 0.125em 0.3em`, `border-radius: min(var(--radius) * 0.6, 0.35em)`.
fn typeset_code(code: &str, size: f32) -> gui::Span {
    let em = 0.85 * size;
    span(code)
        .font_family(FontFamily::GEIST_MONO)
        .text_size(em)
        .line_height(em)
        .background(theme().muted)
        .radius((10.0_f32 * 0.6).min(0.35 * em))
        .padding_x(0.3 * em)
        .padding_y(0.125 * em)
}

/// `CodeBlockTypesetDemo`: `.typeset` at 16px, its heading, prose with
/// inline code, and a code block placed in the flow.
fn typeset() -> Div {
    const SIZE: f32 = 16.0;
    let flow = 1.25 * SIZE;
    let p = with_header(CodeBlockProps::default());
    let prose = |d: gui::Text| d.line_height(1.75 * SIZE);
    frame()
        .text_size(SIZE)
        .line_height(1.75 * SIZE)
        .child(
            text("Stream a response")
                .text_size(1.125 * SIZE)
                .line_height(1.125 * SIZE * 1.45)
                .font_weight(600),
        )
        .child(
            prose(text("Create a server route that calls "))
                .span(typeset_code("streamText", SIZE))
                .span(" and returns the stream to the client.")
                // `h3 + *`: the heading owns `1em` below it.
                .margin_top(SIZE),
        )
        .child(
            code_block(p)
                // A component in the flow takes the flow above it, in its own `text-sm`.
                .margin_top(1.25 * 14.0)
                .child(header(
                    p,
                    Some(Icon::FileCode),
                    "server/api/chat.post.ts",
                    vec![code_block_copy_button(CHAT_ROUTE_ONE_LINE).into_element()],
                ))
                .child(code_block_content(
                    p,
                    numbered(),
                    highlighted_lines(CHAT_ROUTE_ONE_LINE, "ts"),
                )),
        )
        .child(
            prose(text("The "))
                .span(typeset_code("Chat", SIZE))
                .span(" class from ")
                .span(typeset_code("@ai-sdk/vue", SIZE))
                .span(" reads the stream on the client and renders the answer while it is written.")
                .margin_top(flow),
        )
}

/// The demo's state, kept across renders.
struct State {
    wrap: Signal<bool>,
    wrap_lint: Signal<bool>,
    more: Signal<bool>,
    more_scroll: Signal<bool>,
    ran: Signal<bool>,
    failed: Signal<bool>,
    streaming: stream::Streaming,
    markdown: stream::MarkdownChat,
}

pub fn render() -> Render {
    let s = State {
        wrap: Signal::labeled("chat route wrap", false),
        wrap_lint: Signal::labeled("lint wrap", true),
        more: Signal::labeled("show more", false),
        more_scroll: Signal::labeled("show more scroll", false),
        ran: Signal::new(true),
        failed: Signal::new(true),
        streaming: stream::Streaming::new(),
        markdown: stream::MarkdownChat::new(),
    };
    Box::new(move || {
        let d = CodeBlockProps::default();
        let sm = CodeBlockProps {
            size: CodeBlockSize::Sm,
            ..d
        };
        let destructive = CodeBlockProps {
            variant: CodeBlockVariant::Destructive,
            ..d
        };
        let patch: Vec<CodeBlockLine> = PATCH
            .split('\n')
            .map(|l| {
                let variant = if l.starts_with('+') {
                    CodeBlockLineVariant::Added
                } else if l.starts_with('-') {
                    CodeBlockLineVariant::Removed
                } else {
                    CodeBlockLineVariant::Default
                };
                CodeBlockLine::new(l.get(1..).unwrap_or_default(), variant)
            })
            .collect();
        let highlighted: Vec<CodeBlockLine> = NUXT
            .split('\n')
            .enumerate()
            .map(|(i, l)| {
                let v = if matches!(i, 2 | 3) {
                    CodeBlockLineVariant::Highlighted
                } else {
                    CodeBlockLineVariant::Default
                };
                CodeBlockLine::new(l, v)
            })
            .collect();
        page(
            "Code block",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "CodeBlockHeaderDemo.vue",
                    div().width(pct(100.0)).max_width(448.0).child(titled(
                        d,
                        Some(Icon::Terminal),
                        "Terminal",
                        Some(COMMAND),
                        plain(),
                        code_block_lines(INSTALL),
                    )),
                ))
                .child(demo(
                    "CodeBlockBasicDemo.vue",
                    frame().child(code_block(d).child(code_block_content(d, plain(), code_block_lines(BUILD)))),
                ))
                .child(demo(
                    "CodeBlockLineNumbersDemo.vue",
                    frame().child(code_block(d).child(code_block_content(d, numbered(), code_block_lines(PACKAGE)))),
                ))
                .child(demo(
                    "CodeBlockDiffDemo.vue",
                    frame().child(titled(d, None, "components/Greeting.vue", Some(PATCH), plain(), patch)),
                ))
                .child(demo(
                    "CodeBlockFloatingActionsDemo.vue",
                    frame().child(
                        code_block(d)
                            .child(code_block_content(d, plain(), code_block_lines(DEPENDENCIES)))
                            .child(code_block_floating_actions(d, [code_block_copy_button(DEPENDENCIES)])),
                    ),
                ))
                .child(demo(
                    "CodeBlockDestructiveDemo.vue",
                    frame().child(titled(
                        destructive,
                        None,
                        "Error",
                        Some(ERROR),
                        wrapped(),
                        code_block_lines(ERROR),
                    )),
                ))
                .child(demo(
                    "CodeBlockSizeDemo.vue",
                    frame()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(16.0)
                        .child(titled(
                            d,
                            Some(Icon::Terminal),
                            "Terminal",
                            Some(OUTPUT),
                            wrapped(),
                            code_block_lines(OUTPUT),
                        ))
                        .child(titled(
                            sm,
                            Some(Icon::Terminal),
                            "Terminal",
                            Some(OUTPUT),
                            wrapped(),
                            code_block_lines(OUTPUT),
                        )),
                ))
                .child(demo(
                    "CodeBlockWrapDemo.vue",
                    frame()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(16.0)
                        .child(code_block(d).child(code_block_content(d, plain(), code_block_lines(TOO_LARGE))))
                        .child(code_block(d).child(code_block_content(d, wrapped(), code_block_lines(TOO_LARGE)))),
                ))
                .child(demo(
                    "CodeBlockHighlightedLinesDemo.vue",
                    frame().child(code_block(d).child(code_block_content(d, numbered(), highlighted))),
                ))
                .child(demo(
                    "CodeBlockStartLineDemo.vue",
                    frame().child(titled(
                        d,
                        None,
                        "server/api/projects.get.ts",
                        None,
                        CodeBlockContentProps {
                            start_line: 995,
                            ..numbered()
                        },
                        highlighted_lines(PROJECTS, "ts"),
                    )),
                ))
                .child(demo("CodeBlockDemo.vue", {
                    let p = with_header(d);
                    let wrap = s.wrap.get();
                    div().width(pct(100.0)).max_width(512.0).child(
                        code_block(p)
                            .child(header(
                                p,
                                Some(Icon::FileCode),
                                "server/api/chat.post.ts",
                                vec![
                                    wrap_button(&s.wrap).into_element(),
                                    download_button().into_element(),
                                    code_block_copy_button(CHAT_ROUTE).into_element(),
                                ],
                            ))
                            .child(code_block_content(
                                p,
                                CodeBlockContentProps { wrap, ..numbered() },
                                marked(CHAT_ROUTE, "ts", |i| {
                                    if (5..=9).contains(&i) {
                                        CodeBlockLineVariant::Highlighted
                                    } else {
                                        CodeBlockLineVariant::Default
                                    }
                                }),
                            )),
                    )
                }))
                .child(demo("CodeBlockActionsDemo.vue", {
                    let p = with_header(d);
                    frame().child(
                        code_block(p)
                            .child(header(
                                p,
                                Some(Icon::Terminal),
                                "Terminal",
                                vec![
                                    wrap_button(&s.wrap_lint).into_element(),
                                    download_button().into_element(),
                                    code_block_copy_button(LINT).into_element(),
                                ],
                            ))
                            .child(code_block_content(
                                p,
                                CodeBlockContentProps {
                                    wrap: s.wrap_lint.get(),
                                    ..plain()
                                },
                                code_block_lines(LINT),
                            )),
                    )
                }))
                .child(demo(
                    "CodeBlockAnsiDemo.vue",
                    frame().child(titled(
                        d,
                        Some(Icon::Terminal),
                        "Terminal",
                        Some(&ansi_plain(TESTS)),
                        wrapped(),
                        highlighted_lines(TESTS, "ansi"),
                    )),
                ))
                .child(demo("CodeBlockHeightDemo.vue", {
                    let p = with_header(d);
                    let logs = logs();
                    frame().child(
                        code_block(p)
                            .child(header(
                                p,
                                None,
                                "Logs",
                                vec![code_block_copy_button(logs.clone()).into_element()],
                            ))
                            .child(
                                code_block_content(p, plain(), code_block_lines(&logs))
                                    .max_height(192.0)
                                    .mask(ScrollFade {
                                        axis: Axis::Vertical,
                                        start: true,
                                        end: true,
                                        size: None,
                                    }),
                            ),
                    )
                }))
                .child(demo(
                    "CodeBlockHighlightingDemo.vue",
                    frame().child(titled(
                        d,
                        None,
                        "getWeather output",
                        None,
                        plain(),
                        highlighted_lines(WEATHER, "json"),
                    )),
                ))
                .child(demo(
                    "CodeBlockLanguagesDemo.vue",
                    frame()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(16.0)
                        .children(SAMPLES.map(|(language, title, code)| {
                            titled(d, None, title, None, plain(), highlighted_lines(code, language))
                        })),
                ))
                .child(demo("CodeBlockChatDemo.vue", {
                    let p = MessageProps::default();
                    frame()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(16.0)
                        .text_size(14.0)
                        .line_height(20.0)
                        .child(user_turn("How do I stream a response?"))
                        .child(
                            message(p).child(
                                message_content(p)
                                    .gap(12.0)
                                    .child(text("Create a server route that returns the stream:"))
                                    .child(titled(
                                        d,
                                        None,
                                        "server/api/chat.post.ts",
                                        Some(CHAT_ROUTE_SHORT),
                                        plain(),
                                        highlighted_lines(CHAT_ROUTE_SHORT, "ts"),
                                    ))
                                    .child(text("The client reads it with Chat from @ai-sdk/vue.")),
                            ),
                        )
                }))
                .child(demo("CodeBlockShowMoreDemo.vue", show_more(&s.more, false)))
                .child(demo("CodeBlockShowMoreScrollDemo.vue", show_more(&s.more_scroll, true)))
                .child(demo(
                    "CodeBlockTabsDemo.vue",
                    tabs()
                        .default_value("route")
                        .width(pct(100.0))
                        .max_width(448.0)
                        .child(tabs_list().children(FILES.map(|(value, title, _, _)| {
                            tabs_trigger(value).child(title.rsplit('/').next().unwrap_or(title))
                        })))
                        .children(FILES.iter().map(|(value, title, language, code)| {
                            tabs_content(*value).child(titled(
                                d,
                                None,
                                title,
                                Some(code),
                                numbered(),
                                highlighted_lines(code, language),
                            ))
                        })),
                ))
                .child(demo(
                    "CodeBlockTraceDemo.vue",
                    frame().child(
                        trace()
                            .child(ran(&s.ran, false, "Ran node --version", NODE_VERSION))
                            .child(ran(&s.failed, true, "Failed to run pnpm dev", ADDRESS_IN_USE)),
                    ),
                ))
                .child(demo(
                    "CodeBlockRTLDemo.vue",
                    frame()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(12.0)
                        .text_size(14.0)
                        .line_height(20.0)
                        .direction(Direction::Rtl)
                        .child(text("ثبّت الحزم، ثم شغّل خادم التطوير."))
                        .child(titled(
                            d,
                            Some(Icon::Terminal),
                            "الطرفية",
                            Some(INSTALL_AND_RUN),
                            numbered(),
                            highlighted_lines(INSTALL_AND_RUN, "bash"),
                        )),
                ))
                .child(demo("CodeBlockTypesetDemo.vue", typeset()))
                .child(demo("CodeBlockStreamingDemo.vue", s.streaming.view()))
                .child(demo("CodeBlockMarkdownDemo.vue", s.markdown.view())),
        )
    })
}

/// The centre of the first node named `name` in the last frame.
fn named(s: &Shot, name: &str) -> Result<(f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .find(|n| n.name == Some(name))
        .map(|n| n.bounds(gui::inspect::OUTSIDE))
        .map(|b| (b.x + b.width / 2.0, b.y + b.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound(name.to_owned()))
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = named(s, "code-block-copy-button")?;
    s.hover(x, y);
    s.frame("hover copy")?;
    s.click(x, y);
    s.frame("copied")?;
    if gui::clipboard::paste().as_deref() != Some(COMMAND) {
        return Err(ScenarioError::Unhandled(
            "the copy button did not put the command on the clipboard".to_owned(),
        ));
    }
    let (x, y) = named(s, "wrap-lines")?;
    s.click(x, y);
    s.frame("chat route wrapped")?;
    s.click_text("Show more")?;
    s.frame("shown in full")?;
    s.click_text("Chat.vue")?;
    s.frame("chat tab")?;
    s.click_text("Ran node --version")?;
    s.frame("output folded")?;
    s.click_text("Replay")?;
    wait(s, 400);
    s.frame("blocks streaming")?;
    let (x, y) = named(s, "markdown-replay")?;
    s.click(x, y);
    wait(s, 900);
    s.frame("answer streaming")?;
    Ok(())
}
