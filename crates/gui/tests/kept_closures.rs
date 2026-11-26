//! ADR-0015, the kept-closure rule: work the engine runs later on its own
//! (a timer, an effect, a cleanup, a hook after layout, a mail handler) is
//! kept only in `engine::Stored`, which carries its window; any other
//! closure gui, primitives or shadcn keeps says here why it runs only
//! inside a turn that already has its window.
//!
//! The lint is by type, parsed with `syn`, over every source file of the
//! three crates (test modules and `tests.rs` files aside). A hit is:
//! - a field (of a struct, an enum's variant, a static) whose type holds a
//!   `dyn Fn`, `dyn FnMut` or `dyn FnOnce` outside a `Stored<…>`;
//! - a type definition's type parameter bounded by `Fn*`, in its generics
//!   or its where clause (what it keeps is a closure);
//! - a type alias to either;
//! - a static declared inside a macro (`engine_local!`, `thread_local!`)
//!   whose tokens hold `dyn Fn*`.
//!
//! Each hit names its item, not its line; each must be in [`ALLOWED`] with
//! a reason, and an entry that matches no hit fails (stale).

#![expect(
    clippy::expect_used,
    reason = "a source that cannot be read or parsed is a failed test"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::visit::Visit;

/// A pure function or a tracked read, called inline by its caller.
const PURE: &str = "a pure function or a tracked read: called inline by its caller (a render, a layout or paint phase, a handler), in that caller's turn and window; it schedules nothing";
/// A handler.
const HANDLER: &str = "a handler: runs only in a dispatch, a turn of the window whose tree holds its node";
/// A render.
const RENDER: &str = "a render or a part of one: called while its root renders, in its window's frame";
/// Borrowed for one call.
const BORROWED: &str = "borrowed for one call (a lifetime-bound reference), never kept past it";
/// A layout hook.
const LAYOUT: &str = "asked in its window's layout, a phase of that window's frame";

/// Every closure kept outside `Stored`, as `(hit, why it is not work the
/// engine runs later on its own)`. A hit is `crate:file: item`.
#[rustfmt::skip]
const ALLOWED: &[(&str, &str)] = &[
    // gui: kept in a Stored elsewhere, or the engine's own.
    ("gui:component.rs: Instance.mounted", "on_mounted: kept by its instance (its root's window), made Stored as it is queued after the first render"),
    ("gui:component.rs: Instance.updated", "on_updated: kept by its instance, made Stored each time it is queued"),
    ("gui:reactive/runtime.rs: type Run", "an effect's body: Effect::new wraps the user's closure in Stored"),
    ("gui:reactive/runtime.rs: type Cleanup", "on_cleanup wraps the user's closure in Stored; on_release is the engine's bookkeeping"),
    ("gui:reactive/runtime.rs: type Update", "a computed's getter: pulled by its reader, in the reader's turn"),
    ("gui:mailbox.rs: type Drain", "the engine's drain; the user's handler inside it is Stored"),
    ("gui:mailbox.rs: type Wake", "the engine's wake across threads: rings the window, runs no user code"),
    ("gui:app.rs: Closing.root", "the engine's own: its root's closer (`Root::closer`), run once in close_window inside the window's own turn as the pipeline ends (ADR-0035, an Ends routed to the world that made it)"),
    ("gui:engine/release.rs: type Job","the release queue: engine bookkeeping, drained as a window closes or a frame ends"),
    ("gui:gpu/mod.rs: type LostWaker", "the device-lost wake: rings the window, runs no user code"),
    ("gui:compositor/thread.rs: RenderThread.to_ui", "the render thread's channel to the UI thread"),
    ("gui:window.rs: Platform.start", "makes the window, once, before any turn of it"),
    ("gui:testing.rs: type Run", "the deterministic test scheduler's steps"),
    ("gui:list/view.rs: type Job", "a row job, run on the worker thread; its Views is a WindowOwned and ends with its window"),
    ("gui:list/view.rs: type Recipe", "makes row jobs in the frame of the window that asked"),
    ("gui:list/view.rs: type Order", "the user's sort order: pure, run by the worker on the rows"),
    ("gui:list/view.rs: Inner.wake", "asks the views' latest request again (engine code): what it wakes is effects, which run through Stored in their own window"),
    ("gui:list/virtual_list.rs: type Waiter", "a weak Inner.wake (above)"),
    // gui: handlers.
    ("gui:element.rs: type Handler", HANDLER),
    ("gui:element.rs: type KeyHandler", HANDLER),
    ("gui:element.rs: type WheelHandler", HANDLER),
    ("gui:element.rs: type PressFocus", HANDLER),
    ("gui:element.rs: type PressFocuses", HANDLER),
    ("gui:element.rs: type Consumes", HANDLER),
    ("gui:element.rs: Handlers.hover", HANDLER),
    ("gui:interact.rs: type HoverHandler", HANDLER),
    ("gui:layer.rs: type DismissHandler", "a handler: runs in the layer stack's dispatch, a turn of its window"),
    ("gui:chunk/plot.rs: Plot.hover", HANDLER),
    ("gui:canvas/mod.rs: type Movable", "asked in a drag's dispatch"),
    // gui: reads, pure functions and renders.
    ("gui:element.rs: TabStop::Bound.0", PURE),
    ("gui:element.rs: DrawSpec.make", "a drawing: made in its window's paint"),
    ("gui:element.rs: TextSpec.bound", "a bound text: read in its window's layout"),
    ("gui:value.rs: Value::Bound.0", PURE),
    ("gui:value.rs: Run::Write.0", "a style write: pure, in its window's layout"),
    ("gui:value/source.rs: GroupCell.read", "a source's read: in its reader's phase"),
    ("gui:layout/binding.rs: type TextBinding", "a bound text: read in its window's layout"),
    ("gui:layout/binding.rs: type DrawBinding", "a drawing: made in its window's paint"),
    ("gui:layout/attrs.rs: Def.read","a bound attribute: read in its window's layout, where its node resolves (ADR-0026's accessibility phase)"),
    ("gui:layout/attrs.rs: type HeldWrite", "the engine's own: writes a value a binding held over a panic, in a cold build of its window's layout; no caller's closure"),
    ("gui:chunk/plot.rs: type XFn", "a plot's range: read in paint"),
    ("gui:chunk/plot.rs: type YFn", "a plot's range: read in paint"),
    ("gui:component.rs: Callback.f", "Callback: called by its caller, in its turn"),
    ("gui:component.rs: Derive.0", "Derive: called by its caller; its kind is registered"),
    ("gui:component.rs: type Render", RENDER),
    ("gui:component.rs: type Fallthrough", "a style transform: pure, in render"),
    ("gui:component/control.rs: Desc.items", "a control node's source: pure, run tracked by its node in the structure commit (ADR-0026)"),
    ("gui:component/control.rs: Desc.key", "a row's key of its item: pure, run with the source"),
    ("gui:component/control.rs: Desc.view", "a row's view: pure, run once per mount, by the body rule"),
    ("gui:component/control.rs: Show.when", "a branch's condition: pure, the source of its node"),
    ("gui:component/control.rs: Show.then", "a branch's view: pure, run once per mount"),
    ("gui:component/control.rs: Show.otherwise", "the other branch's view: pure, run once per mount"),
    ("gui:motion.rs: StageReport.0", "reported through a hook after layout (Stored)"),
    ("gui:members.rs: type Watch", "asked when members change, in the frame"),
    ("gui:members.rs: type Join", "asked when members change, in the frame"),
    ("gui:scroll.rs: type Hook", LAYOUT),
    ("gui:scroll.rs: type Reveal", LAYOUT),
    ("gui:scroll.rs: type Take", LAYOUT),
    ("gui:scroll.rs: type Wheeled", "asked in a wheel's dispatch"),
    ("gui:list/virtual_list.rs: type Hook", LAYOUT),
    ("gui:list/virtual_list.rs: type ActiveFrom", LAYOUT),
    // gui: borrowed.
    ("gui:canvas/plan.rs: type Huge", BORROWED),
    ("gui:layout/engine/mod.rs: type Job", BORROWED),
    ("gui:text/access.rs: type Place", BORROWED),
    // primitives: options and names, read where the component acts.
    ("primitives:accordion.rs: State.options", PURE),
    ("primitives:avatar.rs: AvatarImage.load", PURE),
    ("primitives:avatar.rs: Fallback.status", PURE),
    ("primitives:carousel.rs: CarouselNames.indicator", PURE),
    ("primitives:carousel.rs: CarouselNames.slide_label", PURE),
    ("primitives:carousel.rs: CarouselNames.tab", PURE),
    ("primitives:carousel.rs: State.options", PURE),
    ("primitives:collapsible.rs: Collapsible.availability", PURE),
    ("primitives:collapsible.rs: Collapsible.keep_mounted", PURE),
    ("primitives:combobox.rs: TextValue.of", PURE),
    ("primitives:combobox.rs: type Display", PURE),
    ("primitives:command.rs: type Holds", PURE),
    ("primitives:dialog.rs: Dialog.options", PURE),
    ("primitives:drawer.rs: Drawer.options", PURE),
    ("primitives:form.rs: type Encode", PURE),
    ("primitives:form.rs: type Rule", PURE),
    ("primitives:hover_card.rs: HoverCard.options", PURE),
    ("primitives:listbox.rs: type Source", PURE),
    ("primitives:pagination.rs: PaginationNames.page", PURE),
    ("primitives:pagination.rs: State.options", PURE),
    ("primitives:popover.rs: Popover.options", PURE),
    ("primitives:scroll_area.rs: State.options", PURE),
    ("primitives:splitter.rs: State.options", PURE),
    ("primitives:tabs.rs: State.options", PURE),
    ("primitives:tags_input.rs: State.tag_availability", PURE),
    ("primitives:tags_input.rs: type Convert", PURE),
    ("primitives:tags_input.rs: type Shown", PURE),
    ("primitives:tooltip.rs: Tooltip.overrides", PURE),
    ("primitives:tooltip.rs: TooltipProvider.options", PURE),
    ("primitives:tooltip.rs: engine_local! static CLOSERS", "a tooltip's closer: called only by open_alone for the tooltip open in the same window (OPEN is keyed by window_id), so in a turn of the window that made it; test a_tooltip_opened_in_one_window_closes_none_of_another"),
    ("primitives:tree.rs: TreeData.availability", PURE),
    ("primitives:tree.rs: TreeData.key", PURE),
    ("primitives:tree.rs: TreeData.label", PURE),
    ("primitives:tree.rs: type Children", PURE),
    ("primitives:tree.rs: type Roots", PURE),
    // shadcn: looks, slots and parts of renders.
    ("shadcn:alert_dialog.rs: AlertDialogButton.part", RENDER),
    ("shadcn:checkbox.rs: type Class", RENDER),
    ("shadcn:command.rs: Rows.row", BORROWED),
    ("shadcn:command.rs: type RowFn", RENDER),
    ("shadcn:input.rs: TimeInput.availability", PURE),
    ("shadcn:input.rs: type Refine", RENDER),
    ("shadcn:input.rs: type SegmentLook", RENDER),
    ("shadcn:message_scroller.rs: MessageScrollerViewportProps.viewport", RENDER),
    ("shadcn:message_scroller.rs: type IsAnchor", PURE),
    ("shadcn:message_scroller.rs: type Meta", PURE),
    ("shadcn:message_scroller.rs: type Row", RENDER),
    ("shadcn:native_select.rs: type BoxClass", RENDER),
    ("shadcn:navigation_menu.rs: type PanelSlot", RENDER),
    ("shadcn:questionnaire.rs: QuestionnaireView.0", RENDER),
    ("shadcn:sonner.rs: ToastDescription::View.0", RENDER),
    ("shadcn:sonner.rs: type ToastClass", RENDER),
    ("shadcn:stepper.rs: type Fill", RENDER),
    ("shadcn:theme/handle.rs: Edit.0", "a theme edit: applied by its caller, in its turn"),
    ("shadcn:toggle_group.rs: ToggleGroupContext.look", PURE),
    ("gui:port.rs: type Deliver", "a port's deliver: kept only inside Entry.deliver, a Stored"),
    // K4g: bounds written only on an impl (CLIP's lint gaps).
    ("gui:component.rs: Pending<S: Fn>", "a component's setup, kept until its description mounts: run once, in the render that mounts it (RENDER)"),
    ("gui:value.rs: BoundWrite<W: Fn>", "a style write: pure, in its window's layout (as Run::Write.0)"),
    ("gui:value.rs: Mount<W: Fn>", "a bound style write's mount: its write is BoundWrite's, run in its window's layout"),
    ("gui:value.rs: FixedOr<S: Fn>", "a setter's value or closure: read where its node resolves, as Value::Bound.0 (PURE)"),
    ("gui:value/source.rs: Closure<F: Fn>", "a setter's closure: read where its node resolves, in its window's phase (PURE)"),
    ("gui:value/source.rs: Mapped<F: Fn>", "`signal.map(f)`: f reads only what it is given, where its node resolves (PURE)"),
    ("gui:value/source.rs: Grouped<F: Fn>", "`group.map(f)`: f reads only what it is given, where its node resolves (PURE)"),
    ("primitives:collection.rs: FromFn<A: Fn>", PURE),
    ("primitives:collection.rs: FromFn<L: Fn>", PURE),
    ("primitives:listbox.rs: OptionsFn<A: Fn>", PURE),
    ("primitives:listbox.rs: OptionsFn<L: Fn>", PURE),
    ("primitives:listbox.rs: OptionsFn<V: Fn>", PURE),
    // K4g: closure types kept in a store a later turn reads (CLIP's lint
    // gaps). Each keeps a type listed above, for the reason given there.
    ("gui:mailbox.rs: engine_local! static BOXES", "the drains (type Drain, above): the engine's own, the user's handler inside each is Stored"),
    ("gui:mailbox.rs: engine_local! static WAKE", "the window's wake (type Wake, above): rings the window, runs no user code"),
    ("gui:layer.rs: engine_local! static ANSWERS", "the layers' dismiss handlers (type DismissHandler, above): run in the layer stack's dispatch, a turn of its window"),
    ("gui:list/virtual_list.rs: engine_local! static LISTS", "the lists' states, each with its active-row hook (State.active_from, below): asked in its window's layout"),
    ("gui:list/virtual_list.rs: State.active_from", "a Holder of the list's ActiveFrom (above): asked in its window's layout"),
    ("primitives:accordion.rs: Accordion.state", "the part's State in a signal; its options closure (State.options, above) is PURE"),
    ("primitives:carousel.rs: Carousel.state", "the part's State in a signal; its options closure (State.options, above) is PURE"),
    ("primitives:pagination.rs: Pagination.state", "the part's State in a signal; its options closure (State.options, above) is PURE"),
    ("primitives:scroll_area.rs: ScrollArea.state", "the part's State in a signal; its options closure (State.options, above) is PURE"),
    ("primitives:splitter.rs: Splitter.state", "the part's State in a signal; its options closure (State.options, above) is PURE"),
    ("primitives:tabs.rs: Tabs.state", "the part's State in a signal; its options closure (State.options, above) is PURE"),
    ("primitives:tags_input.rs: TagsInput.state", "the part's State in a signal; its availability closure (State.tag_availability, above) is PURE"),
    ("primitives:command.rs: Command.holds", "the filter (type Holds, above) in a signal: PURE, called where the list filters"),
    ("primitives:form.rs: FormField.rule", "the field's rule (type Rule, above) in a signal: PURE, called where the field validates"),
    ("primitives:hover.rs: Grace.on_exit", "a Callback in a signal: called by the grace area's own pointer handling, a dispatch of its window; it runs as its maker"),
    ("primitives:presence.rs: Present.tell", "a Callback in a signal: called as the node reports a stage, through a hook after layout (Stored); it runs as its maker"),
    ("primitives:tooltip.rs: engine_local! static ASKS", "a TooltipProvider whose options closure (TooltipProvider.options, above) is PURE"),
    ("primitives:tooltip.rs: engine_local! static WINDOW", "a TooltipProvider whose options closure (TooltipProvider.options, above) is PURE"),
    ("shadcn:message_scroller.rs: Engine.anchor_of", "a Holder of the anchor test (type IsAnchor, above): PURE"),
    ("shadcn:navigation_menu.rs: Panels.0", "the panels' slots (type PanelSlot, above) in a signal: renders, called where the viewport renders"),
];

/// What the whole sources say a closure is, beyond `dyn Fn*` (CLIP's lint
/// gaps, K4g): a trait of the crates that is a closure (an `Fn*`
/// supertrait, a blanket impl over an `Fn*`-bounded type, or a `call`
/// method), and a type of the crates that keeps one (it is a hit, or
/// `Callback` and `Derive`), which counts where it is kept in a store a
/// later turn reads: a signal, a holder, a registry, an app handle.
#[derive(Default)]
struct Known {
    fn_traits: BTreeSet<String>,
    /// Each type name the sources define, and the files that define one.
    defined: std::collections::BTreeMap<String, BTreeSet<String>>,
    /// Of those, the files whose type of that name keeps a closure.
    wrappers: std::collections::BTreeMap<String, BTreeSet<String>>,
}

impl Known {
    /// Whether `name`, written in `file`, is a type that keeps a closure:
    /// the file's own type of that name if it defines one, else the
    /// crates' (every type of that name keeps one); `Callback` and
    /// `Derive` always.
    fn is_wrapper(&self, name: &str, file: &str) -> bool {
        let wraps = self.wrappers.get(name);
        match self.defined.get(name) {
            Some(files) if files.contains(file) => wraps.is_some_and(|w| w.contains(file)),
            Some(files) => wraps.is_some_and(|w| files.is_subset(w)),
            None => matches!(name, "Callback" | "Derive"),
        }
    }
}

/// The containers that keep a value for whoever reads it later.
const STORES: &[&str] = &[
    "Signal",
    "Computed",
    "Holder",
    "Registry",
    "Keyed",
    "Collection",
    "AppHandle",
    "ForkMap",
];

/// What a type holds.
struct Holds<'k> {
    known: &'k Known,
    /// The file the type is written in.
    file: &'k str,
    /// How many stores the visit is inside.
    stored: usize,
    closure: bool,
}

impl<'a> Visit<'a> for Holds<'_> {
    fn visit_type_path(&mut self, t: &'a syn::TypePath) {
        let Some(last) = t.path.segments.last() else {
            return;
        };
        // What a Stored keeps carries its window: not a hit.
        if last.ident == "Stored" {
            return;
        }
        if self.stored > 0 && self.known.is_wrapper(&last.ident.to_string(), self.file) {
            self.closure = true;
        }
        let store = STORES.iter().any(|s| last.ident == s);
        if store {
            self.stored = self.stored.saturating_add(1);
        }
        syn::visit::visit_type_path(self, t);
        if store {
            self.stored = self.stored.saturating_sub(1);
        }
    }

    fn visit_type_trait_object(&mut self, t: &'a syn::TypeTraitObject) {
        if t.bounds.iter().any(|b| is_fn_bound(b, self.known)) {
            self.closure = true;
        }
        syn::visit::visit_type_trait_object(self, t);
    }

    fn visit_type_impl_trait(&mut self, t: &'a syn::TypeImplTrait) {
        if t.bounds.iter().any(|b| is_fn_bound(b, self.known)) {
            self.closure = true;
        }
        syn::visit::visit_type_impl_trait(self, t);
    }
}

fn is_fn_bound(b: &syn::TypeParamBound, known: &Known) -> bool {
    match b {
        syn::TypeParamBound::Trait(t) => t.path.segments.last().is_some_and(|s| {
            s.ident == "Fn"
                || s.ident == "FnMut"
                || s.ident == "FnOnce"
                || known.fn_traits.contains(&s.ident.to_string())
        }),
        _ => false,
    }
}

fn holds_closure(t: &syn::Type, known: &Known, file: &str) -> bool {
    let mut h = Holds {
        known,
        file,
        stored: 0,
        closure: false,
    };
    h.visit_type(t);
    h.closure
}

/// The traits of a file that are closures: an `Fn*` supertrait, a method
/// named `call`, `call_mut` or `call_once`, or a blanket impl for an
/// `Fn*`-bounded type.
#[derive(Default)]
struct Traits(BTreeSet<String>);

impl<'a> Visit<'a> for Traits {
    fn visit_item_trait(&mut self, t: &'a syn::ItemTrait) {
        let none = Known::default();
        let calls = t.items.iter().any(|i| {
            matches!(i, syn::TraitItem::Fn(f) if matches!(f.sig.ident.to_string().as_str(), "call" | "call_mut" | "call_once"))
        });
        if calls || t.supertraits.iter().any(|b| is_fn_bound(b, &none)) {
            self.0.insert(t.ident.to_string());
        }
    }

    fn visit_item_impl(&mut self, i: &'a syn::ItemImpl) {
        let none = Known::default();
        if let Some((_, path, _)) = &i.trait_
            && let syn::Type::Path(p) = &*i.self_ty
            && let Some(ty) = p.path.get_ident()
            && fn_bounded(&i.generics, &none).contains(&ty.to_string())
            && let Some(name) = path.segments.last()
        {
            self.0.insert(name.ident.to_string());
        }
    }
}

/// The type parameters `g` bounds by `Fn*` (or a closure trait), in its
/// list or its where clause.
fn fn_bounded(g: &syn::Generics, known: &Known) -> BTreeSet<String> {
    let mut bounded = BTreeSet::new();
    for p in g.type_params() {
        if p.bounds.iter().any(|b| is_fn_bound(b, known)) {
            bounded.insert(p.ident.to_string());
        }
    }
    if let Some(w) = &g.where_clause {
        for pred in &w.predicates {
            if let syn::WherePredicate::Type(t) = pred
                && t.bounds.iter().any(|b| is_fn_bound(b, known))
                && let syn::Type::Path(p) = &t.bounded_ty
                && let Some(id) = p.path.get_ident()
            {
                bounded.insert(id.to_string());
            }
        }
    }
    bounded
}

fn is_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<TokenStream>()
                .is_ok_and(|t| t.into_iter().any(|t| matches!(t, TokenTree::Ident(i) if i == "test")))
    })
}

/// The hits of one file.
struct Lint<'f> {
    file: &'f str,
    known: &'f Known,
    /// The file's type definitions' parameters, by name, in order: what an
    /// impl's arguments stand for.
    params: std::collections::BTreeMap<String, Vec<String>>,
    hits: Vec<String>,
}

impl<'f> Lint<'f> {
    fn new(file: &'f str, known: &'f Known, syntax: &syn::File) -> Self {
        let mut params = std::collections::BTreeMap::new();
        for item in &syntax.items {
            let (name, g) = match item {
                syn::Item::Struct(s) => (&s.ident, &s.generics),
                syn::Item::Enum(e) => (&e.ident, &e.generics),
                _ => continue,
            };
            params.insert(name.to_string(), g.type_params().map(|p| p.ident.to_string()).collect());
        }
        Self {
            file,
            known,
            params,
            hits: Vec::new(),
        }
    }

    fn hit(&mut self, item: String) {
        if !self.hits.iter().any(|h| h.ends_with(&format!(": {item}"))) {
            self.hits.push(format!("{}: {item}", self.file));
        }
    }

    /// A type definition's parameters bounded by `Fn*`.
    fn generics(&mut self, name: &syn::Ident, g: &syn::Generics) {
        for p in fn_bounded(g, self.known) {
            self.hit(format!("{name}<{p}: Fn>"));
        }
    }

    /// An impl's parameters bounded by `Fn*` that fill a parameter of the
    /// type it implements: a field of that parameter keeps a closure,
    /// though the type's definition bounds nothing. Named by the type's own
    /// parameter when the file defines it, so a bound written on both is
    /// one hit.
    fn impl_bounds(&mut self, i: &syn::ItemImpl) {
        let bounded = fn_bounded(&i.generics, self.known);
        if bounded.is_empty() {
            return;
        }
        let syn::Type::Path(p) = &*i.self_ty else { return };
        let Some(last) = p.path.segments.last() else { return };
        let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
            return;
        };
        let types = args.args.iter().filter_map(|a| match a {
            syn::GenericArgument::Type(t) => Some(t),
            _ => None,
        });
        for (k, t) in types.enumerate() {
            if let syn::Type::Path(tp) = t
                && let Some(id) = tp.path.get_ident()
                && bounded.contains(&id.to_string())
            {
                let own = self
                    .params
                    .get(&last.ident.to_string())
                    .and_then(|ps| ps.get(k).cloned())
                    .unwrap_or_else(|| id.to_string());
                self.hit(format!("{}<{own}: Fn>", last.ident));
            }
        }
    }

    fn fields(&mut self, owner: &str, fields: &syn::Fields) {
        for (i, f) in fields.iter().enumerate() {
            if is_test(&f.attrs) || !holds_closure(&f.ty, self.known, self.file) {
                continue;
            }
            let name = f.ident.as_ref().map_or_else(|| i.to_string(), ToString::to_string);
            self.hit(format!("{owner}.{name}"));
        }
    }

    /// The statics a macro declares whose tokens hold `dyn Fn*`.
    fn macro_tokens(&mut self, name: &str, tokens: TokenStream) {
        let mut flat = Vec::new();
        flatten(tokens, &mut flat);
        let mut last_static = None;
        let mut said = BTreeSet::new();
        let mut before: Option<String> = None;
        for t in &flat {
            let TokenTree::Ident(b) = t else {
                before = None;
                continue;
            };
            let b = b.to_string();
            let after_static = before.as_deref() == Some("static");
            if after_static {
                last_static = Some(b.clone());
            }
            // Only what a static the macro declares keeps: a macro that
            // declares none (`parts_whole!`) keeps nothing. A static's
            // value is a store: a closure type of the crates kept in it
            // counts as `dyn Fn` does.
            let closure = (before.as_deref() == Some("dyn")
                && (b == "Fn" || b == "FnMut" || b == "FnOnce" || self.known.fn_traits.contains(&b)))
                || (!after_static && self.known.is_wrapper(&b, self.file));
            if closure
                && let Some(at) = &last_static
                && said.insert(at.clone())
            {
                self.hit(format!("{name}! static {at}"));
            }
            before = Some(b);
        }
    }
}

fn flatten(tokens: TokenStream, out: &mut Vec<TokenTree>) {
    for t in tokens {
        if let TokenTree::Group(g) = &t {
            flatten(g.stream(), out);
        } else {
            out.push(t);
        }
    }
}

impl<'a> Visit<'a> for Lint<'_> {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !is_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if !is_test(&f.attrs) {
            syn::visit::visit_item_fn(self, f);
        }
    }

    fn visit_item_impl(&mut self, i: &'a syn::ItemImpl) {
        if !is_test(&i.attrs) {
            self.impl_bounds(i);
            syn::visit::visit_item_impl(self, i);
        }
    }

    fn visit_item_struct(&mut self, s: &'a syn::ItemStruct) {
        if is_test(&s.attrs) {
            return;
        }
        self.generics(&s.ident, &s.generics);
        self.fields(&s.ident.to_string(), &s.fields);
    }

    fn visit_item_enum(&mut self, e: &'a syn::ItemEnum) {
        if is_test(&e.attrs) {
            return;
        }
        self.generics(&e.ident, &e.generics);
        for v in &e.variants {
            self.fields(&format!("{}::{}", e.ident, v.ident), &v.fields);
        }
    }

    fn visit_item_union(&mut self, u: &'a syn::ItemUnion) {
        self.generics(&u.ident, &u.generics);
        self.fields(&u.ident.to_string(), &syn::Fields::Named(u.fields.clone()));
    }

    fn visit_item_type(&mut self, t: &'a syn::ItemType) {
        if is_test(&t.attrs) {
            return;
        }
        if holds_closure(&t.ty, self.known, self.file) {
            self.hit(format!("type {}", t.ident));
        }
        self.generics(&t.ident, &t.generics);
    }

    fn visit_item_static(&mut self, s: &'a syn::ItemStatic) {
        if !is_test(&s.attrs) && holds_closure(&s.ty, self.known, self.file) {
            self.hit(format!("static {}", s.ident));
        }
    }

    fn visit_item_macro(&mut self, m: &'a syn::ItemMacro) {
        if is_test(&m.attrs) {
            return;
        }
        let name = m
            .mac
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        self.macro_tokens(&name, m.mac.tokens.clone());
    }
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("src") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("tests.rs") {
            out.push(path);
        }
    }
}

/// Every source of gui, primitives and shadcn, named `crate:file`, parsed.
fn parsed() -> Vec<(String, syn::File)> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut all = Vec::new();
    for name in ["gui", "primitives", "shadcn"] {
        let root = crates.join(name).join("src");
        let mut files = Vec::new();
        sources(&root, &mut files);
        for path in files {
            let rel = path
                .strip_prefix(&root)
                .expect("under src")
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&path).expect("source");
            let file = syn::parse_file(&text).expect("every source parses");
            all.push((format!("{name}:{rel}"), file));
        }
    }
    all
}

/// The type a hit names (`Name.field`, `type Name`, `Name<F: Fn>`,
/// `Name::Variant.0`): a type that keeps a closure.
fn hit_type(hit: &str) -> Option<String> {
    let item = hit.rsplit_once(": ").map_or(hit, |(_, i)| i);
    let item = item.strip_prefix("type ").unwrap_or(item);
    if item.contains('!') || item.starts_with("static ") {
        return None;
    }
    let end = item.find(['.', '<', ':']).unwrap_or(item.len());
    item.get(..end).map(str::to_owned)
}

/// Every hit in `sources`: the closure traits found first, then the hits
/// with no store counted, whose types (with `Callback` and `Derive`) are
/// the wrappers a store must not keep, then the hits again with them.
fn hits_of(sources: &[(String, syn::File)]) -> BTreeSet<String> {
    let mut traits = Traits::default();
    let mut defined: std::collections::BTreeMap<String, BTreeSet<String>> = std::collections::BTreeMap::new();
    for (at, file) in sources {
        traits.visit_file(file);
        let mut types = Types(Vec::new());
        types.visit_file(file);
        for name in types.0 {
            defined.entry(name).or_default().insert(at.clone());
        }
    }
    let mut known = Known {
        fn_traits: traits.0,
        defined,
        wrappers: std::collections::BTreeMap::new(),
    };
    let run = |known: &Known| {
        let mut all = BTreeSet::new();
        for (at, file) in sources {
            let mut lint = Lint::new(at, known, file);
            lint.visit_file(file);
            all.extend(lint.hits);
        }
        all
    };
    // To a fixed point: a type keeping a wrapper in a store is a wrapper.
    let mut hits = run(&known);
    loop {
        for h in &hits {
            if let (Some((file, _)), Some(name)) = (h.split_once(": "), hit_type(h)) {
                known.wrappers.entry(name).or_default().insert(file.to_owned());
            }
        }
        let next = run(&known);
        if next == hits {
            return hits;
        }
        hits = next;
    }
}

/// The names of the types a file defines (structs, enums, aliases), test
/// modules aside.
struct Types(Vec<String>);

impl<'a> Visit<'a> for Types {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !is_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_struct(&mut self, s: &'a syn::ItemStruct) {
        self.0.push(s.ident.to_string());
    }

    fn visit_item_enum(&mut self, e: &'a syn::ItemEnum) {
        self.0.push(e.ident.to_string());
    }

    fn visit_item_type(&mut self, t: &'a syn::ItemType) {
        self.0.push(t.ident.to_string());
    }
}

/// Every hit in gui, primitives and shadcn src.
fn hits() -> BTreeSet<String> {
    hits_of(&parsed())
}

/// ADR-0015: every closure the three crates keep is in `Stored` or said
/// why in [`ALLOWED`]; no entry there is stale.
#[test]
fn every_closure_kept_is_stored_or_said_why() {
    let hits = hits();
    let unsaid: Vec<_> = hits.iter().filter(|h| !ALLOWED.iter().any(|(a, _)| a == h)).collect();
    let stale: Vec<_> = ALLOWED.iter().filter(|(a, _)| !hits.contains(*a)).collect();
    assert!(
        unsaid.is_empty(),
        "kept closures with no Stored and no reason:\n{}",
        unsaid
            .iter()
            .map(|h| format!("    (\"{h}\", \"\"),"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(stale.is_empty(), "entries that match nothing: {stale:#?}");
}

/// The lint finds each kind of hit, and a closure inside `Stored` is none.
#[test]
fn the_lint_finds_each_kind_and_passes_stored() {
    let src = r"
        struct A { f: Box<dyn Fn()>, s: Stored<Box<dyn FnOnce()>> }
        struct B<F: FnMut()> { f: F }
        struct C<F> where F: FnOnce() { f: Option<F> }
        enum D { V(Rc<dyn Fn(u8) -> u8>) }
        type E = Rc<dyn Fn()>;
        static G: Option<fn()> = None;
        engine_local! { static H: RefCell<Vec<Box<dyn Fn()>>> = RefCell::default(); }
        parts_whole!(Stored<Box<dyn FnOnce()>>);
        #[cfg(test)] struct T { f: Box<dyn Fn()> }
    ";
    let file = syn::parse_file(src).expect("parses");
    assert_eq!(
        hits_of(&[("x".to_owned(), file)]).into_iter().collect::<Vec<_>>(),
        [
            "x: A.f",
            "x: B<F: Fn>",
            "x: C<F: Fn>",
            "x: D::V.0",
            "x: engine_local! static H",
            "x: type E",
        ]
    );
}

/// CLIP's lint gaps (K4g), each found, and each passing once the closure
/// goes (the revert proof: `good` is `bad` with the closure taken out):
/// - a field of a type parameter bounded by `Fn*` only on an impl;
/// - a closure wrapper (`Callback`, or a type of the crates that keeps
///   one) kept in a store a later turn reads (a signal, an app handle, a
///   static);
/// - `AppHandle<dyn Fn>`;
/// - a trait of the crates that is a closure (an `Fn` supertrait, a
///   blanket impl over closures, a `call` method), as `dyn` or as a bound.
#[test]
fn the_lint_closes_the_clip_gaps() {
    let bad = r"
        struct P<F> { f: F }
        impl<F: Fn()> P<F> { fn go(&self) { (self.f)() } }
        struct Q<G> { g: G }
        impl<G> Q<G> where G: FnMut(u8) {}
        struct R { on: Signal<Callback<u8>> }
        struct Keeps { f: Rc<dyn Fn()> }
        struct S { k: Signal<Option<Keeps>> }
        struct T { h: AppHandle<dyn Fn()> }
        engine_local! { static U: RefCell<Vec<Callback<()>>> = RefCell::default(); }
        trait Act: Fn(u8) {}
        trait Run { fn call(&self); }
        trait Hook {}
        impl<F: Fn()> Hook for F {}
        struct V { a: Box<dyn Act>, r: Rc<dyn Run>, h: Box<dyn Hook> }
        struct W<H: Hook> { h: H }
    ";
    let bad = syn::parse_file(bad).expect("parses");
    assert_eq!(
        hits_of(&[("x".to_owned(), bad)]).into_iter().collect::<Vec<_>>(),
        [
            "x: Keeps.f",
            "x: P<F: Fn>",
            "x: Q<G: Fn>",
            "x: R.on",
            "x: S.k",
            "x: T.h",
            "x: V.a",
            "x: V.h",
            "x: V.r",
            "x: W<H: Fn>",
            "x: engine_local! static U",
        ]
    );
    let good = r"
        struct P<F> { f: F }
        impl<F: Clone> P<F> {}
        struct Q<G> { g: G }
        impl<G> Q<G> where G: Copy {}
        struct R { on: Signal<u8>, given: Callback<u8> }
        struct Keeps { f: Rc<u8> }
        struct S { k: Signal<Option<Keeps>> }
        struct T { h: AppHandle<dyn Any> }
        engine_local! { static U: RefCell<Vec<u8>> = RefCell::default(); }
        trait Act: Clone {}
        trait Run { fn go(&self); }
        trait Hook {}
        impl Hook for u8 {}
        struct V { a: Box<dyn Act>, r: Rc<dyn Run>, h: Box<dyn Hook> }
        struct W<H: Hook> { h: H }
    ";
    let good = syn::parse_file(good).expect("parses");
    assert!(
        hits_of(&[("x".to_owned(), good)]).is_empty(),
        "nothing left once the closures went"
    );
}
