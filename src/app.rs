//! Global application state, tab bar, virtualized page viewport,
//! password unlock card, open-source licenses dialog, Drag & Drop support, and keyboard shortcuts.

use std::collections::HashSet;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Align, Color32, ColorImage, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Frame, IconData, Id, Key, Layout, Margin, Pos2, Rect, RichText, ScrollArea, Sense, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Vec2,
};

use crate::cache::TextureCache;
use crate::tab::{Tab, TabId, TabState, PAGE_SPACING};
use crate::worker::{PdfWorkerHandle, RenderRequest, RenderResponse};

/// Current application version displayed on the welcome screen and in the Licenses window.
pub const APP_VERSION_LABEL: &str = "Version 0.6 Beta";

/// Embedded MIT License of Deimos Reader.
const DEIMOS_MIT_LICENSE: &str = include_str!("../LICENSE");

/// Embedded Orbitron Bold (700) font matching the title typography of `ifconfig.sol12.net`.
const ORBITRON_BOLD_TTF: &[u8] = include_bytes!("../assets/Orbitron-Bold.ttf");

/// Embedded high-resolution (256x256) static Mars planet logo rendered with the `ifconfig.sol12.net` shader.
const MARS_LOGO_PNG: &[u8] = include_bytes!("../assets/mars_logo.png");

/// Font family name registered for the `"Deimos Reader"` title.
const ORBITRON_FAMILY: &str = "Orbitron";

/// Third-party open source component license entry.
struct ThirdPartyLicense {
    name: &'static str,
    version_or_author: &'static str,
    license_name: &'static str,
    repository_url: &'static str,
    notice: &'static str,
}

/// Complete list of third-party open-source libraries and assets used by Deimos Reader.
const THIRD_PARTY_LICENSES: &[ThirdPartyLicense] = &[
    ThirdPartyLicense {
        name: "PDFium",
        version_or_author: "Google LLC / The Chromium Authors & Foxit Software Inc.",
        license_name: "BSD 3-Clause / Apache-2.0",
        repository_url: "https://pdfium.googlesource.com/pdfium/",
        notice: "Copyright 2014 The PDFium Authors. All rights reserved.\n\
Redistribution and use in source and binary forms, with or without modification, are permitted \
provided that the conditions of the 3-Clause BSD License are met.",
    },
    ThirdPartyLicense {
        name: "pdfium-render",
        version_or_author: "0.8 — Alastair Carey",
        license_name: "MIT OR Apache-2.0",
        repository_url: "https://github.com/ajrcarey/pdfium-render",
        notice: "Copyright (c) 2021-2026 Alastair Carey.\n\
High-level idiomatic Rust wrapper around Pdfium. Licensed under either of Apache License, \
Version 2.0 or MIT license at your option.",
    },
    ThirdPartyLicense {
        name: "egui & eframe",
        version_or_author: "0.31 — Emil Ernerfeldt & Rerun Technologies AB",
        license_name: "MIT OR Apache-2.0",
        repository_url: "https://github.com/emilk/egui",
        notice: "Copyright (c) 2018-2026 Emil Ernerfeldt <emil.ernerfeldt@gmail.com>.\n\
Immediate mode GUI library and native windowing framework in Rust. Licensed under MIT or Apache-2.0.",
    },
    ThirdPartyLicense {
        name: "rfd (Rusty File Dialog)",
        version_or_author: "0.15 — PolyMeilex",
        license_name: "MIT",
        repository_url: "https://github.com/PolyMeilex/rfd",
        notice: "Copyright (c) 2020 PolyMeilex.\n\
Native OS file dialogs for Windows (Win32) and Linux (GTK3 / XDG Desktop Portal). Licensed under MIT.",
    },
    ThirdPartyLicense {
        name: "crossbeam-channel",
        version_or_author: "0.5 — The Crossbeam Project Developers",
        license_name: "MIT OR Apache-2.0",
        repository_url: "https://github.com/crossbeam-rs/crossbeam",
        notice: "Copyright (c) 2019 The Crossbeam Project Developers.\n\
Multi-producer multi-consumer channels for message passing. Licensed under MIT or Apache-2.0.",
    },
    ThirdPartyLicense {
        name: "zeroize",
        version_or_author: "1.8 — The RustCrypto Project Developers",
        license_name: "MIT OR Apache-2.0",
        repository_url: "https://github.com/RustCrypto/utils/tree/master/zeroize",
        notice: "Copyright (c) 2018-2026 The RustCrypto Project Developers.\n\
Securely zeroizes secrets from memory using volatile writes and compiler fences. Licensed under MIT or Apache-2.0.",
    },
    ThirdPartyLicense {
        name: "lru",
        version_or_author: "0.13 — Jerome Froelich",
        license_name: "MIT",
        repository_url: "https://github.com/jeromefroe/lru-rs",
        notice: "Copyright (c) 2016 Jerome Froelich.\n\
Bounded Least-Recently-Used (LRU) cache implementation in Rust. Licensed under MIT.",
    },
    ThirdPartyLicense {
        name: "png",
        version_or_author: "0.18 — The image-rs Developers",
        license_name: "MIT OR Apache-2.0",
        repository_url: "https://github.com/image-rs/image-png",
        notice: "Copyright (c) 2015-2026 The image-rs Developers.\n\
Pure-Rust PNG decoder and encoder. Licensed under MIT or Apache-2.0.",
    },
    ThirdPartyLicense {
        name: "Orbitron Typeface",
        version_or_author: "Matt McInerney / The League of Moveable Type",
        license_name: "SIL Open Font License 1.1 (OFL-1.1)",
        repository_url: "https://github.com/theleagueof/orbitron",
        notice: "Copyright (c) 2009-2020 Matt McInerney (matt@pixelspread.com).\n\
This Font Software is licensed under the SIL Open Font License, Version 1.1.",
    },
    ThirdPartyLicense {
        name: "Mars Surface Texture",
        version_or_author: "Solar System Scope / NASA Goddard Space Flight Center",
        license_name: "CC BY 4.0",
        repository_url: "https://www.solarsystemscope.com/textures/",
        notice: "Mars equirectangular planetary map courtesy of Solar System Scope / NASA (Creative Commons Attribution 4.0 International).",
    },
];

/// Decodes the embedded `mars_logo.png` into raw RGBA8 bytes and `(width, height)`.
fn decode_mars_logo_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let decoder = png::Decoder::new(Cursor::new(MARS_LOGO_PNG));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let bytes = &buf[..info.buffer_size()];

    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for &[r, g, b] in bytes.as_chunks::<3>().0 {
                out.extend_from_slice(&[r, g, b, 255]);
            }
            out
        }
        _ => return None,
    };

    Some((rgba, info.width, info.height))
}

/// Loads the high-resolution Mars icon for the OS window title bar and taskbar.
pub fn load_mars_icon_data() -> Option<Arc<IconData>> {
    let (rgba, width, height) = decode_mars_logo_rgba()?;
    Some(Arc::new(IconData {
        rgba,
        width,
        height,
    }))
}

/// Key tracking an in-flight render request `(TabId, page_index, zoom_pct)` to avoid duplicate requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct InFlightKey {
    tab_id: TabId,
    page_index: usize,
    zoom_pct: u16,
}

/// Main application state for Deimos Reader.
pub struct DeimosApp {
    /// Ordered list of open document tabs.
    tabs: Vec<Tab>,
    /// Identifier of the currently active tab.
    active_tab: Option<TabId>,
    /// Monotonic counter used to allocate unique `TabId`s.
    next_tab_id: u64,
    /// Non-blocking communication handle to the background PDF worker thread.
    worker: PdfWorkerHandle,
    /// Bounded LRU cache of GPU page textures (max 15 pages per tab).
    texture_cache: TextureCache,
    /// Set of page render requests currently in flight.
    in_flight: HashSet<InFlightKey>,
    /// Static GPU texture for the Mars planet brand logo.
    mars_logo_texture: Option<TextureHandle>,
    /// Controls visibility of the Open Source Licenses window.
    show_licenses_window: bool,
}

impl DeimosApp {
    /// Initializes the application, configures fonts and dark theme visuals, loads the static Mars logo texture,
    /// and starts the background PDF worker thread.
    pub fn new(cc: &eframe::CreationContext<'_>, initial_files: Vec<PathBuf>) -> Self {
        Self::configure_fonts(&cc.egui_ctx);
        Self::configure_visuals(&cc.egui_ctx);

        let mars_logo_texture = decode_mars_logo_rgba().map(|(rgba, width, height)| {
            let image = ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
            cc.egui_ctx
                .load_texture("deimos_mars_logo", image, TextureOptions::LINEAR)
        });

        let worker = PdfWorkerHandle::spawn(Some(cc.egui_ctx.clone()));
        let mut app = Self {
            tabs: Vec::new(),
            active_tab: None,
            next_tab_id: 1,
            worker,
            texture_cache: TextureCache::default(),
            in_flight: HashSet::new(),
            mars_logo_texture,
            show_licenses_window: false,
        };

        for path in initial_files {
            app.open_file_in_tab(path);
        }

        app
    }

    /// Registers the embedded Orbitron Bold (700) font family for the application title while keeping
    /// the default proportional font for all body and UI text.
    fn configure_fonts(ctx: &egui::Context) {
        let mut fonts = FontDefinitions::default();
        fonts.font_data.insert(
            "Orbitron-Bold".to_owned(),
            Arc::new(FontData::from_static(ORBITRON_BOLD_TTF)),
        );

        let mut family = vec!["Orbitron-Bold".to_owned()];
        if let Some(default_prop) = fonts.families.get(&FontFamily::Proportional) {
            family.extend(default_prop.iter().cloned());
        }
        fonts
            .families
            .insert(FontFamily::Name(ORBITRON_FAMILY.into()), family);

        ctx.set_fonts(fonts);
    }

    /// Applies a clean, high-contrast dark theme suited for extended document reading.
    fn configure_visuals(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(24, 25, 28);
        visuals.window_fill = Color32::from_rgb(30, 32, 36);
        visuals.extreme_bg_color = Color32::from_rgb(16, 17, 20);
        ctx.set_visuals(visuals);
    }

    /// Opens a PDF file in a new tab and dispatches an `OpenDocument` request to the worker.
    pub fn open_file_in_tab(&mut self, path: PathBuf) {
        let tab_id = TabId(self.next_tab_id);
        self.next_tab_id = self.next_tab_id.wrapping_add(1);

        let tab = Tab::new(tab_id, path.clone());
        self.tabs.push(tab);
        self.active_tab = Some(tab_id);

        self.worker.send(RenderRequest::OpenDocument { tab_id, path });
    }

    /// Opens the native OS file dialog (`rfd`) to select one or more PDF files.
    fn prompt_open_files(&mut self) {
        if let Some(files) = rfd::FileDialog::new()
            .set_title("Open PDF Files")
            .add_filter("PDF Documents (*.pdf)", &["pdf", "PDF"])
            .pick_files()
        {
            for path in files {
                self.open_file_in_tab(path);
            }
        }
    }

    /// Closes a tab by its `TabId` and immediately releases its RAM and GPU resources.
    fn close_tab(&mut self, tab_id: TabId) {
        let Some(pos) = self.tabs.iter().position(|t| t.id == tab_id) else {
            return;
        };

        self.tabs.remove(pos);
        self.texture_cache.remove_tab(tab_id);
        self.in_flight.retain(|k| k.tab_id != tab_id);
        self.worker.send(RenderRequest::CloseTab { tab_id });

        if self.active_tab == Some(tab_id) {
            if self.tabs.is_empty() {
                self.active_tab = None;
            } else {
                let next_idx = pos.min(self.tabs.len() - 1);
                self.active_tab = Some(self.tabs[next_idx].id);
            }
        }
    }

    /// Cycles to the next (`delta = 1`) or previous (`delta = -1`) tab.
    fn cycle_tabs(&mut self, delta: isize) {
        if self.tabs.len() <= 1 {
            return;
        }
        let current_idx = self
            .active_tab
            .and_then(|id| self.tabs.iter().position(|t| t.id == id))
            .unwrap_or(0);

        let len = self.tabs.len() as isize;
        let next_idx = (current_idx as isize + delta).rem_euclid(len) as usize;
        self.active_tab = Some(self.tabs[next_idx].id);
    }

    /// Non-blockingly drains all responses sent by the background worker thread.
    fn poll_worker_responses(&mut self, ctx: &egui::Context) {
        while let Some(response) = self.worker.try_recv() {
            match response {
                RenderResponse::DocumentReady {
                    tab_id,
                    page_count,
                    page_dimensions,
                } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.transition_to_ready(page_count, page_dimensions);
                    }
                }

                RenderResponse::DocumentLocked { tab_id } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.transition_to_locked();
                    }
                }

                RenderResponse::AuthenticationFailed { tab_id, error_msg } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.record_failed_auth(error_msg);
                    }
                }

                RenderResponse::DocumentError { tab_id, error } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.transition_to_error(error);
                    }
                }

                RenderResponse::PageRendered {
                    tab_id,
                    page_index,
                    zoom_pct,
                    width,
                    height,
                    pixels,
                } => {
                    self.in_flight.remove(&InFlightKey {
                        tab_id,
                        page_index,
                        zoom_pct,
                    });

                    // Only upload to VRAM if the tab is still open at this exact zoom level.
                    if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
                        if tab.zoom_pct() == zoom_pct {
                            self.texture_cache.insert_rgba(
                                ctx,
                                tab_id,
                                crate::cache::PageTextureKey {
                                    page_index,
                                    zoom_pct,
                                },
                                [width, height],
                                pixels,
                            );
                        }
                    }
                }
            }
        }
    }

    /// Applies a zoom level change for a tab and cancels obsolete queued renders.
    fn apply_zoom_change_for_tab(&mut self, tab_id: TabId, new_zoom_pct: u16) {
        self.in_flight
            .retain(|k| k.tab_id != tab_id || k.zoom_pct == new_zoom_pct);
        self.texture_cache.evict_stale_zoom(tab_id, new_zoom_pct);
        self.worker.send(RenderRequest::CancelObsolete {
            tab_id,
            active_zoom_pct: new_zoom_pct,
            keep_range: None,
        });
    }

    /// Handles all keyboard shortcuts (`Ctrl+O`, `Ctrl+W`, `Ctrl+Tab`, `Ctrl+Shift+Tab`,
    /// `Ctrl++`, `Ctrl+-`, `Ctrl+0`, `Ctrl+Wheel` in 5% notches) and Drag & Drop file events.
    fn handle_shortcuts_and_inputs(&mut self, ctx: &egui::Context) {
        let mut open_requested = false;
        let mut close_active = false;
        let mut tab_cycle: isize = 0;
        let mut zoom_action: Option<ZoomAction> = None;

        ctx.input_mut(|input| {
            // Ctrl + O: Open one or more PDF files
            if input.consume_key(egui::Modifiers::COMMAND, Key::O) {
                open_requested = true;
            }

            // Ctrl + W: Close active tab
            if input.consume_key(egui::Modifiers::COMMAND, Key::W) {
                close_active = true;
            }

            // Ctrl + Shift + Tab: Previous tab
            if input.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::Tab) {
                tab_cycle = -1;
            } else if input.consume_key(egui::Modifiers::COMMAND, Key::Tab) {
                // Ctrl + Tab: Next tab
                tab_cycle = 1;
            }

            // Ctrl + 0: Reset zoom to 100%
            if input.consume_key(egui::Modifiers::COMMAND, Key::Num0) {
                zoom_action = Some(ZoomAction::Reset);
            }

            // Ctrl + '+' (or '=' on keyboards where '+' shares the '=' key): Zoom in (25%)
            if input.consume_key(egui::Modifiers::COMMAND, Key::Plus)
                || input.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::Plus)
                || input.consume_key(egui::Modifiers::COMMAND, Key::Equals)
                || input.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::Equals)
            {
                zoom_action = Some(ZoomAction::In);
            }

            // Ctrl + '-': Zoom out (25%)
            if input.consume_key(egui::Modifiers::COMMAND, Key::Minus) {
                zoom_action = Some(ZoomAction::Out);
            }

            // Ctrl + Mouse Wheel: Zoom in / out in strict 5% steps
            if input.modifiers.command {
                let zoom_delta = input.zoom_delta();
                if (zoom_delta - 1.0).abs() > 0.005 {
                    if zoom_delta > 1.0 {
                        zoom_action = Some(ZoomAction::WheelIn);
                    } else {
                        zoom_action = Some(ZoomAction::WheelOut);
                    }
                    input.raw_scroll_delta.y = 0.0;
                } else if input.raw_scroll_delta.y.abs() > 0.1 {
                    if input.raw_scroll_delta.y > 0.0 {
                        zoom_action = Some(ZoomAction::WheelIn);
                    } else {
                        zoom_action = Some(ZoomAction::WheelOut);
                    }
                    input.raw_scroll_delta.y = 0.0;
                }
            }
        });

        if open_requested {
            self.prompt_open_files();
        }

        if close_active {
            if let Some(active_id) = self.active_tab {
                self.close_tab(active_id);
            }
        }

        if tab_cycle != 0 {
            self.cycle_tabs(tab_cycle);
        }

        if let Some(action) = zoom_action {
            if let Some(active_id) = self.active_tab {
                let mut changed_pct = None;
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == active_id) {
                    if matches!(tab.state, TabState::Ready { .. }) {
                        let changed = match action {
                            ZoomAction::In => tab.zoom_in(),
                            ZoomAction::Out => tab.zoom_out(),
                            ZoomAction::WheelIn => tab.zoom_in_wheel(),
                            ZoomAction::WheelOut => tab.zoom_out_wheel(),
                            ZoomAction::Reset => tab.reset_zoom(),
                        };
                        if changed {
                            changed_pct = Some(tab.zoom_pct());
                        }
                    }
                }
                if let Some(new_pct) = changed_pct {
                    self.apply_zoom_change_for_tab(active_id, new_pct);
                }
            }
        }

        // Drag & Drop PDF files into the window
        let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
        for dropped in dropped_files {
            if let Some(path) = dropped.path {
                self.open_file_in_tab(path);
            }
        }
    }

    /// Renders the minimalist top bar (horizontal tabs, `+` button, `Page X / Y` indicator, and zoom controls).
    fn render_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("deimos_top_bar")
            .exact_height(38.0)
            .frame(
                Frame::NONE
                    .fill(Color32::from_rgb(20, 21, 24))
                    .inner_margin(Margin::symmetric(8, 4))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(42, 44, 50))),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let active_info = self
                        .active_tab
                        .and_then(|id| self.tabs.iter().find(|t| t.id == id))
                        .and_then(|tab| match &tab.state {
                            TabState::Ready { page_count, .. } => {
                                Some((tab.id, tab.current_page + 1, *page_count, tab.zoom_pct()))
                            }
                            _ => None,
                        });

                    let right_reserved = if active_info.is_some() { 260.0 } else { 20.0 };
                    let tabs_max_width = (ui.available_width() - right_reserved).max(120.0);

                    let mut tab_to_activate: Option<TabId> = None;
                    let mut tab_to_close: Option<TabId> = None;
                    let mut open_dialog = false;

                    ui.allocate_ui_with_layout(
                        Vec2::new(tabs_max_width, ui.available_height()),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ScrollArea::horizontal()
                                .id_salt("deimos_tabs_scroll")
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                                )
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;

                                    for tab in &self.tabs {
                                        let is_active = self.active_tab == Some(tab.id);
                                        let bg_color = if is_active {
                                            Color32::from_rgb(44, 48, 58)
                                        } else {
                                            Color32::from_rgb(28, 30, 35)
                                        };
                                        let border_color = if is_active {
                                            Color32::from_rgb(90, 130, 210)
                                        } else {
                                            Color32::from_rgb(48, 50, 58)
                                        };

                                        let tab_frame = Frame::NONE
                                            .fill(bg_color)
                                            .corner_radius(CornerRadius::same(5))
                                            .inner_margin(Margin::symmetric(8, 3))
                                            .stroke(Stroke::new(1.0_f32, border_color));

                                        let frame_resp = tab_frame.show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                let label_text = if tab.title.chars().count() > 26 {
                                                    let truncated: String =
                                                        tab.title.chars().take(24).collect();
                                                    format!("{truncated}…")
                                                } else {
                                                    tab.title.clone()
                                                };

                                                let title_resp = ui.add(
                                                    egui::Label::new(
                                                        RichText::new(label_text)
                                                            .size(13.0)
                                                            .color(if is_active {
                                                                Color32::WHITE
                                                            } else {
                                                                Color32::from_rgb(185, 190, 200)
                                                            }),
                                                    )
                                                    .sense(Sense::click()),
                                                );

                                                if title_resp.clicked() {
                                                    tab_to_activate = Some(tab.id);
                                                }
                                                if title_resp.middle_clicked() {
                                                    tab_to_close = Some(tab.id);
                                                }
                                                title_resp.on_hover_text(
                                                    tab.path.display().to_string(),
                                                );

                                                let close_btn = ui.add(
                                                    egui::Button::new(
                                                        RichText::new("×").size(14.0).color(
                                                            Color32::from_rgb(175, 180, 190),
                                                        ),
                                                    )
                                                    .frame(false),
                                                );
                                                if close_btn.clicked() {
                                                    tab_to_close = Some(tab.id);
                                                }
                                                close_btn.on_hover_text("Close Tab (Ctrl+W)");
                                            });
                                        });

                                        if frame_resp.response.clicked() {
                                            tab_to_activate = Some(tab.id);
                                        }
                                        if frame_resp.response.middle_clicked() {
                                            tab_to_close = Some(tab.id);
                                        }
                                    }

                                    // '+' button to open a PDF file
                                    let plus_btn = ui.add(
                                        egui::Button::new(
                                            RichText::new(" + ").size(15.0).strong(),
                                        )
                                        .min_size(Vec2::new(28.0, 24.0)),
                                    );
                                    if plus_btn.clicked() {
                                        open_dialog = true;
                                    }
                                    plus_btn.on_hover_text("Open PDF File (Ctrl+O)");
                                });
                        },
                    );

                    // Right-aligned indicators: Page X / Y and Zoom Level
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some((tab_id, current_page_1based, total_pages, zoom_pct)) =
                            active_info
                        {
                            let mut zoom_op: Option<ZoomAction> = None;

                            if ui
                                .add(egui::Button::new("+").min_size(Vec2::new(22.0, 22.0)))
                                .on_hover_text("Zoom In (Ctrl++)")
                                .clicked()
                            {
                                zoom_op = Some(ZoomAction::In);
                            }

                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(format!("{zoom_pct}%"))
                                            .monospace()
                                            .size(12.5),
                                    )
                                    .min_size(Vec2::new(52.0, 22.0)),
                                )
                                .on_hover_text("Reset Zoom to 100% (Ctrl+0)")
                                .clicked()
                            {
                                zoom_op = Some(ZoomAction::Reset);
                            }

                            if ui
                                .add(egui::Button::new("−").min_size(Vec2::new(22.0, 22.0)))
                                .on_hover_text("Zoom Out (Ctrl+-)")
                                .clicked()
                            {
                                zoom_op = Some(ZoomAction::Out);
                            }

                            ui.separator();

                            ui.label(
                                RichText::new(format!(
                                    "Page {current_page_1based} / {total_pages}"
                                ))
                                .size(13.0)
                                .color(Color32::from_rgb(210, 214, 224)),
                            );

                            if let Some(op) = zoom_op {
                                let mut new_pct = None;
                                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                                    let changed = match op {
                                        ZoomAction::In => tab.zoom_in(),
                                        ZoomAction::Out => tab.zoom_out(),
                                        ZoomAction::WheelIn => tab.zoom_in_wheel(),
                                        ZoomAction::WheelOut => tab.zoom_out_wheel(),
                                        ZoomAction::Reset => tab.reset_zoom(),
                                    };
                                    if changed {
                                        new_pct = Some(tab.zoom_pct());
                                    }
                                }
                                if let Some(pct) = new_pct {
                                    self.apply_zoom_change_for_tab(tab_id, pct);
                                }
                            }
                        }
                    });

                    if let Some(id) = tab_to_activate {
                        self.active_tab = Some(id);
                    }
                    if let Some(id) = tab_to_close {
                        self.close_tab(id);
                    }
                    if open_dialog {
                        self.prompt_open_files();
                    }
                });
            });
    }

    /// Renders the central panel according to the active tab's state machine.
    fn render_central_panel(&mut self, ctx: &egui::Context) {
        let pixels_per_point = ctx.pixels_per_point().max(1.0);

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(Color32::from_rgb(24, 25, 28)))
            .show(ctx, |ui| {
                let Some(active_id) = self.active_tab else {
                    self.render_empty_state(ui);
                    return;
                };

                let Some(tab_idx) = self.tabs.iter().position(|t| t.id == active_id) else {
                    return;
                };

                let current_state = self.tabs[tab_idx].state.clone();

                match current_state {
                    TabState::Loading => {
                        let title = self.tabs[tab_idx].title.clone();
                        ui.centered_and_justified(|ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space(ui.available_height() * 0.42);
                                ui.spinner();
                                ui.add_space(10.0);
                                ui.label(
                                    RichText::new(format!("Opening \"{title}\"…"))
                                        .size(15.0)
                                        .color(Color32::from_rgb(190, 195, 205)),
                                );
                            });
                        });
                    }

                    TabState::Locked {
                        retry_count: _,
                        error_msg,
                    } => {
                        self.render_locked_state(ui, tab_idx, error_msg);
                    }

                    TabState::Ready {
                        page_count,
                        page_dimensions,
                    } => {
                        self.render_ready_viewport(
                            ui,
                            tab_idx,
                            page_count,
                            &page_dimensions,
                            pixels_per_point,
                        );
                    }

                    TabState::Error(err_msg) => {
                        let title = self.tabs[tab_idx].title.clone();
                        ui.vertical_centered(|ui| {
                            ui.add_space((ui.available_height() * 0.35).max(30.0));
                            Frame::NONE
                                .fill(Color32::from_rgb(36, 28, 30))
                                .corner_radius(CornerRadius::same(8))
                                .inner_margin(Margin::same(24))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(180, 70, 70)))
                                .show(ui, |ui| {
                                    ui.set_max_width(460.0);
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new(title)
                                                .size(16.0)
                                                .strong()
                                                .color(Color32::WHITE),
                                        );
                                        ui.add_space(8.0);
                                        ui.label(
                                            RichText::new(&err_msg)
                                                .size(13.5)
                                                .color(Color32::from_rgb(240, 130, 130)),
                                        );
                                    });
                                });
                        });
                    }
                }
            });
    }

    /// Renders the welcome screen when no document tab is open:
    /// - Centered: static high-resolution Mars planet to the left of `"Deimos Reader"` in Orbitron Bold,
    ///   with the standard proportional font used below.
    /// - Bottom-left corner: current software version (`"Version 0.6 Beta"`).
    /// - Bottom-right corner: `"Licenses"` button opening the Open Source Licenses window.
    fn render_empty_state(&mut self, ui: &mut egui::Ui) {
        let panel_rect = ui.max_rect();

        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.35).max(40.0));

            let title_font = FontId::new(28.0, FontFamily::Name(ORBITRON_FAMILY.into()));
            let title_text = "Deimos Reader";
            let title_galley =
                ui.painter()
                    .layout_no_wrap(title_text.to_owned(), title_font.clone(), Color32::WHITE);

            let logo_size = 48.0_f32;
            let gap = 12.0_f32;
            let total_width = if self.mars_logo_texture.is_some() {
                logo_size + gap + title_galley.size().x
            } else {
                title_galley.size().x
            };
            let row_height = logo_size.max(title_galley.size().y);

            let (row_rect, _) =
                ui.allocate_exact_size(Vec2::new(total_width, row_height), Sense::hover());

            let mut cursor_x = row_rect.min.x;
            if let Some(tex) = &self.mars_logo_texture {
                let logo_rect = Rect::from_min_size(
                    Pos2::new(cursor_x, row_rect.center().y - logo_size * 0.5),
                    Vec2::splat(logo_size),
                );
                let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
                ui.painter().image(tex.id(), logo_rect, uv, Color32::WHITE);
                cursor_x += logo_size + gap;
            }

            let text_pos = Pos2::new(
                cursor_x,
                row_rect.center().y - title_galley.size().y * 0.5,
            );
            ui.painter().galley(text_pos, title_galley, Color32::WHITE);

            ui.add_space(10.0);
            ui.label(
                RichText::new(
                    "Drag and drop a PDF file here or press Ctrl + O to open a document.",
                )
                .size(14.0)
                .color(Color32::from_rgb(150, 156, 168)),
            );
            ui.add_space(18.0);
            if ui
                .add(
                    egui::Button::new(RichText::new("Open PDF File…").size(14.0))
                        .min_size(Vec2::new(190.0, 34.0)),
                )
                .clicked()
            {
                self.prompt_open_files();
            }
        });

        // Bottom footer bar pinned to the bottom-left and bottom-right corners of the welcome screen
        let footer_height = 32.0_f32;
        let footer_margin_x = 16.0_f32;
        let footer_margin_bottom = 12.0_f32;
        let footer_rect = Rect::from_min_max(
            Pos2::new(
                panel_rect.min.x + footer_margin_x,
                (panel_rect.max.y - footer_height - footer_margin_bottom).max(panel_rect.min.y),
            ),
            Pos2::new(
                panel_rect.max.x - footer_margin_x,
                panel_rect.max.y - footer_margin_bottom,
            ),
        );

        let mut footer_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(footer_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );

        footer_ui.label(
            RichText::new(APP_VERSION_LABEL)
                .size(12.5)
                .color(Color32::from_rgb(125, 130, 142)),
        );

        footer_ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let licenses_btn = ui.add(
                egui::Button::new(
                    RichText::new("Licenses")
                        .size(12.5)
                        .color(Color32::from_rgb(195, 200, 212)),
                )
                .min_size(Vec2::new(86.0, 26.0)),
            );
            if licenses_btn
                .on_hover_text("View Deimos Reader MIT License and Open Source notices")
                .clicked()
            {
                self.show_licenses_window = true;
            }
        });
    }

    /// Renders the modal/floating window displaying the Deimos Reader MIT License and all
    /// third-party Open Source component licenses.
    fn render_licenses_window(&mut self, ctx: &egui::Context) {
        if !self.show_licenses_window {
            return;
        }

        let mut open = self.show_licenses_window;
        let mut close_clicked = false;

        egui::Window::new("Open Source Licenses — Deimos Reader")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([640.0, 520.0])
            .min_size([460.0, 340.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        if let Some(tex) = &self.mars_logo_texture {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                            let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
                            ui.painter().image(tex.id(), rect, uv, Color32::WHITE);
                            ui.add_space(6.0);
                        }
                        ui.label(
                            RichText::new("Deimos Reader")
                                .font(FontId::new(17.0, FontFamily::Name(ORBITRON_FAMILY.into())))
                                .color(Color32::WHITE),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!("({APP_VERSION_LABEL})"))
                                .size(13.0)
                                .color(Color32::from_rgb(160, 166, 178)),
                        );
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(6.0);

                    let scroll_height = (ui.available_height() - 44.0).max(180.0);
                    ScrollArea::vertical()
                        .id_salt("deimos_licenses_scroll")
                        .max_height(scroll_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            // Section 1: Deimos Reader MIT License
                            Frame::NONE
                                .fill(Color32::from_rgb(22, 24, 28))
                                .corner_radius(CornerRadius::same(6))
                                .inner_margin(Margin::same(14))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(52, 56, 66)))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(
                                        RichText::new("Deimos Reader — MIT License")
                                            .size(14.5)
                                            .strong()
                                            .color(Color32::from_rgb(249, 115, 22)),
                                    );
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new(DEIMOS_MIT_LICENSE.trim())
                                            .monospace()
                                            .size(12.0)
                                            .color(Color32::from_rgb(205, 210, 220)),
                                    );
                                });

                            ui.add_space(14.0);
                            ui.label(
                                RichText::new("Third-Party Open Source Software & Assets")
                                    .size(14.5)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            ui.add_space(6.0);

                            for item in THIRD_PARTY_LICENSES {
                                Frame::NONE
                                    .fill(Color32::from_rgb(22, 24, 28))
                                    .corner_radius(CornerRadius::same(6))
                                    .inner_margin(Margin::same(12))
                                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(44, 47, 56)))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(
                                                RichText::new(item.name)
                                                    .size(13.5)
                                                    .strong()
                                                    .color(Color32::WHITE),
                                            );
                                            ui.label(
                                                RichText::new(format!("— {}", item.license_name))
                                                    .size(12.5)
                                                    .color(Color32::from_rgb(249, 115, 22)),
                                            );
                                        });
                                        ui.label(
                                            RichText::new(format!(
                                                "{} • {}",
                                                item.version_or_author, item.repository_url
                                            ))
                                            .size(11.5)
                                            .color(Color32::from_rgb(145, 150, 162)),
                                        );
                                        ui.add_space(4.0);
                                        ui.label(
                                            RichText::new(item.notice)
                                                .size(12.0)
                                                .color(Color32::from_rgb(195, 200, 210)),
                                        );
                                    });
                                ui.add_space(8.0);
                            }
                        });

                    ui.add_space(8.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Close").size(13.5))
                                    .min_size(Vec2::new(96.0, 28.0)),
                            )
                            .clicked()
                        {
                            close_clicked = true;
                        }
                    });
                });
            });

        self.show_licenses_window = open && !close_clicked;
    }

    /// In the `Locked` state, displays a centered card containing:
    /// - The file name and `"Encrypted PDF Document"`.
    /// - A masked password field (`egui::TextEdit::singleline(&mut password).password(true)`),
    ///   with automatic focus and `Enter` key submission.
    /// - An `"Unlock"` button.
    fn render_locked_state(
        &mut self,
        ui: &mut egui::Ui,
        tab_idx: usize,
        error_msg: Option<String>,
    ) {
        let mut submit_password = false;

        ui.vertical_centered(|ui| {
            let vertical_pad = ((ui.available_height() - 230.0) * 0.5).max(24.0);
            ui.add_space(vertical_pad);

            Frame::NONE
                .fill(Color32::from_rgb(32, 34, 40))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(24))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(65, 70, 82)))
                .show(ui, |ui| {
                    ui.set_width(360.0);
                    ui.vertical_centered(|ui| {
                        let tab = &mut self.tabs[tab_idx];

                        ui.label(
                            RichText::new(&tab.title)
                                .size(16.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Encrypted PDF Document")
                                .size(13.5)
                                .color(Color32::from_rgb(185, 190, 205)),
                        );

                        ui.add_space(16.0);

                        let pwd_edit_id = Id::new(("deimos_pwd_input", tab.id.0));
                        let edit_response = ui.add_enabled(
                            !tab.is_authenticating,
                            egui::TextEdit::singleline(&mut tab.password_input)
                                .id(pwd_edit_id)
                                .password(true)
                                .hint_text("Password")
                                .desired_width(280.0),
                        );

                        if tab.focus_password_field {
                            edit_response.request_focus();
                            tab.focus_password_field = false;
                        }

                        let enter_pressed =
                            ui.input(|i| i.key_pressed(Key::Enter)) && edit_response.lost_focus()
                                || (edit_response.has_focus()
                                    && ui.input(|i| i.key_pressed(Key::Enter)));

                        ui.add_space(12.0);

                        let unlock_clicked = ui
                            .add_enabled(
                                !tab.is_authenticating,
                                egui::Button::new(RichText::new("Unlock").size(14.0).strong())
                                    .min_size(Vec2::new(140.0, 32.0)),
                            )
                            .clicked();

                        if (enter_pressed || unlock_clicked) && !tab.is_authenticating {
                            submit_password = true;
                        }

                        if let Some(err) = error_msg {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new(err)
                                    .size(13.0)
                                    .color(Color32::from_rgb(235, 95, 95)),
                            );
                        }
                    });
                });
        });

        if submit_password {
            let tab = &mut self.tabs[tab_idx];
            let tab_id = tab.id;
            // Extract the password into `Zeroizing<String>` and immediately wipe the UI `String` via `.zeroize()`.
            let password = tab.take_password_zeroized();
            self.worker
                .send(RenderRequest::AuthenticatePassword { tab_id, password });
        }
    }

    /// High-performance virtual scrolling viewport: computes visible pages, requests only visible
    /// pages (+1 above and +1 below) from the worker, and draws cached GPU textures from the LRU cache.
    fn render_ready_viewport(
        &mut self,
        ui: &mut egui::Ui,
        tab_idx: usize,
        page_count: usize,
        page_dimensions: &[(f32, f32)],
        pixels_per_point: f32,
    ) {
        let tab_id = self.tabs[tab_idx].id;
        let zoom = self.tabs[tab_idx].zoom;
        let zoom_pct = self.tabs[tab_idx].zoom_pct();
        let pending_scroll = self.tabs[tab_idx].pending_scroll_to_y.take();

        let mut scroll_area = ScrollArea::vertical()
            .id_salt(("deimos_pdf_scroll", tab_id.0))
            .auto_shrink([false, false]);

        if let Some(y_offset) = pending_scroll {
            scroll_area = scroll_area.vertical_scroll_offset(y_offset);
        }

        let scroll_out = scroll_area.show_viewport(ui, |ui, viewport_rect| {
            ui.set_min_width(ui.available_width());

            // Compute pages currently visible inside the scroll viewport rectangle
            let visible_range = self.tabs[tab_idx]
                .compute_visible_pages(viewport_rect.min.y, viewport_rect.max.y);

            if let Some(range) = visible_range {
                self.tabs[tab_idx].current_page = range.current_page;

                // Cancel queued renders outside the active pre-fetch window `[request_start..=request_end]`
                self.worker.send(RenderRequest::CancelObsolete {
                    tab_id,
                    active_zoom_pct: zoom_pct,
                    keep_range: Some((range.request_start, range.request_end)),
                });

                // Request only pages in `[request_start..=request_end]` that are neither cached nor in flight
                for page_idx in range.request_start..=range.request_end {
                    if page_idx < page_count
                        && !self.texture_cache.contains(tab_id, page_idx, zoom_pct)
                    {
                        let key = InFlightKey {
                            tab_id,
                            page_index: page_idx,
                            zoom_pct,
                        };
                        if self.in_flight.insert(key) {
                            let effective_scale = zoom * pixels_per_point;
                            self.worker.send(RenderRequest::RenderPage {
                                tab_id,
                                page_index: page_idx,
                                zoom_pct,
                                scale: effective_scale,
                            });
                        }
                    }
                }
            }

            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(PAGE_SPACING);

                for (page_idx, &(w_pts, h_pts)) in page_dimensions.iter().enumerate() {
                    let page_size = Vec2::new((w_pts * zoom).max(1.0), (h_pts * zoom).max(1.0));
                    let (rect, _response) = ui.allocate_exact_size(page_size, Sense::hover());

                    let is_in_viewport = visible_range.is_some_and(|r| {
                        page_idx >= r.first_visible && page_idx <= r.last_visible
                    });

                    if is_in_viewport {
                        let painter = ui.painter();
                        painter.rect_filled(rect, CornerRadius::ZERO, Color32::WHITE);

                        if let Some(texture) =
                            self.texture_cache.get(tab_id, page_idx, zoom_pct)
                        {
                            let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
                            painter.image(texture.id(), rect, uv, Color32::WHITE);
                        } else {
                            painter.text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                format!("Page {}…", page_idx + 1),
                                FontId::proportional(14.0),
                                Color32::from_rgb(130, 135, 145),
                            );
                        }

                        painter.rect_stroke(
                            rect,
                            CornerRadius::ZERO,
                            Stroke::new(1.0_f32, Color32::from_rgb(15, 16, 18)),
                            StrokeKind::Outside,
                        );
                    }

                    ui.add_space(PAGE_SPACING);
                }
            });
        });

        self.tabs[tab_idx].scroll_offset = scroll_out.state.offset.y;
    }
}

/// Zoom actions triggered by keyboard shortcuts, mouse wheel (5% steps), or toolbar buttons (25% steps).
#[derive(Debug, Clone, Copy)]
enum ZoomAction {
    In,
    Out,
    WheelIn,
    WheelOut,
    Reset,
}

impl eframe::App for DeimosApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker_responses(ctx);
        self.handle_shortcuts_and_inputs(ctx);
        self.render_top_bar(ctx);
        self.render_central_panel(ctx);
        self.render_licenses_window(ctx);
    }
}
