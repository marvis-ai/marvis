//! XDG screencast portal + PipeWire backed [`FrameSource`] for Linux.
//!
//! Consent is the portal's job: `select_sources` + `start` presents the
//! compositor's own dialog (GNOME/KDE), which returns a PipeWire node id
//! plus an `OpenPipeWireRemote` fd. With `PersistMode::ExplicitlyRevoked`
//! the portal also hands back a restore token — stored under `~/.marvis`
//! — that lets later sessions skip the dialog entirely, which is what
//! the ambient `primary_display_source` path relies on.
//!
//! PipeWire then delivers buffers on a `ThreadLoop` thread; `process`
//! memcpy's the mapped BGRA into a [`RawFrame`] and hands off to the
//! shared `frame_pipe` worker — dedupe, resize, JPEG — identical to the
//! ScreenCaptureKit and WGC paths.
//!
//! Linux has no app-side source enumeration (the portal owns selection
//! by design), so `pick_candidates`/`resolve_candidate`/`thumb_for`
//! don't exist here — `portal_pick_blocking` drives the whole pick.

use std::fs;
use std::io::Cursor;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
    StartCastOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode, ResponseError, Session};
use ashpd::Error as PortalError;
use parking_lot::Mutex;
use pipewire::context::ContextBox;
use pipewire::keys;
use pipewire::properties::properties;
use pipewire::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pipewire::spa::param::video::{VideoFormat, VideoInfoRaw};
use pipewire::spa::param::ParamType;
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::pod::{self, Value};
use pipewire::spa::utils::{Direction, Fraction, Rectangle, SpaTypes};
use pipewire::stream::{StreamBox, StreamFlags};
use pipewire::thread_loop::ThreadLoopBox;

use super::frame_pipe::{encode_frame, run_worker, RawFrame};
use super::{Frame, FrameSource};
use crate::paths;

/// Frame channel depth — a stalled worker drops frames, never stalls
/// the PipeWire loop thread.
const FRAME_QUEUE: usize = 2;
/// PipeWire/pipeline setup report deadline for `start` — local
/// socket connect, not user-paced (the portal prompt already happened).
const READY_TIMEOUT: Duration = Duration::from_secs(10);
/// One-shot captures give up after this.
const ONESHOT_TIMEOUT: Duration = Duration::from_secs(10);
/// `stop()` polls the PipeWire thread this often.
const STOP_POLL: Duration = Duration::from_millis(50);

/// What a resolved picker answer (or a silent token-restore handshake)
/// hands to [`LinuxCapture::new`]: the live portal session (kept alive
/// for the capture's duration — dropping it kills the stream), the
/// PipeWire remote fd, and the node id of the granted stream.
#[derive(Clone)]
pub struct Source {
    _session: Arc<Session<Screencast>>,
    fd: Arc<OwnedFd>,
    node_id: u32,
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Source")
            .field("node_id", &self.node_id)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Portal handshake
// ---------------------------------------------------------------------------

fn restore_token() -> Option<String> {
    let path = paths::portal_token_file();
    fs::read_to_string(path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn save_restore_token(token: &str) {
    if let Err(e) = fs::write(paths::portal_token_file(), token) {
        log::warn!("capture: failed to persist portal restore token: {e}");
    }
}

/// One portal round-trip: session → SelectSources → Start → streams →
/// PipeWire remote fd. `prompt` selects whether the portal may show its
/// dialog (the explicit pick path) or must answer silently from the
/// restore token alone (the ambient path). `Ok(None)` is a user cancel.
async fn portal_session(
    prompt: bool,
) -> Result<Option<(Session<Screencast>, OwnedFd, u32, u32, u32, &'static str)>> {
    let proxy = Screencast::new().await.context("portal connect failed")?;
    let session = proxy
        .create_session(CreateSessionOptions::default())
        .await
        .context("portal CreateSession failed")?;

    let mut options = SelectSourcesOptions::default()
        .set_sources(SourceType::Monitor | SourceType::Window)
        .set_cursor_mode(CursorMode::Embedded)
        .set_multiple(false)
        .set_persist_mode(PersistMode::ExplicitlyRevoked);
    let token = if prompt { None } else { restore_token() };
    if let Some(t) = token.as_deref() {
        options = options.set_restore_token(t);
    }
    proxy
        .select_sources(&session, options)
        .await
        .context("portal SelectSources failed")?
        .response()
        .context("portal SelectSources rejected")?;

    let request = proxy
        .start(&session, None, StartCastOptions::default())
        .await
        .context("portal Start failed")?;
    let streams = match request.response() {
        Ok(streams) => streams,
        Err(PortalError::Response(ResponseError::Cancelled)) => return Ok(None),
        Err(e) => return Err(e).context("portal Start rejected"),
    };

    if let Some(token) = streams.restore_token() {
        save_restore_token(token);
    }
    let stream = streams
        .streams()
        .first()
        .ok_or_else(|| anyhow!("portal returned no streams"))?;
    let node_id = stream.pipe_wire_node_id();
    let (w, h) = stream.size().unwrap_or((0, 0));
    let kind = match stream.source_type() {
        Some(SourceType::Window) => "window",
        _ => "display",
    };
    let fd = proxy
        .open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default())
        .await
        .context("portal OpenPipeWireRemote failed")?;
    Ok(Some((session, fd, node_id, w as u32, h as u32, kind)))
}

/// `primary_display_source` — the auto-capture path shared by
/// `enter_main`/`capture_start`/`toggle_capture`. Silent only: without a
/// saved restore token the portal can't grant access without its
/// dialog, so this errors (callers warn and leave capture off); the
/// user-facing `capture_pick_begin` flow seeds the token.
pub(crate) fn primary_display_source() -> Result<(Source, u32, u32)> {
    let outcome = pollster::block_on(portal_session(false))?;
    let (session, fd, node_id, w, h, _) =
        outcome.ok_or_else(|| anyhow!("portal restore cancelled"))?;
    Ok((
        Source {
            _session: Arc::new(session),
            fd: Arc::new(fd),
            node_id,
        },
        w,
        h,
    ))
}

/// `capture_pick_begin`'s worker entry: drive the portal's own source
/// dialog on this thread and resolve to a ready [`Source`]. `Ok(None)`
/// is a user cancel — a silent no-op, matching the other pickers.
pub(crate) fn portal_pick_blocking() -> Result<Option<(Source, u32, u32, &'static str, String)>> {
    let Some((session, fd, node_id, w, h, kind)) = pollster::block_on(portal_session(true))?
    else {
        return Ok(None);
    };
    let label = if kind == "window" { "Window" } else { "Screen" }.to_string();
    Ok(Some((
        Source {
            _session: Arc::new(session),
            fd: Arc::new(fd),
            node_id,
        },
        w,
        h,
        kind,
        label,
    )))
}

// ---------------------------------------------------------------------------
// PipeWire pump
// ---------------------------------------------------------------------------

/// Per-stream listener state: the negotiated format (set once
/// `ParamType::Format` arrives) and the frame channel.
struct StreamState {
    tx: SyncSender<RawFrame>,
    format: Option<VideoFormat>,
    size: Rectangle,
    warned: bool,
}

/// The EnumFormat pod we offer the portal's producer: raw BGRA/BGRx
/// video, any size/framerate — the compositor picks.
fn enum_format_pod() -> Result<Vec<u8>> {
    let obj = pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRA,
            VideoFormat::BGRx
        ),
        pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle {
                width: 320,
                height: 240,
            },
            Rectangle { width: 1, height: 1 },
            Rectangle {
                width: 8192,
                height: 8192,
            }
        ),
        pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: 0, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction {
                num: 1000,
                denom: 1,
            }
        ),
    );
    let (cursor, _len) = PodSerializer::serialize(Cursor::new(Vec::<u8>::new()), &Value::Object(obj))
        .map_err(|e| anyhow!("EnumFormat pod serialize failed: {e:?}"))?;
    Ok(cursor.into_inner())
}

/// Copy a PipeWire buffer's pixels into a [`RawFrame`]. Only BGRA/BGRx
/// is accepted — that is all the EnumFormat offer allows, so anything
/// else means the producer ignored our params (warn once, drop).
fn extract_frame(state: &mut StreamState, buffer: &mut pipewire::buffer::Buffer<'_>) {
    let Some(data) = buffer.datas_mut().first_mut() else {
        return;
    };
    let chunk = data.chunk();
    // stride is signed (a bottom-up producer can go negative) — a raw
    // `as usize` cast wraps it huge before `.max` can clamp it.
    let (offset, stride, size) = (
        chunk.offset() as usize,
        usize::try_from(chunk.stride()).unwrap_or(0),
        chunk.size() as usize,
    );
    if size == 0 {
        return;
    }
    let Some(format) = state.format else {
        return; // Format param hasn't landed yet.
    };
    if format != VideoFormat::BGRA && format != VideoFormat::BGRx {
        if !state.warned {
            state.warned = true;
            log::warn!("capture: unexpected PipeWire format {format:?} — dropping frames");
        }
        return;
    }
    let (w, h) = (state.size.width, state.size.height);
    if w == 0 || h == 0 {
        return;
    }
    let Some(map) = data.data() else {
        return;
    };
    let end = (offset + size).min(map.len());
    if offset >= end {
        return;
    }
    let raw = RawFrame {
        data: map[offset..end].to_vec(),
        width: w,
        height: h,
        bytes_per_row: stride.max(w as usize * 4),
    };
    let _ = state.tx.try_send(raw);
}

/// Owns one PipeWire session on a dedicated thread: build loop →
/// context → core → stream → connect, then run the loop until `stop`.
/// Setup errors go back through `ready` so `start`/`oneshot` see them.
fn pw_main(
    source: Source,
    tx: SyncSender<RawFrame>,
    ready: mpsc::Sender<Result<(), String>>,
    stop: Arc<AtomicBool>,
) {
    pipewire::init();
    if let Err(e) = pw_run(&source, tx, &ready, &stop) {
        let _ = ready.send(Err(format!("{e:#}")));
    }
}

/// Setup + park — separate from `pw_main` so `?` cleanup leaves the
/// loop stopped. All PipeWire objects borrow each other, so everything
/// lives and dies inside this one scope (drop order: listener → stream
/// → core → context → loop).
fn pw_run(
    source: &Source,
    tx: SyncSender<RawFrame>,
    ready: &mpsc::Sender<Result<(), String>>,
    stop: &Arc<AtomicBool>,
) -> Result<()> {
    let thread_loop = unsafe { ThreadLoopBox::new(Some("marvis-capture"), None) }
        .map_err(|e| anyhow!("pw_thread_loop_new: {e}"))?;
    let context = ContextBox::new(thread_loop.loop_(), None)
        .map_err(|e| anyhow!("pw_context_new: {e}"))?;
    let core = context
        // `connect_fd` takes ownership — hand it a clone so `source`'s
        // fd stays paired with its live portal session.
        .connect_fd(source.fd.try_clone().context("portal fd clone")?, None)
        .map_err(|e| anyhow!("pw_context_connect_fd: {e}"))?;

    let stream = StreamBox::new(
        &core,
        "marvis-capture",
        properties! {
            *keys::MEDIA_TYPE => "Video",
            *keys::MEDIA_CATEGORY => "Capture",
            *keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| anyhow!("pw_stream_new: {e}"))?;

    let _listener = stream
        .add_local_listener_with_user_data(StreamState {
            tx,
            format: None,
            size: Rectangle {
                width: 0,
                height: 0,
            },
            warned: false,
        })
        .param_changed(|_stream, state, id, pod| {
            if id != ParamType::Format.as_raw() {
                return;
            }
            let Some(pod) = pod else { return };
            let mut info = VideoInfoRaw::default();
            if info.parse(pod).is_ok() {
                state.format = Some(info.format());
                state.size = info.size();
            }
        })
        .process(|stream, state| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            extract_frame(state, &mut buffer);
        })
        .register()
        .map_err(|e| anyhow!("stream listener register: {e}"))?;

    let pod_bytes = enum_format_pod()?;
    let pod_obj = pipewire::spa::pod::Pod::from_bytes(&pod_bytes)
        .ok_or_else(|| anyhow!("EnumFormat pod round-trip failed"))?;
    stream
        .connect(
            Direction::Input,
            Some(source.node_id),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut [pod_obj],
        )
        .map_err(|e| anyhow!("pw_stream_connect: {e}"))?;

    thread_loop.start();
    let _ = ready.send(Ok(()));
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(STOP_POLL);
    }
    thread_loop.stop();
    Ok(())
}

struct PwHandle {
    thread: JoinHandle<()>,
    stop: Arc<AtomicBool>,
}

/// Spawn the PipeWire thread; resolve setup success/failure before
/// returning so `start`'s `is_running` is truthful.
fn spawn_pw(source: Source, tx: SyncSender<RawFrame>) -> Result<PwHandle> {
    let (ready_tx, ready_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = Arc::clone(&stop);
        thread::spawn(move || pw_main(source, tx, ready_tx, stop))
    };
    match ready_rx.recv_timeout(READY_TIMEOUT) {
        Ok(Ok(())) => Ok(PwHandle { thread, stop }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(anyhow!(e))
        }
        Err(e) => {
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
            Err(anyhow!("pipewire setup timed out: {e}"))
        }
    }
}

// ---------------------------------------------------------------------------
// FrameSource
// ---------------------------------------------------------------------------

struct Running {
    pw: PwHandle,
    worker: JoinHandle<()>,
    stop: Arc<AtomicBool>,
}

/// The production [`FrameSource`] on Linux.
pub struct LinuxCapture {
    source: Mutex<Option<Source>>,
    state: Mutex<Option<Running>>,
}

impl LinuxCapture {
    pub fn new(source: Source, _width: u32, _height: u32, _fps: u32) -> Result<Self> {
        Ok(Self {
            source: Mutex::new(Some(source)),
            state: Mutex::new(None),
        })
    }
}

impl FrameSource for LinuxCapture {
    fn start(&self, on_frame: Box<dyn Fn(Frame) + Send>) {
        let Some(source) = self.source.lock().take() else {
            log::warn!("capture: LinuxCapture already consumed");
            return;
        };
        let (tx, rx) = mpsc::sync_channel(FRAME_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || run_worker(rx, stop, on_frame))
        };
        match spawn_pw(source, tx) {
            Ok(pw) => {
                *self.state.lock() = Some(Running { pw, worker, stop });
            }
            Err(e) => {
                log::warn!("capture: PipeWire start failed: {e}");
                stop.store(true, Ordering::Relaxed);
                let _ = worker.join();
            }
        }
    }

    fn stop(&self) {
        let Some(running) = self.state.lock().take() else {
            return;
        };
        running.pw.stop.store(true, Ordering::Relaxed);
        let _ = running.pw.thread.join();
        running.stop.store(true, Ordering::Relaxed);
        let _ = running.worker.join();
    }

    fn is_running(&self) -> bool {
        let state = self.state.lock();
        state
            .as_ref()
            .is_some_and(|r| !r.pw.thread.is_finished() && !r.worker.is_finished())
    }
}

impl Drop for LinuxCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One-shot screenshot of the primary display — the "read my screen"
/// path when ambient recording is off. Needs the saved restore token;
/// without it there's nothing silent to capture.
pub(crate) fn shot_fullscreen() -> Result<Option<Frame>> {
    let (source, _, _) = primary_display_source()?;
    let (tx, rx) = mpsc::sync_channel::<RawFrame>(FRAME_QUEUE);
    let pw = spawn_pw(source, tx)?;
    let raw = rx.recv_timeout(ONESHOT_TIMEOUT).ok();
    pw.stop.store(true, Ordering::Relaxed);
    let _ = pw.thread.join();
    Ok(raw.and_then(|raw| encode_frame(&raw, 0)))
}
