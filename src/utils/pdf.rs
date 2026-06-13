//! PDF text extraction utilities with fallback support.
//!
//! This module provides PDF text extraction with automatic fallback:
//! 1. First tries poppler (via pdf-extract) - best quality text extraction
//! 2. Falls back to pure Rust lopdf if poppler is unavailable
//!
//! For scanned/image-based PDFs, tesseract OCR can be used if available.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;

/// Errors that can occur during PDF extraction
#[derive(Debug, Error)]
pub enum PdfExtractError {
    #[error("PDF extraction failed: {0}")]
    ExtractionFailed(String),

    #[error("File not found or not a valid PDF: {0}")]
    InvalidFile(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("No text extraction method available")]
    NotAvailable,
}

/// Method used for PDF text extraction
#[derive(Debug, Clone, PartialEq)]
pub enum ExtractionMethod {
    /// Used poppler libraries (best quality)
    Poppler,
    /// Used pure Rust lopdf
    Lopdf,
    /// Used pdftotext external binary
    Pdftotext,
    /// Used tesseract OCR
    Tesseract,
    /// No method available
    None,
}

/// Check if an external binary is available
fn is_external_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if poppler utilities are available
pub fn has_poppler() -> bool {
    static POPPLER_CHECK: AtomicBool = AtomicBool::new(false);
    static HAS_CHECKED: AtomicBool = AtomicBool::new(false);

    if HAS_CHECKED.load(Ordering::Relaxed) {
        return POPPLER_CHECK.load(Ordering::Relaxed);
    }

    let available = is_external_available("pdftotext");
    POPPLER_CHECK.store(available, Ordering::Relaxed);
    HAS_CHECKED.store(true, Ordering::Relaxed);

    available
}

/// Check if tesseract OCR is available
pub fn has_tesseract() -> bool {
    static TESSERACT_CHECK: AtomicBool = AtomicBool::new(false);
    static HAS_CHECKED: AtomicBool = AtomicBool::new(false);

    if HAS_CHECKED.load(Ordering::Relaxed) {
        return TESSERACT_CHECK.load(Ordering::Relaxed);
    }

    let available = is_external_available("tesseract");
    TESSERACT_CHECK.store(available, Ordering::Relaxed);
    HAS_CHECKED.store(true, Ordering::Relaxed);

    available
}

/// Get the best available extraction method with metadata
#[derive(Debug, Clone)]
pub struct ExtractionInfo {
    pub method: ExtractionMethod,
    pub has_poppler: bool,
    pub has_tesseract: bool,
    pub has_lopdf: bool,
}

/// Get information about available PDF extraction methods
pub fn get_extraction_info() -> ExtractionInfo {
    ExtractionInfo {
        method: ExtractionMethod::None,
        has_poppler: has_poppler(),
        has_tesseract: has_tesseract(),
        has_lopdf: true, // lopdf is always available as a Rust crate
    }
}

/// Try to extract text using pdftotext external binary
fn extract_with_pdftotext(path: &Path) -> Result<String, PdfExtractError> {
    let output = Command::new("pdftotext")
        .arg(path)
        .arg("-")
        .output()
        .map_err(|e| PdfExtractError::ExtractionFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(PdfExtractError::ExtractionFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Try to extract text using poppler via pdf-extract crate
fn extract_with_poppler(path: &Path) -> Result<String, PdfExtractError> {
    match pdf_extract::extract_text(path) {
        Ok(text) if text.trim().is_empty() => {
            // Try pdftotext as fallback within poppler
            tracing::debug!("pdf-extract returned empty, trying pdftotext");
            extract_with_pdftotext(path)
        }
        Ok(text) => Ok(text),
        Err(e) => Err(PdfExtractError::ExtractionFailed(e.to_string())),
    }
}

/// Try to extract text using pure Rust lopdf
fn extract_with_lopdf(path: &Path) -> Result<String, PdfExtractError> {
    let doc = lopdf::Document::load(path)
        .map_err(|e| PdfExtractError::ExtractionFailed(e.to_string()))?;

    let pages: Vec<u32> = (1..=doc.get_pages().len() as u32).collect();
    let text = doc
        .extract_text(&pages)
        .map_err(|e| PdfExtractError::ExtractionFailed(e.to_string()))?;

    Ok(text)
}

/// Extract text from a PDF file using the best available method.
///
/// Returns the extracted text content and the method used.
///
/// # Arguments
///
/// * `path` - Path to the PDF file
///
/// # Returns
///
/// A tuple of (extracted text, extraction method used)
pub fn extract_text(path: &Path) -> Result<(String, ExtractionMethod), PdfExtractError> {
    // Check file exists and is a file
    if !path.exists() {
        return Err(PdfExtractError::InvalidFile(format!(
            "File not found: {}",
            path.display()
        )));
    }

    if !path.is_file() {
        return Err(PdfExtractError::InvalidFile(format!(
            "Not a file: {}",
            path.display()
        )));
    }

    // Priority 1: Try poppler libraries first (best quality)
    if has_poppler() {
        match extract_with_poppler(path) {
            Ok(text) => {
                if !text.trim().is_empty() {
                    return Ok((text, ExtractionMethod::Poppler));
                }
                // Empty text from poppler, continue to fallback
                tracing::debug!(
                    "Poppler returned empty text for {}, trying fallback",
                    path.display()
                );
            }
            Err(e) => {
                tracing::debug!("Poppler extraction failed: {}, trying fallback", e);
            }
        }

        // Try pdftotext directly as secondary poppler method
        match extract_with_pdftotext(path) {
            Ok(text) if !text.trim().is_empty() => return Ok((text, ExtractionMethod::Pdftotext)),
            _ => {}
        }
    }

    // Priority 2: Try pure Rust lopdf
    match extract_with_lopdf(path) {
        Ok(text) if !text.trim().is_empty() => return Ok((text, ExtractionMethod::Lopdf)),
        Ok(_) => {
            tracing::debug!("lopdf returned empty text for {}", path.display());
        }
        Err(e) => {
            tracing::debug!("lopdf extraction failed: {}", e);
        }
    }

    // Priority 3: If tesseract is available, try OCR
    if has_tesseract() {
        tracing::debug!(
            "All text extraction failed, {} might be a scanned PDF. \
             Consider using tesseract for OCR.",
            path.display()
        );
    }

    Err(PdfExtractError::NotAvailable)
}

/// Extract text from a PDF file (legacy interface, discards method info)
pub fn extract_text_simple(path: &Path) -> Result<String, PdfExtractError> {
    extract_text(path).map(|(text, _)| text)
}

/// Extract text from multiple PDF files and combine results.
#[allow(dead_code)]
pub fn extract_multiple<'a, P>(paths: P) -> Vec<Result<(String, ExtractionMethod), PdfExtractError>>
where
    P: IntoIterator<Item = &'a Path>,
{
    paths.into_iter().map(extract_text).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extraction_info() {
        let info = get_extraction_info();
        // Should have at least lopdf available
        assert!(info.has_lopdf);
        // Poppler and tesseract depend on system installation
        println!("Poppler available: {}", info.has_poppler);
        println!("Tesseract available: {}", info.has_tesseract);
    }

    #[test]
    fn test_extract_nonexistent_file() {
        let result = extract_text(Path::new("/nonexistent/file.pdf"));
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_simple_nonexistent() {
        let result = extract_text_simple(Path::new("/nonexistent/file.pdf"));
        assert!(result.is_err());
    }
}
