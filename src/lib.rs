//! Shared by the request builder (this crate's binary, desktop and web) and the search runner
//! (`runner/`, desktop only): the request model and KeyQL generator, the `.req` file format, and
//! the look of both apps.

pub mod query;
pub mod reqfile;
pub mod ui_kit;
