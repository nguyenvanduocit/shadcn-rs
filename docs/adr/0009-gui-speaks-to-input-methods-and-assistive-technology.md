# 0009. `gui` speaks to input methods and assistive technology

Status: accepted, 2026-11-20.

## Context

`gui` took input only as key presses and told the platform nothing about what it drew. Two kinds of people could not
use an app built on it:

- **Anyone typing through an input method.** Vietnamese Telex, Japanese and Chinese input compose text before it becomes
  text (macOS marked text). winit delivers that as `WindowEvent::Ime` only when the window allows it, and `gui` never
  did, so none of it could be typed.
- **Anyone using a screen reader.** VoiceOver saw a window with a title and three buttons: the app's content had no
  accessibility tree.

Remit's desktop app is the first app built on `gui`, and its owner types Vietnamese to his agents.

## Options

1. **Platform APIs directly** (`NSTextInputClient`, `NSAccessibility`): full control, but `unsafe` (forbidden in the
   workspace) and one implementation per platform.
2. **winit's IME events and AccessKit** (`accesskit_winit`): both are what winit-based toolkits use (egui, Slint,
   Xilem), one implementation for macOS, Windows and Linux, and no `unsafe` of our own.
3. **A text field per platform widget** (a native `NSTextField` overlaid): native IME and accessibility for inputs only,
   nothing for the rest of the frame, and a second renderer to keep in step.

## Decision

Option 2.

- **A composition is part of the edit, not of the value.** `TextEdit::preedit` holds what the input method is composing
  (`Preedit`: its text and caret). The input shows it at the caret, underlined; the value takes it only on commit, as a
  browser's composition does. A click inside it maps back to the value (`TextEdit::uncomposed`). A password takes none.
- **The window follows focus.** After each frame the window asks for `ImeTarget` (the focused input that takes composed
  text, and its caret). It allows the input method only then, puts its candidates at the caret, and restarts it when focus
  moves to another input; a composition left in an input that lost focus is dropped.
- **A frame is an AccessKit tree.** `access::tree` is pure: each laid-out node becomes a node with the role, name and
  state of its `Semantics`, and its content implies the rest (text is a `Label` whose value is its text, an input is a
  `TextInput`, `MultilineTextInput` or `PasswordInput`, a box that takes clicks is a `Button` named by the text inside it,
  an icon or drawing with no name is left out). Ids are hashes of node paths, so a screen reader keeps its place across
  frames that rebuild everything. Each layer hangs from the window.
- **Semantics are said as ARIA says them.** `role(Role::…)`, `aria_label`, `aria_checked` (`Toggled`, so a checkbox can
  be mixed), `aria_selected`, `aria_expanded`, `aria_hidden`: a port from the web maps one for one. `shadcn` sets them
  on its checkbox, switch, radio, toggle, tabs, accordion, dialogs, sheets, drawers and menus.
- **Actions come back as input.** AccessKit's requests arrive through the event loop as `Input::Act`: `Click` runs the
  node's click handler as a key press does, `Focus` focuses it, `SetValue` sets a text input's value.
- **It costs nothing unless someone listens.** The window sends the tree from `update_if_active`, which builds it only
  while assistive technology is attached.

## Consequences

- Verified on macOS: VoiceOver's API (System Events) reads the docs app's 276 elements, buttons named from their text,
  switches as `AXSwitch` with their value, tabs as `AXTabButton` named and selected; `AXPress` on a sidebar entry opens
  its page. Composition is verified headless (`Scenario::compose`, `Scenario::commit`); typing through the system input
  method is verified only by a person at the keyboard.
- A window opens hidden and shows once the adapter is attached, as AccessKit requires.
- The tree is rebuilt whole each frame while a screen reader listens: linear in the frame's nodes.

## Revisit when

- A screen reader user needs to move by word or line inside text, or hear the caret and selection in an input: AccessKit
  text runs (`Role::TextRun`, character offsets) per paragraph.
- Text that arrives on its own (an agent's streamed reply, a toast) must be announced: `aria-live`, an AccessKit live
  setting on the node that receives it.
- A link inside a paragraph (`SpanTarget`) must be reachable by itself: a `Link` child per span.
- A screen reader scrolls a list it cannot see the end of: `ScrollIntoView` and scroll actions.
- Rebuilding the tree each frame shows in the bench while a screen reader listens: send only the nodes that changed.
