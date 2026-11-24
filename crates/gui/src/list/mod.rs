//! Lists and tables of any number of rows (ADR-0014): row heights in an
//! [`ExtentTree`], a [`virtual_list`] that builds only the rows in view and
//! a prefetch margin, a [`View`] of the data sorted and filtered off the UI
//! thread, a [`Selection`] kept in data rows, and a table's [`Columns`].

pub mod columns;
pub mod extent_tree;
pub mod view;
pub mod virtual_list;

pub use columns::{Columns, Width};
pub use extent_tree::ExtentTree;
pub use view::{Selection, View, Views};
pub use virtual_list::{Anchor, Heights, ListLayout, ListState, RowLayout, virtual_list, virtual_list_in};
