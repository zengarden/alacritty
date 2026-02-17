# Alacritty Compact Tabs Development Plan

This file tracks the incremental PR plan for implementing internal compact tabs
in Alacritty while keeping the default behavior stable.

## Progress

- 2026-02-17: PR-0 completed.
  - Added `window.tabs.mode` (`Native|Compact`, default `Native`).
  - Added parser tests for tabs mode.
  - Documented tabs mode in `alacritty.5` man page.
- 2026-02-17: PR-1 completed.
  - Added internal `tabs` module with `TabId`, `TabContext`, `TabManager`.
  - Migrated session-scoped state from `WindowContext` into active `TabContext`.
  - Kept runtime behavior single-tab and unchanged.
  - Updated config-reload message bar access to go through `WindowContext` tab accessors.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-2 completed (keyboard flow, no tab bar UI yet).
  - Added compact-tab event path:
    `CreateTab`, `SelectNextInternalTab`, `SelectPreviousInternalTab`,
    `SelectInternalTab`, `SelectLastInternalTab`.
  - Added macOS action routing by `window.tabs.mode`:
    `Compact` now uses internal tab actions for `CreateNewTab` and tab switching actions.
    `Native` keeps platform tab behavior unchanged.
  - Added internal tab creation/switch logic in `WindowContext`:
    create tab session, activate tab, and trigger redraw/update on tab change.
  - Progress summary:
    internal tabs are now functional via keyboard in `Compact` mode, while
    visual compact tab UI is intentionally still absent at this stage.
    After `Cmd+T`, the new tab becomes active immediately; existing tabs are
    still reachable through tab-switch shortcuts.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-3 completed (event/timer routing isolation by tab).
  - Extended app event model with optional `tab_id`:
    `Event` now carries both `window_id` and `tab_id`.
  - Wired PTY-side `EventProxy` to emit events tagged with the originating tab.
  - Extended scheduler timer identity:
    `TimerId` now keys on `(topic, window_id, tab_id)`.
  - Routed per-tab timers/events with tab affinity:
    search delay, cursor blink, blink timeout, and selection-scrolling timers
    are now scheduled/unscheduled with active `tab_id`.
  - Added tab-aware filtering in `WindowContext::handle_event`:
    queued user events not targeting the active tab are dropped.
  - Added tab-aware fast-path guards in `Processor::user_event` for
    terminal `Wakeup`/`Exit` to prevent inactive-tab redraw churn and
    inactive-tab exit from tearing down the active window session.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-4 completed (compact tab bar rendering, read-only).
  - Reserved one top line in `Compact` mode during layout update.
  - Added compact tab bar render path in `Display`:
    top bar background + tab labels + active-tab visual highlight.
  - Added per-frame tab bar damage region in compact mode to keep updates stable.
  - Routed terminal rendering and terminal-space damage through a shifted
    terminal viewport (`terminal_size_info`) so content starts below the tab bar.
  - Aligned input/hit-testing coordinate space with the shifted terminal viewport
    through `ActionContext::size_info` and hint-mouse mapping updates.
  - Message bar and search bar positioning updated to remain at the bottom while
    preserving compact top bar.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-5 completed (mouse hit-testing and click switching).
  - Added compact tab-bar hit-testing path in `Display` (`compact_tab_at_position`).
  - Added input-level tab hit test hook and cursor-shape update:
    tab bar hover now shows pointer cursor in compact mode.
  - Added mouse click tab switching:
    left-click on compact tab labels switches the active internal tab.
  - Added `Action::Close` and remapped macOS `Cmd+W` from `Quit` to `Close`.
  - Added compact close path:
    `Cmd+W` now closes only the active internal tab in compact mode, and closes
    the window only when the last internal tab is closed.
  - Added per-tab timer cleanup on tab close via `Scheduler::unschedule_tab`.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-6 completed (tab title source and refresh policy).
  - Added per-tab title state in `tabs` module:
    `TabTitle { osc, fallback }` with `display()` priority (`OSC` > fallback).
  - Added cwd-based fallback title generation on tab creation:
    fallback uses cwd basename when available, otherwise `"Shell"`.
  - Compact tab bar now renders real tab titles instead of static `Tab N`.
  - Added duplicate-title disambiguation in compact tab labels
    (example: `foo (1)`, `foo (2)`).
  - Routed `TerminalEvent::Title/ResetTitle` in `Processor::user_event`
    through `WindowContext::set_tab_osc_title/reset_tab_osc_title`, so inactive
    tabs also keep title state in sync.
  - Active-tab changes and active-tab title updates now resync window title
    according to `preserve_title`/`dynamic_title` and tabs mode semantics.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-7 phase-1 completed (tab-exit lifecycle).
  - Added tab-manager removal by `TabId` (`contains/remove`) and normalized
    active-index adjustment when removing non-active tabs.
  - Added `WindowContext::handle_tab_exit(tab_id, scheduler)`:
    - inactive tab exit now removes only that tab,
    - active tab exit with remaining tabs switches to another tab,
    - last-tab exit returns a close-window signal.
  - Updated `Processor::user_event` `TerminalEvent::Exit` branch to delegate
    close behavior to tab lifecycle logic instead of active-tab-only filtering.
  - Preserved `hold` behavior: exit does not close tab/window when hold is enabled.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-6 follow-up fix (active tab title sync robustness).
  - Added `Term::title()` getter in `alacritty_terminal`.
  - Added draw-time sync in `WindowContext`:
    active tab title now reconciles from terminal state before compact tab-bar
    entries are built.
  - This provides a fallback path when `OSC` title updates are emitted but
    event-driven tab-title state is missed in edge flows.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-7 phase-2 completed (window/app close entry parity).
  - Added app-level close events:
    `EventType::CloseWindow` and `EventType::Quit`.
  - Added unified window close helpers in event processor:
    `close_window_context` / `close_all_windows`, including timer cleanup and
    final-window shutdown behavior.
  - Updated `WindowEvent::CloseRequested` to request `CloseWindow` instead of
    forcing active-tab `terminal.exit()`.
  - Updated input actions:
    - `Action::Close` now closes the entire window in non-compact paths, while
      compact mode still closes only active internal tab.
    - `Action::Quit` now performs application-level quit (`EventType::Quit`)
      instead of exiting only active tab.
  - `TerminalEvent::Exit` final window removal now reuses the unified close
    helper to keep cleanup semantics consistent.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-1 completed (render/update overhead trimming).
  - Tightened tab-title update path to avoid no-op redraws:
    `TabTitle::set_osc` / `reset_osc` now report whether state changed.
  - `WindowContext` now redraws compact tab bar on title events only when title
    state actually changed.
  - Replaced tab-bar full-frame damage with top-line-only damage in compact mode,
    reducing repaint scope when only tab labels/active highlight change.
  - Removed unnecessary `display.pending_update` scheduling on inactive-tab
    collection changes, avoiding extra layout/update work.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-2 completed (tab-bar entry caching and invalidation).
  - Added `WindowContext` tab-bar cache fields:
    `tab_bar_entries_cache` + `tab_bar_entries_dirty`.
  - Added explicit invalidation hooks for active-tab changes, tab collection
    changes, title changes, and tabs-mode config changes.
  - Replaced per-event/per-frame tab-bar entry rebuild with
    `refresh_tab_bar_entries_cache()` and reused cached entries in both draw and
    input event handling.
  - Limited draw-time active-title synchronization to `Compact` mode only.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-3 completed (macOS compact titlebar integration).
  - On macOS, when `window.tabs.mode = "Compact"` and
    `window.decorations = "Full"`, startup window attributes now auto-switch to
    `Transparent` titlebar mode (`title hidden + transparent titlebar +
    fullsize content view`).
  - This keeps explicit user decoration choices (`Transparent`, `Buttonless`,
    `None`) unchanged, while making default compact mode visually closer to
    integrated/compact tab UI.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-4 completed (compact tab visual refinement).
  - Added macOS left safe area reservation for traffic-light buttons in compact
    mode tab layout/hit-testing, preventing overlap with tab labels.
  - Reworked compact tab bar colors to derive from theme primary colors instead
    of footer bar inverted palette, so tab bar background matches theme
    background direction.
  - Reworked compact label style from bracketed (`[title]`) to padded compact
    segments (` title `), closer to iTerm compact appearance.
  - Added distinct active/inactive tab styling via separate foreground/background
    blends, improving active-tab recognizability.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-5 completed (iTerm-like active indicator refinement).
  - Compact tab bar background now uses theme primary background directly to
    eliminate residual mismatch with terminal theme backdrop.
  - Reworked active-tab styling from filled segment background to iTerm-like
    bottom indicator line + brighter active label foreground.
  - Inactive tabs keep flat background with muted foreground, reducing visual
    blockiness and improving vertical rhythm against macOS titlebar controls.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-6 completed (macOS vertical alignment tuning).
  - Added macOS compact titlebar geometry model:
    compact top bar now reserves native-like titlebar height (instead of fixed
    one terminal cell) when decorations are present.
  - Tab label baseline is vertically centered within the compact titlebar area,
    improving alignment against traffic-light controls.
  - Terminal viewport offset now uses the same compact titlebar height, fixing
    asymmetric top/bottom spacing under traffic lights.
  - Updated tab hit-testing to follow the shifted/centered label row.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-7 completed (native titlebar metrics + compact spacing polish).
  - Replaced macOS compact titlebar height hardcode with runtime native metric:
    `Window::titlebar_height()` now derives titlebar height from
    `NSWindow::frame - contentLayoutRect` and converts it to physical pixels.
  - Compact tab geometry now uses two levels:
    native visual titlebar height for label alignment, plus a small macOS-only
    bottom content gap so terminal content does not appear too close under
    traffic-light controls.
  - Expanded compact tab hit-testing vertical bounds to cover the full compact
    top bar region (not just the text row), making tab click behavior more
    consistent with the visual bar area.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-8 completed (compact vertical spacing convergence).
  - Reduced macOS compact content-gap from `4px` to `1px` to avoid oversized
    bottom spacing under the traffic-light/title row.
  - Removed macOS label-row upward nudge and restored strict geometric centering
    of tab labels inside the compact visual titlebar region, improving
    top/bottom balance.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-9 completed (bottom-edge equalization pass).
  - Reduced macOS compact content-gap from `1px` to `0px` to eliminate the
    remaining slight bottom-edge surplus under the compact tab label row.
  - Kept all other compact geometry unchanged, isolating this as a pure
    boundary-equalization micro-adjustment.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-10 completed (bottom-margin regression fix + titlebar micro-calibration).
  - Fixed a compact-mode geometry regression where terminal content could not
    reach the bottom edge: `terminal_size_info` now applies top inset by
    increasing both `padding_y` and `height`, avoiding accidental extra bottom
    margin from symmetric padding projection math.
  - Added a small macOS titlebar visual calibration (`-1px`) on top of native
    `contentLayoutRect` height to reduce residual bottom-heavy appearance near
    traffic-light controls.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-11 completed (terminal bottom reachability hard fix).
  - Reworked compact top reservation from fixed one-line logic to
    `ceil(tab_visual_height / cell_height)` line reservation, so terminal grid
    capacity is always consistent with compact top inset.
  - `terminal_size_info` now applies compact inset as `reserved_lines *
    cell_height` (line-aligned), eliminating bottom clipping/margin artifacts
    caused by mixed pixel-based inset with line-based grid sizing.
  - Updated draw/update paths to use the same dynamic compact reserved-line
    count, keeping search/message/tab offsets consistent.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-12 completed (render/input geometry split for bottom reachability).
  - Introduced `compact_tab_top_inset()` helper and split compact geometry into:
    input-hit-test `terminal_size_info` (top-shifted model) and
    renderer-facing `terminal_render_size_info` (bottom-anchored model).
  - Draw path now uses `terminal_render_size_info` so compact top inset no
    longer lifts the OpenGL viewport bottom edge, fixing persistent "cannot
    reach window bottom" behavior.
  - Kept update/highlight/input mapping on `terminal_size_info` to preserve
    existing top-bar mouse/selection hit behavior while rendering is corrected.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-13 completed (bottom anchoring with reserved-line aware render height).
  - `terminal_render_size_info` now computes compact render height from
    `2*padding_y + (screen_lines + bottom_reserved_lines) * cell_height`,
    instead of subtracting only tab inset pixels.
  - Draw path now computes `bottom_reserved_lines` from active search/message
    state and feeds it into `terminal_render_size_info`, so renderer geometry
    remains bottom-anchored while still leaving space for bottom bars.
  - This removes residual bottom unreachable space caused by cell remainder
    mismatch between reserved lines and viewport height.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-14 completed (compact vertical padding elimination).
  - Added `effective_window_padding()` and switched compact mode to force
    vertical padding to `0` for geometry/window-size calculations while keeping
    horizontal padding unchanged.
  - Disabled dynamic padding in compact mode to avoid reintroducing bottom
    margins from remainder distribution.
  - Applied this consistently in initial display size, update-time size
    recomputation, and `window_size()` bootstrap path.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-15 completed (terminal row-count/render-geometry sync hardening).
  - `Display::draw` now captures `terminal.screen_lines()` and feeds it into
    compact render-size computation, instead of assuming
    `display.size_info.screen_lines`.
  - `terminal_render_size_info` now explicitly syncs `SizeInfo.screen_lines`
    with the live terminal row count before deriving compact render height.
  - This removes potential persistent bottom-gap artifacts caused by transient
    divergence between display layout row count and terminal grid row count.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 debugging notes added (bottom-reachability process and pitfalls).
  - Iteration process (phase-10 ~ phase-15):
    started from spacing/padding tuning, then moved to compact inset line
    reservation, then split input/render geometry, and finally hardened with
    runtime terminal-row synchronization.
  - Root-cause cluster identified during the iterations:
    - renderer viewport/projection and terminal geometry could diverge in compact
      path if only viewport changed but text projection sizing was still derived
      from another `SizeInfo`,
    - UI and terminal rects shared one draw list in some paths, making geometry
      assumptions easy to accidentally mix,
    - relying only on `display.size_info.screen_lines` could lag behind live
      terminal grid row count in transitional frames.
  - Key pitfalls for future compact geometry work:
    - in text pass, `set_viewport` alone is insufficient when projection
      uniforms depend on size; use `renderer.resize(...)` on geometry changes,
    - avoid deriving bottom anchoring from mixed pixel/line heuristics; keep
      one authoritative row-count source for render math,
    - keep terminal-space and UI-space rect rendering separate when their
      coordinate/size models differ.
  - Effective final direction:
    compact render height and screen-lines now follow live terminal rows with
    bottom reserved bars included, while top compact bar remains separately
    managed.
- 2026-02-17: PR-8 phase-16 completed (compact title vertical micro-alignment).
  - Added macOS compact tab-title optical baseline shift (`+2px`) so tab labels
    sit lower and align closer to traffic-light controls.
  - Decoupled active-tab underline from text row position and anchored it to
    compact tab-bar bottom edge.
  - This prevents title baseline tuning from unintentionally moving the active
    underline away from the bar bottom.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-17 completed (title baseline retune + stable tab slot widths).
  - Retuned macOS compact title baseline shift from `+2px` to `+4px` for
    stronger visual alignment with traffic-light controls.
  - Reworked compact tab layout from title-length-driven variable widths to a
    slot-based fixed width layout (per frame), so tab segment width no longer
    changes with dynamic title length updates.
  - Applied the same slot layout model to mouse hit-testing, ensuring click
    behavior stays consistent with rendered geometry after width stabilization.
  - Active-tab underline remains anchored to tab-bar bottom and now spans the
    stable active slot width.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.
- 2026-02-17: PR-8 phase-18 completed (tab-title projection fix + horizontal centering).
  - Identified root cause for "title still sticks to top": tab title draw path
    adjusted `tab_size_info.padding_y` but did not refresh text projection with
    that size, so rendered glyphs still used the default top-aligned projection.
  - Fixed by applying `renderer.resize(&tab_size_info)` before compact tab
    title text draw, and restoring default projection after tab-bar draw.
  - Updated slot label composition to center tab titles horizontally within each
    stable slot (instead of left-leaning padding), while keeping fixed slot
    width and stable hit-testing behavior.
  - Retuned baseline shift from `+4px` to `+1px` after projection fix, since
    the previous larger shift was compensating for missing projection update.
  - Validation: `cargo check -p alacritty` and `cargo test -p alacritty` passed.

## Guardrails

1. Keep default behavior unchanged:
   `window.tabs.mode = "Native"` must preserve existing behavior.
2. Each PR must compile and be independently verifiable/revertible.
3. Performance budget:
   - Single-tab path should have near-zero regression.
   - Multi-tab overhead should scale with session count, without unnecessary redraws.

## PR Roadmap

### PR-0: Baseline and Config Guardrail
- Add config key `window.tabs.mode = "Native|Compact"` (default `Native`).
- Document the new option in man docs.
- No behavior change in runtime logic yet.

Verification:
- `cargo check -p alacritty`
- `cargo test -p alacritty`
- Manual smoke run confirms native path unchanged.

### PR-1: Internal Tab Data Model (No Behavior Change)
- Introduce `TabId`, `TabContext`, `TabManager`.
- Keep runtime behavior single-tab only for now.
- Move session-centric state from `WindowContext` into `TabContext`.

Verification:
- Build/tests pass.
- Runtime behavior unchanged vs PR-0.

### PR-2: Compact Keyboard Flow (No Tab Bar Yet)
- Under `Compact` mode:
  - `Cmd+T` creates internal tab.
  - `Cmd+1..9`, `Cmd+Tab`, `Cmd+Shift+Tab` switch internal tabs.
- `Native` mode keeps existing macOS tabbing behavior.

Verification:
- Shortcuts work in compact mode.
- Native mode remains unchanged.

### PR-3: Event and Timer Isolation by Tab
- Extend routing from `(window_id)` to `(window_id, tab_id)`.
- Extend scheduler timer identity with `tab_id`.
- Prevent background tabs from triggering foreground redraw churn.

Verification:
- Background tab output does not jitter active tab.
- Active-tab timer operations are isolated by `tab_id`.
- Tab-close timer/resource cleanup is now wired for the `Cmd+W` close path.

### PR-4: Compact Tab Bar Rendering (Read-Only)
- Reserve one top line for compact tabs.
- Draw tab titles with active highlight and truncation.
- Keep existing bottom search/message bar layout correct.

Verification:
- Layout stable under resize/font scale.
- Search/message bars unaffected.

### PR-5: Mouse Interaction and Hit Testing
- Add tab bar hover/click hit testing.
- Click switches active tab.
- Cursor shape updates correctly over tab bar.

Verification:
- Mouse switching works reliably.
- Text selection and scrolling behavior remain correct.

### PR-6: Tab Title Source and Refresh Policy
- Title priority:
  1) terminal title (`OSC`),
  2) fallback to shell/cwd.
- Keep titles distinguishable and updated promptly.

Verification:
- Different tabs show distinguishable names.
- Title updates propagate correctly.

### PR-7: Tab Close Lifecycle
- Finalize close lifecycle parity across all close entry points.
- Ensure tab/window close behavior remains consistent with compact mode semantics.
- Keep PTY/timer/event cleanup robust under repeated close/open cycles.

Verification:
- No crashes/leaks under repeated open/close.
- Window close behavior remains intuitive.

### PR-8: Performance and Memory Optimization
- Render active tab only.
- Reduce unnecessary damage/refresh for inactive tabs.

Verification:
- Single-tab performance close to PR-0 baseline.
- Multi-tab switching remains smooth.

### PR-9: Docs, Migration Notes, and Regression Coverage
- Update docs/man pages for compact mode behavior.
- Add targeted regression tests for config and tab workflows.

Verification:
- CI green with updated docs/tests.

### PR-10 (Optional): UX Enhancements
- Per-tab close button.
- Drag-to-reorder inside window.
- Optional pinning semantics.

Verification:
- Interaction consistency with no significant performance regressions.
