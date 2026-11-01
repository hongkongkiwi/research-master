//! Fetch and unpack arXiv LaTeX e-print sources.

use std::io::{Cursor, Read};

use flate2::read::GzDecoder;

use crate::sources::SourceError;
use crate::utils::{api_retry_config, with_retry, HttpClient};

/// Upper bound on the returned LaTeX text, in characters.
const MAX_MAIN_TEX_CHARS: usize = 200_000;

/// File listing plus the main TeX source of an arXiv e-print.
#[derive(Debug, Clone)]
pub struct LatexSource {
    pub files: Vec<String>,
    pub main_tex: Option<String>,
}

/// Download the e-print bundle for `arxiv_id` (`2310.12345`,
/// `arXiv:2310.12345`, `2301.00001v2`) and return its TeX contents.
pub async fn fetch_arxiv_latex(arxiv_id: &str) -> Result<LatexSource, SourceError> {
    let id = normalize_arxiv_id(arxiv_id)?;
    let url = format!("https://export.arxiv.org/e-print/{id}");
    let client = HttpClient::new()?;
    let bytes = with_retry(api_retry_config(), || {
        let client = client.clone();
        let url = url.clone();
        async move { fetch_eprint(&client, &url).await }
    })
    .await?;
    parse_eprint(&bytes)
}

/// Strip an optional `arXiv:` prefix and reject ids that cannot appear in a URL.
fn normalize_arxiv_id(arxiv_id: &str) -> Result<String, SourceError> {
    let trimmed = arxiv_id.trim();
    let stripped = match trimmed.get(..6) {
        Some(prefix) if prefix.eq_ignore_ascii_case("arxiv:") => &trimmed[6..],
        _ => trimmed,
    };
    if stripped.is_empty() || stripped.chars().any(char::is_whitespace) {
        return Err(SourceError::InvalidRequest(format!(
            "Invalid arXiv id: '{arxiv_id}'"
        )));
    }
    Ok(stripped.to_string())
}

async fn fetch_eprint(client: &HttpClient, url: &str) -> Result<Vec<u8>, SourceError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| SourceError::Network(format!("e-print request failed: {e}")))?;
    let status = response.status();
    if !status.is_success() {
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(SourceError::NotFound(format!("No e-print found at {url}")));
        }
        return Err(SourceError::Network(format!(
            "e-print request returned HTTP {status}"
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| SourceError::Network(format!("Failed to read e-print response: {e}")))?;
    Ok(bytes.to_vec())
}

/// Decode an e-print payload: a tar.gz bundle, a gzipped single .tex file,
/// or a bare .tex file. ZIP bundles are rejected.
fn parse_eprint(bytes: &[u8]) -> Result<LatexSource, SourceError> {
    if bytes.starts_with(b"PK\x03\x04") {
        return Err(SourceError::Other(
            "ZIP arXiv e-prints are unsupported".to_string(),
        ));
    }
    let decoded: Vec<u8> = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        GzDecoder::new(bytes)
            .read_to_end(&mut out)
            .map_err(|e| SourceError::Parse(format!("gzip decompression failed: {e}")))?;
        out
    } else {
        bytes.to_vec()
    };
    if let Some(bundle) = parse_bundle(&decoded) {
        return Ok(bundle);
    }
    let text = String::from_utf8_lossy(&decoded).to_string();
    Ok(LatexSource {
        files: vec!["main.tex".to_string()],
        main_tex: Some(truncate_chars(&text, MAX_MAIN_TEX_CHARS)),
    })
}

/// Interpret `bytes` as a tar bundle and pick the main .tex file; None when
/// the bytes are not a valid tar archive. The main file is the .tex entry
/// containing `\documentclass`, preferring `main.tex`/`paper.tex`.
fn parse_bundle(bytes: &[u8]) -> Option<LatexSource> {
    let mut archive = tar::Archive::new(Cursor::new(bytes));
    let entries = archive.entries().ok()?;
    let mut files = Vec::new();
    let mut tex_entries: Vec<(String, String)> = Vec::new();
    for entry in entries {
        let mut entry = entry.ok()?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let name = entry.path().ok()?.to_string_lossy().to_string();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).ok()?;
        let content = String::from_utf8_lossy(&content).to_string();
        files.push(name.clone());
        if name.ends_with(".tex") {
            tex_entries.push((name, content));
        }
    }
    if files.is_empty() {
        return None;
    }
    let with_class: Vec<(String, String)> = tex_entries
        .iter()
        .filter(|(_, content)| content.contains(r"\documentclass"))
        .cloned()
        .collect();
    let main = with_class
        .iter()
        .find(|(name, _)| name == "main.tex" || name == "paper.tex")
        .or_else(|| with_class.first())
        .or_else(|| tex_entries.first())?;
    Some(LatexSource {
        files,
        main_tex: Some(truncate_chars(&main.1, MAX_MAIN_TEX_CHARS)),
    })
}

/// Shorten `text` to at most `max` characters without splitting a character.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn gz(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).expect("gzip write");
        encoder.finish().expect("gzip finish")
    }

    fn tar_bytes(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, name, content.as_bytes())
                .expect("tar append");
        }
        builder.into_inner().expect("tar finish")
    }

    #[test]
    fn parses_tar_gz_bundle_and_picks_documentclass_entry() {
        let payload = gz(&tar_bytes(&[
            ("sections/intro.tex", "Intro text without a preamble."),
            (
                "paper.tex",
                "\\documentclass{article}\n\\begin{document}Body\\end{document}",
            ),
            ("fig.png", "fake png bytes"),
        ]));
        let parsed = parse_eprint(&payload).expect("parse bundle");
        assert_eq!(parsed.files.len(), 3);
        assert!(parsed.files.contains(&"paper.tex".to_string()));
        assert!(parsed
            .main_tex
            .expect("main tex")
            .contains("\\documentclass"));
    }

    #[test]
    fn prefers_main_tex_when_several_entries_have_documentclass() {
        let payload = gz(&tar_bytes(&[
            ("aux.tex", "\\documentclass{book}"),
            ("main.tex", "\\documentclass{article}"),
        ]));
        let parsed = parse_eprint(&payload).expect("parse bundle");
        assert_eq!(parsed.main_tex.as_deref(), Some("\\documentclass{article}"));
    }

    #[test]
    fn parses_single_gzip_tex_file() {
        let payload = gz(b"\\documentclass{article}\nhello");
        let parsed = parse_eprint(&payload).expect("parse single file");
        assert_eq!(parsed.files, vec!["main.tex".to_string()]);
        assert!(parsed
            .main_tex
            .expect("main tex")
            .contains("\\documentclass"));
    }

    #[test]
    fn rejects_zip_payloads() {
        let err = parse_eprint(b"PK\x03\x04not really a zip").expect_err("zip rejected");
        assert!(matches!(err, SourceError::Other(ref msg) if msg.contains("ZIP")));
    }

    #[test]
    fn truncates_main_tex_on_char_boundary() {
        let long = "é".repeat(MAX_MAIN_TEX_CHARS + 1);
        let truncated = truncate_chars(&long, MAX_MAIN_TEX_CHARS);
        assert_eq!(truncated.chars().count(), MAX_MAIN_TEX_CHARS);
    }

    #[test]
    fn normalizes_arxiv_ids() {
        assert_eq!(normalize_arxiv_id("2310.12345").expect("ok"), "2310.12345");
        assert_eq!(
            normalize_arxiv_id("arXiv:2310.12345").expect("ok"),
            "2310.12345"
        );
        assert_eq!(
            normalize_arxiv_id("ARXIV:2301.00001v2").expect("ok"),
            "2301.00001v2"
        );
        assert_eq!(
            normalize_arxiv_id("  2310.12345  ").expect("ok"),
            "2310.12345"
        );
        assert!(normalize_arxiv_id("arXiv:").is_err());
        assert!(normalize_arxiv_id("2301 00001").is_err());
    }
}
