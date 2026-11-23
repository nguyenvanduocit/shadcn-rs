//! Behaviour without look: the state, keyboard tables, parts and context of
//! reka-ui's primitives (`references/reka-ui/packages/core/src`), built on
//! `gui`'s engine (ADR-0022, ADR-0024, ADR-0025).
//!
//! `gui` ← `primitives` ← `shadcn`. `gui` keeps what only the engine can
//! know (the layer stack, focus scopes, the input modality, gestures,
//! inertness, relations, `Members`, `Bind`/`Model`). This crate holds the
//! rules every widget follows on top of it: which item an arrow key reaches,
//! which item is a group's one Tab stop, what typing a letter selects, how a
//! click selects, how a filter matches, how a form submits and resets. It
//! holds no colour, size or spacing: `shadcn` draws each part once and hands
//! it to the part functions here, which add the behaviour
//! (`crates/primitives/README.md` has the conventions).
//!
//! The building blocks, reka's groups 1 and 2
//! (`docs/research-2026-11-21-primitives.md` §5, §6):
//!
//! - [`availability`]: the one notion of "cannot be used", in two levels;
//! - [`context`]: a part reading its root's state, and the error a part
//!   outside its root reports;
//! - [`direction`]: the reading direction a primitive navigates by;
//! - [`collection`]: items in display order, mounted ([`collection::Collection`])
//!   or by data index over any number of rows ([`collection::Items`]);
//! - [`keymap`]: arrow navigation, one keymap rule with wrap chosen per
//!   primitive;
//! - [`roving`]: a group that is one Tab stop;
//! - [`typeahead`]: typing to reach an item, the 1000 ms buffer;
//! - [`selection`]: toggle or replace, ranges with Shift;
//! - [`filter`]: matching a search by collation, case and accents aside;
//! - [`label`], [`separator`], [`visually_hidden`]: what assistive technology
//!   is told;
//! - [`relation`]: a trigger tied to its content by typed relations;
//! - [`presence`]: open, kept mounted while closed, or gone, and the four
//!   stages the engine reports on the way;
//! - [`popper`]: a floating part's place beside its anchor, on `gui`'s
//!   `Placement`, with its arrow and transform origin;
//! - [`form`]: fields, checks, submission and reset;
//! - [`time_field`]: a time typed segment by segment (reka's `TimeField`,
//!   on the date field's segment core).
//!
//! Dates (group 7): [`date`] (days, calendar systems, locales),
//! [`calendar`], [`range_calendar`], [`date_field`] and [`date_picker`].
//!
//! The controlled or uncontrolled value of every primitive is `gui`'s
//! [`gui::Bind`] read through [`gui::use_model`]: its owner is decided once,
//! as the instance is made, and a write equal to the value does nothing,
//! whoever owns it (ADR-0025).

pub mod accordion;
pub mod alert_dialog;
pub mod aspect_ratio;
pub mod autocomplete;
pub mod availability;
pub mod avatar;
pub mod calendar;
pub mod carousel;
pub mod checkbox;
pub mod collapsible;
pub mod collection;
pub mod combobox;
pub mod command;
pub mod context;
pub mod context_menu;
pub mod date;
pub mod date_field;
pub mod date_picker;
pub mod dialog;
pub mod direction;
pub mod drawer;
pub mod dropdown_menu;
pub mod filter;
pub mod form;
pub mod hover;
pub mod hover_card;
pub mod input_otp;
pub mod keymap;
pub mod label;
pub mod listbox;
pub mod menu;
pub mod menubar;
pub mod navigation_menu;
pub mod number_field;
pub mod number_format;
pub mod pagination;
pub mod pin_input;
pub mod popover;
pub mod popper;
pub mod presence;
pub mod progress;
pub mod radio_group;
pub mod range_calendar;
pub mod relation;
pub mod roving;
pub mod scroll_area;
pub mod select;
pub mod selection;
pub mod separator;
pub mod slider;
pub mod splitter;
pub mod stepper;
pub mod switch;
pub mod tabs;
pub mod tags_input;
pub mod time_field;
pub mod toast;
pub mod toggle;
pub mod toggle_group;
pub mod toolbar;
pub mod tooltip;
pub mod tree;
pub mod typeahead;
pub mod visually_hidden;

pub use availability::Availability;
pub use collection::{Collection, Item, Items};
pub use direction::{DirectionScope, ReadDirection, provide_direction, use_direction};
pub use keymap::{Arrows, Intent, Keymap, Wrap};
pub use roving::{Roving, RovingItem};
pub use selection::SelectionBehavior;
pub use typeahead::Typeahead;
