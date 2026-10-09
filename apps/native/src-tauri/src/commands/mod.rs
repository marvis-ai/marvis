//! IPC command handlers, grouped by domain. Registered once in
//! `crate::run`'s `tauri::generate_handler!`; every name is re-exported at
//! crate root so command internals keep resolving as `crate::helper`.

mod ask;
mod capture;
mod config;
mod dictation;
mod files;
mod listen;
mod misc;
mod permissions;
mod presets;
mod providers;
mod sessions;
mod voice_models;
mod voiceprint;
mod windows;

pub(crate) use ask::*;
pub(crate) use capture::*;
pub(crate) use config::*;
pub(crate) use dictation::*;
pub(crate) use files::*;
pub(crate) use listen::*;
pub(crate) use misc::*;
pub(crate) use permissions::*;
pub(crate) use presets::*;
pub(crate) use providers::*;
pub(crate) use sessions::*;
pub(crate) use voice_models::*;
pub(crate) use voiceprint::*;
pub(crate) use windows::*;
