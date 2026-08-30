//! Input handling — all `handle_*_key` methods for `App`.
//!
//! Each modal and view mode has its own handler, dispatched from the top-level
//! `handle_key`. Methods are `pub(super)` so only the `app` module sees them.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keys::{Action, Chord, Context, KeyMatch};
use crate::profiles;

use super::tasks::{bracketed_paste, complete_path, fetch_agents, fetch_search_index, seed_prompt};
use super::{
    App, AppMsg, ChatMode, NotesState, PaletteAction, PaletteItem, Picker, PickerItem, PickerKind,
    RenameForm, SearchState, SpawnForm, SplitDir, Tab, TabKind, View, Workspace,
};

/// Ex-command verbs surfaced in the Ctrl-K palette as a "command" source — the
/// fuzzy-findable mirror of the `:` command line. `(verb, description)`.
const COMMAND_PALETTE: &[(&str, &str)] = &[
    ("vsplit", "split window into columns"),
    ("split", "split window into rows"),
    ("only", "keep only the focused pane"),
    ("close", "close the focused pane"),
    ("spawn", "new agent"),
    ("term", "new terminal tab"),
    ("notes", "open the notes scratchpad"),
    ("nodes", "remote worker nodes (wake a sleeping machine)"),
    ("runs", "workflow runs, subagents + plan"),
    ("changes", "files the agent changed (docked pane)"),
    ("pin", "pin / unpin the agent (harpoon)"),
    ("search", "search transcripts across all agents"),
    ("model", "switch the model (live)"),
    ("permission", "cycle the permission mode"),
    ("handoff", "hand off to another provider"),
    ("rename", "rename the agent"),
    ("filter", "filter the sidebar"),
    ("dashboard", "go to the dashboard"),
    ("help", "keybindings"),
    ("quit", "quit"),
];

#[cfg(test)]
mod testutil;

mod dialogs;
mod dispatch;
mod nav;
mod panes;
mod pickers;
mod query;
mod questions;
