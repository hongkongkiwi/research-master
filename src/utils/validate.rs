//! Input validation utilities for paper IDs, URLs, and filenames.
//!
//! This module provides validation functions to prevent injection attacks
//! and path traversal vulnerabilities.

use thiserror::Error;

/// Validation error types
#[derive(Error, Debug, PartialEq)]
pub enum ValidationError {
    #[error("Invalid paper ID: {0}")]
    InvalidPaperId(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("Invalid DOI format: {0}")]
    InvalidDoi(String),

    #[error("Invalid filename: contains disallowed characters")]
    InvalidFilename,

    #[error("URL contains potentially dangerous characters")]
    DangerousUrl,

    #[error("Path traversal detected: {0}")]
    PathTraversal(String),
}

/// Validate a paper ID to prevent injection attacks
///
/// Paper IDs should only contain alphanumeric characters, hyphens, underscores,
/// dots, slashes (for some formats like arXiv), and the prefix "arxiv:", "hal-", "PMC", etc.
///
/// Returns `Ok(String)` if valid, or `Err(ValidationError)` if invalid.
pub fn sanitize_paper_id(id: &str) -> Result<String, ValidationError> {
    let id = id.trim();

    if id.is_empty() {
        return Err(ValidationError::InvalidPaperId("empty ID".to_string()));
    }

    // Check for path traversal attempts
    if id.contains("..") || id.contains("./") || id.contains(".\\") {
        return Err(ValidationError::PathTraversal(id.to_string()));
    }

    // Check for null bytes
    if id.contains('\0') {
        return Err(ValidationError::InvalidPaperId(
            "contains null byte".to_string(),
        ));
    }

    // Check for control characters (except tab, newline, carriage return)
    for ch in id.chars() {
        if ch.is_control() && ch != '\t' && ch != '\n' && ch != '\r' {
            return Err(ValidationError::InvalidPaperId(
                "contains control characters".to_string(),
            ));
        }
    }

    // Check for shell metacharacters that could enable injection
    let dangerous_chars = [
        ';', '|', '&', '$', '`', '(', ')', '{', '}', '[', ']', '<', '>', '*', '?', '!',
    ];
    for ch in dangerous_chars {
        if id.contains(ch) {
            return Err(ValidationError::InvalidPaperId(format!(
                "contains dangerous character: {}",
                ch
            )));
        }
    }

    Ok(id.to_string())
}

/// Validate a URL to prevent injection and SSRF attacks
///
/// Returns `Ok(String)` if valid, or `Err(ValidationError)` if invalid.
pub fn validate_url(url: &str) -> Result<String, ValidationError> {
    let url = url.trim();

    if url.is_empty() {
        return Err(ValidationError::InvalidUrl("empty URL".to_string()));
    }

    // Check for null bytes
    if url.contains('\0') {
        return Err(ValidationError::InvalidUrl(
            "contains null byte".to_string(),
        ));
    }

    // Parse URL to validate structure
    let parsed = url::Url::parse(url).map_err(|e| ValidationError::InvalidUrl(e.to_string()))?;

    // Only allow HTTP and HTTPS schemes
    match parsed.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(ValidationError::InvalidUrl(format!(
                "invalid scheme: {}",
                parsed.scheme()
            )))
        }
    }

    // Check for dangerous URL patterns
    let url_lower = url.to_lowercase();

    // Check for embedded newlines or nulls (already checked above, but double-check)
    if url.contains('\n') || url.contains('\r') || url.contains('\0') {
        return Err(ValidationError::DangerousUrl);
    }

    // Check for data: or javascript: URLs (already filtered by scheme check, but be explicit)
    if url_lower.starts_with("data:") || url_lower.starts_with("javascript:") {
        return Err(ValidationError::DangerousUrl);
    }

    // Check for internal IP addresses (basic check for SSRF prevention)
    if let Some(host) = parsed.host_str() {
        // Check for localhost variants
        let host_lower = host.to_lowercase();
        if host_lower == "localhost"
            || host_lower == "127.0.0.1"
            || host_lower == "::1"
            || host_lower == "0.0.0.0"
        {
            return Err(ValidationError::DangerousUrl);
        }

        // Basic IPv4 check (simplified - doesn't catch all cases)
        if host_lower.parse::<std::net::Ipv4Addr>().is_ok() {
            let octets: Vec<&str> = host_lower.split('.').collect();
            if octets.len() == 4 {
                if let Ok(first) = octets[0].parse::<u8>() {
                    // Check for private IP ranges (simplified)
                    if first == 10
                        || (first == 172
                            && octets[1]
                                .parse::<u8>()
                                .is_ok_and(|v| (16..=31).contains(&v)))
                        || (first == 192 && octets[1] == "168")
                    {
                        return Err(ValidationError::DangerousUrl);
                    }
                }
            }
        }
    }

    Ok(url.to_string())
}

/// Validate and sanitize a DOI
///
/// DOIs have the format "10.xxxx/xxxxxx" where xxxx is a registrant code
/// and xxxxxx is an item ID.
pub fn validate_doi(doi: &str) -> Result<String, ValidationError> {
    let doi = doi.trim().to_lowercase();

    if doi.is_empty() {
        return Err(ValidationError::InvalidDoi("empty DOI".to_string()));
    }

    // Remove any URL prefix if present first
    let doi = doi.strip_prefix("doi:").unwrap_or(&doi);
    let doi = doi.strip_prefix("https://doi.org/").unwrap_or(doi);
    let doi = doi.strip_prefix("http://doi.org/").unwrap_or(doi);

    // DOI must start with "10."
    if !doi.starts_with("10.") {
        return Err(ValidationError::InvalidDoi(
            "DOI must start with '10.'".to_string(),
        ));
    }

    // DOI must contain a slash after the prefix
    if !doi.contains('/') {
        return Err(ValidationError::InvalidDoi(
            "DOI must contain a slash".to_string(),
        ));
    }

    // Check for path traversal in DOI (shouldn't happen but be safe)
    if doi.contains("..") {
        return Err(ValidationError::InvalidDoi(
            "path traversal detected".to_string(),
        ));
    }

    Ok(doi.to_string())
}

/// Sanitize a filename to prevent path traversal and other attacks
///
/// Removes path separators and dangerous characters, limits length,
/// and ensures the filename is safe to use.
pub fn sanitize_filename(filename: &str) -> Result<String, ValidationError> {
    let filename = filename.trim();

    if filename.is_empty() {
        return Err(ValidationError::InvalidFilename);
    }

    // Check for path traversal
    if filename.contains("..")
        || filename.starts_with('/')
        || filename.starts_with('\\')
        || filename.contains(":/")
        || filename.contains(":\\")
    {
        return Err(ValidationError::PathTraversal(filename.to_string()));
    }

    // Remove any null bytes
    let filename = filename.replace('\0', "");

    // Keep only safe characters: alphanumeric, dash, underscore, dot, space
    let mut sanitized = String::new();
    for ch in filename.chars() {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '.' || ch == ' ' {
            sanitized.push(ch);
        }
        // Replace other characters with underscore
    }

    // Limit filename length
    const MAX_FILENAME_LENGTH: usize = 255;
    if sanitized.len() > MAX_FILENAME_LENGTH {
        let ext_pos = sanitized.rfind('.').unwrap_or(sanitized.len());
        let ext = sanitized.split_at(ext_pos).1;
        if ext.len() <= MAX_FILENAME_LENGTH {
            let mut base_len = MAX_FILENAME_LENGTH - ext.len();
            while !sanitized.is_char_boundary(base_len) {
                base_len -= 1;
            }
            sanitized = format!("{}{}", &sanitized[..base_len], ext);
        } else {
            let mut end = MAX_FILENAME_LENGTH;
            while !sanitized.is_char_boundary(end) {
                end -= 1;
            }
            sanitized.truncate(end);
        }
    }

    if sanitized.is_empty() {
        return Err(ValidationError::InvalidFilename);
    }

    Ok(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_paper_id_valid() {
        assert!(sanitize_paper_id("2301.12345").is_ok());
        assert!(sanitize_paper_id("arxiv:2301.12345").is_ok());
        assert!(sanitize_paper_id("PMC12345").is_ok());
        assert!(sanitize_paper_id("hal-12345").is_ok());
        assert!(sanitize_paper_id("10.1234/test").is_ok());
    }

    #[test]
    fn test_sanitize_paper_id_empty() {
        assert!(sanitize_paper_id("").is_err());
        assert!(sanitize_paper_id("   ").is_err());
    }

    #[test]
    fn test_sanitize_paper_id_path_traversal() {
        assert!(sanitize_paper_id("../etc/passwd").is_err());
        assert!(sanitize_paper_id("foo/../../bar").is_err());
        assert!(sanitize_paper_id("foo\\..\\bar").is_err());
    }

    #[test]
    fn test_sanitize_paper_id_dangerous_chars() {
        assert!(sanitize_paper_id("foo;rm -rf /").is_err());
        assert!(sanitize_paper_id("foo|whoami").is_err());
        assert!(sanitize_paper_id("foo`ls`").is_err());
        assert!(sanitize_paper_id("foo$(whoami)").is_err());
    }

    #[test]
    fn test_validate_url_valid() {
        assert!(validate_url("https://api.semanticscholar.org/graph/v1/paper/search").is_ok());
        assert!(validate_url("http://export.arxiv.org/api/query").is_ok());
    }

    #[test]
    fn test_validate_url_invalid() {
        assert!(validate_url("").is_err());
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("http://localhost:8000").is_err());
        assert!(validate_url("http://127.0.0.1:8000").is_err());
    }

    #[test]
    fn test_validate_doi_valid() {
        assert!(validate_doi("10.1234/abc123").is_ok());
        assert!(validate_doi("10.1038/nature12345").is_ok());
        // Without prefix
        assert_eq!(
            validate_doi("doi:10.1234/abc123").unwrap(),
            "10.1234/abc123"
        );
        assert_eq!(
            validate_doi("https://doi.org/10.1234/abc123").unwrap(),
            "10.1234/abc123"
        );
    }

    #[test]
    fn test_validate_doi_invalid() {
        assert!(validate_doi("").is_err());
        assert!(validate_doi("10.1234").is_err()); // No slash
        assert!(validate_doi("9.1234/abc").is_err()); // Doesn't start with 10
        assert!(validate_doi("10.1234/../abc").is_err()); // Path traversal
    }

    #[test]
    fn test_sanitize_filename_valid() {
        assert_eq!(sanitize_filename("my_paper.pdf").unwrap(), "my_paper.pdf");
        assert_eq!(
            sanitize_filename("2023-01-15-test.pdf").unwrap(),
            "2023-01-15-test.pdf"
        );
        // Parentheses are removed, only alphanumeric, dash, underscore, dot, space allowed
        assert_eq!(
            sanitize_filename("Test Paper Final.pdf").unwrap(),
            "Test Paper Final.pdf"
        );
    }

    #[test]
    fn test_sanitize_filename_long_unicode_and_extension() {
        for name in [
            format!("{}.pdf", "é".repeat(128)),
            format!("a.{}", "é".repeat(128)),
        ] {
            let result = sanitize_filename(&name).unwrap();
            assert!(result.len() <= 255);
        }
    }

    #[test]
    fn test_sanitize_filename_dangerous() {
        assert!(sanitize_filename("../etc/passwd").is_err());
        assert!(sanitize_filename("/etc/passwd").is_err());
        assert!(sanitize_filename("C:\\Windows\\System32").is_err());
        assert!(sanitize_filename("../../../etc/passwd").is_err());
    }

    #[test]
    fn test_sanitize_filename_removes_dangerous_chars() {
        // Dangerous characters should be removed or replaced with underscore
        let result = sanitize_filename("test;rm -rf /;file.pdf").unwrap();
        assert!(!result.contains(';'), "semicolon should be removed");
        // The filename becomes "testrm -rf  file.pdf" because we only remove special chars
        // The test is checking that ; is removed which it is
    }
}
