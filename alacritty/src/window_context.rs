//! Terminal window context.

use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::mem;
#[cfg(not(windows))]
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use glutin::config::Config as GlutinConfig;
use glutin::display::GetGlDisplay;
#[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
use glutin::platform::x11::X11GlConfigExt;
use log::info;
use serde_json as json;
use winit::event::{Event as WinitEvent, Modifiers, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::WindowId;

use alacritty_terminal::event::Event as TerminalEvent;
use alacritty_terminal::event_loop::{EventLoop as PtyEventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::Direction;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{LineDamageBounds, Term, TermMode};
use alacritty_terminal::tty;

use crate::cli::{ParsedOptions, WindowOptions};
use crate::clipboard::Clipboard;
use crate::config::window::TabsMode;
use crate::config::UiConfig;
use crate::display::window::Window;
use crate::display::{Display, TabBarEntry};
use crate::event::{ActionContext, Event, EventProxy, Mouse, SearchState, TouchPurpose};
#[cfg(unix)]
use crate::logging::LOG_TARGET_IPC_CONFIG;
use crate::message_bar::MessageBuffer;
use crate::scheduler::Scheduler;
use crate::tabs::{TabContext, TabId, TabManager};
use crate::{input, renderer};

/// Event context for one individual Alacritty window.
pub struct WindowContext {
    pub display: Display,
    pub dirty: bool,
    event_queue: Vec<WinitEvent<Event>>,
    tabs: TabManager,
    tab_bar_entries_cache: Vec<TabBarEntry>,
    tab_bar_entries_dirty: bool,
    cursor_blink_timed_out: bool,
    prev_bell_cmd: Option<Instant>,
    modifiers: Modifiers,
    mouse: Mouse,
    touch: TouchPurpose,
    occluded: bool,
    preserve_title: bool,
    config: Rc<UiConfig>,
}

impl WindowContext {
    /// Create initial window context that does bootstrapping the graphics API we're going to use.
    pub fn initial(
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<Event>,
        config: Rc<UiConfig>,
        mut options: WindowOptions,
    ) -> Result<Self, Box<dyn Error>> {
        let raw_display_handle = event_loop.display_handle().unwrap().as_raw();

        let mut identity = config.window.identity.clone();
        options.window_identity.override_identity_config(&mut identity);

        // Windows has different order of GL platform initialization compared to any other platform;
        // it requires the window first.
        #[cfg(windows)]
        let window = Window::new(event_loop, &config, &identity, &mut options)?;
        #[cfg(windows)]
        let raw_window_handle = Some(window.raw_window_handle());

        #[cfg(not(windows))]
        let raw_window_handle = None;

        let gl_display = renderer::platform::create_gl_display(
            raw_display_handle,
            raw_window_handle,
            config.debug.prefer_egl,
        )?;
        let gl_config = renderer::platform::pick_gl_config(&gl_display, raw_window_handle)?;

        #[cfg(not(windows))]
        let window = Window::new(
            event_loop,
            &config,
            &identity,
            &mut options,
            #[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
            gl_config.x11_visual(),
        )?;

        // Create context.
        let gl_context =
            renderer::platform::create_gl_context(&gl_display, &gl_config, raw_window_handle)?;

        let display = Display::new(window, gl_context, &config, false)?;

        Self::new(display, config, options, proxy)
    }

    /// Create additional context with the graphics platform other windows are using.
    pub fn additional(
        gl_config: &GlutinConfig,
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<Event>,
        config: Rc<UiConfig>,
        mut options: WindowOptions,
        config_overrides: ParsedOptions,
    ) -> Result<Self, Box<dyn Error>> {
        let gl_display = gl_config.display();

        let mut identity = config.window.identity.clone();
        options.window_identity.override_identity_config(&mut identity);

        // Check if new window will be opened as a tab.
        // This must be done before `Window::new()`, which unsets `window_tabbing_id`.
        #[cfg(target_os = "macos")]
        let tabbed = options.window_tabbing_id.is_some();
        #[cfg(not(target_os = "macos"))]
        let tabbed = false;

        let window = Window::new(
            event_loop,
            &config,
            &identity,
            &mut options,
            #[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
            gl_config.x11_visual(),
        )?;

        // Create context.
        let raw_window_handle = window.raw_window_handle();
        let gl_context =
            renderer::platform::create_gl_context(&gl_display, gl_config, Some(raw_window_handle))?;

        let display = Display::new(window, gl_context, &config, tabbed)?;

        let mut window_context = Self::new(display, config, options, proxy)?;

        // Set the config overrides at startup.
        //
        // These are already applied to `config`, so no update is necessary.
        window_context.active_tab_mut().window_config = config_overrides;

        Ok(window_context)
    }

    /// Create a new terminal window context.
    fn new(
        display: Display,
        config: Rc<UiConfig>,
        options: WindowOptions,
        proxy: EventLoopProxy<Event>,
    ) -> Result<Self, Box<dyn Error>> {
        let mut pty_config = config.pty_config();
        options.terminal_options.override_pty_config(&mut pty_config);

        let preserve_title = options.window_identity.title.is_some();

        info!(
            "PTY dimensions: {:?} x {:?}",
            display.size_info.screen_lines(),
            display.size_info.columns()
        );

        let initial_tab_id = TabId::new(0);
        let event_proxy = EventProxy::new(proxy, display.window.id(), Some(initial_tab_id));

        // Create the terminal.
        //
        // This object contains all of the state about what's being displayed. It's
        // wrapped in a clonable mutex since both the I/O loop and display need to
        // access it.
        let terminal = Term::new(config.term_options(), &display.size_info, event_proxy.clone());
        let terminal = Arc::new(FairMutex::new(terminal));

        // Create the PTY.
        //
        // The PTY forks a process to run the shell on the slave side of the
        // pseudoterminal. A file descriptor for the master side is retained for
        // reading/writing to the shell.
        let pty = tty::new(&pty_config, display.size_info.into(), display.window.id().into())?;

        #[cfg(not(windows))]
        let master_fd = pty.file().as_raw_fd();
        #[cfg(not(windows))]
        let shell_pid = pty.child().id();

        // Create the pseudoterminal I/O loop.
        //
        // PTY I/O is ran on another thread as to not occupy cycles used by the
        // renderer and input processing. Note that access to the terminal state is
        // synchronized since the I/O loop updates the state, and the display
        // consumes it periodically.
        let event_loop = PtyEventLoop::new(
            Arc::clone(&terminal),
            event_proxy.clone(),
            pty,
            pty_config.drain_on_exit,
            config.debug.ref_test,
        )?;

        // The event loop channel allows write requests from the event processor
        // to be sent to the pty loop and ultimately written to the pty.
        let loop_tx = event_loop.channel();

        // Kick off the I/O thread.
        let _io_thread = event_loop.spawn();

        // Start cursor blinking, in case `Focused` isn't sent on startup.
        if config.cursor.style().blinking {
            event_proxy.send_event(TerminalEvent::CursorBlinkingChange.into());
        }

        let fallback_title = Self::fallback_tab_title(pty_config.working_directory.as_deref());
        let tab = TabContext::new(
            initial_tab_id,
            fallback_title,
            terminal,
            Notifier(loop_tx),
            #[cfg(not(windows))]
            master_fd,
            #[cfg(not(windows))]
            shell_pid,
        );

        // Create context for the Alacritty window.
        let mut context = WindowContext {
            preserve_title,
            display,
            tabs: TabManager::new(tab),
            tab_bar_entries_cache: Vec::new(),
            tab_bar_entries_dirty: true,
            config,
            cursor_blink_timed_out: Default::default(),
            prev_bell_cmd: Default::default(),
            event_queue: Default::default(),
            modifiers: Default::default(),
            occluded: Default::default(),
            mouse: Default::default(),
            touch: Default::default(),
            dirty: Default::default(),
        };
        context.refresh_window_title();

        Ok(context)
    }

    #[inline]
    fn active_tab(&self) -> &TabContext {
        self.tabs.active()
    }

    #[inline]
    fn active_tab_mut(&mut self) -> &mut TabContext {
        self.tabs.active_mut()
    }

    #[inline]
    pub fn active_tab_id(&self) -> TabId {
        self.tabs.active_id()
    }

    #[inline]
    pub fn is_active_tab(&self, tab_id: TabId) -> bool {
        self.active_tab_id() == tab_id
    }

    #[inline]
    pub fn message_buffer(&self) -> &MessageBuffer {
        &self.active_tab().message_buffer
    }

    #[inline]
    pub fn message_buffer_mut(&mut self) -> &mut MessageBuffer {
        &mut self.active_tab_mut().message_buffer
    }

    fn fallback_tab_title(working_directory: Option<&Path>) -> String {
        let Some(working_directory) = working_directory else {
            return String::from("Shell");
        };

        if let Some(name) = working_directory.file_name() {
            let name = name.to_string_lossy();
            if !name.is_empty() {
                return name.into_owned();
            }
        }

        let cwd = working_directory.to_string_lossy();
        if cwd.is_empty() {
            String::from("Shell")
        } else {
            cwd.into_owned()
        }
    }

    fn refresh_window_title(&mut self) {
        if self.preserve_title {
            return;
        }

        let title = if !self.config.window.dynamic_title {
            self.config.window.identity.title.clone()
        } else if matches!(self.config.window.tabs.mode, TabsMode::Compact) {
            self.active_tab().title.display().to_owned()
        } else {
            self.active_tab().title.osc().unwrap_or(&self.config.window.identity.title).to_owned()
        };

        self.display.window.set_title(title);
    }

    fn mark_tab_bar_dirty(&mut self) {
        if !matches!(self.config.window.tabs.mode, TabsMode::Compact) {
            return;
        }

        let columns = self.display.size_info.columns();
        if columns == 0 {
            return;
        }

        let damage = LineDamageBounds::new(0, 0, columns.saturating_sub(1));
        self.display.damage_tracker.frame().damage_line(damage);
        self.display.damage_tracker.next_frame().damage_line(damage);
        self.dirty = true;
        if self.display.window.has_frame {
            self.display.window.request_redraw();
        }
    }

    fn on_tab_title_change(&mut self, active_tab: bool) {
        self.invalidate_tab_bar_entries();
        if matches!(self.config.window.tabs.mode, TabsMode::Compact) {
            self.mark_tab_bar_dirty();
        }

        if active_tab {
            self.refresh_window_title();
        }
    }

    fn sync_active_tab_title_from_terminal(&mut self) {
        if !matches!(self.config.window.tabs.mode, TabsMode::Compact) {
            return;
        }

        let osc_title = {
            let tab = self.active_tab();
            tab.terminal.lock().title().map(ToOwned::to_owned)
        };

        let title_changed = {
            let tab = self.active_tab_mut();
            match osc_title {
                Some(osc_title) => tab.title.set_osc(osc_title),
                None => tab.title.reset_osc(),
            }
        };

        if title_changed {
            self.on_tab_title_change(true);
        }
    }

    pub fn set_tab_osc_title(&mut self, tab_id: Option<TabId>, title: String) {
        let tab_id = tab_id.unwrap_or(self.tabs.active_id());
        let is_active = tab_id == self.tabs.active_id();
        if let Some(tab) = self.tabs.tab_mut(tab_id) {
            if tab.title.set_osc(title) {
                self.on_tab_title_change(is_active);
            }
        }
    }

    pub fn reset_tab_osc_title(&mut self, tab_id: Option<TabId>) {
        let tab_id = tab_id.unwrap_or(self.tabs.active_id());
        let is_active = tab_id == self.tabs.active_id();
        if let Some(tab) = self.tabs.tab_mut(tab_id) {
            if tab.title.reset_osc() {
                self.on_tab_title_change(is_active);
            }
        }
    }

    fn invalidate_tab_bar_entries(&mut self) {
        self.tab_bar_entries_dirty = true;
    }

    fn compute_compact_tab_bar_entries(&self) -> Vec<TabBarEntry> {
        if !matches!(self.config.window.tabs.mode, TabsMode::Compact) {
            return Vec::new();
        }

        let mut title_counts = HashMap::<&str, usize>::new();
        for tab in self.tabs.iter() {
            *title_counts.entry(tab.title.display()).or_insert(0) += 1;
        }

        let mut duplicate_seen = HashMap::<&str, usize>::new();
        let active_index = self.tabs.active_index();
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let base_title = tab.title.display();
                let title = if title_counts.get(base_title).copied().unwrap_or(0) > 1 {
                    let count = duplicate_seen.entry(base_title).or_insert(0);
                    *count += 1;
                    format!("{base_title} ({count})")
                } else {
                    base_title.to_owned()
                };

                TabBarEntry { title, is_active: index == active_index }
            })
            .collect()
    }

    fn refresh_tab_bar_entries_cache(&mut self) {
        if !matches!(self.config.window.tabs.mode, TabsMode::Compact) {
            self.tab_bar_entries_cache.clear();
            self.tab_bar_entries_dirty = false;
            return;
        }

        if self.tab_bar_entries_dirty {
            self.tab_bar_entries_cache = self.compute_compact_tab_bar_entries();
            self.tab_bar_entries_dirty = false;
        }
    }

    /// Update the terminal window to the latest config.
    pub fn update_config(&mut self, new_config: Rc<UiConfig>) {
        let old_config = mem::replace(&mut self.config, new_config);
        let tabs_mode_changed = old_config.window.tabs.mode != self.config.window.tabs.mode;

        // Apply ipc config if there are overrides.
        let config = self.config.clone();
        self.config = self.active_tab_mut().window_config.override_config_rc(config);

        self.display.update_config(&self.config);
        for tab in self.tabs.iter_mut() {
            tab.terminal.lock().set_options(self.config.term_options());
        }

        // Reload cursor if its thickness has changed.
        if (old_config.cursor.thickness() - self.config.cursor.thickness()).abs() > f32::EPSILON {
            self.display.pending_update.set_cursor_dirty();
        }

        if old_config.font != self.config.font {
            let scale_factor = self.display.window.scale_factor as f32;
            // Do not update font size if it has been changed at runtime.
            if self.display.font_size == old_config.font.size().scale(scale_factor) {
                self.display.font_size = self.config.font.size().scale(scale_factor);
            }

            let font = self.config.font.clone().with_size(self.display.font_size);
            self.display.pending_update.set_font(font);
        }

        // Always reload the theme to account for auto-theme switching.
        self.display.window.set_theme(self.config.window.theme());

        // Update display if either padding options or resize increments were changed.
        let window_config = &old_config.window;
        if window_config.padding(1.) != self.config.window.padding(1.)
            || window_config.dynamic_padding != self.config.window.dynamic_padding
            || window_config.resize_increments != self.config.window.resize_increments
            || tabs_mode_changed
        {
            self.display.pending_update.dirty = true;
        }

        if tabs_mode_changed {
            self.invalidate_tab_bar_entries();
        }

        // Update title on config reload according to the following table.
        //
        // │cli │ dynamic_title │ current_title == old_config ││ set_title │
        // │ Y  │       _       │              _              ││     N     │
        // │ N  │       Y       │              Y              ││     Y     │
        // │ N  │       Y       │              N              ││     N     │
        // │ N  │       N       │              _              ││     Y     │
        if !self.preserve_title
            && (!self.config.window.dynamic_title
                || self.display.window.title() == old_config.window.identity.title)
        {
            self.refresh_window_title();
        }

        let opaque = self.config.window_opacity() >= 1.;

        // Disable shadows for transparent windows on macOS.
        #[cfg(target_os = "macos")]
        self.display.window.set_has_shadow(opaque);

        #[cfg(target_os = "macos")]
        self.display.window.set_option_as_alt(self.config.window.option_as_alt());

        // Change opacity and blur state.
        self.display.window.set_transparent(!opaque);
        self.display.window.set_blur(self.config.window.blur);

        // Update hint keys.
        self.display.hint_state.update_alphabet(self.config.hints.alphabet());

        // Update cursor blinking.
        let event = Event::new(TerminalEvent::CursorBlinkingChange.into(), None);
        self.event_queue.push(event.into());

        self.dirty = true;
    }

    /// Get reference to the window's configuration.
    #[cfg(unix)]
    pub fn config(&self) -> &UiConfig {
        &self.config
    }

    /// Clear the window config overrides.
    #[cfg(unix)]
    pub fn reset_window_config(&mut self, config: Rc<UiConfig>) {
        // Clear previous window errors.
        self.message_buffer_mut().remove_target(LOG_TARGET_IPC_CONFIG);

        self.active_tab_mut().window_config.clear();

        // Reload current config to pull new IPC config.
        self.update_config(config);
    }

    /// Add new window config overrides.
    #[cfg(unix)]
    pub fn add_window_config(&mut self, config: Rc<UiConfig>, options: &ParsedOptions) {
        // Clear previous window errors.
        self.message_buffer_mut().remove_target(LOG_TARGET_IPC_CONFIG);

        self.active_tab_mut().window_config.extend_from_slice(options);

        // Reload current config to pull new IPC config.
        self.update_config(config);
    }

    /// Create a new internal tab in this window.
    pub fn create_tab(
        &mut self,
        proxy: EventLoopProxy<Event>,
        options: WindowOptions,
    ) -> Result<(), Box<dyn Error>> {
        let tab_id = self.tabs.allocate_id();

        let mut pty_config = self.config.pty_config();
        options.terminal_options.override_pty_config(&mut pty_config);

        let event_proxy = EventProxy::new(proxy, self.display.window.id(), Some(tab_id));

        let terminal =
            Term::new(self.config.term_options(), &self.display.size_info, event_proxy.clone());
        let terminal = Arc::new(FairMutex::new(terminal));

        // Preserve focus state when creating the tab.
        let focused = self.active_tab().terminal.lock().is_focused;
        terminal.lock().is_focused = focused;

        let pty =
            tty::new(&pty_config, self.display.size_info.into(), self.display.window.id().into())?;

        #[cfg(not(windows))]
        let master_fd = pty.file().as_raw_fd();
        #[cfg(not(windows))]
        let shell_pid = pty.child().id();

        let event_loop = PtyEventLoop::new(
            Arc::clone(&terminal),
            event_proxy,
            pty,
            pty_config.drain_on_exit,
            self.config.debug.ref_test,
        )?;

        let loop_tx = event_loop.channel();
        let _io_thread = event_loop.spawn();

        let fallback_title = Self::fallback_tab_title(pty_config.working_directory.as_deref());
        let tab = TabContext::new(
            tab_id,
            fallback_title,
            terminal,
            Notifier(loop_tx),
            #[cfg(not(windows))]
            master_fd,
            #[cfg(not(windows))]
            shell_pid,
        );
        let index = self.tabs.push(tab);
        let _ = self.tabs.set_active(index);

        self.on_active_tab_change();
        Ok(())
    }

    /// Select the next internal tab.
    pub fn select_next_tab(&mut self) {
        if self.tabs.set_active_next() {
            self.on_active_tab_change();
        }
    }

    /// Select the previous internal tab.
    pub fn select_previous_tab(&mut self) {
        if self.tabs.set_active_previous() {
            self.on_active_tab_change();
        }
    }

    /// Select an internal tab by zero-based index.
    pub fn select_tab_at_index(&mut self, index: usize) {
        if self.tabs.set_active(index) {
            self.on_active_tab_change();
        }
    }

    /// Select the last internal tab.
    pub fn select_last_tab(&mut self) {
        if self.tabs.set_active_last() {
            self.on_active_tab_change();
        }
    }

    /// Reorder internal tabs by moving one tab index to another.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if self.tabs.move_tab(from, to) {
            self.on_tab_collection_change();
        }
    }

    /// Close the active internal tab.
    pub fn close_active_tab(&mut self, scheduler: &mut Scheduler) {
        if self.tabs.len() <= 1 {
            self.display.window.hold = false;
            self.active_tab_mut().terminal.lock().exit();
            return;
        }

        let tab = self.tabs.remove_active();
        scheduler.unschedule_tab(self.id(), tab.id);
        let _ = tab.notifier.0.send(Msg::Shutdown);
        self.on_active_tab_change();
    }

    /// Handle terminal process exit for a specific tab.
    ///
    /// Returns `true` when the entire window should be closed.
    pub fn handle_tab_exit(&mut self, tab_id: Option<TabId>, scheduler: &mut Scheduler) -> bool {
        let tab_id = tab_id.unwrap_or(self.tabs.active_id());
        if !self.tabs.contains(tab_id) {
            return false;
        }

        if self.display.window.hold {
            return false;
        }

        if self.tabs.len() <= 1 {
            return true;
        }

        let active_id = self.tabs.active_id();
        if let Some(tab) = self.tabs.remove(tab_id) {
            scheduler.unschedule_tab(self.id(), tab.id);
            let _ = tab.notifier.0.send(Msg::Shutdown);
        }

        if tab_id == active_id {
            self.on_active_tab_change();
        } else {
            self.on_tab_collection_change();
        }

        false
    }

    fn on_tab_collection_change(&mut self) {
        self.invalidate_tab_bar_entries();
        self.mark_tab_bar_dirty();
    }

    fn on_active_tab_change(&mut self) {
        self.invalidate_tab_bar_entries();
        self.display.pending_update.dirty = true;
        self.mark_tab_bar_dirty();
        self.refresh_window_title();

        // Recompute blinking state for the newly active tab.
        let event = Event::new(TerminalEvent::CursorBlinkingChange.into(), None);
        self.event_queue.push(event.into());
    }

    /// Draw the window.
    pub fn draw(&mut self, scheduler: &mut Scheduler) {
        self.sync_active_tab_title_from_terminal();
        self.refresh_tab_bar_entries_cache();

        let WindowContext { display, dirty, occluded, tabs, tab_bar_entries_cache, config, .. } =
            self;
        let tab_bar_entries = tab_bar_entries_cache.as_slice();

        display.window.requested_redraw = false;

        if *occluded {
            return;
        }

        *dirty = false;

        // Force the display to process any pending display update.
        display.process_renderer_update();

        // Request immediate re-draw if visual bell animation is not finished yet.
        let compact_tab_animation_active = display.compact_tab_animation_active();
        if !display.visual_bell.completed() || compact_tab_animation_active {
            // We can get an OS redraw which bypasses alacritty's frame throttling, thus
            // marking the window as dirty when we don't have frame yet.
            if display.window.has_frame {
                display.window.request_redraw();
            } else {
                *dirty = true;
            }
        }

        // Redraw the window.
        let tab = tabs.active_mut();
        let terminal = tab.terminal.lock();
        display.draw(
            terminal,
            scheduler,
            &tab.message_buffer,
            tab_bar_entries,
            config.as_ref(),
            &mut tab.search_state,
        );
    }

    /// Process events for this terminal window.
    pub fn handle_event(
        &mut self,
        #[cfg(target_os = "macos")] event_loop: &ActiveEventLoop,
        event_proxy: &EventLoopProxy<Event>,
        clipboard: &mut Clipboard,
        scheduler: &mut Scheduler,
        event: WinitEvent<Event>,
    ) {
        if !Self::event_targets_tab(&event, self.active_tab_id()) {
            return;
        }

        let should_process_immediately = self.should_process_immediately(&event);
        let is_redraw_requested =
            matches!(event, WinitEvent::WindowEvent { event: WindowEvent::RedrawRequested, .. });

        match event {
            WinitEvent::AboutToWait
            | WinitEvent::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                // Skip further event handling with no staged updates.
                if self.event_queue.is_empty() {
                    return;
                }

                // Continue to process all pending events.
            },
            event => {
                self.event_queue.push(event);
                if !should_process_immediately {
                    return;
                }
            },
        }

        self.refresh_tab_bar_entries_cache();

        let WindowContext {
            display,
            dirty,
            event_queue,
            tabs,
            tab_bar_entries_cache,
            cursor_blink_timed_out,
            prev_bell_cmd,
            modifiers,
            mouse,
            touch,
            occluded,
            preserve_title,
            config,
            ..
        } = self;
        let tab_bar_entries = tab_bar_entries_cache.as_slice();

        let active_tab_id = tabs.active_id();
        let tab = tabs.active_mut();
        let mut terminal = tab.terminal.lock();

        let old_is_searching = tab.search_state.history_index.is_some();

        let context = ActionContext {
            tab_id: active_tab_id,
            cursor_blink_timed_out,
            prev_bell_cmd,
            message_buffer: &mut tab.message_buffer,
            inline_search_state: &mut tab.inline_search_state,
            search_state: &mut tab.search_state,
            tab_bar_entries,
            modifiers,
            notifier: &mut tab.notifier,
            display,
            mouse,
            touch,
            dirty,
            occluded,
            terminal: &mut terminal,
            #[cfg(not(windows))]
            master_fd: tab.master_fd,
            #[cfg(not(windows))]
            shell_pid: tab.shell_pid,
            preserve_title: *preserve_title,
            config: config.as_ref(),
            event_proxy,
            #[cfg(target_os = "macos")]
            event_loop,
            clipboard,
            scheduler,
        };
        let mut processor = input::Processor::new(context);

        for event in event_queue.drain(..) {
            if Self::event_targets_tab(&event, active_tab_id) {
                processor.handle_event(event);
            }
        }

        // Process DisplayUpdate events.
        if display.pending_update.dirty {
            Self::submit_display_update(
                &mut terminal,
                display,
                &mut tab.notifier,
                &tab.message_buffer,
                &mut tab.search_state,
                old_is_searching,
                config.as_ref(),
            );
            *dirty = true;
        }

        if *dirty || mouse.hint_highlight_dirty {
            *dirty |= display.update_highlighted_hints(
                &terminal,
                config.as_ref(),
                mouse,
                modifiers.state(),
            );
            mouse.hint_highlight_dirty = false;
        }

        // Don't call `request_redraw` when event is `RedrawRequested` since the `dirty` flag
        // represents the current frame, but redraw is for the next frame.
        if *dirty && display.window.has_frame && !*occluded && !is_redraw_requested {
            display.window.request_redraw();
        }
    }

    /// ID of this terminal context.
    pub fn id(&self) -> WindowId {
        self.display.window.id()
    }

    /// Write the ref test results to the disk.
    pub fn write_ref_test_results(&self) {
        // Dump grid state.
        let mut grid = self.active_tab().terminal.lock().grid().clone();
        grid.initialize_all();
        grid.truncate();

        let serialized_grid = json::to_string(&grid).expect("serialize grid");

        let size_info = &self.display.size_info;
        let size = TermSize::new(size_info.columns(), size_info.screen_lines());
        let serialized_size = json::to_string(&size).expect("serialize size");

        let serialized_config = format!("{{\"history_size\":{}}}", grid.history_size());

        File::create("./grid.json")
            .and_then(|mut f| f.write_all(serialized_grid.as_bytes()))
            .expect("write grid.json");

        File::create("./size.json")
            .and_then(|mut f| f.write_all(serialized_size.as_bytes()))
            .expect("write size.json");

        File::create("./config.json")
            .and_then(|mut f| f.write_all(serialized_config.as_bytes()))
            .expect("write config.json");
    }

    /// Submit the pending changes to the `Display`.
    fn submit_display_update(
        terminal: &mut Term<EventProxy>,
        display: &mut Display,
        notifier: &mut Notifier,
        message_buffer: &MessageBuffer,
        search_state: &mut SearchState,
        old_is_searching: bool,
        config: &UiConfig,
    ) {
        // Compute cursor positions before resize.
        let num_lines = terminal.screen_lines();
        let cursor_at_bottom = terminal.grid().cursor.point.line + 1 == num_lines;
        let origin_at_bottom = if terminal.mode().contains(TermMode::VI) {
            terminal.vi_mode_cursor.point.line == num_lines - 1
        } else {
            search_state.direction == Direction::Left
        };

        display.handle_update(terminal, notifier, message_buffer, search_state, config);

        let new_is_searching = search_state.history_index.is_some();
        if !old_is_searching && new_is_searching {
            // Scroll on search start to make sure origin is visible with minimal viewport motion.
            let display_offset = terminal.grid().display_offset();
            if display_offset == 0 && cursor_at_bottom && !origin_at_bottom {
                terminal.scroll_display(Scroll::Delta(1));
            } else if display_offset != 0 && origin_at_bottom {
                terminal.scroll_display(Scroll::Delta(-1));
            }
        }
    }

    #[inline]
    fn event_targets_tab(event: &WinitEvent<Event>, tab_id: TabId) -> bool {
        match event {
            WinitEvent::UserEvent(event) => {
                event.tab_id().is_none_or(|event_tab_id| event_tab_id == tab_id)
            },
            _ => true,
        }
    }

    /// Decide whether an input event should bypass batching.
    ///
    /// Compact-tab interactions need low latency for press-to-select and drag behavior.
    fn should_process_immediately(&mut self, event: &WinitEvent<Event>) -> bool {
        if !matches!(self.config.window.tabs.mode, TabsMode::Compact) {
            return false;
        }

        match event {
            WinitEvent::WindowEvent {
                event: WindowEvent::MouseInput { button: MouseButton::Left, .. },
                ..
            } => {
                self.refresh_tab_bar_entries_cache();
                let mouse = &self.mouse;
                self.display
                    .compact_tab_at_position(
                        self.config.as_ref(),
                        self.tab_bar_entries_cache.as_slice(),
                        mouse.x,
                        mouse.y,
                    )
                    .is_some()
                    || mouse.tab_drag_source.is_some()
            },
            WinitEvent::WindowEvent { event: WindowEvent::CursorMoved { .. }, .. } => {
                self.mouse.tab_drag_source.is_some()
            },
            _ => false,
        }
    }
}

impl Drop for WindowContext {
    fn drop(&mut self) {
        for tab in self.tabs.iter_mut() {
            // Shutdown the terminal's PTY.
            let _ = tab.notifier.0.send(Msg::Shutdown);
        }
    }
}
