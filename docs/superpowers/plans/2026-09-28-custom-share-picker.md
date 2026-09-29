# Custom Share Picker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the native `SCContentSharingPicker` UX with a
Marvis-owned picker window — Screens / Windows / Apps grid with live
thumbnails — that hides the bar while picking.

**Architecture:** A lazily-built borderless glass `picker`
`WebviewWindow` (`?view=picker`, content-protected via the shared
`build_window`). `capture_pick_list` enumerates
`SCShareableContent` into `PickCandidate` metas; a blocking task
thumbs each via `SCScreenshotManager::capture_sample_buffer` →
`extract_raw` → `jpeg_at` → base64 → `picker:thumb` emits. Select
re-resolves the opaque id (`d:`/`w:`/`a:`) against fresh content →
`SCContentFilter` → retarget via stop+start → `start_capture`.

**Tech Stack:** Rust, Tauri 2, `screencapturekit` 10.0.3 (`macos_14_0`
already enabled), `base64` 0.22 (already a dep), React/TS, `bun`.

## Global Constraints

- Lock order `gate_transition → gate → capture → ring`; never hold a
  mutex guard across `.await`; `refresh_tray_menu` never under
  `pool`/`config`/`capture` locks.
- No screen pixels to disk; thumbnail JPEGs are memory-only, bound to
  `picker:thumb` emits — a deliberate scoped exception to the
  no-pixels-to-JS rule (Marvis-owned content-protected window only).
- Marvis's own windows never listed (content-protected + pid filter).
- `com.getmarvis.marvis` is the own bundle id.
- Frontend: arrow-function components, `Icon`-suffixed lucide names,
  named exports for non-view components (views stay default-export
  per `App.tsx` convention), `bun` scripts.
- Keep `capture_pick_and_start` (native picker) registered — working
  fallback; the bar stops calling it, its frontend export stays as a
  command-surface mirror.
- Spec: `docs/superpowers/specs/2026-09-28-custom-share-picker-design.md`

---

### Task 1: Picker candidates + id resolution + thumbnails

**Files:**

- Modify: `apps/native/src-tauri/src/capture/mod.rs` — add DTOs
- Modify: `apps/native/src-tauri/src/capture/macos.rs` — enumeration,
  resolution, thumbnail, `jpeg_at` refactor

**Interfaces:**

- Produces:
  - `pub struct PickCandidate { id: String, kind: &'static str,
    label: String, sub: Option<String>, w: u32, h: u32,
    thumb_of: Option<String> }` — `Serialize`
  - `pub struct PickResolution { pub filter: SCContentFilter,
    pub w: u32, pub h: u32, pub kind: &'static str,
    pub label: String }`
  - `pub(crate) fn pick_candidates() -> anyhow::Result<Vec<PickCandidate>>`
  - `pub(crate) fn resolve_candidate(id: &str)
    -> anyhow::Result<PickResolution>`
  - `pub(crate) fn thumb_for(id: &str) -> Option<String>` (base64 jpeg)
  - `fn jpeg_at(raw: &RawFrame, target_w: u32)
    -> Option<(Vec<u8>, u32, u32)>` (shared with `encode_frame`)

- [ ] **Step 1: Add the DTOs to `capture/mod.rs`**

At the top of `mod.rs` (after the existing `use` block — the file
already imports `serde::Serialize` for `Frame`; if not, add
`use serde::Serialize;`):

```rust
/// One shareable target offered by the picker window — meta only;
/// thumbnails arrive over `picker:thumb` emits.
#[derive(Debug, Clone, Serialize)]
pub struct PickCandidate {
    /// Opaque resolver key: `"d:<display_id>"`, `"w:<window_id>"`,
    /// `"a:<bundle_id>"` — re-resolved against fresh content on pick.
    pub id: String,
    /// `"display" | "window" | "app"` — matches `CaptureTarget.kind`.
    pub kind: &'static str,
    /// Primary card text — `"Screen N"`, window title, or app name.
    pub label: String,
    /// Secondary line — owning app name for window cards.
    pub sub: Option<String>,
    /// Aspect hint (points/pixels) for the card's thumbnail frame.
    pub w: u32,
    pub h: u32,
    /// `"app"` only: the `"w:..."` id whose thumbnail this card reuses
    /// — an app capture composites at display size, so a real app
    /// thumb would be a mostly-empty display shot.
    pub thumb_of: Option<String>,
}

/// A picker id resolved against fresh `SCShareableContent` — the
/// filter, stream dims, and `capture:state` target fields.
pub struct PickResolution {
    pub filter: screencapturekit::stream::content_filter::SCContentFilter,
    pub w: u32,
    pub h: u32,
    /// `"display" | "window" | "app"`
    pub kind: &'static str,
    pub label: String,
}
```

Verify the actual `SCContentFilter` import path in `macos.rs`'s use
block and mirror it here (crate root or
`stream::content_filter::SCContentFilter`).

- [ ] **Step 2: Write the failing tests**

In `apps/native/src-tauri/src/capture/macos.rs`'s test module
(alongside `encode_frame_caps_width_at_target`), extend encode tests
to the shared helper and add a pure id-shape test:

```rust
#[test]
fn jpeg_at_caps_width_at_target() {
    // 800x400 BGRA → 480 wide must produce a 480x240 jpeg.
    let raw = RawFrame {
        data: vec![0u8; 800 * 400 * 4],
        width: 800,
        height: 400,
        bytes_per_row: 800 * 4,
    };
    let (_jpeg, w, h) = jpeg_at(&raw, 480).expect("encode failed");
    assert_eq!((w, h), (480, 240));
}

#[test]
fn jpeg_at_passes_small_sources_through() {
    let raw = RawFrame {
        data: vec![0u8; 320 * 200 * 4],
        width: 320,
        height: 200,
        bytes_per_row: 320 * 4,
    };
    let (_jpeg, w, h) = jpeg_at(&raw, 480).expect("encode failed");
    assert_eq!((w, h), (320, 200));
}

/// `pick_candidates` can never run in tests (needs real
/// `SCShareableContent`), so guard the exclusion logic at source:
/// `pick_window_ok` must drop own-pid windows AND filter by
/// layer/screen/size.
#[test]
fn pick_window_ok_excludes_own_pid_and_nonstandard_windows() {
    let source = include_str!("macos.rs");
    let body = source
        .split("fn pick_window_ok(")
        .nth(1)
        .and_then(|rest| rest.split("\npub(crate) fn ").next())
        .expect("pick_window_ok body not found");
    for needle in [
        "is_on_screen()",
        "window_layer() != 0",
        "app.process_id() != own_pid",
    ] {
        assert!(body.contains(needle), "pick_window_ok must check {needle}");
    }
}
```

Run: `cargo test capture::macos`
Expected: FAIL — `jpeg_at`/`pick_window_ok` do not exist.

- [ ] **Step 3: Refactor `encode_frame` over `jpeg_at` + add the
  picker functions**

In `macos.rs`, split `encode_frame` so the resize+JPEG tail is
reusable (Frame stamping stays in `encode_frame`):

```rust
/// BGRA → `(jpeg, out_w, out_h)` width-capped to `target_w`,
/// aspect preserved, q80 — shared by the stream/shot path
/// (`TARGET_WIDTH`) and picker thumbnails (`THUMB_WIDTH`).
fn jpeg_at(raw: &RawFrame, target_w: u32) -> Option<(Vec<u8>, u32, u32)> {
    let view = BgraView {
        data: &raw.data,
        width: raw.width,
        height: raw.height,
        bytes_per_row: raw.bytes_per_row,
    };
    let mut jpeg = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY);
    let (out_w, out_h, encoded) = if raw.width > target_w {
        let out_h =
            (u64::from(raw.height) * u64::from(target_w)
                / u64::from(raw.width)) as u32;
        let out_h = out_h.max(1);
        let resized =
            imageops::resize(&view, target_w, out_h, FilterType::Triangle);
        (target_w, out_h, encoder.encode_image(&resized))
    } else {
        (raw.width, raw.height, encoder.encode_image(&view))
    };
    match encoded {
        Ok(()) => Some((jpeg, out_w, out_h)),
        Err(e) => {
            log::warn!("jpeg encode failed ({out_w}x{out_h}): {e}");
            None
        }
    }
}

/// BGRA → JPEG `Frame` at `TARGET_WIDTH` — stream worker + one-shot
/// share this so both produce identical `Frame`s.
fn encode_frame(raw: &RawFrame, hash: u64) -> Option<Frame> {
    let (jpeg, width, height) = jpeg_at(raw, TARGET_WIDTH)?;
    Some(Frame {
        jpeg,
        width,
        height,
        ts: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        hash,
    })
}
```

Then add (imports: `use base64::{engine::general_purpose, Engine as _};`
`use std::collections::HashMap;`
`use screencapturekit::shareable_content::SCShareableContentInfo;` —
adjust to the crate's real paths):

```rust
/// Smallest window worth listing — drops palette/tooling slivers.
const PICK_MIN_W: f64 = 140.0;
const PICK_MIN_H: f64 = 100.0;
/// Picker thumbnail width — cards are ~360 CSS px; 480 stays crisp
/// on 2x displays without multi-hundred-KB payloads.
const THUMB_WIDTH: u32 = 480;

/// Window eligibility shared by `pick_candidates` passes: on-screen,
/// normal layer, reasonable size, not ours.
fn pick_window_ok(
    w: &SCWindow,
    own_pid: i32,
) -> Option<screencapturekit::shareable_content::SCRunningApplication> {
    if !w.is_on_screen() || w.window_layer() != 0 {
        return None;
    }
    let f = w.frame();
    if f.size.width < PICK_MIN_W || f.size.height < PICK_MIN_H {
        return None;
    }
    let app = w.owning_application()?;
    (app.process_id() != own_pid).then_some(app)
}

/// Every shareable candidate: displays in order, eligible windows,
/// then one app entry per distinct window owner.
pub(crate) fn pick_candidates() -> anyhow::Result<Vec<PickCandidate>> {
    let content = SCShareableContent::get()?;
    let own_pid = std::process::id() as i32;
    let mut out = Vec::new();
    for (i, d) in content.displays().iter().enumerate() {
        out.push(PickCandidate {
            id: format!("d:{}", d.display_id()),
            kind: "display",
            label: format!("Screen {}", i + 1),
            sub: None,
            w: d.width(),
            h: d.height(),
            thumb_of: None,
        });
    }
    let windows = content.windows();
    // bundle_id → (app_name, largest window id, its area, w, h)
    let mut apps: HashMap<String, (String, u32, f64, u32, u32)> =
        HashMap::new();
    for w in &windows {
        let Some(app) = pick_window_ok(w, own_pid) else {
            continue;
        };
        let f = w.frame();
        let label = w
            .title()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| app.application_name());
        out.push(PickCandidate {
            id: format!("w:{}", w.window_id()),
            kind: "window",
            label,
            sub: Some(app.application_name()),
            w: f.size.width as u32,
            h: f.size.height as u32,
            thumb_of: None,
        });
        let area = f.size.width * f.size.height;
        let entry = apps.entry(app.bundle_identifier()).or_insert_with(|| {
            (app.application_name(), w.window_id(), area,
             f.size.width as u32, f.size.height as u32)
        });
        if area > entry.2 {
            *entry = (app.application_name(), w.window_id(), area,
                      f.size.width as u32, f.size.height as u32);
        }
    }
    // Stable order: sort apps by name so the section doesn't jitter.
    let mut apps: Vec<_> = apps.into_iter().collect();
    apps.sort_by(|a, b| a.1 .0.cmp(&b.1 .0));
    for (bundle, (name, win_id, _area, w, h)) in apps {
        out.push(PickCandidate {
            id: format!("a:{bundle}"),
            kind: "app",
            label: name,
            sub: None,
            w,
            h,
            thumb_of: Some(format!("w:{win_id}")),
        });
    }
    Ok(out)
}
```

`resolve_candidate` — one fn shared by `thumb_for` and
`capture_pick_select`:

```rust
/// Re-resolve a picker id against FRESH shareable content — windows
/// move/close between list and pick, so a stale id errors rather
/// than silently capturing the wrong thing.
pub(crate) fn resolve_candidate(
    id: &str,
) -> anyhow::Result<PickResolution> {
    let content = SCShareableContent::get()?;
    let own_pid = std::process::id() as i32;
    // Borrow Marvis's own windows the same way primary_display_filter
    // does — `windows()` returns an owned Vec, so bind it first.
    let windows = content.windows();
    let own: Vec<&SCWindow> = windows
        .iter()
        .filter(|w| {
            w.owning_application()
                .is_some_and(|a| a.process_id() == own_pid)
        })
        .collect();
    let (filter, kind, label) = if let Some(did) = id.strip_prefix("d:") {
        let did: u32 = did.parse()
            .map_err(|_| anyhow::anyhow!("bad display id"))?;
        let d = content
            .displays()
            .into_iter()
            .find(|d| d.display_id() == did)
            .ok_or_else(|| anyhow::anyhow!("display no longer available"))?;
        let idx = content.displays().iter()
            .position(|x| x.display_id() == did).unwrap_or(0);
        (
            SCContentFilter::create()
                .with_display(&d)
                .with_excluding_windows(&own)
                .build(),
            "display",
            format!("Screen {}", idx + 1),
        )
    } else if let Some(wid) = id.strip_prefix("w:") {
        let wid: u32 = wid.parse()
            .map_err(|_| anyhow::anyhow!("bad window id"))?;
        let win = content
            .windows()
            .into_iter()
            .find(|w| w.window_id() == wid)
            .ok_or_else(|| anyhow::anyhow!("window no longer available"))?;
        let label = win
            .title()
            .filter(|t| !t.trim().is_empty())
            .or_else(|| {
                win.owning_application()
                    .map(|a| a.application_name())
            })
            .unwrap_or_else(|| "Window".into());
        (
            SCContentFilter::create().with_window(&win).build(),
            "window",
            label,
        )
    } else if let Some(bundle) = id.strip_prefix("a:") {
        let app = content
            .applications()
            .into_iter()
            .find(|a| a.bundle_identifier() == bundle)
            .ok_or_else(|| anyhow::anyhow!("app no longer running"))?;
        // Display containing the app's largest eligible window's
        // center; first display as fallback.
        let mut best: Option<(f64, u32)> = None; // (area, display_id)
        for w in &windows {
            if pick_window_ok(w, own_pid).is_none() {
                continue;
            }
            let f = w.frame();
            let (cx, cy) = (f.mid_x(), f.mid_y());
            let Some(d) = content.displays().iter().find(|d| {
                d.frame().contains_point(
                    screencapturekit::cg::CGPoint { x: cx, y: cy },
                )
            }) else { continue };
            let area = f.size.width * f.size.height;
            if best.map_or(true, |(a, _)| area > a) {
                best = Some((area, d.display_id()));
            }
        }
        let did = best.map(|(_, d)| d).unwrap_or_else(|| {
            content.displays().first()
                .map(|d| d.display_id()).unwrap_or(0)
        });
        let d = content
            .displays()
            .into_iter()
            .find(|d| d.display_id() == did)
            .ok_or_else(|| anyhow::anyhow!("no shareable display"))?;
        (
            SCContentFilter::create()
                .with_display(&d)
                .with_including_applications(&[&app], &[])
                .build(),
            "app",
            app.application_name(),
        )
    } else {
        return Err(anyhow::anyhow!("unrecognized picker id"));
    };
    // Zero dims would build a broken MacosCapture — the info lookup
    // is required (it populated for the native picker path).
    let (w, h) = SCShareableContentInfo::for_filter(&filter)
        .map(|i| i.pixel_size())
        .ok_or_else(|| anyhow::anyhow!("filter info unavailable"))?;
    Ok(PickResolution { filter, w, h, kind, label })
}

`thumb_for`:

```rust
/// One ~`THUMB_WIDTH`-wide JPEG for a candidate — the picker card
/// image, base64 (the emit payload is a string).
pub(crate) fn thumb_for(id: &str) -> Option<String> {
    let res = resolve_candidate(id).ok()?;
    if res.w == 0 || res.h == 0 {
        return None;
    }
    let tw = THUMB_WIDTH.min(res.w);
    let th = ((u64::from(res.h) * u64::from(tw)) / u64::from(res.w))
        .max(1) as u32;
    let config = SCStreamConfiguration::new()
        .with_width(tw)
        .with_height(th)
        .with_pixel_format(PixelFormat::BGRA)
        .with_shows_cursor(false);
    let sample =
        SCScreenshotManager::capture_sample_buffer(&res.filter, &config)
            .ok()?;
    let raw = extract_raw(&sample)?;
    let (jpeg, _, _) = jpeg_at(&raw, tw)?;
    Some(general_purpose::STANDARD.encode(jpeg))
}
```

And re-export the DTOs: in `macos.rs` add
`use crate::capture::{PickCandidate, PickResolution};`
(`capture/mod.rs` declares them; check how `Frame`/`RingBuffer` are
re-exported — `mod.rs` is `pub(crate)` used via `crate::capture::X`).

- [ ] **Step 4: Run tests + check**

Run: `cargo test capture::macos && cargo check`
Expected: PASS — new `jpeg_at` tests green, existing encode tests
still green.

- [ ] **Step 5: Commit**

```bash
git add apps/native/src-tauri/src/capture/
git commit -m "capture: picker candidates, id resolution, thumbnails"
```

---

### Task 2: Picker window in `WindowPool`

**Files:**

- Modify: `apps/native/src-tauri/src/windows/mod.rs`

**Interfaces:**

- Produces:
  - `pub const PICKER_LABEL: &str = "picker";`
  - `WindowPool::show_picker(&mut self, app: &AppHandle)`
  - `WindowPool::hide_picker(&self)`
- Consumes: `build_window`, `accent_glass_tint`, `AppState::accent`.

- [ ] **Step 1: Add the slot + constants**

```rust
/// The share-picker surface (`?view=picker`) — lazy like `prefs`,
/// borderless glass via `build_window` (content-protected, so it
/// never appears in its own candidate list).
pub const PICKER_LABEL: &str = "picker";
const PICKER_W: f64 = 760.0;
const PICKER_H: f64 = 560.0;
const PICKER_RADIUS: f64 = 16.0;
```

`WindowPool` gains `picker: Option<WebviewWindow>` — init `None` in
`create_bar_only` AND `new_empty`.

- [ ] **Step 2: `show_picker` / `hide_picker` / center helper**

```rust
/// Show (lazily building) the share picker centered on the display
/// under the pointer, then announce `picker:open` — the view
/// refetches `capture_pick_list` on it (first open can race the
/// still-loading webview; its mount covers that).
pub fn show_picker(&mut self, app: &AppHandle) {
    if self.picker.is_none() {
        let tint = accent_glass_tint(
            &app.state::<crate::AppState>().accent(),
        );
        match build_window(
            app, PICKER_LABEL, PICKER_W, PICKER_H, PICKER_RADIUS, tint,
        ) {
            Ok(win) => self.picker = Some(win),
            Err(e) => {
                log::warn!("windows: picker build failed: {e}");
                return;
            }
        }
    }
    let Some(win) = self.picker.clone() else { return };
    center_on_pointer_display(&win);
    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit_to(PICKER_LABEL, "picker:open", ());
}

pub fn hide_picker(&self) {
    if let Some(win) = &self.picker {
        let _ = win.hide();
    }
}
```

```rust
/// Center `win` on the monitor containing the pointer — physical px
/// math (monitor position/size are physical; the window's logical
/// size scales by `scale_factor`). Primary monitor on any miss.
fn center_on_pointer_display(win: &WebviewWindow) {
    let monitor = win
        .cursor_position()
        .ok()
        .and_then(|pos| {
            win.available_monitors().ok()?.into_iter().find(|m| {
                let (p, s) = (m.position(), m.size());
                pos.x >= p.x
                    && pos.x < p.x + s.width as i32
                    && pos.y >= p.y
                    && pos.y < p.y + s.height as i32
            })
        })
        .or_else(|| win.primary_monitor().ok().flatten())
        .or_else(|| {
            win.available_monitors().ok()?.into_iter().next()
        });
    let Some(mon) = monitor else { return };
    let scale = mon.scale_factor();
    let (mp, ms) = (mon.position(), mon.size());
    let (pw, ph) = ((PICKER_W * scale) as i32, (PICKER_H * scale) as i32);
    let x = mp.x + (ms.width as i32 - pw).max(0) / 2;
    let y = mp.y + (ms.height as i32 - ph).max(0) / 2;
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
}
```

Check `Emitter` is in scope for `app.emit_to` (already imported at
mod.rs:15).

- [ ] **Step 3: Check + commit**

Run: `cargo check`
Expected: clean (dead-code warnings only if commands aren't wired
yet — that's Task 3; `#[allow(dead_code)]` is NOT needed since Task 3
lands immediately after, but if warnings block, they resolve there).

```bash
git add apps/native/src-tauri/src/windows/mod.rs
git commit -m "windows: picker slot — lazy borderless panel on pointer display"
```

---

### Task 3: Picker commands

**Files:**

- Modify: `apps/native/src-tauri/src/lib.rs` — 4 commands +
  registration + tests

**Interfaces:**

- Consumes: Task 1's `pick_candidates`/`resolve_candidate`/`thumb_for`/
  `PickResolution`; Task 2's `PICKER_LABEL`/`show_picker`/`hide_picker`;
  existing `start_capture`/`stop_capture`/`Gate::Main`/
  `gate_transition`/`pool`.
- Produces: commands `capture_pick_begin`, `capture_pick_list`,
  `capture_pick_select`, `capture_pick_cancel`.

- [ ] **Step 1: Write the failing source-guard tests**

In `lib.rs`'s `tests` module, next to
`capture_start_is_gate_guarded_but_capture_stop_is_not`:

```rust
/// The custom picker's begin/select share `capture_start`'s
/// crafted-invoke guard: `gate_transition` held across the check +
    /// action so a racing leave-Main can't interleave. `select`
    /// additionally stops a live capture first — `start_capture` is
    /// idempotent and would silently keep the old scope.
    #[test]
    fn capture_pick_commands_are_gate_guarded() {
        let source = include_str!("lib.rs");
        let body = |sig: &str| {
            source
                .split(sig)
                .nth(1)
                .and_then(|rest| rest.split("\n#[tauri::command]").next())
                .unwrap_or_else(|| panic!("{sig} body not found"))
        };
        for sig in [
            "fn capture_pick_begin(app: AppHandle)",
            "fn capture_pick_list(app: AppHandle)",
            "fn capture_pick_select(app: AppHandle, id: String)",
        ] {
            let b = body(sig);
            let lock = b
                .find("state.gate_transition.lock()")
                .unwrap_or_else(|| panic!("{sig} must hold gate_transition"));
            let check = b
                .find("*state.gate.lock() != Gate::Main")
                .unwrap_or_else(|| panic!("{sig} must check Gate::Main"));
            assert!(lock < check, "{sig}: lock must precede the check");
        }
        let select = body("fn capture_pick_select(app: AppHandle, id: String)");
        let stop = select
            .find("stop_capture(&app)")
            .expect("select must stop a live capture before retargeting");
        let start = select
            .find("start_capture(&app")
            .expect("select must start_capture with the resolved filter");
        assert!(stop < start, "select must stop before starting");
    }
```

Run: `cargo test capture_pick_commands_are_gate_guarded`
Expected: FAIL — bodies don't exist.

- [ ] **Step 2: Implement the commands**

In the `// Commands — permissions / capture` section, after
`capture_pick_and_start`:

```rust
/// Idle record button: hide the bar and open the share-picker window.
/// Gate-guarded like `capture_start` — a crafted invoke outside Main
/// must not surface the picker (or a capture behind it). The bar is
/// re-shown by `capture_pick_select`/`capture_pick_cancel`.
#[tauri::command]
fn capture_pick_begin(app: AppHandle) {
    let state = app.state::<AppState>();
    let _transition = state.gate_transition.lock();
    if *state.gate.lock() != Gate::Main {
        log::warn!("capture_pick_begin dropped while gate != Main");
        return;
    }
    let mut pool = state.pool.lock();
    if let Some(bar) = pool.bar() {
        let _ = bar.hide();
    }
    pool.show_picker(&app);
}

/// The picker's candidate list — meta only, returned fast; a detached
/// blocking task then thumbs each candidate and emits `picker:thumb`
/// to the picker window (SCK calls must not run on the async
/// executor; `capture_sample_buffer` is a sync Cocoa call).
/// App cards reuse their largest window's thumb — the map fills as
/// windows emit, so apps need no extra capture. Gate-guarded: a
/// crafted invoke outside Main could otherwise enumerate window
/// titles.
#[tauri::command]
fn capture_pick_list(app: AppHandle) -> Result<Vec<PickCandidate>, String> {
    let state = app.state::<AppState>();
    let _transition = state.gate_transition.lock();
    if *state.gate.lock() != Gate::Main {
        return Err("picker is only available in the main window".into());
    }
    let metas = capture::pick_candidates().map_err(|e| e.to_string())?;
    let app2 = app.clone();
    let list = metas.clone();
    tauri::async_runtime::spawn(async move {
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let mut thumbs: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for m in &list {
                let jpeg = if m.kind == "app" {
                    m.thumb_of
                        .as_ref()
                        .and_then(|w| thumbs.get(w).cloned())
                } else {
                    let j = capture::thumb_for(&m.id);
                    if let Some(j) = &j {
                        thumbs.insert(m.id.clone(), j.clone());
                    }
                    j
                };
                if let Some(jpeg) = jpeg {
                    let _ = app2.emit_to(
                        windows::PICKER_LABEL,
                        "picker:thumb",
                        json!({ "id": m.id, "jpeg": jpeg }),
                    );
                }
            }
        })
        .await;
    });
    Ok(metas)
}

/// Picker card click: re-resolve the id against fresh content, stop a
/// live capture if one raced in, start scoped, restore the bar.
/// Errors on stale ids ("no longer available") — the picker shows it
/// and refetches.
#[tauri::command]
fn capture_pick_select(app: AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _transition = state.gate_transition.lock();
    if *state.gate.lock() != Gate::Main {
        return Err("picker is only available in the main window".into());
    }
    let res = capture::resolve_candidate(&id).map_err(|e| e.to_string())?;
    if state
        .capture
        .lock()
        .as_ref()
        .is_some_and(MacosCapture::is_running)
    {
        stop_capture(&app);
    }
    start_capture(
        &app,
        res.filter,
        res.w,
        res.h,
        Some(CaptureTarget {
            kind: res.kind,
            label: res.label,
        }),
    );
    let mut pool = state.pool.lock();
    pool.hide_picker();
    if let Some(bar) = pool.bar() {
        let _ = bar.show();
    }
    Ok(())
}

/// Esc / Cancel: drop the picker, restore the bar. No gate check —
/// cancel must always be safe.
#[tauri::command]
fn capture_pick_cancel(app: AppHandle) {
    let state = app.state::<AppState>();
    let mut pool = state.pool.lock();
    pool.hide_picker();
    if let Some(bar) = pool.bar() {
        let _ = bar.show();
    }
}
```

Register in `invoke_handler` next to `capture_pick_and_start`:

```rust
            capture_pick_begin,
            capture_pick_list,
            capture_pick_select,
            capture_pick_cancel,
```

Imports: `use crate::capture::{PickCandidate};` — check what lib.rs
already imports from capture (`MacosCapture`, `primary_display_filter`
are used unqualified — follow the same use block).

- [ ] **Step 3: Run tests + check**

Run: `cargo test && cargo check`
Expected: all green (source-guard test included).

- [ ] **Step 4: Commit**

```bash
git add apps/native/src-tauri/src
git commit -m "capture: pick_begin/list/select/cancel commands"
```

---

### Task 4: Frontend command/event surface

**Files:**

- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/lib/events.ts`

**Interfaces:**

- Produces: `PickCandidate` type, `capturePickBegin/List/Select/Cancel`,
  `EV_PICKER_OPEN`, `EV_PICKER_THUMB`.

- [ ] **Step 1: `commands.ts` additions**

Next to `capturePickAndStart`:

```ts
/** One shareable target offered by the picker — meta only; thumbs
 *  arrive over `picker:thumb`. `id` is the opaque `"d:"/"w:"/"a:"`
 *  resolver key `capturePickSelect` echoes back. */
export interface PickCandidate {
  id: string;
  kind: 'display' | 'window' | 'app';
  label: string;
  sub: string | null;
  w: number;
  h: number;
  /** "app" only — the window id whose thumb this card reuses. */
  thumb_of: string | null;
}

/** Idle record button — hides the bar, opens the picker window. */
export const capturePickBegin = () =>
  invoke<void>('capture_pick_begin');
/** Meta list for the picker grid; thumbs follow on `picker:thumb`. */
export const capturePickList = () =>
  invoke<PickCandidate[]>('capture_pick_list');
/** Card click — resolves void; rejects with a string error on
 *  stale ids. */
export const capturePickSelect = (id: string) =>
  invoke<void>('capture_pick_select', { id });
/** Esc / Cancel — drops the picker, restores the bar. */
export const capturePickCancel = () =>
  invoke<void>('capture_pick_cancel');
```

- [ ] **Step 2: `events.ts` additions**

```ts
/** Emitted to the picker window by `show_picker` — the view refetches
 * `capture_pick_list` on it. The view ALSO fetches on mount: first
 * open can emit before this webview's listener exists. */
export const EV_PICKER_OPEN = 'picker:open';
/** Per-candidate thumbnail, emitted to the picker as each renders —
 * `{ id, jpeg }` where jpeg is base64 (pick-list background task). */
export const EV_PICKER_THUMB = 'picker:thumb';
export interface PickerThumbPayload {
  id: string;
  jpeg: string;
}
```

- [ ] **Step 3: Check + commit**

Run: `cd apps/native && bun run check-types`
Expected: clean.

```bash
git add apps/native/src/lib/commands.ts apps/native/src/lib/events.ts
git commit -m "picker: command + event surface"
```

---

### Task 5: Picker view + routing + bar wiring

**Files:**

- Create: `apps/native/src/lib/pick-groups.ts`
- Create: `apps/native/src/lib/pick-groups.test.ts`
- Create: `apps/native/src/views/Picker.tsx`
- Modify: `apps/native/src/App.tsx`
- Modify: `apps/native/src/views/Bar.tsx` — start arm swap

**Interfaces:**

- Consumes: Task 4's surface; `useTauriEvent`; `glass-stage`/
  `glass-surface`/`ICON_BTN`/`cn`; `openDevTools`.
- Produces: `?view=picker` rendering; record button → `capturePickBegin`.

- [ ] **Step 1: Write the failing test (`pick-groups`)**

`apps/native/src/lib/pick-groups.test.ts` only — the impl doesn't
exist yet:

```ts
import { describe, expect, test } from 'bun:test';
import { groupCandidates } from './pick-groups';
import type { PickCandidate } from './commands';

const c = (id: string, kind: PickCandidate['kind']): PickCandidate => ({
  id,
  kind,
  label: id,
  sub: null,
  w: 100,
  h: 100,
  thumb_of: null,
});

describe('groupCandidates', () => {
  test('partitions by kind preserving order', () => {
    const g = groupCandidates([
      c('d:1', 'display'),
      c('w:9', 'window'),
      c('w:3', 'window'),
      c('a:x', 'app'),
      c('d:2', 'display'),
    ]);
    expect(g.screens.map((c) => c.id)).toEqual(['d:1', 'd:2']);
    expect(g.windows.map((c) => c.id)).toEqual(['w:9', 'w:3']);
    expect(g.apps.map((c) => c.id)).toEqual(['a:x']);
  });

  test('empty input gives empty sections', () => {
    const g = groupCandidates([]);
    expect(g).toEqual({ screens: [], windows: [], apps: [] });
  });
});
```

Run: `cd apps/native && bun test src/lib/pick-groups.test.ts`
Expected: FAIL — `Cannot find module './pick-groups'`.

- [ ] **Step 2: Implement `pick-groups.ts`**

```ts
import type { PickCandidate } from './commands';

export interface PickGroups {
  screens: PickCandidate[];
  windows: PickCandidate[];
  apps: PickCandidate[];
}

/** Section the flat candidate list — backend order is already
 *  displays → windows → apps; grouping just partitions. */
export const groupCandidates = (cs: PickCandidate[]): PickGroups => ({
  screens: cs.filter((c) => c.kind === 'display'),
  windows: cs.filter((c) => c.kind === 'window'),
  apps: cs.filter((c) => c.kind === 'app'),
});
```

Run: `cd apps/native && bun test src/lib/pick-groups.test.ts`
Expected: PASS.

- [ ] **Step 3: Extend the `@marvis/ui` icon barrel**

AGENTS.md rule 9 — all icons come through `packages/ui/src/index.ts`
suffixed. The barrel currently lacks `MonitorIcon`/`AppWindowIcon`/
`LayoutGridIcon` — add them to the existing `lucide-react` export
list (alphabetical where the list is):

```ts
export { ..., AppWindowIcon, LayoutGridIcon, MonitorIcon } from
  'lucide-react';
```

Merge into whichever existing `export { ... } from 'lucide-react'`
block the file uses — do not add a second import from lucide-react.

- [ ] **Step 4: `views/Picker.tsx`**

Model on `AlertToast.tsx` (default-export view, `glass-stage` stage,
dev-only `openDevTools` context menu). Card grid: thumbnail box
`aspect-video` with `img` (`data:image/jpeg;base64,${thumb}`) or a
skeleton `animate-pulse` div; label + sub below; sections rendered
only when non-empty; footer with inline error text + Cancel button.

```tsx
/**
 * `?view=picker` — the share-picker surface (760×560 borderless glass,
 * `windows/mod.rs` centers it on the pointer's display). Replaces the
 * native SCContentSharingPicker: `capture_pick_list` returns the
 * candidate meta fast, then `picker:thumb` emits fill card images in
 * as each ~480px JPEG lands. Esc or Cancel → `capture_pick_cancel`
 * (Rust re-shows the bar); a card click → `capture_pick_select`
 * (Rust stops any live capture, starts scoped, re-shows the bar).
 * Stale ids reject — show the error and refetch.
 */
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { MonitorIcon, AppWindowIcon, LayoutGridIcon, XIcon }
  from '@marvis/ui';
import {
  capturePickCancel,
  capturePickList,
  capturePickSelect,
  openDevTools,
  type PickCandidate,
} from '../lib/commands';
import {
  EV_PICKER_OPEN,
  EV_PICKER_THUMB,
  useTauriEvent,
  type PickerThumbPayload,
} from '../lib/events';
import { groupCandidates } from '../lib/pick-groups';

const Picker = () => {
  const [cands, setCands] = useState<PickCandidate[]>([]);
  const [thumbs, setThumbs] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    setError(null);
    setThumbs({});
    void capturePickList()
      .then(setCands)
      .catch(() => setError('Could not list shareable content'));
  }, []);

  // Mount covers the first open (the emit can race this webview's
  // load); picker:open drives every later open.
  useEffect(() => refresh(), [refresh]);
  useTauriEvent(EV_PICKER_OPEN, refresh);
  useTauriEvent<PickerThumbPayload>(EV_PICKER_THUMB, (p) => {
    setThumbs((t) => ({ ...t, [p.id]: p.jpeg }));
  });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        void capturePickCancel().catch(() => {});
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const pick = (id: string) => {
    void capturePickSelect(id).catch((e) => {
      setError(typeof e === 'string' ? e : 'Pick failed');
      refresh();
    });
  };
  const cancel = () => void capturePickCancel().catch(() => {});

  const groups = groupCandidates(cands);
  const section = (
    title: string,
    icon: ReactNode,
    items: PickCandidate[],
  ) =>
    items.length > 0 && (
      <section className='mb-3'>
        <header className='mb-1.5 flex items-center gap-1.5 px-0.5 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground'>
          {icon}
          {title}
          <span className='font-normal'>({items.length})</span>
        </header>
        <div className='grid grid-cols-3 gap-2'>
          {items.map((item) => (
            <button
              key={item.id}
              type='button'
              onClick={() => pick(item.id)}
              className='group rounded-lg border border-transparent p-1 text-left transition-colors hover:border-[color-mix(in_oklch,var(--accent)_45%,var(--border))] hover:bg-[color-mix(in_oklch,var(--fg)_6%,transparent)] focus-visible:border-accent focus-visible:outline-none'>
              <div className='aspect-video w-full overflow-hidden rounded-md bg-[color-mix(in_oklch,var(--fg)_7%,transparent)]'>
                {thumbs[item.id] ? (
                  <img
                    src={`data:image/jpeg;base64,${thumbs[item.id]}`}
                    alt=''
                    className='h-full w-full object-cover object-top'
                  />
                ) : (
                  <div className='h-full w-full animate-pulse' />
                )}
              </div>
              <p className='mt-1 truncate text-[12px] font-medium leading-tight'>
                {item.label}
              </p>
              {item.sub && (
                <p className='truncate text-[10.5px] text-muted-foreground'>
                  {item.sub}
                </p>
              )}
            </button>
          ))}
        </div>
      </section>
    );

  return (
    <div
      className='glass-stage h-full p-1.5'
      onContextMenu={(e) => {
        if (import.meta.env.DEV) {
          e.preventDefault();
          void openDevTools().catch(() => {});
        }
      }}>
      <div className='glass-surface flex h-full flex-col rounded-2xl border border-border bg-[color-mix(in_oklch,var(--surface)_94%,transparent)] shadow-[0_24px_60px_-20px_color-mix(in_oklch,var(--fg)_40%,transparent)] backdrop-blur-xl'>
        <header className='flex items-center gap-2 px-3.5 pt-3 pb-2'>
          <p className='flex-1 text-[13.5px] font-semibold'>
            Choose what to record
          </p>
          <button
            type='button'
            onClick={cancel}
            className='rounded-md p-1 text-muted-foreground transition-colors hover:bg-[color-mix(in_oklch,var(--fg)_8%,transparent)] hover:text-foreground'
            title='Cancel (Esc)'
            aria-label='Cancel'>
            <XIcon className='size-4' />
          </button>
        </header>
        <div className='min-h-0 flex-1 overflow-y-auto px-3 pb-2'>
          {section(
            'Screens',
            <MonitorIcon className='size-3.5' />,
            groups.screens,
          )}
          {section(
            'Windows',
            <AppWindowIcon className='size-3.5' />,
            groups.windows,
          )}
          {section(
            'Apps',
            <LayoutGridIcon className='size-3.5' />,
            groups.apps,
          )}
          {cands.length === 0 && !error && (
            <p className='py-10 text-center text-xs text-muted-foreground'>
              Looking for shareable content…
            </p>
          )}
          {error && (
            <p className='py-3 text-center text-xs text-destructive'>
              {error}
            </p>
          )}
        </div>
      </div>
    </div>
  );
};

export default Picker;
```

Verify icon names exist in `@marvis/ui`'s export list
(`MonitorIcon` does — the bar uses it; `AppWindowIcon`,
`Squares2X2Icon`, `XIcon` — check `packages/ui/src/index.ts` and
swap for whatever's exported, e.g. `LayoutGridIcon`, `AppWindowIcon`,
`PanelsTopLeftIcon`).

- [ ] **Step 5: `App.tsx` routing**

```tsx
import Picker from './views/Picker';
// in the switch:
    case 'picker':
      return <Picker />;
```

- [ ] **Step 6: `Bar.tsx` start-arm swap**

In `toggleCapture`, the `wantRunning` arm swaps the native picker
call for the custom picker open (identical `.catch`/`.finally`
handling — the invoke still resolves once the picker window shows):

```ts
    if (wantRunning) {
      // The custom share picker: hides the bar and shows candidate
      // thumbs — the pick (or cancel) lands via capture_pick_select /
      // capture_pick_cancel, so there's no status to check here.
      void capturePickBegin()
        .catch(() => raise(failure))
        .finally(() => setBusy(false));
      return;
    }
```

Update the import (`capturePickAndStart` → `capturePickBegin`; keep
the `capturePickAndStart` export in commands.ts — it stays a surface
mirror like `captureStart`).

- [ ] **Step 7: Verify + commit**

Run:

```bash
cd apps/native && bun run check-types && bun test && bun run build
```

Expected: all clean.

```bash
git add apps/native/src
git commit -m "picker: view, routing, record button → custom picker"
```

---

### Task 6: Manual macOS verification

**Files:** none — a human pass on a real desktop.

- [ ] `bun run build:dev`; press the bar's record button:
  - bar hides; picker opens centered on the pointer's display
  - Screens/Windows/Apps populate; thumbnails fill in progressively
  - Esc or ✕ cancels → bar returns, nothing captured
  - click a window card → picker closes, bar returns, capture starts,
    `capture:state.target` = `{kind:'window', label}` (record button
    tooltip shows it); ask via Cmd+Enter reflects the window's content
  - click an app card → capture scopes to that app's windows
  - re-open while recording → button is Stop (picker unreachable —
    correct per spec)
  - close a listed window, then pick it → inline error + refetch
