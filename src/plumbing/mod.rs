mod main;
pub use main::main;

#[path = "progress.rs"]
mod progress_impl;
pub use progress_impl::show_progress;

pub(crate) mod options;
