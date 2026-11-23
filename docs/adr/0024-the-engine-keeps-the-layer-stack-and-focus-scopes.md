# 0024. The engine keeps the layer stack and the focus scopes

Status: accepted, 2026-11-22; built 2026-11-22 (M10 S6a). Keeps reka-ui's observable guarantees and numbers for
DismissableLayer, FocusScope and Presence, and builds them from what `gui` knows rather than from the DOM. Supersedes
nothing; it replaces the per-component handling in `crates/shadcn` (`dialog::return_focus`, `FocusReturn`,
`MenuState::return_focus_on_close`, an Escape `on_key_down` on each overlay, `set_focus(None)` as a dialog opens) and
`gui`'s `on_click_outside`.

## Context

Before this change, every overlay in `crates/shadcn` handled its own Escape, presses outside and focus return. The
rules disagreed from file to file. A dialog dropped focus to nothing as it opened. Nothing stopped the page under a
modal from taking the pointer or the wheel. Screen readers read the whole page under a dialog.

reka-ui (`references/reka-ui/packages/core/src`) implements these behaviours with DOM mechanisms:

- `DismissableLayer`: a module-level stack of layer elements; `pointer-events: none` on the body; `focusin` and
  `pointerdown` listeners on the document; a branch set for "inside".
- `FocusScope`: a stack of scopes; `focusin`/`focusout` listeners; sentinels for Tab.
- `Presence`: exit animations.
- `hideOthers` (aria-hidden on siblings) for a modal.

`gui` knows everything those mechanisms guess at:

- the tree;
- which deferred layer each node is in, and which node declared it;
- the paint order;
- the focus;
- the input method's state.

`docs/research-2026-11-21-primitives.md` §2 to §5 lists the guarantees to keep and the thresholds.

## Decision

A node declares a layer with `Div::layer(Layer)`. The engine (`gui::layer`, `gui::focus`, `gui::interact`) keeps the
stack and applies every rule below. Components only answer requests.

### The stack

- Each entry is a node with a `Layer` spec, which holds:
  - `modal`;
  - a `DismissPolicy` (Escape, pointer outside, focus outside, scroll);
  - an `on_dismiss` handler;
  - an optional `FocusScope`;
  - an optional `LayerHandle`.
- The entry's owner is the node that declared the deferred layer the entry sits in (the trigger's side of the
  tree).
- **Stack order is opening order, and paint order follows it**, as reka's stack and portals appended to `<body>`
  have it. The main tree comes first, then each deferred layer in the order it opened: layout keeps each layer's place
  in opening order across frames and lays the layers out in that order. Layers that open in the same frame keep the
  order layout finds them in. A layer declared inside another opens after it, so it is above it. Escape, the top
  focus scope and the modal floor all read this one order: the layer opened last is on top, wherever it is declared.
- An entry inside an inert subtree, or inside an invisible one, is not in the stack. This includes a subtree leaving
  under an exit animation, which motion makes inert: a closing layer leaves the stack as its exit begins. Focus is
  given back and the modal lifts at that moment, not when the animation ends, as reka's `Presence` unmounts the
  content's behaviour first.

### Dismiss requests

- Every request goes to `on_dismiss(&DismissReason)` and is answered with `Dismiss::Close` or `Dismiss::Veto`.
  A veto keeps the layer open.
- The reasons are:
  - `Escape`;
  - `PointerOutside { x, y, kind, button, modifiers }`;
  - `FocusOutside`;
  - `Scroll`;
  - `Programmatic`, which `LayerHandle::dismiss` sends. Every policy takes it.
- A component reads the reason to decide. For example, a modal dialog vetoes a secondary press and a Ctrl+press, as
  reka's `DialogContentModal` does.

### Inside

Inside is defined once, in `Stack::inside`, by ownership rather than by DOM order. Node `h` is inside entry `e` when
any of these holds:

- `h` is in `e`'s subtree;
- `h` is in a layer whose owner, followed transitively, is inside `e` (a submenu or a nested popover is inside its
  menu);
- `e` is not modal and `h` is in its owner's subtree (pressing a popover's own trigger is not a press outside it; reka
  excludes the trigger by hand in each component).

The climb stops at the main tree and at `e`'s own layer. So a dialog's overlay, a sibling of the content in the
content's layer, is outside the content, and a press on it is `PointerOutside`.

This differs from reka. There, every layer after `e` in DOM order counts as inside `e`. Here, an unrelated toast or
panel above a dialog does not count as inside it.

The focus scope uses the same climb without the owner's subtree (`Stack::in_scope`). A popover's trigger is not one
of its Tab stops.

### Escape

- Escape goes first, as any key does, to the focused node's own key handler, then to its text editing, then to its
  ancestors' key handlers. Any of them may consume it, as a filter that clears its text does.
- If none of them consumes it, Escape goes to the **top** live entry whose policy takes Escape, and to no other. The
  entry consumes Escape even when it vetoes.
- Escape is never sent to a layer while an input method is composing (`InputState::composing_now`): the input method
  owns it then.

### Pointer outside

- A mouse or pen dismisses as the button goes down, on any button, as reka's `pointerdownoutside` does: primary,
  secondary, middle, and the others (`PointerButton::Other(n)`, back and forward being 3 and 4 as the DOM numbers
  them). Only the primary clicks and only the secondary runs `on_secondary_click`.
- A touch dismisses only at a tap (see Gestures), so a pan that starts outside a popover does not close it.
- A press outside is asked of every live entry that takes it, top first. Each one gets its own request.

### Focus outside and scroll

- When focus comes to a node outside an entry whose policy takes focus outside, the entry is asked after the frame
  settles. An entry is not asked in the frame it opened in.
- A wheel, or a pan, that scrolls a container asks every live entry whose owner is inside that container and whose
  policy takes `Scroll`. This is reka's tooltip rule, which closes the tooltip when its trigger scrolls away.

### Modal

- The top modal entry sets the **floor**: the first node of its frame layer. Nodes below the floor take no hit test,
  no hover, no wheel and no press. The compositor also hit-tests a barrier the size of the viewport just below the
  floor. That barrier stops the render thread's wheel scrolling and pointer events too.
- Below the floor, nodes cannot keep focus, and Tab, actions and Escape do not reach them. In AccessKit the modal's
  node is `modal`, and the roots below the floor are `hidden` (reka's `hideOthers`).
- Layers above the modal (its own submenus, or a layer declared after it) stay live.

### Focus scopes

`FocusScope { trapped, cycle, auto_focus, restore }`. The defaults are not trapped, cycling, `AutoFocus::First` and
restoring.

- **Opening.**
  - When a scope opens and focus is not already in it, focus goes to one of:
    - its first Tab stop (`First`);
    - its region (`Region`);
    - a chosen node (`Node`);
    - nowhere new (`Keep`).
  - Engine moves show a focus ring only when the input modality is `Keyboard`.
- **The scope above pauses the one below.** The top live scope is the active one. Tab and the trap apply only to it.
- **Trapped.**
  - Tab and Shift+Tab stay in the scope's stops, cycling at the ends when `cycle` holds and stopping there when it
    does not.
  - A press outside does not move focus out.
  - Focus moved out by code is pulled back to the node inside that held it last, else to the first stop.
- **Not trapped.** A scope that cycles wraps Tab at its ends. One that does not cycle lets Tab go on in the page's
  order.
- **Tab order.** The page's Tab order is tree order, with each deferred layer's nodes placed right after its owner's
  subtree, as a portal placed at the trigger would be.
- **Returning focus.** When a scope closes with `restore` set, focus goes back if the focused node went with the
  scope or is below a modal. It goes back to what held focus as the scope opened, or, if that node is gone, to the
  first focusable node of the owner (its trigger), else to the nearest focusable ancestor. "What held focus as it
  opened" means focus as the previous frame left it whenever the scope's own code moved focus inside it in the frame
  it opened (a menu focuses its content).
- **Focus taken elsewhere is not taken back.** Focus is not returned in these cases:
  - focus moved to another live node since the scope opened;
  - a press on nothing focusable dropped focus;
  - a press outside closed the layer, and it was not a primary press without Control outside a modal layer
    (reka's `hasInteractedOutside`, as its non-modal dialog, popover and dropdown menu set it, and its modal
    popover and dropdown menu for a right click). Focus that went with the layer is then dropped, as a browser
    leaves it on `<body>`.
- A component focuses its trigger itself only where reka does so unconditionally: Escape out of a menu or a
  combobox, and an item chosen. A press or focus outside leaves focus to these rules.
- **Scopes closing together** are handled bottom first, so the outermost one decides where focus goes.

### Input modality

- `gui::focus::modality()` is one engine fact: `Keyboard` after any key, `Pointer` after a pointer move, press or
  touch.
- Every focus move shows a ring exactly when the modality is `Keyboard` as it moves, whoever moves it: the engine,
  a `FocusHandle::focus()`, or `set_focus`. `set_focus(id, true)` says the keyboard moved it, so it makes the
  modality `Keyboard` first. The ring (`focus_visible`) is written only from the modality and cannot disagree
  with it. As `:focus-visible` after a key, code focusing a node while the user is at the keyboard shows the
  ring.

### Gestures

`gui::gesture` holds one state machine per touch or pen contact. A mouse does not go through it: its press is a
click wherever it is released on the same node, as Chrome's `click` is, with no slop; it dismisses as it goes down;
and its secondary button stands for the long press.

- A press released within `SLOP` = **10 px** of where it went down is a tap. 10 px is reka's NumberField distance for
  a touch or pen press.
- A press that moves past `SLOP` is a pan: a drag if a drag handler is under it, otherwise a scroll from where it went
  down.
- A touch or pen press held `LONG_PRESS` = **700 ms** within the slop is a long press. 700 ms is reka's ContextMenu
  `pressOpenDelay`. A long press runs `on_long_press`, else the secondary click, and it is no tap.
- Moves of `NOISE` = **0.01 px** or less are not moves.

### Inert

- `Div::inert(true)` makes a subtree, and every layer declared in it, take:
  - no hit test;
  - no focus (handles there are not focusable);
  - no Tab stop;
  - no layer entry;
  - no accessibility node.

### Relations

- `Div::node_ref(NodeRef)` names a node.
- `labelled_by`, `described_by`, `controls`, `owns` and `active_descendant` set AccessKit relations by `NodeRef`.
- A relation is sent only to targets present in the tree. When a target appears or goes, the node that points at it
  is sent again.

## Consequences

- Overlays in `crates/shadcn` keep their look and lose their behaviour code:
  - dialog, alert dialog, sheet and drawer: modal, trapped, Escape, and the overlay when allowed;
  - popover: a non-modal scope, with `open_auto_focus(false)` mapped to `Keep`;
  - tooltip: Escape, pointer outside and scroll, never focus outside;
  - hover card;
  - combobox;
  - navigation menu panel;
  - menus: the root's content is the menu's layer, modal except in a menubar; Escape from any level closes the whole
    menu and focuses its trigger; a press or focus outside only closes it.
  - combobox: a scope that does not trap, as its popover's; Escape focuses the trigger.
- A dialog now focuses its first Tab stop as it opens, as reka's `FocusScope` does. Before this change it dropped
  focus to nothing.
- `on_click_outside` is gone. A layer and its dismiss policy replace it.
- `primitives` (S6b) builds reka's parts on these facts and does not reimplement them.

## Revisit when

- The platform reports pen input apart from touch. Today a pen arrives as a touch with an altitude
  (`PointerKind::Pen`), and its barrel button is not reported, so a pen has no secondary press.
- A layer needs to be raised without reopening (a window-like panel brought to the front on a press): opening
  order would then become "last raised" order.
