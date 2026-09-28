# Custom share picker — design

Date: 2026-09-28. Branch: `feature/screen_reader_with_background_ai_request`.

Replaces the native `SCContentSharingPicker` UX with a dedicated
Marvis-owned picker window: record click hides the bar and opens a
centered grid of shareable candidates — Screens / Windows / Apps —
with live thumbnails. Patterned after the Zoom-style share sheet.

## Goals / non-goals

- Goal: a picker the size of a real panel, live thumbnails, sections
  for screen / window / app (the approved layout: sections, no icon
  filter row).
- Goal: bar hides while picking; returns on pick or cancel.
- Non-goal: app-icon filter row (needs NSWorkspace; rejected), share
  audio, smoother-video toggles, picker refresh button (cancel+reopen
  covers staleness), blur-to-cancel.
- The native `capture_pick_and_start` command stays registered as a
  working fallback; the frontend stops calling it.

## Flow

```text
record click (idle) → capture_pick_begin
  → Gate::Main check (same guard shape as capture_start)
  → windows pool: bar.hide() → show_picker() (lazy-built on first
    use, same as the alert window) → emit_to(PICKER, "picker:open")
  → Picker view invokes capture_pick_list on mount AND on each
    picker:open (first open may emit before the webview loads —
    mount covers it; later opens re-fetch on the event)
  → enumerate → Vec<PickCandidate> (labels/ids only — fast)
  → blocking task thumbs each candidate → emit_to(PICKER,
    "picker:thumb", { id, jpeg: base64 }) — cards fill in
    progressively
  → click → capture_pick_select(id)
    → re-enumerate SCShareableContent (fresh — stale ids error)
    → resolve to SCContentFilter (+ pixel size, CaptureTarget)
    → running? stop_capture first (retarget)
    → start_capture → hide_picker → bar.show()
    → capture:state emit already inside start_capture
  → Esc / Cancel → capture_pick_cancel → hide_picker → bar.show()
```

`capture_pick_begin` while a capture runs is not reachable (the bar
start arm only fires when `!captureRunning`), but `select` still
retargets defensively via stop+start — same rule the native picker
learned in Task 7 (the idempotent start path ignores new filters).

## Candidates

`PickCandidate` (serde → JS):

```text
{ id: String, kind: 'display'|'window'|'app',
  label: String, sub: Option<String>, w: u32, h: u32 }
```

`id` is opaque: `"d:<display_id>"`, `"w:<window_id>"`,
`"a:<bundle_id>"`. `sub` carries the owning app name for window
cards; `w`/`h` are the card's aspect ratio hints.

- **Screens**: `content.displays()` in order; label `Screen N`;
  filter `create().with_display(&d).with_excluding_windows(&own)`
  (own-pid windows excluded, matching `primary_display_filter`).
- **Windows**: `content.windows()` filtered to `is_on_screen() &&
  window_layer() == 0 && frame ≥ 140×100`, excluding own pid. Label =
  `title()` when non-empty else the app name; `sub` = app name.
  Filter = `with_window(&w)` — occluded windows still produce their
  own pixels.
- **Apps**: distinct `owning_application()`s of the listed windows,
  deduped by bundle id, own bundle excluded. Label = app name; thumb
  reuses the largest listed window's thumbnail (no extra capture).
  Filter = `with_display(d).with_including_applications(&[app], &[])`
  where `d` is the display containing that window's center
  (`CGRect::contains` on `SCDisplay::frame()`; fallback first
  display).

Resolution on select: `SCShareableContent::get()` fresh, find the
item by id (window_id / display_id / bundle_id), build filter, get
`pixel_size()` via `SCShareableContentInfo::for_filter`. A stale id →
`Err("That item is no longer available")`; the picker shows the error
inline and re-invokes `capture_pick_list`.

## Thumbnails

`SCScreenshotManager::capture_sample_buffer(&filter, &config)` per
display and window candidate with `SCStreamConfiguration` sized to
~480px wide (aspect-preserved) so SCK returns a small buffer
directly; `extract_raw` → JPEG. `encode_frame`'s resize+JPEG core is
refactored to take a target width (1600 for the vision path, ~480 for
thumbs). `base64` (already a dependency) encodes for the
`picker:thumb` emit. Memory only, event-bounded.

App thumbs add zero SCK calls — the backend emits a `picker:thumb`
for the app id using the same JPEG bytes as its source window.

**Privacy exception (deliberate)**: thumbnails are screen pixels
serialized into a Marvis-owned, content-protected, ~480px JS window.
They never reach a provider, never hit disk. This is a scoped
exception to the "no pixels to JS" invariant — user-facing picker
imagery, not pipeline material.

## Picker window

`windows/mod.rs` gains a `picker: Option<WebviewWindow>` pool slot.
Built via the shared `build_window` (borderless, transparent,
always-on-top, skip-taskbar, content-protected, accent-tinted glass,
lazy like `alert`) at ~760×560, `corner_radius` ~16, centered on the
display under the cursor (fallback primary). `show_picker` shows +
`set_focus()`; `hide_picker` hides. Content protection means the
picker never appears in its own candidate list; own-pid exclusion is
belt-and-suspenders.

`?view=picker` routes in `App.tsx` (new `case 'picker'` next to
`alert`/`prefs`).

## Frontend

- `views/Picker.tsx`: three sections (Screens / Windows / Apps), card
  grid, skeleton cards until `picker:thumb` arrives, fetches on
  mount + `picker:open` (see flow note), click →
  `capturePickSelect`, Esc + Cancel button → `capturePickCancel`,
  inline error + auto-refetch on stale-id failure. Arrow components,
  `Icon`-suffixed lucide names, named exports.
- `commands.ts`: `capturePickBegin/List/Select/Cancel` +
  `PickCandidate` type.
- `events.ts`: `EV_PICKER_OPEN`, `EV_PICKER_THUMB` + payload types.
- `Bar.tsx`: record start arm swaps `capturePickAndStart()` →
  `capturePickBegin()`; stop arm unchanged. `capturePickAndStart`
  export stays as a command-surface mirror (same convention as the
  unused `captureStart`).

## Concurrency / locking

- Lock order stays `gate_transition → gate → capture → ring`;
  `capture_pick_begin/select/cancel` are ordinary commands (not
  main-thread callbacks) so normal `lock()` guards apply — no
  `try_lock` needed.
- Thumbnail capture runs in a detached `spawn_blocking` batch after
  `capture_pick_list` returns — enumeration is not re-done per thumb;
  the freshly-built filters from list time are reused.
- No mutex guard crosses `.await`.

## Errors

- Picker begin outside `Gate::Main` → error toast (same as
  `capture_start`).
- `SCShareableContent::get()` failure on list → picker shows empty
  state with retry hint (Cancel still works).
- Per-candidate thumb failure → that card keeps its skeleton/label;
  still selectable (the id resolves independently of the thumb).
- Select on a stale id → inline "no longer available" + auto-refetch.
- Select while capture raced to running → stop+start retarget.
- `capture_pick_cancel` on an already-hidden picker → no-op.

## Testing

- Rust (automated): `PickCandidate` id encode/parse roundtrip;
  source-guard tests in the existing style — `capture_pick_begin` and
  `capture_pick_select` gate-guarded, own-pid/bundle exclusion present
  in `pick_candidates`, `select` stops before re-starting.
  JPEG-at-width via the existing `encode_frame` tests extended to the
  thumb width.
- Rust (manual, `#[ignore]` like existing capture tests): picker list
  against the live desktop.
- Frontend: `bun test` where a convention exists; always
  `check-types` + `build`.
- Manual macOS pass: record → picker opens centered, bar hidden;
  sections populate; thumbs fill in; pick starts scoped capture
  (window/app/display); Esc cancels; bar returns; re-open while
  running unreachable (button is stop).
