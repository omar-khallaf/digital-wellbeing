//! Reusable UI primitives built on `gpui_component`.
//!
//! These wrap the component library's styling so every screen shares one
//! visual language: a `Card` panel, a `StatCard` KPI tile, and a `SectionTitle`.

mod delegates;
mod policy;
mod primitives;
mod rows;
mod selectors;

pub use gpui_component::list::{List, ListDelegate, ListItem, ListState};

pub use delegates::*;
pub use policy::*;
pub use primitives::*;
pub use rows::*;
pub use selectors::*;
