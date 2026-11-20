//! Values an app provides once and any component reads, as Vue's
//! `provide`/`inject`: a theme, a locale. Keyed by type; one value per type.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    static VALUES: RefCell<HashMap<TypeId, Rc<dyn Any>>> = RefCell::new(HashMap::new());
}

/// Make `value` the one every [`use_context`] of its type returns.
pub fn provide<T: 'static>(value: T) {
    VALUES.with(|v| v.borrow_mut().insert(TypeId::of::<T>(), Rc::new(value)));
}

/// The value provided for `T`, if any.
#[must_use]
pub fn use_context<T: Clone + 'static>() -> Option<T> {
    VALUES.with(|v| {
        v.borrow()
            .get(&TypeId::of::<T>())
            .and_then(|any| any.downcast_ref::<T>())
            .cloned()
    })
}
