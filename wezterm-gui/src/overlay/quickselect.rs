use config::keyassignment::{QuickSelectArguments, ScrollbackEraseMode};
use mux::domain::DomainId;
use mux::pane::{ForEachPaneLogicalLine, LogicalLine, Pane, PaneId, WithPaneLines};
use mux::renderable::*;
use parking_lot::{MappedMutexGuard, Mutex};
use rangeset::RangeSet;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use termwiz::cell::{Cell, CellAttributes};
use termwiz::color::AnsiColor;
use termwiz::surface::{SequenceNo, SEQ_ZERO};
use url::Url;
use wezterm_term::color::ColorPalette;
use wezterm_term::{
    Clipboard, KeyCode, KeyModifiers, Line, MouseEvent, StableRowIndex, TerminalSize,
};

#[derive(Debug)]
struct MatchResult {
    range: Range<usize>,
    label: String,
}

struct QuickSelectRenderable {
    delegate: Arc<dyn Pane>,
    /// The most recently queried set of matches
    by_line: HashMap<StableRowIndex, Vec<MatchResult>>,
    selection: String,

    viewport: Option<StableRowIndex>,
    last_bar_pos: Option<StableRowIndex>,

    dirty_results: RangeSet<StableRowIndex>,
    width: usize,
    height: usize,

    args: QuickSelectArguments,
}

impl QuickSelectRenderable {
    fn compute_search_row(&self) -> StableRowIndex {
        let dims = self.delegate.get_dimensions();
        let top = self.viewport.unwrap_or_else(|| dims.physical_top);
        let bottom = (top + dims.viewport_rows as StableRowIndex).saturating_sub(1);
        bottom
    }

    fn check_for_resize(&mut self) {
        let dims = self.delegate.get_dimensions();
        if dims.cols == self.width && dims.viewport_rows == self.height {
            return;
        }

        self.width = dims.cols;
        self.height = dims.viewport_rows;
    }
}
