//! GROBID client: PDF header and reference extraction against a self-hosted
//! GROBID service, returning TEI-derived [`Paper`]s.

use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use reqwest::header::{ACCEPT, CONTENT_TYPE};

use crate::models::{Paper, PaperBuilder, SourceType};
use crate::sources::SourceError;
use crate::utils::xml::{XmlEvent, XmlReader};
use crate::utils::HttpClient;

/// GROBID base URL used when `GROBID_URL` is unset.
const DEFAULT_GROBID_URL: &str = "http://localhost:8070";

/// Largest PDF read into memory for GROBID processing.
const MAX_PDF_BYTES: u64 = 100 * 1024 * 1024;

const REFERENCES_ENDPOINT: &str =
    "/api/processReferences?consolidateHeader=1&consolidateCitations=1";
const HEADER_ENDPOINT: &str = "/api/processHeaderDocument?consolidateHeader=1";
const TEI_ACCEPT: &str = "application/tei+xml";

/// Start command for a local GROBID when the service is unreachable.
const DOCKER_HINT: &str = "docker run --rm -p 8070:8070 lfoppiano/grobid:0.8.0";

/// GROBID service base URL; `GROBID_URL` overrides the local default.
pub fn grobid_base() -> String {
    match std::env::var("GROBID_URL") {
        Ok(url) if !url.trim().is_empty() => url.trim().trim_end_matches('/').to_string(),
        _ => DEFAULT_GROBID_URL.to_string(),
    }
}

/// Extract a PDF's reference list as papers, in citation order.
///
/// Papers carry synthesized ids `grobid:{filename}:{index}`; an empty
/// reference list is a normal result, not an error.
pub async fn extract_references(
    client: &HttpClient,
    pdf_path: &Path,
) -> Result<Vec<Paper>, SourceError> {
    let tei = post_pdf_tei(
        &grobid_base(),
        client,
        pdf_path,
        MAX_PDF_BYTES,
        REFERENCES_ENDPOINT,
    )
    .await?;
    let mut papers = parse_references_tei(&tei)?;
    let filename = file_label(pdf_path);
    for (index, paper) in papers.iter_mut().enumerate() {
        paper.paper_id = format!("grobid:{filename}:{index}");
    }
    Ok(papers)
}

/// Extract a PDF's header (title, authors, year, DOI, abstract) as one paper.
///
/// `paper_id` is the DOI when GROBID found one, else `grobid:{filename}`.
pub async fn extract_header(client: &HttpClient, pdf_path: &Path) -> Result<Paper, SourceError> {
    let tei = post_pdf_tei(
        &grobid_base(),
        client,
        pdf_path,
        MAX_PDF_BYTES,
        HEADER_ENDPOINT,
    )
    .await?;
    match parse_header_tei(&tei)? {
        Some(mut paper) => {
            if paper.doi.is_none() {
                paper.paper_id = format!("grobid:{}", file_label(pdf_path));
            }
            Ok(paper)
        }
        None => Err(SourceError::Parse(
            "GROBID header response contained no title".to_string(),
        )),
    }
}

/// POST the PDF as multipart field `input` to one GROBID endpoint and return
/// the TEI response body. `max_pdf_bytes` is a parameter so tests can force
/// the oversize rejection without a huge fixture.
async fn post_pdf_tei(
    base_url: &str,
    client: &HttpClient,
    pdf_path: &Path,
    max_pdf_bytes: u64,
    endpoint: &str,
) -> Result<String, SourceError> {
    let meta = std::fs::metadata(pdf_path).map_err(|e| {
        SourceError::InvalidRequest(format!("cannot read PDF at {}: {e}", pdf_path.display()))
    })?;
    if !meta.is_file() {
        return Err(SourceError::InvalidRequest(format!(
            "{} is not a file",
            pdf_path.display()
        )));
    }
    if meta.len() > max_pdf_bytes {
        return Err(SourceError::InvalidRequest(format!(
            "PDF at {} is {} bytes, over the {max_pdf_bytes} byte GROBID limit",
            pdf_path.display(),
            meta.len()
        )));
    }
    let pdf = std::fs::read(pdf_path).map_err(SourceError::Io)?;
    let (boundary, body) = multipart_form(&file_label(pdf_path), &pdf);

    let url = format!("{base_url}{endpoint}");
    // The shared rate-limited builder has no body setter, so use the raw
    // reqwest client; GROBID is self-hosted, so rate limiting is not needed.
    let response = client
        .client()
        .post(&url)
        .header(ACCEPT, TEI_ACCEPT)
        .header(
            CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .map_err(|e| transport_error(&url, e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(SourceError::Api(format!(
            "GROBID returned HTTP {status} for {endpoint}"
        )));
    }
    response
        .text()
        .await
        .map_err(|e| SourceError::Parse(format!("Failed to read GROBID response: {e}")))
}

/// Transport failures (refused connections, DNS, timeouts) all surface as
/// Network errors naming the env var and the Docker start command.
fn transport_error(url: &str, err: reqwest::Error) -> SourceError {
    SourceError::Network(format!(
        "GROBID request to {url} failed: {err}. Point GROBID_URL at a running GROBID \
         service (default {DEFAULT_GROBID_URL}) or start one: {DOCKER_HINT}"
    ))
}

/// Encode the PDF as a single-file multipart/form-data body named `input`;
/// returns the boundary and the full body. Hand-rolled because the crate's
/// reqwest build has no multipart feature.
fn multipart_form(filename: &str, pdf: &[u8]) -> (String, Vec<u8>) {
    let boundary = format!("research-master-grobid-{:x}", now_nanos());
    let mut body = Vec::with_capacity(pdf.len() + 512);
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"input\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: application/pdf\r\n\r\n");
    body.extend_from_slice(pdf);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// File name used in synthesized paper ids.
fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Text we are currently collecting inside one element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextMode {
    None,
    Forename,
    Surname,
    TitleA,
    TitleM,
    TitleJ,
    TitlePlain,
    Abstract,
    Date,
    Doi,
}

/// One parsed `<biblStruct>` before paper construction.
#[derive(Default)]
struct Bibl {
    title_a: String,
    title_m: String,
    title_j: String,
    authors: Vec<String>,
    year: Option<String>,
    doi: Option<String>,
}

/// Parse `<biblStruct>` entries from GROBID reference TEI into papers with
/// provisional ids `grobid:{index}`; [`extract_references`] stamps the final
/// `grobid:{filename}:{index}` ids.
pub(crate) fn parse_references_tei(xml: &str) -> Result<Vec<Paper>, SourceError> {
    let mut reader = XmlReader::new(xml);
    let mut papers: Vec<Paper> = Vec::new();
    let mut bibl: Option<Bibl> = None;
    let mut in_author = false;
    let mut forenames: Vec<String> = Vec::new();
    let mut surname = String::new();
    let mut mode = TextMode::None;

    loop {
        match reader.read_event() {
            Ok(XmlEvent::Event(Event::Start(ref e))) => match local_name(e).as_str() {
                "biblStruct" => {
                    bibl = Some(Bibl::default());
                    mode = TextMode::None;
                }
                "author" => {
                    in_author = true;
                    forenames.clear();
                    surname.clear();
                    mode = TextMode::None;
                }
                "forename" if in_author => mode = TextMode::Forename,
                "surname" if in_author => mode = TextMode::Surname,
                "title" => {
                    if let Some(b) = bibl.as_mut() {
                        mode = match attr_value(e, "level").as_deref() {
                            Some("a") if b.title_a.is_empty() => TextMode::TitleA,
                            Some("m") if b.title_m.is_empty() => TextMode::TitleM,
                            Some("j") if b.title_j.is_empty() => TextMode::TitleJ,
                            _ => TextMode::None,
                        };
                    }
                }
                "date" => {
                    if let Some(b) = bibl.as_mut() {
                        if b.year.is_none() {
                            match attr_value(e, "when").and_then(|w| year_of(&w)) {
                                Some(y) => b.year = Some(y),
                                None => mode = TextMode::Date,
                            }
                        }
                    }
                }
                "idno" => {
                    if let Some(b) = bibl.as_mut() {
                        if b.doi.is_none()
                            && attr_value(e, "type").is_some_and(|t| t.eq_ignore_ascii_case("doi"))
                        {
                            mode = TextMode::Doi;
                        }
                    }
                }
                // Unknown elements keep the current mode so text inside
                // nested markup still accumulates.
                _ => {}
            },
            Ok(XmlEvent::Event(Event::End(ref e))) => match end_local_name(e).as_str() {
                "forename" | "surname" | "title" | "date" | "idno" => mode = TextMode::None,
                "author" => {
                    mode = TextMode::None;
                    if in_author {
                        let mut name = forenames.join(" ");
                        if !surname.is_empty() {
                            if !name.is_empty() {
                                name.push(' ');
                            }
                            name.push_str(&surname);
                        }
                        if !name.is_empty() {
                            if let Some(b) = bibl.as_mut() {
                                b.authors.push(name);
                            }
                        }
                        in_author = false;
                        forenames.clear();
                        surname.clear();
                    }
                }
                "biblStruct" => {
                    mode = TextMode::None;
                    in_author = false;
                    if let Some(b) = bibl.take() {
                        let index = papers.len();
                        papers.push(bibl_to_paper(b, index));
                    }
                }
                _ => {}
            },
            Ok(XmlEvent::Event(Event::Empty(ref e))) if local_name(e) == "date" => {
                if let Some(b) = bibl.as_mut() {
                    if b.year.is_none() {
                        b.year = attr_value(e, "when").and_then(|w| year_of(&w));
                    }
                }
            }
            Ok(XmlEvent::Text(text)) => {
                let Ok(text) = text else {
                    continue;
                };
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                if let Some(b) = bibl.as_mut() {
                    match mode {
                        TextMode::Forename => forenames.push(text.to_string()),
                        TextMode::Surname => surname = text.to_string(),
                        TextMode::TitleA => append_text(&mut b.title_a, text),
                        TextMode::TitleM => append_text(&mut b.title_m, text),
                        TextMode::TitleJ => append_text(&mut b.title_j, text),
                        TextMode::Date if b.year.is_none() => b.year = year_of(text),
                        TextMode::Doi if b.doi.is_none() => b.doi = Some(text.to_string()),
                        _ => {}
                    }
                }
            }
            Ok(XmlEvent::Event(Event::Eof)) => break,
            Ok(_) => {}
            Err(e) => return Err(SourceError::Parse(format!("invalid GROBID TEI: {e}"))),
        }
    }
    Ok(papers)
}

fn bibl_to_paper(bibl: Bibl, index: usize) -> Paper {
    let title = [bibl.title_a, bibl.title_m, bibl.title_j]
        .into_iter()
        .find(|t| !t.is_empty())
        .unwrap_or_default();
    let mut builder = PaperBuilder::new(
        format!("grobid:{index}"),
        title,
        String::new(),
        SourceType::Other("grobid".to_string()),
    )
    .authors(bibl.authors.join("; "));
    if let Some(doi) = bibl.doi {
        builder = builder.doi(doi);
    }
    if let Some(year) = bibl.year {
        builder = builder.published_date(year);
    }
    builder.build()
}

/// Parse the header TEI (processHeaderDocument) into one paper. `paper_id` is
/// the DOI when present, else a placeholder that [`extract_header`] replaces
/// with `grobid:{filename}`. `Ok(None)` when no title element exists.
pub(crate) fn parse_header_tei(xml: &str) -> Result<Option<Paper>, SourceError> {
    let mut reader = XmlReader::new(xml);
    let mut title_plain = String::new();
    let mut title_a = String::new();
    let mut title_m = String::new();
    let mut title_j = String::new();
    let mut abstract_text = String::new();
    let mut authors: Vec<String> = Vec::new();
    let mut year: Option<String> = None;
    let mut doi: Option<String> = None;
    let mut in_author = false;
    let mut forenames: Vec<String> = Vec::new();
    let mut surname = String::new();
    let mut mode = TextMode::None;

    loop {
        match reader.read_event() {
            Ok(XmlEvent::Event(Event::Start(ref e))) => match local_name(e).as_str() {
                "title" => {
                    mode = match attr_value(e, "level").as_deref() {
                        None if title_plain.is_empty() => TextMode::TitlePlain,
                        Some("a") if title_a.is_empty() => TextMode::TitleA,
                        Some("m") if title_m.is_empty() => TextMode::TitleM,
                        Some("j") if title_j.is_empty() => TextMode::TitleJ,
                        _ => TextMode::None,
                    };
                }
                "abstract" => mode = TextMode::Abstract,
                "author" => {
                    in_author = true;
                    forenames.clear();
                    surname.clear();
                    mode = TextMode::None;
                }
                "forename" if in_author => mode = TextMode::Forename,
                "surname" if in_author => mode = TextMode::Surname,
                "date" => {
                    if year.is_none() {
                        match attr_value(e, "when").and_then(|w| year_of(&w)) {
                            Some(y) => year = Some(y),
                            None => mode = TextMode::Date,
                        }
                    }
                }
                "idno" if doi.is_none() => {
                    if attr_value(e, "type").is_some_and(|t| t.eq_ignore_ascii_case("doi")) {
                        mode = TextMode::Doi;
                    }
                }
                _ => {}
            },
            Ok(XmlEvent::Event(Event::End(ref e))) => match end_local_name(e).as_str() {
                "title" | "abstract" | "forename" | "surname" | "date" | "idno" => {
                    mode = TextMode::None
                }
                "author" => {
                    mode = TextMode::None;
                    if in_author {
                        let mut name = forenames.join(" ");
                        if !surname.is_empty() {
                            if !name.is_empty() {
                                name.push(' ');
                            }
                            name.push_str(&surname);
                        }
                        // GROBID repeats authors in titleStmt and in the
                        // sourceDesc biblStruct; keep one entry each.
                        if !name.is_empty() && !authors.contains(&name) {
                            authors.push(name);
                        }
                        in_author = false;
                        forenames.clear();
                        surname.clear();
                    }
                }
                _ => {}
            },
            Ok(XmlEvent::Event(Event::Empty(ref e)))
                if local_name(e) == "date" && year.is_none() =>
            {
                year = attr_value(e, "when").and_then(|w| year_of(&w));
            }
            Ok(XmlEvent::Text(text)) => {
                let Ok(text) = text else {
                    continue;
                };
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                match mode {
                    TextMode::TitlePlain => append_text(&mut title_plain, text),
                    TextMode::TitleA => append_text(&mut title_a, text),
                    TextMode::TitleM => append_text(&mut title_m, text),
                    TextMode::TitleJ => append_text(&mut title_j, text),
                    TextMode::Abstract => append_text(&mut abstract_text, text),
                    TextMode::Forename => forenames.push(text.to_string()),
                    TextMode::Surname => surname = text.to_string(),
                    TextMode::Date if year.is_none() => year = year_of(text),
                    TextMode::Doi if doi.is_none() => doi = Some(text.to_string()),
                    _ => {}
                }
            }
            Ok(XmlEvent::Event(Event::Eof)) => break,
            Ok(_) => {}
            Err(e) => return Err(SourceError::Parse(format!("invalid GROBID TEI: {e}"))),
        }
    }

    let title = [title_plain, title_a, title_m, title_j]
        .into_iter()
        .find(|t| !t.is_empty());
    let Some(title) = title else {
        return Ok(None);
    };
    let mut builder = PaperBuilder::new(
        doi.clone().unwrap_or_else(|| "grobid:header".to_string()),
        title,
        String::new(),
        SourceType::Other("grobid".to_string()),
    )
    .authors(authors.join("; "))
    .abstract_text(abstract_text);
    if let Some(d) = doi {
        builder = builder.doi(d);
    }
    if let Some(y) = year {
        builder = builder.published_date(y);
    }
    Ok(Some(builder.build()))
}

/// Join the split text events of one element with single spaces.
fn append_text(slot: &mut String, text: &str) {
    if !slot.is_empty() {
        slot.push(' ');
    }
    slot.push_str(text);
}

/// First run of four ASCII digits in a TEI date (`when` attribute or text).
fn year_of(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    bytes
        .windows(4)
        .position(|w| w.iter().all(|b| b.is_ascii_digit()))
        .map(|i| value[i..i + 4].to_string())
}

fn local_name(e: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(e.name().as_ref()).into_owned()
}

fn end_local_name(e: &quick_xml::events::BytesEnd<'_>) -> String {
    String::from_utf8_lossy(e.name().as_ref()).into_owned()
}

fn attr_value(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes()
        .filter_map(|a| a.ok())
        .find(|a| a.key.as_ref() == name.as_bytes())
        .and_then(|a| {
            std::str::from_utf8(a.value.as_ref())
                .ok()
                .map(str::to_string)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{EnvSnapshot, ENV_LOCK};
    use std::net::TcpListener;

    use mockito::Matcher;

    const REFERENCES_TEI: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<TEI xmlns="http://www.tei-c.org/ns/1.0">
  <text>
    <back>
      <listBibl>
        <biblStruct>
          <analytic>
            <title level="a" type="main">Attention Is All You Need</title>
            <author>
              <persName><forename type="first">Ashish</forename><surname>Vaswani</surname></persName>
            </author>
          </analytic>
          <monogr>
            <title level="j">Advances in Neural Information Processing Systems</title>
            <imprint>
              <date type="publication" when="2017">2017</date>
            </imprint>
          </monogr>
          <idno type="DOI">10.5555/3295222</idno>
        </biblStruct>
        <biblStruct>
          <analytic>
            <title level="a">Deep Learning</title>
            <author><persName><forename>Yann</forename><surname>LeCun</surname></persName></author>
            <author><persName><forename>Yoshua</forename><surname>Bengio</surname></persName></author>
          </analytic>
          <monogr>
            <title level="j">Nature</title>
            <imprint><date type="publication" when="2015-05-28">May 2015</date></imprint>
          </monogr>
        </biblStruct>
      </listBibl>
    </back>
  </text>
</TEI>"#;

    const HEADER_TEI: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<TEI xmlns="http://www.tei-c.org/ns/1.0">
  <teiHeader>
    <fileDesc>
      <titleStmt>
        <title level="a" type="main">Attention Is All You Need</title>
        <author>
          <persName><forename type="first">Ashish</forename><surname>Vaswani</surname></persName>
        </author>
      </titleStmt>
      <publicationStmt>
        <date type="publication" when="2017-06-12">June 2017</date>
        <idno type="DOI">10.48550/arXiv.1706.03762</idno>
      </publicationStmt>
      <sourceDesc>
        <biblStruct>
          <analytic>
            <title level="a" type="main">Attention Is All You Need</title>
            <author>
              <persName><forename type="first">Ashish</forename><surname>Vaswani</surname></persName>
            </author>
          </analytic>
        </biblStruct>
      </sourceDesc>
    </fileDesc>
    <profileDesc>
      <abstract>
        <p>The dominant sequence transduction models are based on recurrent networks.</p>
      </abstract>
    </profileDesc>
  </teiHeader>
</TEI>"#;

    /// Build a client with proxy env vars cleared so requests reach the
    /// local mockito server instead of any ambient proxy.
    async fn test_client() -> HttpClient {
        let guard = ENV_LOCK.lock().await;
        let _env = EnvSnapshot::new(&[
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ]);
        const VARS: [&str; 6] = [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ];
        let previous: Vec<(&str, Option<String>)> =
            VARS.iter().map(|v| (*v, std::env::var(v).ok())).collect();
        for var in VARS {
            std::env::remove_var(var);
        }
        let client =
            HttpClient::without_rate_limit("research-master-tests").expect("test HTTP client");
        for (var, value) in previous {
            match value {
                Some(v) => std::env::set_var(var, v),
                None => std::env::remove_var(var),
            }
        }
        drop(_env);
        drop(guard);
        client
    }

    type EnvLockGuard = tokio::sync::MutexGuard<'static, ()>;

    /// Restores the saved env values when dropped at test end.
    struct EnvGuard(Vec<(&'static str, Option<String>)>);

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, value) in self.0.drain(..) {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
        }
    }

    /// Point GROBID_URL at one test's mock server and clear proxies for the
    /// whole test body; hold both guards until the test ends.
    async fn scoped_grobid_env(url: &str) -> (EnvLockGuard, EnvGuard) {
        let lock = ENV_LOCK.lock().await;
        const VARS: [&str; 7] = [
            "GROBID_URL",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ];
        let previous: Vec<(&'static str, Option<String>)> = VARS
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect();
        for name in VARS {
            std::env::remove_var(name);
        }
        std::env::set_var("GROBID_URL", url);
        (lock, EnvGuard(previous))
    }

    fn temp_pdf(name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "research-master-{name}-{}-grobid-test.pdf",
            std::process::id()
        ));
        std::fs::write(&path, contents).expect("write temp PDF");
        path
    }

    #[test]
    fn parse_references_extracts_titles_authors_years_dois() {
        let papers = parse_references_tei(REFERENCES_TEI).expect("parse");
        assert_eq!(papers.len(), 2);

        let first = &papers[0];
        assert_eq!(first.paper_id, "grobid:0");
        assert_eq!(first.title, "Attention Is All You Need");
        assert_eq!(first.authors, "Ashish Vaswani");
        assert_eq!(first.doi.as_deref(), Some("10.5555/3295222"));
        assert_eq!(first.published_date.as_deref(), Some("2017"));
        assert_eq!(first.source, SourceType::Other("grobid".to_string()));

        let second = &papers[1];
        assert_eq!(second.paper_id, "grobid:1");
        assert_eq!(second.title, "Deep Learning");
        assert_eq!(second.authors, "Yann LeCun; Yoshua Bengio");
        assert_eq!(second.published_date.as_deref(), Some("2015"));
        assert!(second.doi.is_none());
    }

    #[test]
    fn parse_references_without_biblstruct_is_empty() {
        let papers =
            parse_references_tei(r#"<TEI><text><p>No references</p></text></TEI>"#).expect("parse");
        assert!(papers.is_empty());
    }

    #[test]
    fn parse_references_returns_empty_on_malformed_xml() {
        // quick-xml pull parsing does not validate overall structure, so
        // truncated input yields no biblStruct entries instead of an error.
        let papers = parse_references_tei("<TEI><biblStruct><title>orphan").expect("parse");
        assert!(papers.is_empty());
    }

    #[test]
    fn parse_header_extracts_all_fields() {
        let paper = parse_header_tei(HEADER_TEI)
            .expect("parse")
            .expect("header paper");
        assert_eq!(paper.title, "Attention Is All You Need");
        assert_eq!(paper.authors, "Ashish Vaswani");
        assert_eq!(paper.doi.as_deref(), Some("10.48550/arXiv.1706.03762"));
        assert_eq!(paper.published_date.as_deref(), Some("2017"));
        assert_eq!(paper.paper_id, "10.48550/arXiv.1706.03762");
        assert!(paper.r#abstract.contains("sequence transduction"));
        assert_eq!(paper.source, SourceType::Other("grobid".to_string()));
    }

    #[test]
    fn parse_header_without_title_is_none() {
        let paper = parse_header_tei(r#"<TEI><teiHeader></teiHeader></TEI>"#).expect("parse");
        assert!(paper.is_none());
    }

    #[tokio::test]
    async fn multipart_request_carries_pdf_under_input_field() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock(
                "POST",
                "/api/processReferences?consolidateHeader=1&consolidateCitations=1",
            )
            .match_header("accept", TEI_ACCEPT)
            .match_header(
                "content-type",
                Matcher::Regex(r"^multipart/form-data; boundary=".to_string()),
            )
            // The mock only matches when the body carries the PDF under the
            // expected form field and filename, asserting the shape.
            .match_body(Matcher::Regex(
                r#"name="input"; filename="[^"]*multipart-shape[^"]*\.pdf""#.to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/tei+xml")
            .with_body(REFERENCES_TEI)
            .expect(1)
            .create_async()
            .await;

        let path = temp_pdf("multipart-shape", b"%PDF-1.4 grobid multipart test");
        let tei = post_pdf_tei(
            &server.url(),
            &test_client().await,
            &path,
            MAX_PDF_BYTES,
            REFERENCES_ENDPOINT,
        )
        .await
        .expect("TEI response");
        assert_eq!(tei, REFERENCES_TEI);

        let _ = std::fs::remove_file(&path);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn oversize_pdf_is_rejected_before_network() {
        let path = temp_pdf("too-big", &[0u8; 64]);
        let err = post_pdf_tei(
            "http://127.0.0.1:9",
            &test_client().await,
            &path,
            16,
            REFERENCES_ENDPOINT,
        )
        .await
        .expect_err("must reject oversize file");
        assert!(matches!(err, SourceError::InvalidRequest(_)), "got {err:?}");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn unreachable_service_maps_to_network_with_setup_hint() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        drop(listener);

        let path = temp_pdf("refused", b"%PDF-1.4");
        let err = post_pdf_tei(
            &format!("http://{addr}"),
            &test_client().await,
            &path,
            MAX_PDF_BYTES,
            HEADER_ENDPOINT,
        )
        .await
        .expect_err("connection refused");
        let message = match err {
            SourceError::Network(message) => message,
            other => panic!("expected Network error, got {other:?}"),
        };
        assert!(message.contains("GROBID_URL"), "got: {message}");
        assert!(message.contains(DOCKER_HINT), "got: {message}");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn grobid_url_env_override_reaches_mock_server() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock(
                "POST",
                "/api/processReferences?consolidateHeader=1&consolidateCitations=1",
            )
            .with_status(200)
            .with_body(REFERENCES_TEI)
            .expect(1)
            .create_async()
            .await;

        let client = test_client().await;
        let (_lock, _env) = scoped_grobid_env(&server.url()).await;
        let path = temp_pdf("env-override", b"%PDF-1.4");
        let papers = extract_references(&client, &path).await.expect("papers");

        assert_eq!(papers.len(), 2);
        let filename = file_label(&path);
        assert_eq!(papers[0].paper_id, format!("grobid:{filename}:0"));
        assert_eq!(papers[1].paper_id, format!("grobid:{filename}:1"));

        let _ = std::fs::remove_file(&path);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn grobid_base_reads_env_and_strips_trailing_slash() {
        let _lock = ENV_LOCK.lock().await;
        let _env = EnvSnapshot::new(&["GROBID_URL"]);
        let previous = std::env::var("GROBID_URL").ok();

        std::env::set_var("GROBID_URL", "http://127.0.0.1:9090/");
        assert_eq!(grobid_base(), "http://127.0.0.1:9090");

        std::env::remove_var("GROBID_URL");
        assert_eq!(grobid_base(), DEFAULT_GROBID_URL);

        match previous {
            Some(v) => std::env::set_var("GROBID_URL", v),
            None => std::env::remove_var("GROBID_URL"),
        }
    }
}
