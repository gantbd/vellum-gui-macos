//! Iced presentation adapter over the existing editor host; no source authority.
#![forbid(unsafe_code)]
#[cfg(target_os = "macos")]
mod attachment;
#[cfg(target_os = "macos")]
mod bridge;
#[cfg(target_os = "macos")]
pub use bridge::*;
#[cfg(target_os = "macos")]
mod comment_draft;
#[cfg(target_os = "macos")]
mod gui;
#[cfg(target_os = "macos")]
pub use gui::Gui;
