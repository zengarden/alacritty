//! Internal tab data model.

#[cfg(not(windows))]
use std::os::unix::io::RawFd;
use std::sync::Arc;

use alacritty_terminal::event_loop::Notifier;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;

use crate::cli::ParsedOptions;
use crate::event::{EventProxy, InlineSearchState, SearchState};
use crate::message_bar::MessageBuffer;

/// Internal tab identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(u64);

impl TabId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// User-visible tab title with OSC override and fallback name.
#[derive(Clone, Debug)]
pub struct TabTitle {
    osc: Option<String>,
    fallback: String,
}

impl TabTitle {
    pub fn new(fallback: String) -> Self {
        Self { osc: None, fallback }
    }

    pub fn display(&self) -> &str {
        self.osc.as_deref().unwrap_or(&self.fallback)
    }

    pub fn osc(&self) -> Option<&str> {
        self.osc.as_deref()
    }

    pub fn set_osc(&mut self, title: String) -> bool {
        let new_osc = if title.trim().is_empty() { None } else { Some(title) };
        if self.osc == new_osc {
            false
        } else {
            self.osc = new_osc;
            true
        }
    }

    pub fn reset_osc(&mut self) -> bool {
        if self.osc.is_none() {
            false
        } else {
            self.osc = None;
            true
        }
    }
}

/// Session state for a single terminal tab.
pub struct TabContext {
    pub id: TabId,
    pub title: TabTitle,
    pub message_buffer: MessageBuffer,
    pub terminal: Arc<FairMutex<Term<EventProxy>>>,
    pub inline_search_state: InlineSearchState,
    pub search_state: SearchState,
    pub notifier: Notifier,
    pub window_config: ParsedOptions,
    #[cfg(not(windows))]
    pub master_fd: RawFd,
    #[cfg(not(windows))]
    pub shell_pid: u32,
}

impl TabContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: TabId,
        fallback_title: String,
        terminal: Arc<FairMutex<Term<EventProxy>>>,
        notifier: Notifier,
        #[cfg(not(windows))] master_fd: RawFd,
        #[cfg(not(windows))] shell_pid: u32,
    ) -> Self {
        Self {
            id,
            title: TabTitle::new(fallback_title),
            message_buffer: Default::default(),
            terminal,
            inline_search_state: Default::default(),
            search_state: Default::default(),
            notifier,
            window_config: Default::default(),
            #[cfg(not(windows))]
            master_fd,
            #[cfg(not(windows))]
            shell_pid,
        }
    }
}

/// Tab collection and active-tab tracking for one window.
pub struct TabManager {
    tabs: Vec<TabContext>,
    active_tab: usize,
    next_id: u64,
}

impl TabManager {
    pub fn new(initial_tab: TabContext) -> Self {
        let next_id = initial_tab.id.get().saturating_add(1);
        Self { tabs: vec![initial_tab], active_tab: 0, next_id }
    }

    pub fn active(&self) -> &TabContext {
        &self.tabs[self.active_tab]
    }

    pub fn active_mut(&mut self) -> &mut TabContext {
        &mut self.tabs[self.active_tab]
    }

    pub fn iter(&self) -> impl Iterator<Item = &TabContext> {
        self.tabs.iter()
    }

    pub fn active_id(&self) -> TabId {
        self.tabs[self.active_tab].id
    }

    pub fn active_index(&self) -> usize {
        self.active_tab
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut TabContext> {
        self.tabs.iter_mut()
    }

    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut TabContext> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    pub fn allocate_id(&mut self) -> TabId {
        let id = TabId::new(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub fn push(&mut self, tab: TabContext) -> usize {
        self.tabs.push(tab);
        self.tabs.len() - 1
    }

    pub fn set_active(&mut self, index: usize) -> bool {
        if index < self.tabs.len() && self.active_tab != index {
            self.active_tab = index;
            true
        } else {
            false
        }
    }

    pub fn set_active_last(&mut self) -> bool {
        let index = self.tabs.len().saturating_sub(1);
        self.set_active(index)
    }

    pub fn set_active_next(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }

        let index = (self.active_tab + 1) % self.tabs.len();
        self.set_active(index)
    }

    pub fn set_active_previous(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }

        let index = if self.active_tab == 0 { self.tabs.len() - 1 } else { self.active_tab - 1 };
        self.set_active(index)
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn contains(&self, id: TabId) -> bool {
        self.tabs.iter().any(|tab| tab.id == id)
    }

    pub fn remove(&mut self, id: TabId) -> Option<TabContext> {
        let index = self.tabs.iter().position(|tab| tab.id == id)?;
        let removed = self.tabs.remove(index);

        if self.tabs.is_empty() {
            self.active_tab = 0;
        } else if self.active_tab > index {
            self.active_tab -= 1;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }

        Some(removed)
    }

    pub fn remove_active(&mut self) -> TabContext {
        let active_id = self.active_id();
        self.remove(active_id).expect("active tab must exist")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_id_roundtrip() {
        let id = TabId::new(7);
        assert_eq!(id.get(), 7);
    }
}
