//! Background worker thread managing per-tab PDF opening, password authentication, and page rasterization.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use zeroize::{Zeroize, Zeroizing};

use crate::pdf_engine::{OpenOutcome, PdfEngineDocument};
use crate::tab::TabId;

/// Requests sent from the UI thread to the background worker thread over `crossbeam-channel`.
#[derive(Debug)]
pub enum RenderRequest {
    /// Requests opening and initial analysis of a PDF file for a tab.
    OpenDocument { tab_id: TabId, path: PathBuf },
    /// Submits a user password (wrapped in `Zeroizing<String>`) to unlock a `Locked` tab.
    AuthenticatePassword {
        tab_id: TabId,
        password: Zeroizing<String>,
    },
    /// Requests rasterization of a specific page at a given zoom scale.
    RenderPage {
        tab_id: TabId,
        page_index: usize,
        zoom_pct: u16,
        scale: f32,
    },
    /// Cancels all queued render requests for a tab that do not match `active_zoom_pct`
    /// or fall outside the visible pre-fetch window `[keep_start..=keep_end]`.
    CancelObsolete {
        tab_id: TabId,
        active_zoom_pct: u16,
        keep_range: Option<(usize, usize)>,
    },
    /// Closes a tab: cancels pending requests and immediately frees the PDF document from RAM.
    CloseTab { tab_id: TabId },
    /// Cleanly shuts down the background worker thread.
    Shutdown,
}

/// Responses sent from the background worker thread back to the UI thread over `crossbeam-channel`.
#[derive(Debug)]
pub enum RenderResponse {
    /// The document is open (or unlocked) and ready to be displayed.
    DocumentReady {
        tab_id: TabId,
        page_count: usize,
        page_dimensions: Vec<(f32, f32)>,
    },
    /// The document is encrypted and requires a user password.
    DocumentLocked { tab_id: TabId },
    /// The submitted password was rejected by the PDF engine.
    AuthenticationFailed { tab_id: TabId, error_msg: String },
    /// The file is corrupted or unreadable.
    DocumentError { tab_id: TabId, error: String },
    /// A single page was rasterized into an RGBA8 pixel buffer.
    PageRendered {
        tab_id: TabId,
        page_index: usize,
        zoom_pct: u16,
        width: usize,
        height: usize,
        pixels: Vec<u8>,
    },
}

/// Individual page rasterization task stored in the worker's deduplicated queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueuedPageJob {
    tab_id: TabId,
    page_index: usize,
    zoom_pct: u16,
    scale_bits: u32,
}

/// Non-blocking communication handle held by the UI thread to interact with the PDF worker.
pub struct PdfWorkerHandle {
    request_tx: Sender<RenderRequest>,
    response_rx: Receiver<RenderResponse>,
    join_handle: Option<JoinHandle<()>>,
}

impl PdfWorkerHandle {
    /// Spawns the dedicated background PDF worker thread.
    pub fn spawn(egui_ctx: Option<egui::Context>) -> Self {
        let (request_tx, request_rx) = crossbeam_channel::unbounded::<RenderRequest>();
        let (response_tx, response_rx) = crossbeam_channel::unbounded::<RenderResponse>();

        let join_handle = thread::Builder::new()
            .name("deimos-pdf-worker".to_string())
            .spawn(move || {
                worker_loop(request_rx, response_tx, egui_ctx);
            })
            .expect("Failed to spawn PDF worker thread");

        Self {
            request_tx,
            response_rx,
            join_handle: Some(join_handle),
        }
    }

    /// Sends a request to the worker without blocking the UI thread.
    pub fn send(&self, request: RenderRequest) {
        let _ = self.request_tx.send(request);
    }

    /// Retrieves the next available worker response without blocking (`try_recv`).
    pub fn try_recv(&self) -> Option<RenderResponse> {
        self.response_rx.try_recv().ok()
    }

    /// Direct receiver reference (used in unit tests).
    #[cfg(test)]
    pub fn receiver(&self) -> &Receiver<RenderResponse> {
        &self.response_rx
    }
}

impl Drop for PdfWorkerHandle {
    fn drop(&mut self) {
        let _ = self.request_tx.send(RenderRequest::Shutdown);
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Main event loop running on the background worker thread.
///
/// Guarantees:
/// 1. Immediate priority for control messages (`OpenDocument`, `AuthenticatePassword`, `CancelObsolete`, `CloseTab`).
/// 2. Deduplication of identical `RenderPage` requests `(tab_id, page_index, zoom_pct)`.
/// 3. Instant cancellation of obsolete renders when zoom changes or a tab is closed.
fn worker_loop(
    request_rx: Receiver<RenderRequest>,
    response_tx: Sender<RenderResponse>,
    egui_ctx: Option<egui::Context>,
) {
    let mut documents: HashMap<TabId, PdfEngineDocument> = HashMap::new();
    let mut render_queue: VecDeque<QueuedPageJob> = VecDeque::new();
    let mut should_shutdown = false;

    loop {
        // When the render queue is empty, park passively waiting for the next request with zero CPU usage.
        if render_queue.is_empty() {
            match request_rx.recv() {
                Ok(first_req) => {
                    process_incoming_request(
                        first_req,
                        &mut documents,
                        &mut render_queue,
                        &response_tx,
                        &egui_ctx,
                        &mut should_shutdown,
                    );
                }
                Err(_) => break,
            }
        }

        if should_shutdown {
            break;
        }

        // Drain all pending channel messages before rasterizing the next page so zoom cancellations,
        // tab closures, or password submissions take effect immediately.
        while let Ok(req) = request_rx.try_recv() {
            process_incoming_request(
                req,
                &mut documents,
                &mut render_queue,
                &response_tx,
                &egui_ctx,
                &mut should_shutdown,
            );
            if should_shutdown {
                break;
            }
        }

        if should_shutdown {
            break;
        }

        // Execute exactly one page rasterization job, then re-check the control channel.
        if let Some(job) = render_queue.pop_front() {
            if let Some(doc) = documents.get(&job.tab_id) {
                let scale = f32::from_bits(job.scale_bits);
                match doc.render_page(job.page_index, scale) {
                    Ok(rendered) => {
                        let _ = response_tx.send(RenderResponse::PageRendered {
                            tab_id: job.tab_id,
                            page_index: job.page_index,
                            zoom_pct: job.zoom_pct,
                            width: rendered.width,
                            height: rendered.height,
                            pixels: rendered.pixels,
                        });
                        if let Some(ctx) = &egui_ctx {
                            ctx.request_repaint();
                        }
                    }
                    Err(err) => {
                        let _ = response_tx.send(RenderResponse::DocumentError {
                            tab_id: job.tab_id,
                            error: err.to_string(),
                        });
                        if let Some(ctx) = &egui_ctx {
                            ctx.request_repaint();
                        }
                    }
                }
            }
        }
    }
}

/// Processes a single incoming request and updates internal worker state.
fn process_incoming_request(
    request: RenderRequest,
    documents: &mut HashMap<TabId, PdfEngineDocument>,
    render_queue: &mut VecDeque<QueuedPageJob>,
    response_tx: &Sender<RenderResponse>,
    egui_ctx: &Option<egui::Context>,
    should_shutdown: &mut bool,
) {
    match request {
        RenderRequest::OpenDocument { tab_id, path } => {
            render_queue.retain(|job| job.tab_id != tab_id);
            match PdfEngineDocument::open(&path) {
                Ok((doc, OpenOutcome::Ready { page_count, page_dimensions })) => {
                    documents.insert(tab_id, doc);
                    let _ = response_tx.send(RenderResponse::DocumentReady {
                        tab_id,
                        page_count,
                        page_dimensions,
                    });
                }
                Ok((doc, OpenOutcome::Locked)) => {
                    documents.insert(tab_id, doc);
                    let _ = response_tx.send(RenderResponse::DocumentLocked { tab_id });
                }
                Err(err) => {
                    documents.remove(&tab_id);
                    let _ = response_tx.send(RenderResponse::DocumentError {
                        tab_id,
                        error: err.to_string(),
                    });
                }
            }
            if let Some(ctx) = egui_ctx {
                ctx.request_repaint();
            }
        }

        RenderRequest::AuthenticatePassword {
            tab_id,
            mut password,
        } => {
            if let Some(doc) = documents.get_mut(&tab_id) {
                let auth_res = doc.authenticate_password(&mut password);
                // Explicitly zeroize the channel message container as well.
                password.zeroize();

                match auth_res {
                    Ok(true) => {
                        let _ = response_tx.send(RenderResponse::DocumentReady {
                            tab_id,
                            page_count: doc.page_count(),
                            page_dimensions: doc.page_dimensions().to_vec(),
                        });
                    }
                    Ok(false) => {
                        let _ = response_tx.send(RenderResponse::AuthenticationFailed {
                            tab_id,
                            error_msg: "Incorrect password".to_string(),
                        });
                    }
                    Err(err) => {
                        let _ = response_tx.send(RenderResponse::DocumentError {
                            tab_id,
                            error: err.to_string(),
                        });
                    }
                }
            } else {
                password.zeroize();
            }

            if let Some(ctx) = egui_ctx {
                ctx.request_repaint();
            }
        }

        RenderRequest::RenderPage {
            tab_id,
            page_index,
            zoom_pct,
            scale,
        } => {
            // Automatically drop any queued jobs for this tab that belong to a different zoom level
            render_queue.retain(|job| job.tab_id != tab_id || job.zoom_pct == zoom_pct);

            // Deduplicate: only enqueue if `(tab_id, page_index, zoom_pct)` is not already queued
            let already_queued = render_queue.iter().any(|job| {
                job.tab_id == tab_id && job.page_index == page_index && job.zoom_pct == zoom_pct
            });

            if !already_queued {
                render_queue.push_back(QueuedPageJob {
                    tab_id,
                    page_index,
                    zoom_pct,
                    scale_bits: scale.to_bits(),
                });
            }
        }

        RenderRequest::CancelObsolete {
            tab_id,
            active_zoom_pct,
            keep_range,
        } => {
            render_queue.retain(|job| {
                if job.tab_id != tab_id {
                    return true;
                }
                if job.zoom_pct != active_zoom_pct {
                    return false;
                }
                if let Some((start, end)) = keep_range {
                    job.page_index >= start && job.page_index <= end
                } else {
                    false
                }
            });
        }

        RenderRequest::CloseTab { tab_id } => {
            render_queue.retain(|job| job.tab_id != tab_id);
            documents.remove(&tab_id);
        }

        RenderRequest::Shutdown => {
            render_queue.clear();
            documents.clear();
            *should_shutdown = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf_engine::tests::build_encrypted_pdf;
    use std::fs;
    use std::time::Duration;

    #[test]
    fn test_worker_open_authenticate_and_render_flow() {
        let temp_dir = std::env::temp_dir();
        let pdf_path = temp_dir.join("deimos_worker_test_locked.pdf");
        fs::write(&pdf_path, build_encrypted_pdf("WorkerPass42", "OwnerPass42")).unwrap();

        let worker = PdfWorkerHandle::spawn(None);
        let tab_id = TabId(42);

        worker.send(RenderRequest::OpenDocument {
            tab_id,
            path: pdf_path.clone(),
        });

        let resp1 = worker
            .receiver()
            .recv_timeout(Duration::from_secs(5))
            .expect("worker open response");
        assert!(matches!(
            resp1,
            RenderResponse::DocumentLocked { tab_id: id } if id == tab_id
        ));

        // Wrong password
        worker.send(RenderRequest::AuthenticatePassword {
            tab_id,
            password: Zeroizing::new("WrongPass".to_string()),
        });
        let resp2 = worker
            .receiver()
            .recv_timeout(Duration::from_secs(5))
            .expect("failed auth response");
        assert!(matches!(
            resp2,
            RenderResponse::AuthenticationFailed { tab_id: id, .. } if id == tab_id
        ));

        // Correct password
        worker.send(RenderRequest::AuthenticatePassword {
            tab_id,
            password: Zeroizing::new("WorkerPass42".to_string()),
        });
        let resp3 = worker
            .receiver()
            .recv_timeout(Duration::from_secs(5))
            .expect("successful auth response");
        assert!(matches!(
            resp3,
            RenderResponse::DocumentReady { tab_id: id, page_count: 1, .. } if id == tab_id
        ));

        // Render page request
        worker.send(RenderRequest::RenderPage {
            tab_id,
            page_index: 0,
            zoom_pct: 100,
            scale: 1.0,
        });
        let resp4 = worker
            .receiver()
            .recv_timeout(Duration::from_secs(5))
            .expect("rendered page response");
        match resp4 {
            RenderResponse::PageRendered {
                tab_id: id,
                page_index,
                zoom_pct,
                width,
                height,
                pixels,
            } => {
                assert_eq!(id, tab_id);
                assert_eq!(page_index, 0);
                assert_eq!(zoom_pct, 100);
                assert_eq!(width, 595);
                assert_eq!(height, 842);
                assert_eq!(pixels.len(), 595 * 842 * 4);
            }
            other => panic!("Unexpected worker response: {other:?}"),
        }

        worker.send(RenderRequest::CloseTab { tab_id });
        let _ = fs::remove_file(&pdf_path);
    }
}
