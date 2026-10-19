//! Tab state machine (`Loading`, `Locked`, `Ready`, `Error`) and viewport visibility math.

use std::path::PathBuf;
use zeroize::{Zeroize, Zeroizing};

/// Minimum allowed zoom level (25%).
pub const MIN_ZOOM: f32 = 0.25;
/// Maximum allowed zoom level (400%).
pub const MAX_ZOOM: f32 = 4.00;
/// Default zoom level (100%).
pub const DEFAULT_ZOOM: f32 = 1.00;
/// Zoom increment/decrement step for keyboard/toolbar buttons (25%).
pub const ZOOM_STEP: f32 = 0.25;
/// Zoom increment/decrement step for Ctrl + Mouse Wheel (5%).
pub const WHEEL_ZOOM_STEP: f32 = 0.05;
/// Vertical spacing between two consecutive pages in the scroll area (in UI points).
pub const PAGE_SPACING: f32 = 14.0;

/// Unique identifier for an open document tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

/// Strict state machine of a document tab.
#[derive(Debug, Clone, PartialEq)]
pub enum TabState {
    /// The file is currently being analyzed by the background worker.
    Loading,
    /// The PDF is encrypted and requires a user password.
    Locked {
        retry_count: u8,
        error_msg: Option<String>,
    },
    /// The document is open and ready (or unlocked successfully).
    Ready {
        page_count: usize,
        /// Pre-calculated `(width_pts, height_pts)` dimensions of each page at 1.0x zoom.
        page_dimensions: Vec<(f32, f32)>,
    },
    /// The file is corrupted or unreadable.
    Error(String),
}

/// Range of visible and pre-fetched pages for a given scroll viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisiblePagesRange {
    /// First page currently visible on screen (0-indexed).
    pub first_visible: usize,
    /// Last page currently visible on screen (0-indexed, inclusive).
    pub last_visible: usize,
    /// First page to request from the worker (includes 1 pre-loaded page above).
    pub request_start: usize,
    /// Last page to request from the worker (includes 1 pre-loaded page below, inclusive).
    pub request_end: usize,
    /// Primary current page (0-indexed) for the `Page X / Y` indicator.
    pub current_page: usize,
}

/// Represents an open PDF document tab in the application.
pub struct Tab {
    pub id: TabId,
    pub path: PathBuf,
    pub title: String,
    pub zoom: f32,
    pub scroll_offset: f32,
    pub current_page: usize,
    pub state: TabState,
    /// Temporary buffer for password input in the UI (immediately wiped via `zeroize`).
    pub password_input: String,
    /// Indicates whether the password text box should automatically receive focus.
    pub focus_password_field: bool,
    /// Indicates that an authentication request is currently being processed by the worker.
    pub is_authenticating: bool,
    /// Indicates whether the scroll offset should be restored on the next frame.
    pub pending_scroll_to_y: Option<f32>,
}

impl Tab {
    /// Creates a new tab in the initial `Loading` state.
    pub fn new(id: TabId, path: PathBuf) -> Self {
        let title = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| path.display().to_string());

        Self {
            id,
            path,
            title,
            zoom: DEFAULT_ZOOM,
            scroll_offset: 0.0,
            current_page: 0,
            state: TabState::Loading,
            password_input: String::new(),
            focus_password_field: false,
            is_authenticating: false,
            pending_scroll_to_y: None,
        }
    }

    /// Returns the quantized zoom factor as an integer percentage (e.g. `100` for `1.0`).
    pub fn zoom_pct(&self) -> u16 {
        (self.zoom * 100.0).round().clamp(25.0, 400.0) as u16
    }

    /// Updates the zoom level clamped between 25% and 400%.
    /// Returns `true` if the effective zoom percentage changed.
    pub fn set_zoom(&mut self, new_zoom: f32) -> bool {
        let old_pct = self.zoom_pct();
        let clamped = new_zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        // Round to the nearest percentage point to prevent floating-point drift.
        self.zoom = (clamped * 100.0).round() / 100.0;
        let new_pct = self.zoom_pct();
        if old_pct != new_pct && old_pct > 0 {
            let ratio = f32::from(new_pct) / f32::from(old_pct);
            self.scroll_offset = (self.scroll_offset * ratio).max(0.0);
            self.pending_scroll_to_y = Some(self.scroll_offset);
            true
        } else {
            false
        }
    }

    /// Zooms in by one standard step (`+25%`).
    pub fn zoom_in(&mut self) -> bool {
        let next = ((self.zoom / ZOOM_STEP).floor() + 1.0) * ZOOM_STEP;
        self.set_zoom(next)
    }

    /// Zooms out by one standard step (`-25%`).
    pub fn zoom_out(&mut self) -> bool {
        let next = ((self.zoom / ZOOM_STEP).ceil() - 1.0) * ZOOM_STEP;
        self.set_zoom(next)
    }

    /// Zooms in by one fine mouse-wheel notch (`+5%`).
    pub fn zoom_in_wheel(&mut self) -> bool {
        let next_pct = (self.zoom_pct().saturating_add(5)).min(400);
        let aligned_pct = (next_pct / 5) * 5;
        self.set_zoom(f32::from(aligned_pct) * WHEEL_ZOOM_STEP / 5.0)
    }

    /// Zooms out by one fine mouse-wheel notch (`-5%`).
    pub fn zoom_out_wheel(&mut self) -> bool {
        let next_pct = self.zoom_pct().saturating_sub(5).max(25);
        let aligned_pct = next_pct.div_ceil(5) * 5;
        self.set_zoom(f32::from(aligned_pct) * WHEEL_ZOOM_STEP / 5.0)
    }

    /// Resets the zoom level to 100%.
    pub fn reset_zoom(&mut self) -> bool {
        self.set_zoom(DEFAULT_ZOOM)
    }

    /// Transitions the tab to the `Locked` state (initial password prompt).
    pub fn transition_to_locked(&mut self) {
        self.password_input.zeroize();
        self.is_authenticating = false;
        self.focus_password_field = true;
        self.state = TabState::Locked {
            retry_count: 0,
            error_msg: None,
        };
    }

    /// Updates the `Locked` state after a failed password attempt.
    pub fn record_failed_auth(&mut self, message: impl Into<String>) {
        self.password_input.zeroize();
        self.is_authenticating = false;
        self.focus_password_field = true;
        let next_retry = match &self.state {
            TabState::Locked { retry_count, .. } => retry_count.saturating_add(1),
            _ => 1,
        };
        self.state = TabState::Locked {
            retry_count: next_retry,
            error_msg: Some(message.into()),
        };
    }

    /// Transitions the tab to the `Ready` state after successful opening or decryption.
    pub fn transition_to_ready(
        &mut self,
        page_count: usize,
        page_dimensions: Vec<(f32, f32)>,
    ) {
        self.password_input.zeroize();
        self.is_authenticating = false;
        self.focus_password_field = false;
        self.current_page = 0;
        self.state = TabState::Ready {
            page_count,
            page_dimensions,
        };
    }

    /// Transitions the tab to the `Error` state.
    pub fn transition_to_error(&mut self, error: impl Into<String>) {
        self.password_input.zeroize();
        self.is_authenticating = false;
        self.focus_password_field = false;
        self.state = TabState::Error(error.into());
    }

    /// Extracts the entered password into a self-wiping `Zeroizing<String>` container
    /// and immediately wipes the UI `password_input` buffer from RAM.
    pub fn take_password_zeroized(&mut self) -> Zeroizing<String> {
        let extracted = Zeroizing::new(self.password_input.clone());
        self.password_input.zeroize();
        self.is_authenticating = true;
        extracted
    }

    /// Computes which pages are visible inside `[viewport_min_y, viewport_max_y]`
    /// as well as the pre-loading window (`+1` page above and `+1` page below).
    pub fn compute_visible_pages(
        &self,
        viewport_min_y: f32,
        viewport_max_y: f32,
    ) -> Option<VisiblePagesRange> {
        let TabState::Ready {
            page_count,
            page_dimensions,
        } = &self.state
        else {
            return None;
        };

        if *page_count == 0 || page_dimensions.is_empty() {
            return None;
        }

        let viewport_center_y = (viewport_min_y + viewport_max_y) * 0.5;
        let mut y_cursor = 0.0_f32;
        let mut first_visible: Option<usize> = None;
        let mut last_visible: usize = 0;
        let mut current_page: usize = 0;
        let mut closest_center_dist = f32::MAX;

        for (idx, &(_w, h)) in page_dimensions.iter().enumerate() {
            let scaled_h = (h * self.zoom).max(1.0);
            let page_top = y_cursor;
            let page_bottom = page_top + scaled_h;
            let page_center = (page_top + page_bottom) * 0.5;

            let dist = (page_center - viewport_center_y).abs();
            if dist < closest_center_dist {
                closest_center_dist = dist;
                current_page = idx;
            }

            if page_bottom >= viewport_min_y && page_top <= viewport_max_y {
                if first_visible.is_none() {
                    first_visible = Some(idx);
                }
                last_visible = idx;
            }

            y_cursor = page_bottom + PAGE_SPACING;
        }

        let first = first_visible.unwrap_or(current_page);
        let last = if first_visible.is_some() {
            last_visible.max(first)
        } else {
            first
        };

        let request_start = first.saturating_sub(1);
        let request_end = (last + 1).min(page_count.saturating_sub(1));

        Some(VisiblePagesRange {
            first_visible: first,
            last_visible: last,
            request_start,
            request_end,
            current_page,
        })
    }
}

impl Drop for Tab {
    fn drop(&mut self) {
        self.password_input.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wheel_zoom_steps_by_5_percent() {
        let mut tab = Tab::new(TabId(1), PathBuf::from("sample.pdf"));
        assert_eq!(tab.zoom_pct(), 100);

        assert!(tab.zoom_in_wheel());
        assert_eq!(tab.zoom_pct(), 105);

        assert!(tab.zoom_in_wheel());
        assert_eq!(tab.zoom_pct(), 110);

        assert!(tab.zoom_out_wheel());
        assert_eq!(tab.zoom_pct(), 105);

        assert!(tab.zoom_out_wheel());
        assert_eq!(tab.zoom_pct(), 100);

        assert!(tab.zoom_out_wheel());
        assert_eq!(tab.zoom_pct(), 95);

        // Verify bounds clamping at 25% and 400%
        tab.set_zoom(MIN_ZOOM);
        assert_eq!(tab.zoom_pct(), 25);
        assert!(!tab.zoom_out_wheel());
        assert_eq!(tab.zoom_pct(), 25);

        tab.set_zoom(MAX_ZOOM);
        assert_eq!(tab.zoom_pct(), 400);
        assert!(!tab.zoom_in_wheel());
        assert_eq!(tab.zoom_pct(), 400);
    }
}
