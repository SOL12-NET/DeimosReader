//! Bounded LRU cache of GPU page textures (`egui::TextureHandle`), limited to at most 15 pages per tab.

use std::collections::HashMap;
use std::num::NonZeroUsize;

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use lru::LruCache;

use crate::tab::TabId;

/// Strict maximum capacity of the per-tab LRU texture cache (maximum 15 pages per tab).
pub const MAX_CACHED_PAGES_PER_TAB: usize = 15;

/// Key identifying a rendered page texture within a tab at a specific zoom percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageTextureKey {
    pub page_index: usize,
    pub zoom_pct: u16,
}

/// Per-tab LRU cache manager for GPU textures (`TabId`).
///
/// Whenever a 16th page is inserted for a tab, or whenever a tab is closed,
/// the evicted `egui::TextureHandle` is immediately dropped (`Drop`), instructing
/// `eframe`/`egui` to free the corresponding GPU texture (VRAM) and RAM buffer.
pub struct TextureCache {
    tabs: HashMap<TabId, LruCache<PageTextureKey, TextureHandle>>,
    max_per_tab: NonZeroUsize,
}

impl Default for TextureCache {
    fn default() -> Self {
        Self::new(MAX_CACHED_PAGES_PER_TAB)
    }
}

impl TextureCache {
    /// Creates a new texture cache bounded to `capacity_per_tab` textures per tab.
    pub fn new(capacity_per_tab: usize) -> Self {
        let max_per_tab = NonZeroUsize::new(capacity_per_tab.max(1)).expect("capacity >= 1");
        Self {
            tabs: HashMap::new(),
            max_per_tab,
        }
    }

    /// Retrieves a texture handle while updating its Least-Recently-Used (LRU) position.
    pub fn get(&mut self, tab_id: TabId, page_index: usize, zoom_pct: u16) -> Option<&TextureHandle> {
        let tab_cache = self.tabs.get_mut(&tab_id)?;
        tab_cache.get(&PageTextureKey {
            page_index,
            zoom_pct,
        })
    }

    /// Checks whether a page texture is cached without updating LRU order.
    pub fn contains(&self, tab_id: TabId, page_index: usize, zoom_pct: u16) -> bool {
        self.tabs.get(&tab_id).is_some_and(|cache| {
            cache.contains(&PageTextureKey {
                page_index,
                zoom_pct,
            })
        })
    }

    /// Uploads an RGBA8 pixel buffer into a GPU `egui::TextureHandle` and inserts it into the tab's LRU cache.
    /// If the tab exceeds 15 cached pages, the least-recently-used texture is evicted and freed immediately.
    pub fn insert_rgba(
        &mut self,
        ctx: &egui::Context,
        tab_id: TabId,
        key: PageTextureKey,
        size: [usize; 2],
        pixels: Vec<u8>,
    ) {
        let [width, height] = size;
        if width == 0
            || height == 0
            || pixels.len() != width.saturating_mul(height).saturating_mul(4)
        {
            return;
        }

        let image = ColorImage::from_rgba_unmultiplied([width, height], &pixels);
        // Free the temporary RAM pixel buffer immediately after constructing the image.
        drop(pixels);

        let texture_name = format!(
            "deimos_tab_{}_p_{}_z_{}",
            tab_id.0, key.page_index, key.zoom_pct
        );
        let handle = ctx.load_texture(texture_name, image, TextureOptions::LINEAR);

        let max_cap = self.max_per_tab;
        let tab_cache = self
            .tabs
            .entry(tab_id)
            .or_insert_with(|| LruCache::new(max_cap));

        tab_cache.put(key, handle);
    }

    /// Evicts all cached textures for a tab whose zoom level differs from `active_zoom_pct`.
    pub fn evict_stale_zoom(&mut self, tab_id: TabId, active_zoom_pct: u16) {
        if let Some(tab_cache) = self.tabs.get_mut(&tab_id) {
            let stale_keys: Vec<PageTextureKey> = tab_cache
                .iter()
                .filter_map(|(&k, _)| (k.zoom_pct != active_zoom_pct).then_some(k))
                .collect();

            for key in stale_keys {
                tab_cache.pop(&key);
            }
        }
    }

    /// Immediately frees all GPU textures associated with a closed tab.
    pub fn remove_tab(&mut self, tab_id: TabId) {
        self.tabs.remove(&tab_id);
    }

    /// Returns the number of cached pages for a given tab.
    #[cfg(test)]
    pub fn tab_cached_count(&self, tab_id: TabId) -> usize {
        self.tabs.get(&tab_id).map_or(0, LruCache::len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lru_cache_eviction_at_15_pages_and_tab_removal() {
        let ctx = egui::Context::default();
        let mut cache = TextureCache::default();
        let tab1 = TabId(1);
        let tab2 = TabId(2);

        // Insert 20 pages into tab 1
        for page_idx in 0..20 {
            cache.insert_rgba(
                &ctx,
                tab1,
                PageTextureKey {
                    page_index: page_idx,
                    zoom_pct: 100,
                },
                [2, 2],
                vec![255u8; 16],
            );
        }

        // Tab 1 cache must never exceed 15 pages
        assert_eq!(cache.tab_cached_count(tab1), MAX_CACHED_PAGES_PER_TAB);
        // First 5 pages (0..5) must have been evicted
        for evicted_page in 0..5 {
            assert!(!cache.contains(tab1, evicted_page, 100));
        }
        // Last 15 pages (5..20) must remain present
        for kept_page in 5..20 {
            assert!(cache.contains(tab1, kept_page, 100));
        }

        // Insert one page into tab 2, then close tab 1
        cache.insert_rgba(
            &ctx,
            tab2,
            PageTextureKey {
                page_index: 0,
                zoom_pct: 100,
            },
            [2, 2],
            vec![128u8; 16],
        );
        assert_eq!(cache.tab_cached_count(tab2), 1);

        cache.remove_tab(tab1);
        assert_eq!(cache.tab_cached_count(tab1), 0);
        assert_eq!(cache.tab_cached_count(tab2), 1);
    }
}
