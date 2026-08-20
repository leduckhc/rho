//! The transcript scroll state. Owned by `rho-tui`. See `SPEC-tui-alternate-screen`.
//!
//! The newest row sits at the bottom, directly above the composer. The offset counts
//! display rows from the oldest row. `total` and `visible` are display rows after
//! wrapping, never logical transcript rows.

/// Where the transcript view sits, and whether it follows new output.
///
/// There is no `Default` derive. A derived default gives `pinned: false`, which parks the
/// view at the oldest row and stops it following output. `TuiState` derives `Default`, so
/// an embedded `Scroll` would inherit that silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scroll {
    /// The first visible display row, counted from the oldest. It is advisory while the
    /// view is pinned, because `first_visible` then derives the position.
    offset: usize,
    /// True while the view follows the newest row.
    pinned: bool,
}

impl Default for Scroll {
    /// The default view follows output.
    fn default() -> Self {
        Self::pinned()
    }
}

/// The rows a wheel event moves. One, because trackpad momentum sends hundreds of events
/// for one gesture.
pub const WHEEL_ROWS: usize = 1;
/// The rows a page key leaves overlapping, so the reader keeps their place.
pub const PAGE_ROWS_MARGIN: usize = 2;

impl Scroll {
    /// A view that follows the newest row. This is the state at startup.
    pub fn pinned() -> Self {
        Self {
            offset: 0,
            pinned: true,
        }
    }

    /// The newest first-visible offset. `visible` is treated as at least one, so a
    /// zero-height view cannot run past the end.
    fn newest_offset(total: usize, visible: usize) -> usize {
        total.saturating_sub(visible.max(1))
    }

    /// The offset the view sits at now, before any move. A pinned view derives the newest
    /// position, so it never holds a stale offset.
    fn current_offset(&self, total: usize, visible: usize) -> usize {
        let newest = Self::newest_offset(total, visible);
        if self.pinned {
            newest
        } else {
            self.offset.min(newest)
        }
    }

    /// The first display row to draw.
    ///
    /// A pinned view derives the newest position here, so it can never hold a stale
    /// offset.
    pub fn first_visible(&self, total: usize, visible: usize) -> usize {
        self.current_offset(total, visible)
    }

    /// Move toward the oldest row, and release the pin.
    ///
    /// The offset is clamped here, where it changes. The pin is released only when rows
    /// exist above the view. When the whole transcript fits, the view keeps following.
    pub fn up(&mut self, rows: usize, total: usize, visible: usize) {
        let newest = Self::newest_offset(total, visible);
        self.offset = self.current_offset(total, visible).saturating_sub(rows);
        if newest > 0 {
            self.pinned = false;
        }
    }

    /// Move toward the newest row. Reaching the newest row restores the pin.
    pub fn down(&mut self, rows: usize, total: usize, visible: usize) {
        let newest = Self::newest_offset(total, visible);
        let moved = self.current_offset(total, visible).saturating_add(rows);
        self.offset = moved.min(newest);
        if self.offset >= newest {
            self.pinned = true;
        }
    }

    /// Jump to the oldest row, and release the pin.
    ///
    /// `to_oldest` and `to_newest` mutate in place. The names come from the reviewed
    /// contract in `SPEC-tui-alternate-screen`, so we keep them and silence the
    /// `to_*`-takes-self-by-value convention lint.
    #[allow(clippy::wrong_self_convention)]
    pub fn to_oldest(&mut self) {
        self.offset = 0;
        self.pinned = false;
    }

    /// Jump to the newest row, and restore the pin.
    #[allow(clippy::wrong_self_convention)]
    pub fn to_newest(&mut self, total: usize, visible: usize) {
        self.offset = Self::newest_offset(total, visible);
        self.pinned = true;
    }

    /// Answer new output. A pinned view needs no work, because `first_visible` derives the
    /// new newest row. An unpinned view is clamped and never moved.
    pub fn on_new_rows(&mut self, total: usize, visible: usize) {
        if !self.pinned {
            self.offset = self.offset.min(Self::newest_offset(total, visible));
        }
    }

    /// Answer a resize, because `visible` changed. Clamp the offset, and restore the pin
    /// when the view lands on the newest row.
    pub fn on_resize(&mut self, total: usize, visible: usize) {
        let newest = Self::newest_offset(total, visible);
        self.offset = self.offset.min(newest);
        if self.offset >= newest {
            self.pinned = true;
        }
    }

    /// True while the view follows the newest row.
    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    /// The display rows hidden above and below, for the rail and a position readout.
    pub fn hidden(&self, total: usize, visible: usize) -> (usize, usize) {
        let first = self.first_visible(total, visible);
        let above = first;
        let below = total.saturating_sub(first.saturating_add(visible.max(1)));
        (above, below)
    }
}
