//! Isolated wrapper around the PDF engine (file opening, encryption detection,
//! memory-safe authentication with `zeroize`, page dimension extraction, and RGBA8 rasterization).

use std::cell::RefCell;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use pdfium_render::prelude::*;
use zeroize::Zeroize;

/// Possible errors returned by the PDF engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    /// I/O error while reading the file from disk.
    Io(String),
    /// Native PDF library could not be found or loaded.
    LibraryLoad(String),
    /// The file is corrupted or is not a valid PDF document.
    Corrupted(String),
    /// The document is password-protected and requires authentication.
    Locked,
    /// Requested page index is out of bounds.
    PageOutOfBounds { page_index: usize, page_count: usize },
    /// Error while rasterizing a page.
    Render(String),
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "Failed to open file: {msg}"),
            Self::LibraryLoad(msg) => write!(f, "Failed to load PDF engine library: {msg}"),
            Self::Corrupted(msg) => write!(f, "Corrupted or unreadable PDF file: {msg}"),
            Self::Locked => write!(f, "Password-protected PDF document"),
            Self::PageOutOfBounds {
                page_index,
                page_count,
            } => write!(
                f,
                "Page {} is out of bounds (total pages: {})",
                page_index + 1,
                page_count
            ),
            Self::Render(msg) => write!(f, "Failed to render page: {msg}"),
        }
    }
}

impl std::error::Error for PdfError {}

/// Result of rasterizing a single PDF page in RAM (RGBA8).
#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub width: usize,
    pub height: usize,
    /// Raw RGBA8 pixel buffer (`width * height * 4` bytes).
    pub pixels: Vec<u8>,
}

/// Outcome of the initial attempt to open a PDF document.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenOutcome {
    /// The document is open (or unlocked automatically with `""`) and ready for rendering.
    Ready {
        page_count: usize,
        page_dimensions: Vec<(f32, f32)>,
    },
    /// The document is encrypted and requires a user password.
    Locked,
}

thread_local! {
    /// Thread-local PDFium instance dynamically bound to the native library.
    static PDFIUM_INSTANCE: RefCell<Option<&'static Pdfium>> = const { RefCell::new(None) };
}

/// Global mutex serializing FFI calls into the native PDFium library.
static PDFIUM_GUARD: Mutex<()> = Mutex::new(());

/// Loads or retrieves the thread-local PDFium instance.
fn get_pdfium() -> Result<&'static Pdfium, PdfError> {
    PDFIUM_INSTANCE.with(|cell| {
        if let Some(instance) = *cell.borrow() {
            return Ok(instance);
        }

        let _guard = PDFIUM_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let bindings = bind_pdfium_library()?;
        let leaked: &'static Pdfium = Box::leak(Box::new(Pdfium::new(bindings)));
        *cell.borrow_mut() = Some(leaked);
        Ok(leaked)
    })
}

/// Searches for the dynamic PDFium library in standard locations:
/// 1. `PDFIUM_LIB_PATH` environment variable
/// 2. Current executable directory and its parent directories
/// 3. Current working directory (`./`)
/// 4. Operating system library paths (`/usr/lib`, `PATH`, etc.)
fn bind_pdfium_library() -> Result<Box<dyn PdfiumLibraryBindings>, PdfError> {
    let mut candidate_dirs: Vec<PathBuf> = Vec::new();

    if let Ok(env_dir) = std::env::var("PDFIUM_LIB_PATH") {
        candidate_dirs.push(PathBuf::from(env_dir));
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidate_dirs.push(exe_dir.to_path_buf());
            if let Some(parent) = exe_dir.parent() {
                candidate_dirs.push(parent.to_path_buf());
                if let Some(grand_parent) = parent.parent() {
                    candidate_dirs.push(grand_parent.to_path_buf());
                }
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        candidate_dirs.push(cwd);
    }

    for dir in candidate_dirs {
        let lib_name = Pdfium::pdfium_platform_library_name_at_path(&dir);
        if let Ok(bindings) = Pdfium::bind_to_library(lib_name) {
            return Ok(bindings);
        }
    }

    Pdfium::bind_to_system_library().map_err(|err| {
        PdfError::LibraryLoad(format!(
            "Unable to load native PDFium library (pdfium.dll / libpdfium.so): {err}"
        ))
    })
}

/// Quickly checks whether the byte buffer contains a valid `%PDF-` header signature.
fn validate_pdf_header(bytes: &[u8]) -> Result<(), PdfError> {
    if bytes.len() < 8 {
        return Err(PdfError::Corrupted(
            "File is too short to be a valid PDF document.".to_string(),
        ));
    }
    let search_window = &bytes[..bytes.len().min(1024)];
    if !search_window.windows(5).any(|w| w == b"%PDF-") {
        return Err(PdfError::Corrupted(
            "Missing '%PDF-' header signature in file.".to_string(),
        ));
    }
    Ok(())
}

/// Detects whether the PDF byte buffer contains an `/Encrypt` dictionary.
fn contains_encrypt_dictionary(bytes: &[u8]) -> bool {
    bytes.windows(8).any(|w| w == b"/Encrypt")
}

/// Memory-safe wrapper around a PDF document managed by the worker thread.
pub struct PdfEngineDocument {
    /// Raw file bytes retained only while the document remains locked.
    encrypted_bytes: Vec<u8>,
    /// Indicates whether the document still requires password authentication.
    requires_password: bool,
    /// Unlocked PDFium document ready for rendering.
    unlocked_doc: Option<PdfDocument<'static>>,
    /// Pre-calculated total number of pages.
    page_count: usize,
    /// Pre-calculated `(width_pts, height_pts)` dimensions of each page at 1.0x scale.
    page_dimensions: Vec<(f32, f32)>,
}

impl PdfEngineDocument {
    /// Opens a PDF file from disk, detects whether it is encrypted (`needs_password()`),
    /// and if so, automatically tests the empty password `""`.
    pub fn open(path: impl AsRef<Path>) -> Result<(Self, OpenOutcome), PdfError> {
        let mut doc = Self::open_unauthenticated(path)?;

        if doc.needs_password() {
            // Automatically test the empty password "" first
            // (some PDFs have an owner password set but an empty user password).
            let mut empty_password = String::new();
            let unlocked = doc.authenticate_password(&mut empty_password)?;
            if !unlocked {
                return Ok((doc, OpenOutcome::Locked));
            }
        }

        let outcome = OpenOutcome::Ready {
            page_count: doc.page_count(),
            page_dimensions: doc.page_dimensions().to_vec(),
        };
        Ok((doc, outcome))
    }

    /// Loads the file and immediately checks whether it requires a password,
    /// without attempting the automatic empty-password unlock yet.
    pub fn open_unauthenticated(path: impl AsRef<Path>) -> Result<Self, PdfError> {
        let bytes = fs::read(path.as_ref()).map_err(|e| PdfError::Io(e.to_string()))?;
        Self::from_bytes_unauthenticated(bytes)
    }

    /// Initializes a document from an in-memory byte buffer while immediately detecting encryption.
    pub fn from_bytes_unauthenticated(bytes: Vec<u8>) -> Result<Self, PdfError> {
        validate_pdf_header(&bytes)?;
        let pdfium = get_pdfium()?;

        if contains_encrypt_dictionary(&bytes) {
            return Ok(Self {
                encrypted_bytes: bytes,
                requires_password: true,
                unlocked_doc: None,
                page_count: 0,
                page_dimensions: Vec::new(),
            });
        }

        let _guard = PDFIUM_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        match pdfium.load_pdf_from_byte_vec(bytes.clone(), None) {
            Ok(pdf_doc) => {
                let (page_count, page_dimensions) = Self::extract_metadata(&pdf_doc)?;
                Ok(Self {
                    encrypted_bytes: Vec::new(),
                    requires_password: false,
                    unlocked_doc: Some(pdf_doc),
                    page_count,
                    page_dimensions,
                })
            }
            Err(PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError)) => {
                Ok(Self {
                    encrypted_bytes: bytes,
                    requires_password: true,
                    unlocked_doc: None,
                    page_count: 0,
                    page_dimensions: Vec::new(),
                })
            }
            Err(err) => Err(PdfError::Corrupted(err.to_string())),
        }
    }

    /// Returns `true` if the document still requires a password.
    pub fn needs_password(&self) -> bool {
        self.requires_password
    }

    /// Attempts to unlock the document with the provided password and **immediately**
    /// wipes the `password` string from RAM via `.zeroize()`.
    ///
    /// Returns:
    /// - `Ok(true)` if decryption succeeded (page dimensions are pre-calculated).
    /// - `Ok(false)` if the password is incorrect.
    /// - `Err(PdfError)` if the underlying file is corrupted.
    pub fn authenticate_password(&mut self, password: &mut String) -> Result<bool, PdfError> {
        if !self.requires_password {
            password.zeroize();
            return Ok(true);
        }

        let pdfium = match get_pdfium() {
            Ok(p) => p,
            Err(e) => {
                password.zeroize();
                return Err(e);
            }
        };

        let _guard = PDFIUM_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let load_result =
            pdfium.load_pdf_from_byte_vec(self.encrypted_bytes.clone(), Some(password.as_str()));

        // Immediately wipe the cleartext password from RAM after calling the PDF engine.
        password.zeroize();

        match load_result {
            Ok(pdf_doc) => {
                let (page_count, page_dimensions) = Self::extract_metadata(&pdf_doc)?;
                self.encrypted_bytes.zeroize();
                self.encrypted_bytes.clear();
                self.encrypted_bytes.shrink_to_fit();
                self.requires_password = false;
                self.page_count = page_count;
                self.page_dimensions = page_dimensions;
                self.unlocked_doc = Some(pdf_doc);
                Ok(true)
            }
            Err(PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError)) => {
                Ok(false)
            }
            Err(err) => Err(PdfError::Corrupted(err.to_string())),
        }
    }

    /// Returns the total number of pages in the document (`0` while locked).
    pub fn page_count(&self) -> usize {
        self.page_count
    }

    /// Returns the pre-calculated `(width_pts, height_pts)` dimensions of all pages.
    pub fn page_dimensions(&self) -> &[(f32, f32)] {
        &self.page_dimensions
    }

    /// Rasterizes a given page (`page_index`, 0-indexed) at `scale` into an RGBA8 pixel buffer (`Vec<u8>`).
    pub fn render_page(&self, page_index: usize, scale: f32) -> Result<RenderedPage, PdfError> {
        if self.requires_password {
            return Err(PdfError::Locked);
        }

        let doc = self
            .unlocked_doc
            .as_ref()
            .ok_or_else(|| PdfError::Corrupted("Document is not initialized.".to_string()))?;

        if page_index >= self.page_count {
            return Err(PdfError::PageOutOfBounds {
                page_index,
                page_count: self.page_count,
            });
        }

        let page_idx_u16 = u16::try_from(page_index).map_err(|_| PdfError::PageOutOfBounds {
            page_index,
            page_count: self.page_count,
        })?;

        let _guard = PDFIUM_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let page = doc
            .pages()
            .get(page_idx_u16)
            .map_err(|e| PdfError::Render(e.to_string()))?;

        let (w_pts, h_pts) = self.page_dimensions[page_index];
        let clamped_scale = scale.clamp(0.1, 8.0);
        let target_width = ((w_pts * clamped_scale).round() as i32).max(1);
        let target_height = ((h_pts * clamped_scale).round() as i32).max(1);

        // Strictly disable form data and interactive annotations per project scope.
        let render_config = PdfRenderConfig::new()
            .set_target_width(target_width)
            .set_target_height(target_height)
            .render_form_data(false)
            .render_annotations(false);

        let bitmap = page
            .render_with_config(&render_config)
            .map_err(|e| PdfError::Render(e.to_string()))?;

        let width = usize::try_from(bitmap.width()).unwrap_or(target_width as usize);
        let height = usize::try_from(bitmap.height()).unwrap_or(target_height as usize);
        let pixels = bitmap.as_rgba_bytes();

        Ok(RenderedPage {
            width,
            height,
            pixels,
        })
    }

    /// Extracts the total page count and the dimensions in points of every page.
    fn extract_metadata(doc: &PdfDocument<'_>) -> Result<(usize, Vec<(f32, f32)>), PdfError> {
        let count_u16 = doc.pages().len();
        let page_count = usize::from(count_u16);
        if page_count == 0 {
            return Err(PdfError::Corrupted(
                "The PDF document contains no pages.".to_string(),
            ));
        }

        let mut dimensions = Vec::with_capacity(page_count);
        for idx in 0..count_u16 {
            let page = doc
                .pages()
                .get(idx)
                .map_err(|e| PdfError::Corrupted(e.to_string()))?;
            let width = page.width().value.max(1.0);
            let height = page.height().value.max(1.0);
            dimensions.push((width, height));
        }

        Ok((page_count, dimensions))
    }
}

impl Drop for PdfEngineDocument {
    fn drop(&mut self) {
        self.encrypted_bytes.zeroize();
        if self.unlocked_doc.is_some() {
            let _guard = PDFIUM_GUARD.lock().unwrap_or_else(|e| e.into_inner());
            let _ = self.unlocked_doc.take();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tab::{Tab, TabId, TabState};
    use std::io::Write;

    /// Generates a valid multi-page unencrypted PDF 1.4 document in memory for unit tests.
    fn build_sample_unencrypted_pdf() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

        let obj1_offset = out.len();
        out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

        let obj2_offset = out.len();
        out.extend_from_slice(
            b"2 0 obj\n<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>\nendobj\n",
        );

        let obj3_offset = out.len();
        out.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 5 0 R >>\nendobj\n",
        );

        let obj4_offset = out.len();
        out.extend_from_slice(
            b"4 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R >>\nendobj\n",
        );

        let stream_data = b"q 0.9 0.9 0.9 rg 50 50 200 200 re f Q";
        let obj5_offset = out.len();
        write!(
            &mut out,
            "5 0 obj\n<< /Length {} >>\nstream\n",
            stream_data.len()
        )
        .unwrap();
        out.extend_from_slice(stream_data);
        out.extend_from_slice(b"\nendstream\nendobj\n");

        let xref_offset = out.len();
        write!(
            &mut out,
            "xref\n0 6\n\
             0000000000 65535 f \n\
             {:010} 00000 n \n\
             {:010} 00000 n \n\
             {:010} 00000 n \n\
             {:010} 00000 n \n\
             {:010} 00000 n \n\
             trailer\n<< /Size 6 /Root 1 0 R >>\n\
             startxref\n{}\n%%EOF\n",
            obj1_offset, obj2_offset, obj3_offset, obj4_offset, obj5_offset, xref_offset
        )
        .unwrap();

        out
    }

    /// Minimal RFC 1321 MD5 implementation used exclusively in `#[cfg(test)]`
    /// to generate standard-compliant encrypted test PDFs (ISO 32000-1 RC4 40-bit, R=2).
    fn md5_digest(input: &[u8]) -> [u8; 16] {
        const S: [u32; 64] = [
            7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14,
            20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11,
            16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
        ];
        const K: [u32; 64] = [
            0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
            0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
            0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
            0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
            0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
            0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
            0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
            0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
            0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
            0xeb86d391,
        ];

        let mut msg = input.to_vec();
        let bit_len = (msg.len() as u64).wrapping_mul(8);
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bit_len.to_le_bytes());

        let mut a0: u32 = 0x67452301;
        let mut b0: u32 = 0xefcdab89;
        let mut c0: u32 = 0x98badcfe;
        let mut d0: u32 = 0x10325476;

        for chunk in msg.as_chunks::<64>().0 {
            let mut m = [0u32; 16];
            for (i, word) in m.iter_mut().enumerate() {
                let start = i * 4;
                *word = u32::from_le_bytes([
                    chunk[start],
                    chunk[start + 1],
                    chunk[start + 2],
                    chunk[start + 3],
                ]);
            }

            let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
            for i in 0..64 {
                let (f, g) = match i {
                    0..=15 => ((b & c) | (!b & d), i),
                    16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                    32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                    _ => (c ^ (b | !d), (7 * i) % 16),
                };
                let temp = d;
                d = c;
                c = b;
                b = b.wrapping_add(
                    a.wrapping_add(f)
                        .wrapping_add(K[i])
                        .wrapping_add(m[g])
                        .rotate_left(S[i]),
                );
                a = temp;
            }
            a0 = a0.wrapping_add(a);
            b0 = b0.wrapping_add(b);
            c0 = c0.wrapping_add(c);
            d0 = d0.wrapping_add(d);
        }

        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&a0.to_le_bytes());
        out[4..8].copy_from_slice(&b0.to_le_bytes());
        out[8..12].copy_from_slice(&c0.to_le_bytes());
        out[12..16].copy_from_slice(&d0.to_le_bytes());
        out
    }

    fn rc4_crypt(key: &[u8], data: &[u8]) -> Vec<u8> {
        let mut s = [0u8; 256];
        for (i, val) in s.iter_mut().enumerate() {
            *val = i as u8;
        }
        let mut j: usize = 0;
        for i in 0..256 {
            j = (j + usize::from(s[i]) + usize::from(key[i % key.len()])) & 0xFF;
            s.swap(i, j);
        }
        let mut i: usize = 0;
        j = 0;
        let mut out = Vec::with_capacity(data.len());
        for &byte in data {
            i = (i + 1) & 0xFF;
            j = (j + usize::from(s[i])) & 0xFF;
            s.swap(i, j);
            let k = s[(usize::from(s[i]) + usize::from(s[j])) & 0xFF];
            out.push(byte ^ k);
        }
        out
    }

    pub(crate) fn build_encrypted_pdf(user_password: &str, owner_password: &str) -> Vec<u8> {
        const PADDING: [u8; 32] = [
            0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA,
            0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE,
            0x64, 0x53, 0x69, 0x7A,
        ];

        fn pad_password(pwd: &str) -> [u8; 32] {
            let mut out = [0u8; 32];
            let bytes = pwd.as_bytes();
            let copy_len = bytes.len().min(32);
            out[..copy_len].copy_from_slice(&bytes[..copy_len]);
            if copy_len < 32 {
                out[copy_len..].copy_from_slice(&PADDING[..(32 - copy_len)]);
            }
            out
        }

        let file_id: [u8; 16] = *b"DeimosReaderTest";
        let permissions: i32 = -4;

        let owner_source = if owner_password.is_empty() {
            user_password
        } else {
            owner_password
        };
        let owner_padded = pad_password(owner_source);
        let owner_md5 = md5_digest(&owner_padded);
        let owner_rc4_key = &owner_md5[..5];
        let user_padded = pad_password(user_password);
        let o_value = rc4_crypt(owner_rc4_key, &user_padded);

        let mut key_input = Vec::new();
        key_input.extend_from_slice(&user_padded);
        key_input.extend_from_slice(&o_value);
        key_input.extend_from_slice(&permissions.to_le_bytes());
        key_input.extend_from_slice(&file_id);
        let file_key_md5 = md5_digest(&key_input);
        let encryption_key = &file_key_md5[..5];

        let u_value = rc4_crypt(encryption_key, &PADDING);

        fn to_hex(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02X}")).collect()
        }

        let plain_stream = b"q 0.2 0.4 0.8 rg 40 40 120 120 re f Q";
        let mut obj4_key_input = Vec::from(encryption_key);
        obj4_key_input.extend_from_slice(&[4, 0, 0, 0, 0]);
        let obj4_md5 = md5_digest(&obj4_key_input);
        let obj4_rc4_key = &obj4_md5[..10];
        let encrypted_stream = rc4_crypt(obj4_rc4_key, plain_stream);

        let mut out = Vec::new();
        out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

        let obj1_offset = out.len();
        out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

        let obj2_offset = out.len();
        out.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

        let obj3_offset = out.len();
        out.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R >>\nendobj\n",
        );

        let obj4_offset = out.len();
        write!(
            &mut out,
            "4 0 obj\n<< /Length {} >>\nstream\n",
            encrypted_stream.len()
        )
        .unwrap();
        out.extend_from_slice(&encrypted_stream);
        out.extend_from_slice(b"\nendstream\nendobj\n");

        let obj5_offset = out.len();
        write!(
            &mut out,
            "5 0 obj\n<< /Filter /Standard /V 1 /R 2 /Length 40 /P {} /O <{}> /U <{}> >>\nendobj\n",
            permissions,
            to_hex(&o_value),
            to_hex(&u_value)
        )
        .unwrap();

        let id_hex = to_hex(&file_id);
        let xref_offset = out.len();
        write!(
            &mut out,
            "xref\n0 6\n\
             0000000000 65535 f \n\
             {:010} 00000 n \n\
             {:010} 00000 n \n\
             {:010} 00000 n \n\
             {:010} 00000 n \n\
             {:010} 00000 n \n\
             trailer\n<< /Size 6 /Root 1 0 R /Encrypt 5 0 R /ID [<{}> <{}>] >>\n\
             startxref\n{}\n%%EOF\n",
            obj1_offset,
            obj2_offset,
            obj3_offset,
            obj4_offset,
            obj5_offset,
            id_hex,
            id_hex,
            xref_offset
        )
        .unwrap();

        out
    }

    #[test]
    fn test_standard_document_state_machine_and_rasterization() {
        let temp_dir = std::env::temp_dir();
        let pdf_path = temp_dir.join("deimos_test_standard.pdf");
        fs::write(&pdf_path, build_sample_unencrypted_pdf()).expect("write test PDF");

        let mut tab = Tab::new(TabId(1), pdf_path.clone());
        assert_eq!(tab.state, TabState::Loading);

        let (doc, outcome) = PdfEngineDocument::open(&pdf_path).expect("open standard PDF");
        assert!(!doc.needs_password());

        match outcome {
            OpenOutcome::Ready {
                page_count,
                page_dimensions,
            } => {
                tab.transition_to_ready(page_count, page_dimensions);
            }
            OpenOutcome::Locked => panic!("Standard PDF must not be Locked"),
        }

        assert_eq!(
            tab.state,
            TabState::Ready {
                page_count: 2,
                page_dimensions: vec![(595.0, 842.0), (612.0, 792.0)],
            }
        );

        let rendered = doc.render_page(0, 1.0).expect("rasterize page 0");
        assert_eq!(rendered.width, 595);
        assert_eq!(rendered.height, 842);
        assert_eq!(rendered.pixels.len(), 595 * 842 * 4);

        let _ = fs::remove_file(&pdf_path);
    }

    #[test]
    fn test_owner_password_with_empty_user_password_auto_unlocks() {
        let temp_dir = std::env::temp_dir();
        let pdf_path = temp_dir.join("deimos_test_empty_user_pwd.pdf");
        fs::write(&pdf_path, build_encrypted_pdf("", "OwnerOnlySecret")).unwrap();

        let (doc, outcome) =
            PdfEngineDocument::open(&pdf_path).expect("open with empty password auto-test");
        assert!(!doc.needs_password());
        assert_eq!(
            outcome,
            OpenOutcome::Ready {
                page_count: 1,
                page_dimensions: vec![(595.0, 842.0)],
            }
        );

        let _ = fs::remove_file(&pdf_path);
    }

    #[test]
    fn test_locked_document_state_machine_and_zeroize() {
        let temp_dir = std::env::temp_dir();
        let pdf_path = temp_dir.join("deimos_test_locked.pdf");
        fs::write(
            &pdf_path,
            build_encrypted_pdf("DeimosSecret2026!", "OwnerSecret2026!"),
        )
        .unwrap();

        let mut tab = Tab::new(TabId(2), pdf_path.clone());
        assert_eq!(tab.state, TabState::Loading);

        let (mut doc, outcome) =
            PdfEngineDocument::open(&pdf_path).expect("initial open of encrypted PDF");
        assert!(doc.needs_password());
        assert_eq!(outcome, OpenOutcome::Locked);

        tab.transition_to_locked();
        assert_eq!(
            tab.state,
            TabState::Locked {
                retry_count: 0,
                error_msg: None,
            }
        );

        // 1. Test with an incorrect password
        tab.password_input = "WrongPassword123".to_string();
        let mut wrong_pwd = tab.take_password_zeroized();
        assert!(tab.password_input.is_empty());

        let auth_ok = doc
            .authenticate_password(&mut wrong_pwd)
            .expect("authentication attempt");
        assert!(wrong_pwd.is_empty());
        assert!(!auth_ok);
        assert!(doc.needs_password());

        tab.record_failed_auth("Incorrect password");
        assert_eq!(
            tab.state,
            TabState::Locked {
                retry_count: 1,
                error_msg: Some("Incorrect password".to_string()),
            }
        );

        // 2. Test with the valid password
        tab.password_input = "DeimosSecret2026!".to_string();
        let mut valid_pwd = tab.take_password_zeroized();
        assert!(tab.password_input.is_empty());

        let auth_ok = doc
            .authenticate_password(&mut valid_pwd)
            .expect("valid authentication");
        assert!(valid_pwd.is_empty());
        assert!(auth_ok);
        assert!(!doc.needs_password());

        tab.transition_to_ready(doc.page_count(), doc.page_dimensions().to_vec());
        assert_eq!(
            tab.state,
            TabState::Ready {
                page_count: 1,
                page_dimensions: vec![(595.0, 842.0)],
            }
        );

        let rendered = doc
            .render_page(0, 0.5)
            .expect("rasterize after decryption");
        assert_eq!(rendered.width, 298);
        assert_eq!(rendered.height, 421);
        assert_eq!(rendered.pixels.len(), 298 * 421 * 4);

        let _ = fs::remove_file(&pdf_path);
    }

    #[test]
    fn test_corrupted_document_transitions_to_error() {
        let temp_dir = std::env::temp_dir();
        let bad_path = temp_dir.join("deimos_test_corrupted.pdf");
        fs::write(&bad_path, b"Not a valid PDF file at all").unwrap();

        let mut tab = Tab::new(TabId(3), bad_path.clone());
        let res = PdfEngineDocument::open(&bad_path);
        assert!(res.is_err());
        if let Err(err) = res {
            tab.transition_to_error(err.to_string());
        }
        assert!(matches!(tab.state, TabState::Error(_)));

        let _ = fs::remove_file(&bad_path);
    }
}
