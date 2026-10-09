mod catalog;
mod download;
mod manager;

pub(crate) use catalog::*;
pub(crate) use manager::*;

#[cfg(test)]
mod tests;

use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{async_runtime::JoinHandle, AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use crate::paths;
use crate::voice_models::{DownloadErrorPayload, VoiceDownloadError};

/// `sherpa:download-*` — mirrors `EV_SHERPA_*` in `src/lib/events.ts`.
const EV_SHERPA_DOWNLOAD_PROGRESS: &str = "sherpa:download-progress";
const EV_SHERPA_DOWNLOAD_ERROR: &str = "sherpa:download-error";
const SIZE_TOLERANCE_PERCENT: u64 = 10;

