use super::*;
use super::{build::*, geometry::*};

/// Owns the bar window (which now hosts chat/listen as expanded card
/// modes), the alert toast, and the prefs window.
///
/// `bar_rect` is the canonical PILL rect — source of truth for
/// position, edge detection, and `Moved`-debounce persistence. While
/// the card is open the live window IS the card; `expand_dir` +
/// `chat_height` derive the card rect from the pill, and
/// `refresh_bar_rect` derives the pill back, so nothing downstream ever
/// records card geometry.
pub struct WindowPool {
    bar: Option<WebviewWindow>,
    /// Alert toast — built with the bar (it never joins the gate's
    /// lifecycle; it must exist before `Main` to report failures).
    alert: Option<WebviewWindow>,
    /// The decorated preferences window (settings + onboarding). Built
    /// lazily on first `show_prefs` — it is NOT part of `Main`.
    prefs: Option<WebviewWindow>,
    /// The mode `prefs` was last opened in (`"settings"|"onboarding"`) —
    /// the `prefs_mode` command returns it so a `prefs:mode` emit that
    /// raced a still-loading webview isn't lost.
    prefs_mode: String,
    /// The share-picker panel (`?view=picker`) — built lazily on first
    /// `show_picker`, then re-shown; it never joins the gate lifecycle.
    picker: Option<WebviewWindow>,
    /// The preset palette (`?view=palette`) — lazy like `picker`;
    /// blur-dismissed, so it never needs gate cleanup either.
    palette: Option<WebviewWindow>,
    /// Whether the palette holds (or opened with) key focus — the wand
    /// opens it focused; a `/`-typed open leaves the composer key and
    /// only a real user click promotes it. `Focused(false)` dismisses
    /// menu-style only while this is set — an unfocused palette must
    /// not hide itself from the show()/refocus ping-pong it isn't in.
    palette_focused: bool,
    /// Content height last reported by the palette view — the window
    /// hugs the list (auto-fit up to `PALETTE_MAX_H`). Reused across
    /// opens so a reopened palette doesn't flicker through the default.
    palette_h: f64,
    /// The anchor the last open resolved — reused by a height reflow
    /// to keep the same composer-side gap.
    palette_anchor: Option<PaletteAnchor>,
    /// Whether the unified card (chat or listen mode) is open.
    chat_open: bool,
    /// Last reported card CONTENT height — window = `BAR_H + this`.
    /// Reset to the 30% floor on every expand; `window_adjust_height`
    /// then tracks content inside the [30%, 60%] band.
    chat_height: f64,
    /// Which way the card grows — computed at expand time from free
    /// space (`Up` toward the top edge, `Down` toward the bottom).
    expand_dir: Dir,
    /// Last known pill rect (logical). Refreshed from the live window
    /// (deriving the pill when expanded) so user drags aren't lost.
    bar_rect: Rect,
}

/// Debounce counter for the bar's `Moved` event — a drag fires one event
/// per frame, so the persist write waits for the LAST position (400 ms
/// quiet) instead of rewriting `config.toml` on every pixel.
static BAR_MOVE_GEN: AtomicU64 = AtomicU64::new(0);

impl WindowPool {
    /// Build ONLY the bar window — it now hosts the chat/listen card
    /// modes itself, so no feature windows exist to build later.
    /// `show_bar` is the same `onboarding_done` flag — first-run
    /// installs keep the bar hidden until the wizard finishes;
    /// `set_bar_shown` owns it after.
    pub fn create_bar_only(app: &AppHandle, show_bar: bool, accent: &str) -> anyhow::Result<Self> {
        let mut pool = Self {
            bar: None,
            alert: None,
            prefs: None,
            prefs_mode: String::new(),
            picker: None,
            palette: None,
            palette_focused: false,
            palette_h: PALETTE_H,
            palette_anchor: None,
            chat_open: false,
            chat_height: CHAT_DEFAULT_H,
            expand_dir: Dir::Down,
            bar_rect: DEFAULT_WORK,
        };
        let bar = build_window(
            app,
            BAR_LABEL,
            BAR_IDLE_W,
            BAR_H,
            BAR_H / 2.0,
            accent_glass_tint(accent),
            false,
        )?;
        {
            // Persist the resting place on every move — user drags via
            // `data-tauri-drag-region`, edge snaps, and reclamps alike.
            // `Moved` fires per frame during a drag, so the write is
            // debounced: the last generation wins after 400 ms of quiet.
            //
            // `Resized` is different: macOS 26 edge-drags resize
            // borderless windows even with `resizable(false)`, so any
            // stray size snaps back to the canonical bounds — fully for
            // the fixed-size pill, x/width only for the open card (its
            // dragged height is adopted via `window_adjust_height`).
            let app_moved = app.clone();
            let app_resized = app.clone();
            bar.on_window_event(move |event| {
                match event {
                    tauri::WindowEvent::Moved(_) => {
                        let gen = BAR_MOVE_GEN.fetch_add(1, Ordering::Relaxed) + 1;
                        let app = app_moved.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(400));
                            if BAR_MOVE_GEN.load(Ordering::Relaxed) == gen {
                                crate::persist_bar_position(&app);
                            }
                        });
                    }
                    tauri::WindowEvent::Resized(_) => {
                        // Our animator emits `Resized` per tick — only
                        // enforce while no animation is in flight.
                        if movement::is_animating(BAR_LABEL) {
                            return;
                        }
                        // This handler registers before `app.manage(AppState)`
                        // (setup order) — a `Resized` delivered inside
                        // `position_bar_at_startup` must not panic on the
                        // missing state.
                        let Some(state) = app_resized.try_state::<crate::AppState>() else {
                            return;
                        };
                        if let Some(pool) = state.pool.try_lock() {
                            pool.enforce_bar_bounds();
                        };
                    }
                    // An unfocused (`/`-typed) palette belongs to the
                    // composer — the bar losing focus (another app)
                    // retires it so it can't float orphaned over other
                    // windows. A palette CLICK also resigns the bar's
                    // key status — the resign posts before the
                    // palette's own `Focused(true)`, so checking here
                    // would see `palette_focused` still false and kill
                    // the palette mid-click. Deferring one main-queue
                    // turn lets the promotion land first: a real blur
                    // still hides it, a click-through doesn't.
                    tauri::WindowEvent::Focused(false) => {
                        let app = app_resized.clone();
                        let app2 = app_resized.clone();
                        let _ = app.run_on_main_thread(move || {
                            let Some(state) = app2.try_state::<crate::AppState>() else {
                                return;
                            };
                            // `try_lock`, never park: a command holding
                            // `pool` can itself be waiting on this main
                            // thread (window ops) — blocking here is the
                            // ABBA deadlock AGENTS.md §14 names. A lost
                            // race just skips one blur-dismiss.
                            if let Some(mut pool) = state.pool.try_lock() {
                                pool.hide_palette_unfocused(&app2);
                            };
                        });
                    }
                    _ => {}
                }
            });
        }
        pool.bar = Some(bar);
        // Built (hidden) up front so its webview is loaded and listening
        // before the first alert — an emit to a still-loading window
        // would be dropped.
        pool.alert = Some(build_window(
            app,
            ALERT_LABEL,
            ALERT_W,
            ALERT_H,
            14.0,
            None,
            false,
        )?);
        pool.position_bar_at_startup();
        if show_bar {
            if let Some(bar) = &pool.bar {
                let _ = bar.show();
            }
        }
        Ok(pool)
    }

    /// The bar's visibility rule: it floats only when onboarding is done
    /// AND the prefs window isn't showing the wizard — a re-run hides the
    /// bar again until the window closes. `AppState::sync_bar_visibility`
    /// calls this after every prefs show/hide and every
    /// `app.onboarding_done` write.
    pub fn set_bar_shown(&self, onboarding_done: bool) {
        let onboarding_up = self.prefs_mode == "onboarding"
            && self
                .prefs
                .as_ref()
                .is_some_and(|w| w.is_visible().unwrap_or(false));
        if let Some(bar) = &self.bar {
            if onboarding_done && !onboarding_up {
                let _ = bar.show();
            } else {
                let _ = bar.hide();
            }
        }
    }

    /// Windowless pool — the `AppState::for_test` seam: state needs a pool
    /// value even when no runtime (and thus no windows) exists.
    #[cfg(test)]
    pub fn new_empty() -> Self {
        Self {
            bar: None,
            alert: None,
            prefs: None,
            prefs_mode: String::new(),
            picker: None,
            palette: None,
            palette_focused: false,
            palette_h: PALETTE_H,
            palette_anchor: None,
            chat_open: false,
            chat_height: CHAT_DEFAULT_H,
            expand_dir: Dir::Down,
            bar_rect: DEFAULT_WORK,
        }
    }

    pub fn bar(&self) -> Option<&WebviewWindow> {
        self.bar.as_ref()
    }

    /// Slide the alert toast in, centered under the bar's current
    /// target — the expanded card when open, the pill otherwise (for
    /// grow-up they coincide). Deliberately overlaps the card and is
    /// shown last so it lands on top; it is never focused, so a bar
    /// field keeps its focus.
    pub fn show_alert(&mut self) {
        let Some(win) = self.alert.clone() else {
            log::warn!("windows::show_alert: alert window not created");
            return;
        };
        self.reclamp();
        let anchor = self.target_rect();
        let target = clamp_to_work_area(
            Rect {
                x: anchor.center_x() - ALERT_W / 2.0,
                y: anchor.bottom() + layout::PANEL_PAD,
                w: ALERT_W,
                h: ALERT_H,
            },
            self.bar_work_area(),
        );
        set_rect(
            &win,
            Rect {
                y: target.y - SHOW_OFFSET_Y,
                ..target
            },
        );
        let _ = win.show();
        let _ = win.set_always_on_top(true);
        movement::animate(&win, target, ANIM_DUR);
    }

    pub fn hide_alert(&self) {
        if let Some(win) = &self.alert {
            let _ = win.hide();
        }
    }

    /// Show the prefs window in `mode` (`"settings"` or `"onboarding"`).
    /// Built lazily — once it exists the same webview is re-shown and the
    /// mode is pushed via `prefs:mode` (the frontend also re-reads it with
    /// `prefs_mode` on mount, so a show that raced the load still lands).
    /// The window title follows the mode: `Marvis — Settings` /
    /// `Marvis — Set up`. Onboarding mode hides the bar — the wizard
    /// must not share the screen with the floating UI.
    pub fn show_prefs(&mut self, app: &AppHandle, mode: &str) {
        let win = match &self.prefs {
            Some(w) => w.clone(),
            None => match build_prefs_window(app) {
                Ok(w) => {
                    // Windows/Linux have no app menubar — the prefs
                    // window is the one decorated surface, so it hosts
                    // the File/View/Help menu (menubar.rs). `bar_edge`
                    // reads `self` directly: `state.pool` is already
                    // locked by whoever called `show_prefs`.
                    #[cfg(not(target_os = "macos"))]
                    match crate::menubar::build(app, self.bar_edge()) {
                        Ok(menu) => {
                            if let Err(e) = w.set_menu(menu) {
                                log::warn!("windows: prefs menu install failed: {e}");
                            }
                        }
                        Err(e) => log::warn!("windows: prefs menu build failed: {e}"),
                    }
                    // Center only on first show — after that the user's
                    // placement (normal macOS window behavior) is kept.
                    let _ = w.center();
                    self.prefs = Some(w.clone());
                    w
                }
                Err(e) => {
                    log::warn!("windows: prefs window failed to build: {e}");
                    return;
                }
            },
        };
        self.prefs_mode = mode.to_string();
        let _ = app.emit_to(
            PREFS_LABEL,
            "prefs:mode",
            serde_json::json!({ "mode": mode }),
        );
        let _ = win.set_title(match mode {
            "onboarding" => "Marvis — Set up",
            _ => "Marvis — Settings",
        });
        let _ = win.show();
        let _ = win.set_focus();
        // The bar's visibility rule — applied AFTER the prefs window is
        // visible so `set_bar_shown` sees the wizard up.
        let done = app.state::<crate::AppState>().onboarding_done();
        self.set_bar_shown(done);
    }

    pub fn hide_prefs(&self) {
        if let Some(win) = &self.prefs {
            // Drop the focused-only float before hiding — a hidden
            // window must never hold the always-on-top level.
            let _ = win.set_always_on_top(false);
            let _ = win.hide();
        }
    }

    pub fn prefs_mode(&self) -> &str {
        &self.prefs_mode
    }

    /// Show (lazily building) the share picker centered on the display
    /// under the pointer, then announce `picker:open` — the view
    /// refetches `capture_pick_list` on it (first open can race the
    /// still-loading webview; its mount covers that).
    pub fn show_picker(&mut self, app: &AppHandle) -> bool {
        if self.picker.is_none() {
            let tint = accent_glass_tint(&app.state::<crate::AppState>().accent());
            match build_window(
                app,
                PICKER_LABEL,
                PICKER_W,
                PICKER_H,
                PICKER_RADIUS,
                tint,
                false,
            ) {
                Ok(win) => self.picker = Some(win),
                Err(e) => {
                    log::warn!("windows: picker build failed: {e}");
                    return false;
                }
            }
        }
        let Some(win) = self.picker.clone() else {
            return false;
        };
        center_on_pointer_display(&win);
        let _ = win.show();
        let _ = win.set_focus();
        let _ = app.emit_to(PICKER_LABEL, "picker:open", ());
        true
    }

    pub fn hide_picker(&self) {
        if let Some(win) = &self.picker {
            let _ = win.hide();
        }
    }

    /// Show (lazily building) the preset palette below/above the bar —
    /// `palette_rect` picks the side with more room and left-aligns to
    /// `anchor_x` (the composer caret's x in BAR-viewport px; null
    /// falls back to the pointer, then the bar's center) — then
    /// announce `palette:open` with the `/` query seed (the view
    /// refetches `presets_list` on it, same race cover as
    /// `picker:open`). `focused` picks the interaction model: wand
    /// and right-click opens take key focus (own key nav, click-away
    /// blur hides); a `/`-typed open orders front UNFOCUSED so the
    /// composer keeps typing — its `/token` streams in as
    /// `palette:query` and its keys arrive forwarded as `palette:key`.
    /// `palette_focused` records which — `Focused(false)` is the
    /// menu's dismiss only then.
    pub fn show_palette(
        &mut self,
        app: &AppHandle,
        anchor_x: Option<f64>,
        anchor_y: Option<f64>,
        query: Option<String>,
        focused: bool,
    ) -> bool {
        if self.palette.is_none() {
            let tint = accent_glass_tint(&app.state::<crate::AppState>().accent());
            let win = match build_window(
                app,
                PALETTE_LABEL,
                PALETTE_W,
                PALETTE_H,
                PALETTE_RADIUS,
                tint,
                // The `/` open keeps this window non-key — without
                // first-mouse, macOS eats the activating click and the
                // row under the cursor never sees it.
                true,
            ) {
                Ok(win) => win,
                Err(e) => {
                    log::warn!("windows: palette build failed: {e}");
                    return false;
                }
            };
            {
                let app2 = app.clone();
                win.on_window_event(move |event| {
                    // `try_state` like the bar's `Resized` arm — the
                    // handler outlives any gate/state teardown.
                    let Some(state) = app2.try_state::<crate::AppState>() else {
                        return;
                    };
                    match event {
                        // A click on the unfocused palette makes it key
                        // (`accept_first_mouse` delivers the click
                        // through, and the activation promotes it) so
                        // the click-away below still dismisses,
                        // menu-style. `try_lock` — `on_window_event`
                        // runs on the main thread, where a `pool`
                        // holder may be parked on window ops; dropping
                        // on contention beats deadlocking.
                        tauri::WindowEvent::Focused(true) => {
                            if let Some(mut pool) = state.pool.try_lock() {
                                pool.palette_focused = true;
                            }
                        }
                        tauri::WindowEvent::Focused(false) => {
                            if let Some(mut pool) = state.pool.try_lock() {
                                if pool.palette_focused {
                                    pool.hide_palette(&app2);
                                }
                            }
                        }
                        _ => {}
                    }
                });
            }
            self.palette = Some(win);
        }
        let Some(win) = self.palette.clone() else {
            return false;
        };
        // Anchor precedence: the caller's caret x (viewport px, offset
        // by the window's screen position — `outer_position` tracks
        // the expanded card, `bar_rect` only the pill), then the
        // pointer, then the bar's center.
        let anchor_x = anchor_x
            .zip(self.bar.as_ref())
            .and_then(|(ax, bar)| {
                let scale = bar.scale_factor().ok()?;
                let pos: LogicalPosition<f64> = bar.outer_position().ok()?.to_logical(scale);
                Some(pos.x + ax)
            })
            .or_else(|| {
                self.bar.as_ref().and_then(|bar| {
                    let scale = bar.scale_factor().ok()?;
                    let pos: LogicalPosition<f64> = bar.cursor_position().ok()?.to_logical(scale);
                    Some(pos.x)
                })
            })
            .unwrap_or_else(|| self.bar_rect.center_x());
        // The composer row's top edge resolves the same way — viewport
        // px + `outer_position` (the live window, card included). Card
        // mode pops the palette above it; the pill ignores it.
        let anchor_y = anchor_y.zip(self.bar.as_ref()).and_then(|(ay, bar)| {
            let scale = bar.scale_factor().ok()?;
            let pos: LogicalPosition<f64> = bar.outer_position().ok()?.to_logical(scale);
            Some(pos.y + ay)
        });
        self.palette_anchor = Some(PaletteAnchor {
            x: anchor_x,
            top: anchor_y,
        });
        let r = self.palette_placement(anchor_x, anchor_y);
        set_rect(&win, r);
        self.palette_focused = focused;
        if focused {
            let _ = win.show();
            let _ = win.set_focus();
        } else {
            // A `/`-typed open leaves the composer key — order the
            // palette front WITHOUT making key (macOS `orderFront:`;
            // `show()` would steal focus). Elsewhere `show()` is the
            // only primitive — if it does activate, `Focused(true)`
            // promotes the palette into focused-mode anyway.
            order_front_unfocused(&win);
        }
        let _ = app.emit_to(
            PALETTE_LABEL,
            crate::EV_PALETTE_OPEN,
            serde_json::json!({ "query": query }),
        );
        true
    }

    /// Hide the palette and reset its focus mode — announces
    /// `bar:palette-closed` on every hide (pick, Esc, blur, the bar
    /// leaving focus) so the composer stops forwarding keys.
    pub fn hide_palette(&mut self, app: &AppHandle) {
        self.palette_focused = false;
        if let Some(win) = &self.palette {
            let _ = win.hide();
        }
        let _ = app.emit_to(BAR_LABEL, crate::EV_PALETTE_CLOSED, ());
    }

    /// The bar's own blur dismisses an UNFOCUSED palette — a
    /// `/`-opened overlay must not outlive the composer it filters.
    /// A focused palette owns its blur path instead (it took key
    /// status deliberately), so a wand-opened palette survives this.
    pub fn hide_palette_unfocused(&mut self, app: &AppHandle) {
        if !self.palette_focused && self.palette.is_some() {
            self.hide_palette(app);
        }
    }

    /// Forward a composer key to the open palette — the `/`-typed
    /// session keeps the bar focused, so `↑↓`/`Enter`/`Esc`/`Tab`/
    /// `Home`/`End` arrive here and ride `palette:key`.
    pub fn palette_key(&self, app: &AppHandle, key: &str) {
        let _ = app.emit_to(
            PALETTE_LABEL,
            crate::EV_PALETTE_KEY,
            serde_json::json!({ "key": key }),
        );
    }

    /// Push the composer's `/token` as the palette's filter query —
    /// called on every composer edit while an unfocused palette is up.
    pub fn palette_query(&self, app: &AppHandle, query: &str) {
        let _ = app.emit_to(
            PALETTE_LABEL,
            crate::EV_PALETTE_QUERY,
            serde_json::json!({ "query": query }),
        );
    }

    /// The palette's rect for an open or a height reflow. While a card
    /// is up the composer is its bottom-anchored footer — the palette
    /// pops above the row's top edge (`anchor_top`, a screen y) instead
    /// of picking a pill side, which in grow-down would park it on the
    /// card's top header, nowhere near the input. Collapsed, or a
    /// missing edge, keeps `palette_rect`'s pill-side pick.
    fn palette_placement(&self, anchor_x: f64, anchor_top: Option<f64>) -> Rect {
        let work = self.bar_work_area();
        if let (true, Some(top)) = (self.chat_open, anchor_top) {
            return clamp_to_work_area(
                Rect {
                    x: anchor_x,
                    y: top - self.palette_h - layout::PANEL_PAD,
                    w: PALETTE_W,
                    h: self.palette_h,
                },
                work,
            );
        }
        layout::palette_rect(self.bar_rect, anchor_x, PALETTE_W, self.palette_h, work)
    }

    /// The view's content-height report — the palette hugs its list:
    /// clamp to `[MIN, MAX]` (past MAX the list scrolls), then
    /// re-anchor through `palette_placement` so a shrink keeps the same
    /// composer-side gap. Position only while visible — a hidden window
    /// just stores the height for its next show.
    pub fn set_palette_height(&mut self, height: f64) {
        self.palette_h = height
            .clamp(PALETTE_MIN_H, PALETTE_MAX_H)
            .min(self.bar_work_area().h);
        let Some(win) = self.palette.clone() else {
            return;
        };
        if !win.is_visible().unwrap_or(false) {
            return;
        }
        let PaletteAnchor { x, top } = self.palette_anchor.unwrap_or(PaletteAnchor {
            x: self.bar_rect.center_x(),
            top: None,
        });
        let r = self.palette_placement(x, top);
        set_rect(&win, r);
    }

    /// The window's animated destination: the derived card rect while
    /// expanded, the pill rect otherwise. Every mutator operates on
    /// `bar_rect` (the canonical pill) and animates to THIS — an
    /// expanded card moves as a unit under snaps/recenters/reclamps.
    fn target_rect(&self) -> Rect {
        if self.chat_open {
            expanded_rect(
                self.bar_rect,
                self.expand_dir,
                self.chat_height,
                self.bar_work_area(),
            )
        } else {
            self.bar_rect
        }
    }

    /// Whether the card is currently expanded. ask's pre-flight reads
    /// this BEFORE `set_chat_open(true)` to tell a fresh pill send (new
    /// conversation) from a card follow-up.
    pub fn is_chat_open(&self) -> bool {
        self.chat_open
    }

    /// `window_set_chat_open` / tray Toggle / ask
    /// pre-flight: animate the bar
    /// window collapsed ⇄ expanded per §Expansion. Emits nothing — the
    /// webview learns the mode from `ask:*` or its own action (window
    /// height). OPEN is a no-op off-`Main` (mirrors panels not existing
    /// pre-`Main`); CLOSE always proceeds — `leave_main` collapses after
    /// the gate has already flipped.
    pub fn set_chat_open(&mut self, app: &AppHandle, open: bool) {
        if open == self.chat_open {
            return;
        }
        if open && !app.state::<crate::AppState>().gate_is_main() {
            log::warn!("windows::set_chat_open: open dropped while gate != Main");
            return;
        }
        let Some(bar) = self.bar.clone() else { return };
        // Refresh BEFORE the flag flips: collapsed it reads the pill
        // directly; expanded it derives the pill from the (possibly
        // dragged) card so collapse lands where the user left it.
        self.refresh_bar_rect();
        if open {
            let work = self.bar_work_area();
            self.expand_dir = expand_dir_for(self.bar_rect, work);
            // Every open starts at the card band's floor — 30% of the
            // work area — regardless of the last session's height; the
            // webview's first `window_adjust_height` report then grows
            // it to fit content (capped at 60% inside `expanded_rect`).
            self.chat_height = (work.h * layout::CARD_MIN_FRACTION - BAR_H).max(layout::MIN_CHAT_H);
            self.chat_open = true;
            let target = self.target_rect();
            movement::animate(&bar, target, ANIM_DUR);
            set_glass_radius(app, &bar, CARD_RADIUS);
        } else {
            self.chat_open = false;
            movement::animate(&bar, self.bar_rect, ANIM_DUR);
            set_glass_radius(app, &bar, BAR_H / 2.0);
        }
        self.sync_bar_size_limits();
    }

    /// Re-apply the bar's glass effect at its CURRENT radius — called
    /// after an `app.accent` config write so the tint follows the new
    /// accent without waiting for the next pill⇄card morph.
    pub fn refresh_bar_glass(&self, app: &AppHandle) {
        let Some(bar) = self.bar.clone() else { return };
        let radius = if self.chat_open {
            CARD_RADIUS
        } else {
            BAR_H / 2.0
        };
        set_glass_radius(app, &bar, radius);
    }

    /// `window_toggle_all`/tray Toggle: open ⇄ close the card. (The
    /// `toggle_input` hotkey is NOT here — it only morphs the bar's
    /// input pill, webview-side.)
    pub fn toggle_chat(&mut self, app: &AppHandle) {
        self.set_chat_open(app, !self.chat_open);
    }

    /// The work-area edge the bar's center is nearest to — what the
    /// Settings → Bar picker shows as the current position (the bar
    /// can rest anywhere after a drag, so "current" is always an edge
    /// read, not a stored field).
    pub fn bar_edge(&self) -> Dir {
        let work = self.bar_work_area();
        let (cx, cy) = (self.bar_rect.center_x(), self.bar_rect.center_y());
        let d_top = (cy - work.y).abs();
        let d_bottom = (work.bottom() - cy).abs();
        let d_left = (cx - work.x).abs();
        let d_right = (work.right() - cx).abs();
        let min = d_top.min(d_bottom).min(d_left).min(d_right);
        if min == d_bottom {
            Dir::Down
        } else if min == d_left {
            Dir::Left
        } else if min == d_right {
            Dir::Right
        } else {
            Dir::Up
        }
    }

    /// Settings → Bar picker: animate the bar to the named work-area edge
    /// (12 px margin); the open card follows as a unit. The move fires
    /// `Moved`, so the new position persists through the debounced write
    /// — no extra save here.
    pub fn snap_edge(&mut self, dir: Dir) {
        self.reclamp();
        let work = self.bar_work_area();
        let (x, y) = layout::snap_edge(self.bar_rect, dir, work);
        self.bar_rect = Rect {
            x,
            y,
            ..self.bar_rect
        };
        if let Some(bar) = &self.bar {
            movement::animate(bar, self.target_rect(), ANIM_DUR);
        }
    }

    /// Settings → Bar "Re-center": restore the startup default — the
    /// middle of the primary work area. Persists via the `Moved`
    /// debounce like any other move.
    pub fn recenter_bar(&mut self) {
        let work = self.primary_work_area();
        self.bar_rect = Rect {
            x: work.center_x() - BAR_IDLE_W / 2.0,
            y: work.center_y() - BAR_H / 2.0,
            w: BAR_IDLE_W,
            h: BAR_H,
        };
        self.sync_bar_size_limits();
        if let Some(bar) = &self.bar {
            movement::animate(bar, self.target_rect(), ANIM_DUR);
        }
    }

    /// `window_set_bar_expanded(bool)` — the webview reports its
    /// pill⇄input morph so the window can match it: under liquid glass
    /// the capsule IS the window, so idle rests at `BAR_IDLE_W` and any
    /// expanded content (input row, gate cards) needs `BAR_W`. Width
    /// changes recenter on center-x — the capsule blooms symmetrically —
    /// then re-clamp inside the work area.
    pub fn set_bar_expanded(&mut self, expanded: bool) {
        self.refresh_bar_rect();
        let w = if expanded { BAR_W } else { BAR_IDLE_W };
        if (self.bar_rect.w - w).abs() < f64::EPSILON {
            return;
        }
        self.bar_rect = clamp_to_work_area(
            Rect {
                x: self.bar_rect.center_x() - w / 2.0,
                w,
                ..self.bar_rect
            },
            self.bar_work_area(),
        );
        if let Some(bar) = &self.bar {
            movement::animate(bar, self.target_rect(), ANIM_DUR);
        }
        self.sync_bar_size_limits();
    }

    /// `window_adjust_height(px)`: `px` is the desired TOTAL window
    /// height (the frontend measures the whole card). Expanded-only —
    /// ignored when the card is closed. `expanded_rect` clamps to the
    /// card band — `[floor, min(free space, 60% of the work area)]` —
    /// keeping the anchored edge fixed; the clamped result is recorded
    /// for later expands.
    pub fn adjust_height(&mut self, px: f64) {
        if !px.is_finite() {
            log::warn!("windows::adjust_height: non-finite height {px}");
            return;
        }
        if !self.chat_open {
            // Expected race: `chat_open` flips before the collapse
            // animation finishes, and the webview's observers keep
            // reporting heights until the window shrinks past BAR_H.
            log::debug!("windows::adjust_height: ignored while the card is closed");
            return;
        }
        self.refresh_bar_rect();
        let target = expanded_rect(
            self.bar_rect,
            self.expand_dir,
            px - BAR_H,
            self.bar_work_area(),
        );
        self.chat_height = target.h - BAR_H;
        if let Some(bar) = &self.bar {
            movement::animate(bar, target, ANIM_DUR);
        }
    }

    /// Initial bar position: `config.window.bar_x/y` if BOTH are set, else
    /// the middle of the primary work area. Re-clamps to the primary
    /// monitor when the saved position's center is off-screen.
    pub fn position_bar_at_startup(&mut self) {
        let prefs = crate::config::load().window;
        let rect = match (prefs.bar_x, prefs.bar_y) {
            (Some(x), Some(y)) => Rect {
                x,
                y,
                w: BAR_IDLE_W,
                h: BAR_H,
            },
            _ => {
                let work = self.primary_work_area();
                Rect {
                    x: work.center_x() - BAR_IDLE_W / 2.0,
                    y: work.center_y() - BAR_H / 2.0,
                    w: BAR_IDLE_W,
                    h: BAR_H,
                }
            }
        };
        self.bar_rect = rect;
        if let Some(bar) = &self.bar {
            set_rect(bar, rect);
        }
        self.sync_bar_size_limits();
        self.reclamp();
    }

    /// Display-change recovery (no Tauri display event exists): if the bar's
    /// center is no longer inside ANY monitor's work area, re-clamp it into
    /// the primary monitor's. A periodic watch is a Task-18 manual item.
    fn reclamp(&mut self) {
        self.refresh_bar_rect();
        let Some(bar) = self.bar.clone() else { return };
        let (cx, cy) = (self.bar_rect.center_x(), self.bar_rect.center_y());
        let Ok(monitors) = bar.available_monitors() else {
            return;
        };
        let inside = monitors
            .iter()
            .any(|m| logical_work_area(m).contains_point(cx, cy));
        if inside {
            return;
        }
        let Some(primary) = bar
            .primary_monitor()
            .ok()
            .flatten()
            .or_else(|| monitors.into_iter().next())
        else {
            return;
        };
        let clamped = clamp_to_work_area(self.bar_rect, logical_work_area(&primary));
        if clamped != self.bar_rect {
            self.bar_rect = clamped;
            let target = self.target_rect();
            set_rect(&bar, target);
        }
    }

    /// Primary monitor's work area (first available as fallback).
    fn primary_work_area(&self) -> Rect {
        let Some(bar) = &self.bar else {
            return DEFAULT_WORK;
        };
        if let Some(p) = bar.primary_monitor().ok().flatten() {
            return logical_work_area(&p);
        }
        match bar.available_monitors() {
            Ok(ms) if !ms.is_empty() => logical_work_area(&ms[0]),
            _ => DEFAULT_WORK,
        }
    }

    /// Work area containing the bar's center; primary (or first) monitor if
    /// the bar is off-screen; `DEFAULT_WORK` if every query fails.
    fn bar_work_area(&self) -> Rect {
        let Some(bar) = &self.bar else {
            return DEFAULT_WORK;
        };
        let Ok(monitors) = bar.available_monitors() else {
            return DEFAULT_WORK;
        };
        if monitors.is_empty() {
            return DEFAULT_WORK;
        }
        let (cx, cy) = (self.bar_rect.center_x(), self.bar_rect.center_y());
        if let Some(m) = monitors
            .iter()
            .find(|m| logical_work_area(m).contains_point(cx, cy))
        {
            return logical_work_area(m);
        }
        match bar.primary_monitor().ok().flatten() {
            Some(p) => logical_work_area(&p),
            None => logical_work_area(&monitors[0]),
        }
    }

    /// Keep Tahoe's resize affordance honest — macOS 26 edge-drags
    /// borderless windows even with `resizable(false)`, and the ↔
    /// cursor still appears on the side edges. Pinning min == max on
    /// the width axis makes AppKit drop the horizontal affordance and
    /// clamps the drag itself (`enforce_bar_bounds` stays as the
    /// backstop). An open card keeps a free height axis inside the
    /// card band — `[30% of the work area (absolute floor
    /// BAR_H + MIN_CHAT_H), min(free space in the grow direction, 60%
    /// of the work area)]`, the same bounds `expanded_rect` applies —
    /// so an edge-drag can't leave the band and snap back on the next
    /// `window_adjust_height`. Programmatic
    /// `set_size` ignores these limits, so the morph animations are
    /// unaffected.
    fn sync_bar_size_limits(&self) {
        let Some(bar) = &self.bar else { return };
        // `bar_rect` still holds the DEFAULT_WORK sentinel until
        // `position_bar_at_startup` runs — a real bar is never wider
        // than EXPANDED_W.
        if self.bar_rect.w > layout::EXPANDED_W {
            return;
        }
        let (w, min_h, max_h) = if self.chat_open {
            let work = self.bar_work_area();
            let free = if self.expand_dir == Dir::Up {
                self.bar_rect.bottom() - work.y
            } else {
                work.bottom() - self.bar_rect.y
            };
            let min_h = (work.h * layout::CARD_MIN_FRACTION).max(BAR_H + layout::MIN_CHAT_H);
            let max_h = free.min(work.h * layout::CARD_MAX_FRACTION).max(min_h);
            (layout::EXPANDED_W, min_h, max_h)
        } else {
            (self.bar_rect.w, BAR_H, BAR_H)
        };
        let _ = bar.set_min_size(Some(LogicalSize::new(w, min_h)));
        let _ = bar.set_max_size(Some(LogicalSize::new(w, max_h)));
    }

    /// Snap the bar back to its canonical bounds after a stray user
    /// resize — macOS 26 edge-drags resize borderless windows even with
    /// `resizable(false)`. The pill is fixed-size and restores whole;
    /// the open card restores x/width and re-pins the anchored edge
    /// (an edge-drag always changes `w`, and a corner drag can move
    /// `y` too), keeping the dragged height — the webview's next
    /// report then settles it to content-fit inside the card band.
    /// Called on `Resized` when no animation is in flight.
    fn enforce_bar_bounds(&self) {
        let Some(bar) = &self.bar else { return };
        // `bar_rect` still holds the DEFAULT_WORK sentinel until
        // `position_bar_at_startup` runs — a real bar is never wider
        // than EXPANDED_W.
        if self.bar_rect.w > layout::EXPANDED_W {
            return;
        }
        let Some(live) = window_rect(bar) else { return };
        let off = |a: f64, b: f64| (a - b).abs() > 0.5;
        if self.chat_open {
            if !off(live.w, layout::EXPANDED_W) {
                return;
            }
            let target = self.target_rect();
            // Re-pin the anchored edge: grow-down keeps the pill's top,
            // grow-up keeps its bottom — otherwise a corner drag leaves
            // the card detached until the next `window_adjust_height`.
            let y = if self.expand_dir == Dir::Up {
                target.bottom() - live.h
            } else {
                target.y
            };
            set_rect(
                bar,
                Rect {
                    x: target.x,
                    y,
                    w: target.w,
                    ..live
                },
            );
        } else if off(live.w, self.bar_rect.w) || off(live.h, self.bar_rect.h) {
            set_rect(bar, self.bar_rect);
        }
    }

    /// Pull the live bar rect from the OS so user drags stay
    /// authoritative. While the card is open the live rect IS the card —
    /// derive the canonical pill back via `expand_dir` (keeping the
    /// stored pill width) so persistence/edge math never see card
    /// geometry.
    pub(crate) fn refresh_bar_rect(&mut self) {
        let Some(bar) = &self.bar else { return };
        // While an animation drives the bounds the live rect is a
        // transient lerp — and `window_rect` reads position and size as
        // two OS calls, so an animator tick landing between them pairs a
        // stale x with a fresh w. On expansion (x falls while w grows)
        // that tear is always right-biased, and ratcheting it into
        // `bar_rect` pushed the anchor — and every recentered target —
        // gradually right. The committed rect stays authoritative until
        // the animator lands on it.
        if movement::is_animating(bar.label()) {
            return;
        }
        let Some(r) = window_rect(bar) else { return };
        self.bar_rect = if self.chat_open {
            derive_pill_rect(r, self.bar_rect.w, BAR_H, self.expand_dir)
        } else {
            r
        };
    }

    /// `bar_edge` after `refresh_bar_rect` — the committed rect can
    /// trail the window's real `Moved` frame, so every edge read goes
    /// through this. Callers must not already hold `pool`.
    pub(crate) fn live_bar_edge(&mut self) -> Dir {
        self.refresh_bar_rect();
        self.bar_edge()
    }

    /// The resting capsule rect — what `persist_bar_position` writes.
    /// Expansion recenters on center-x, so the persisted anchor is always
    /// the idle capsule: a drag on the expanded bar must not shift where
    /// the capsule reappears next launch.
    pub(crate) fn idle_bar_rect(&mut self) -> Rect {
        self.refresh_bar_rect();
        Rect {
            x: self.bar_rect.x + (self.bar_rect.w - BAR_IDLE_W) / 2.0,
            w: BAR_IDLE_W,
            ..self.bar_rect
        }
    }
}

