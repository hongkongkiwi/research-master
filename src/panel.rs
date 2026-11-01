//! Read only localhost web panel for research history.
//!
//! Started by the CLI as `research-master panel --host 127.0.0.1 --port 3001`.
//! Serves one static page and one JSON feed of recent history. Hard rules:
//!
//! - Loopback only: the host must resolve to 127.0.0.1 or ::1 or startup fails.
//! - Read only: no history, cache or filesystem writes. The only data reads are
//!   `HistoryService::read_entries` and `CacheService::stats`.
//! - Bounded parsing: request heads above `MAX_HEAD_BYTES` are rejected with
//!   413, and the head read times out after 10 seconds.
//! - No untrusted bytes reach HTML: the page is a compile time constant with a
//!   hash based Content Security Policy, and history data is rendered in the
//!   browser through textContent, never HTML strings.

use research_master::utils::{CacheService, CacheStats, HistoryEntry, HistoryService};
use sha2::{Digest, Sha256};
use std::io;
use std::io::ErrorKind;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const MAX_HEAD_BYTES: usize = 8 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const HISTORY_LIMIT: usize = 100;

const HTML_PREFIX: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Research Master History</title>
<style>"##;

const INDEX_CSS: &str = r##":root {
  --paper: #f5f2ea;
  --card: #fdfcf8;
  --ink: #211d17;
  --muted: #6f6759;
  --line: #d9d2c1;
  --accent: #86341f;
  --chip-bg: #ece5d3;
}
@media (prefers-color-scheme: dark) {
  :root {
    --paper: #16130f;
    --card: #1e1a15;
    --ink: #e9e3d4;
    --muted: #9d947f;
    --line: #373126;
    --accent: #d99a77;
    --chip-bg: #29241c;
  }
}
* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body {
  margin: 0;
  background: var(--paper);
  color: var(--ink);
  font: 16px/1.55 system-ui, -apple-system, "Segoe UI", sans-serif;
}
.wrap { max-width: 52rem; margin: 0 auto; padding: 2.5rem 1.25rem 3rem; }
.masthead { border-bottom: 3px double var(--ink); padding-bottom: 1.1rem; margin-bottom: 1.4rem; }
.kicker { margin: 0; font-size: .72rem; font-weight: 700; letter-spacing: .22em; text-transform: uppercase; color: var(--accent); }
h1 { margin: .15rem 0 .35rem; font-family: Georgia, "Times New Roman", serif; font-size: 2.1rem; font-weight: 600; line-height: 1.15; }
.sub { margin: 0; color: var(--muted); }
.stats {
  border: 1px solid var(--line);
  background: var(--card);
  border-radius: 4px;
  padding: .7rem .9rem;
  margin: 0 0 1.4rem;
  font-size: .84rem;
}
.stats-grid { display: grid; grid-template-columns: max-content 1fr; gap: .2rem 1rem; margin: 0; }
.stats-grid dt { font-weight: 600; }
.stats-grid dd { margin: 0; color: var(--muted); overflow-wrap: anywhere; }
.panelbar { display: flex; flex-wrap: wrap; gap: 1rem; align-items: flex-end; margin-bottom: .4rem; }
.field { display: flex; flex-direction: column; }
.field.grow { flex: 1 1 16rem; }
.panelbar label { font-size: .68rem; font-weight: 600; letter-spacing: .12em; text-transform: uppercase; color: var(--muted); margin-bottom: .3rem; }
.panelbar input, .panelbar select {
  font: inherit;
  color: var(--ink);
  background: var(--card);
  border: 1px solid var(--line);
  border-radius: 4px;
  padding: .42rem .55rem;
}
.panelbar input { width: 100%; }
.panelbar button {
  font: inherit;
  color: var(--ink);
  background: var(--card);
  border: 1px solid var(--ink);
  border-radius: 4px;
  padding: .45rem .95rem;
  cursor: pointer;
}
.panelbar button:hover { background: var(--accent); border-color: var(--accent); color: #fffdf7; }
:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.status { margin: .4rem 0 1.1rem; color: var(--muted); font-size: .86rem; }
.log { list-style: none; margin: 0; padding: 0; counter-reset: entry; }
.entry { counter-increment: entry; border-bottom: 1px solid var(--line); padding: .75rem .25rem .8rem 2.6rem; position: relative; }
.entry::before {
  content: counter(entry, decimal-leading-zero);
  position: absolute;
  left: .25rem;
  top: .8rem;
  font-family: Georgia, serif;
  font-size: .95rem;
  color: var(--muted);
}
.entry-head { display: flex; flex-wrap: wrap; gap: .55rem; align-items: baseline; margin-bottom: .15rem; }
.chip {
  font-size: .66rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: .1em;
  background: var(--chip-bg);
  border: 1px solid var(--line);
  border-radius: 3px;
  padding: .08rem .42rem;
}
.chip.search { color: var(--accent); }
.src { color: var(--muted); font-size: .8rem; }
.when { color: var(--muted); font-size: .8rem; font-variant-numeric: tabular-nums; margin-left: auto; }
.entry .q { margin: 0; font-weight: 600; overflow-wrap: anywhere; }
.entry .title { margin: 0; font-family: Georgia, serif; font-size: 1.04rem; overflow-wrap: anywhere; }
.entry .path { margin: .1rem 0 0; color: var(--muted); font-size: .8rem; font-family: ui-monospace, "SF Mono", Menlo, monospace; overflow-wrap: anywhere; }
footer { margin-top: 2.2rem; border-top: 1px solid var(--line); padding-top: .8rem; color: var(--muted); font-size: .8rem; }
@media (max-width: 460px) {
  .wrap { padding: 1.5rem 1rem 2.5rem; }
  .stats-grid { grid-template-columns: 1fr; }
  .entry { padding-left: 2.2rem; }
  .when { margin-left: 0; }
}"##;

const HTML_MIDDLE: &str = r##"</style>
</head>
<body>
<main class="wrap">
<header class="masthead">
<p class="kicker">Research Master</p>
<h1>History</h1>
<p class="sub">Recent searches, downloads and views recorded on this machine. Read only.</p>
</header>
<section class="stats" id="stats" hidden>
<dl class="stats-grid" id="stats-grid"></dl>
</section>
<form class="panelbar" id="filters">
<div class="field grow">
<label for="find">Find</label>
<input type="search" id="find" autocomplete="off" placeholder="Query, title, source or path">
</div>
<div class="field">
<label for="type">Type</label>
<select id="type">
<option value="all">All types</option>
<option value="search">Search</option>
<option value="download">Download</option>
<option value="view">View</option>
</select>
</div>
<div class="field">
<label for="source">Source</label>
<select id="source">
<option value="all">All sources</option>
</select>
</div>
<div class="field">
<button type="button" id="refresh">Refresh</button>
</div>
</form>
<p class="status" id="status" role="status" aria-live="polite">Loading history...</p>
<ol class="log" id="log"></ol>
<p class="status" id="empty" hidden>No history recorded yet.</p>
<footer>Read only. Served on loopback. Nothing leaves this machine.</footer>
</main>
<script>"##;

const PANEL_JS: &str = r##"(function () {
  'use strict';
  var findBox = document.getElementById('find');
  var typeSel = document.getElementById('type');
  var sourceSel = document.getElementById('source');
  var refreshBtn = document.getElementById('refresh');
  var filtersForm = document.getElementById('filters');
  var logList = document.getElementById('log');
  var statusLine = document.getElementById('status');
  var emptyLine = document.getElementById('empty');
  var statsBox = document.getElementById('stats');
  var statsGrid = document.getElementById('stats-grid');

  var entries = [];
  var cacheInfo = null;

  function el(tag, cls, text) {
    var node = document.createElement(tag);
    if (cls) { node.className = cls; }
    if (text !== undefined && text !== null && text !== '') { node.textContent = text; }
    return node;
  }

  function num(v) { return typeof v === 'number' && isFinite(v) ? v : 0; }

  function normType(t) { return (t || '').toLowerCase(); }

  function chipClass(t) {
    if (t === 'search') { return 'chip search'; }
    if (t === 'download') { return 'chip download'; }
    if (t === 'view') { return 'chip view'; }
    return 'chip';
  }

  function fmtTime(ts) {
    if (!ts) { return ''; }
    var d = new Date(ts * 1000);
    return isNaN(d.getTime()) ? String(ts) : d.toLocaleString();
  }

  function isoTime(ts) {
    if (!ts) { return null; }
    var d = new Date(ts * 1000);
    return isNaN(d.getTime()) ? null : d.toISOString();
  }

  function populateSources() {
    var seen = {};
    entries.forEach(function (e) { if (e.source) { seen[e.source] = true; } });
    var current = sourceSel.value;
    while (sourceSel.options.length > 1) { sourceSel.remove(1); }
    Object.keys(seen).sort().forEach(function (name) {
      var opt = el('option', null, name);
      opt.value = name;
      sourceSel.appendChild(opt);
    });
    if (current !== 'all' && seen[current]) { sourceSel.value = current; } else { sourceSel.value = 'all'; }
  }

  function matches(e) {
    if (typeSel.value !== 'all' && normType(e.entry_type) !== typeSel.value) { return false; }
    if (sourceSel.value !== 'all' && (e.source || '') !== sourceSel.value) { return false; }
    var q = findBox.value.trim().toLowerCase();
    if (!q) { return true; }
    return [e.query, e.title, e.details, e.source].join(' ').toLowerCase().indexOf(q) !== -1;
  }

  function renderStats() {
    statsGrid.textContent = '';
    if (!cacheInfo || !cacheInfo.enabled) { statsBox.hidden = true; return; }
    function pair(term, value) {
      statsGrid.appendChild(el('dt', null, term));
      statsGrid.appendChild(el('dd', null, value));
    }
    pair('Cache directory', cacheInfo.cache_dir || 'unknown');
    pair('Cached searches', num(cacheInfo.search_count) + ' files, ' + num(cacheInfo.search_size_kb) + ' KB');
    pair('Cached citations', num(cacheInfo.citation_count) + ' files, ' + num(cacheInfo.citation_size_kb) + ' KB');
    statsBox.hidden = false;
  }

  function renderItem(e) {
    var li = el('li', 'entry');
    var head = el('div', 'entry-head');
    var t = normType(e.entry_type);
    head.appendChild(el('span', chipClass(t), t || 'entry'));
    if (e.source) { head.appendChild(el('span', 'src', e.source)); }
    var shown = fmtTime(e.timestamp);
    if (shown) {
      var when = el('time', 'when', shown);
      var iso = isoTime(e.timestamp);
      if (iso) { when.setAttribute('datetime', iso); }
      head.appendChild(when);
    }
    li.appendChild(head);
    var main = el('p', t === 'search' ? 'q' : 'title', e.title || e.query || '');
    if (main.textContent) { li.appendChild(main); }
    if (e.details) { li.appendChild(el('p', 'path', e.details)); }
    return li;
  }

  function render() {
    var shown = entries.filter(matches);
    logList.textContent = '';
    shown.forEach(function (e) { logList.appendChild(renderItem(e)); });
    if (entries.length === 0) {
      emptyLine.hidden = false;
      statusLine.textContent = 'History is empty.';
      return;
    }
    emptyLine.hidden = true;
    var noun = shown.length === 1 ? 'entry' : 'entries';
    statusLine.textContent = shown.length === entries.length
      ? 'Showing all ' + entries.length + ' entries.'
      : 'Showing ' + shown.length + ' of ' + entries.length + ' ' + noun + '.';
  }

  function load() {
    statusLine.textContent = 'Loading history...';
    emptyLine.hidden = true;
    fetch('/api/history', { headers: { 'Accept': 'application/json' } }).then(function (res) {
      if (!res.ok) { throw new Error('HTTP ' + res.status); }
      return res.json();
    }).then(function (data) {
      entries = Array.isArray(data.entries) ? data.entries : [];
      cacheInfo = data.cache || null;
      populateSources();
      renderStats();
      render();
    }).catch(function () {
      entries = [];
      logList.textContent = '';
      statsBox.hidden = true;
      statusLine.textContent = 'Could not load history. The server may have stopped.';
    });
  }

  filtersForm.addEventListener('submit', function (ev) { ev.preventDefault(); });
  findBox.addEventListener('input', render);
  typeSel.addEventListener('change', render);
  sourceSel.addEventListener('change', render);
  refreshBtn.addEventListener('click', load);
  load();
})();
"##;

const HTML_SUFFIX: &str = r##"</script>
</body>
</html>
"##;

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The rendered page plus the CSP value pinned to its inline style and script.
struct PanelPage {
    html: String,
    csp: String,
}

fn build_page() -> PanelPage {
    let html = format!(
        "{}{}{}{}{}",
        HTML_PREFIX, INDEX_CSS, HTML_MIDDLE, PANEL_JS, HTML_SUFFIX
    );
    let script_hash = inline_block_hash(&html, "script").expect("page must contain a script block");
    let style_hash = inline_block_hash(&html, "style").expect("page must contain a style block");
    let csp = format!(
        "default-src 'none'; style-src 'sha256-{style_hash}'; script-src 'sha256-{script_hash}'; \
         connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    );
    PanelPage { html, csp }
}

/// SHA-256 of the first inline `<tag>` block, base64 encoded for CSP use.
fn inline_block_hash(page: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = page.find(&open)? + open.len();
    let len = page[start..].find(&close)?;
    let digest = Sha256::digest(&page.as_bytes()[start..start + len]);
    Some(base64_encode(digest.as_slice()))
}

fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64_ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(BASE64_ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            BASE64_ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64_ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Resolve `host` and refuse anything that is not a loopback address.
fn resolve_loopback(host: &str, port: u16) -> io::Result<SocketAddr> {
    let addrs = (host, port).to_socket_addrs()?.collect::<Vec<_>>();
    if addrs.is_empty() || addrs.iter().any(|addr| !addr.ip().is_loopback()) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "panel host must resolve only to loopback addresses",
        ));
    }
    addrs.into_iter().next().ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("panel host must be a loopback address (127.0.0.1 or ::1), got {host}"),
        )
    })
}

/// Serve the panel until the process is stopped. Bind fails for non-loopback hosts.
pub async fn run(host: &str, port: u16) -> std::io::Result<()> {
    let addr = resolve_loopback(host, port)?;
    let listener = TcpListener::bind(addr).await?;
    let page = Arc::new(build_page());
    let local = listener.local_addr()?;
    eprintln!("History panel: http://{local} (read only, loopback only)");
    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(connected) => connected,
            Err(err) => {
                tracing::warn!("panel accept failed: {err}");
                continue;
            }
        };
        let page = Arc::clone(&page);
        tokio::spawn(handle_connection(stream, page, local));
    }
}

#[derive(Debug)]
enum HeadRead {
    Complete(String),
    TooLarge,
    Empty,
    Partial,
}

async fn read_head(stream: &mut TcpStream) -> io::Result<HeadRead> {
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(end) = find_head_end(&buf) {
            let head = String::from_utf8_lossy(&buf[..end]).into_owned();
            return Ok(HeadRead::Complete(head));
        }
        if buf.len() >= MAX_HEAD_BYTES {
            return Ok(HeadRead::TooLarge);
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return if buf.is_empty() {
                Ok(HeadRead::Empty)
            } else {
                Ok(HeadRead::Partial)
            };
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

/// Extract (method, path) from the request head, stripping any query string.
fn parse_request(head: &str) -> Option<(String, String)> {
    let first = head.lines().next()?;
    let mut parts = first.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    parts.next()?; // a version token is required
    Some((
        method.to_ascii_uppercase(),
        target.split('?').next().unwrap_or_default().to_string(),
    ))
}

enum Route {
    Index,
    History,
    NotFound,
    NotAllowed,
}

fn route(method: &str, path: &str) -> Route {
    match path {
        "" | "/" => {
            if method == "GET" {
                Route::Index
            } else {
                Route::NotAllowed
            }
        }
        "/api/history" => {
            if method == "GET" {
                Route::History
            } else {
                Route::NotAllowed
            }
        }
        _ => Route::NotFound,
    }
}

fn allowed_host(head: &str, local: SocketAddr) -> bool {
    let hosts: Vec<&str> = head
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect();
    if hosts.len() != 1 {
        return false;
    }
    let authority = hosts[0];
    let allowed = format!("{}:{}", local.ip(), local.port());
    authority == allowed || authority == format!("localhost:{}", local.port())
}

fn render_json_doc(entries: &[HistoryEntry], stats: &CacheStats) -> serde_json::Value {
    serde_json::json!({
        "entries": entries,
        "cache": {
            "enabled": stats.enabled,
            "cache_dir": stats.cache_dir.display().to_string(),
            "search_count": stats.search_count,
            "citation_count": stats.citation_count,
            "search_size_kb": stats.search_size_kb,
            "citation_size_kb": stats.citation_size_kb,
            "total_size_kb": stats.total_size_kb,
            "ttl_search_seconds": stats.ttl_search.as_secs(),
            "ttl_citations_seconds": stats.ttl_citations.as_secs(),
        }
    })
}

fn http_response(
    status: &str,
    content_type: &str,
    extra_headers: &[(&str, &str)],
    body: &str,
) -> String {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    head.push_str(body);
    head
}

async fn write_and_close(stream: &mut TcpStream, response: &str) {
    if stream.write_all(response.as_bytes()).await.is_ok() {
        let _ = stream.flush().await;
    }
    let _ = stream.shutdown().await;
}

async fn send_simple(stream: &mut TcpStream, status: &str, body: &str) {
    let response = http_response(status, "text/plain; charset=utf-8", &[], body);
    write_and_close(stream, &response).await;
}

async fn handle_connection(mut stream: TcpStream, page: Arc<PanelPage>, local: SocketAddr) {
    let head = match tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut stream)).await {
        Ok(Ok(HeadRead::Complete(head))) => head,
        Ok(Ok(HeadRead::Empty)) => return,
        Ok(Ok(HeadRead::Partial)) => {
            send_simple(&mut stream, "400 Bad Request", "Incomplete request\n").await;
            return;
        }
        Ok(Ok(HeadRead::TooLarge)) => {
            send_simple(
                &mut stream,
                "413 Content Too Large",
                "Request head too large\n",
            )
            .await;
            return;
        }
        Ok(Err(_)) => return,
        Err(_) => {
            send_simple(&mut stream, "408 Request Timeout", "Request timed out\n").await;
            return;
        }
    };

    let (method, path) = match parse_request(&head) {
        Some(parts) => parts,
        None => {
            send_simple(&mut stream, "400 Bad Request", "Malformed request line\n").await;
            return;
        }
    };

    if !allowed_host(&head, local) {
        send_simple(&mut stream, "403 Forbidden", "Invalid Host header\n").await;
        return;
    }

    match route(&method, &path) {
        Route::Index => {
            let response = http_response(
                "200 OK",
                "text/html; charset=utf-8",
                &[
                    ("Content-Security-Policy", page.csp.as_str()),
                    ("Cache-Control", "no-store"),
                    ("X-Frame-Options", "DENY"),
                ],
                &page.html,
            );
            write_and_close(&mut stream, &response).await;
        }
        Route::History => {
            let read = tokio::task::spawn_blocking(move || {
                HistoryService::new().read_entries(HISTORY_LIMIT)
            })
            .await;
            match read {
                Ok(Ok(entries)) => {
                    let stats =
                        CacheService::from_config(research_master::config::get_config().cache)
                            .stats();
                    let doc = render_json_doc(&entries, &stats);
                    if let Ok(body) = serde_json::to_string(&doc) {
                        let response = http_response(
                            "200 OK",
                            "application/json",
                            &[("Cache-Control", "no-store")],
                            &body,
                        );
                        write_and_close(&mut stream, &response).await;
                    } else {
                        send_simple(
                            &mut stream,
                            "500 Internal Server Error",
                            "History serialization failed\n",
                        )
                        .await;
                    }
                }
                _ => {
                    send_simple(
                        &mut stream,
                        "500 Internal Server Error",
                        "History read failed\n",
                    )
                    .await;
                }
            }
        }
        Route::NotFound => {
            send_simple(&mut stream, "404 Not Found", "Not found\n").await;
        }
        Route::NotAllowed => {
            let response = http_response(
                "405 Method Not Allowed",
                "text/plain; charset=utf-8",
                &[("Allow", "GET")],
                "Method not allowed\n",
            );
            write_and_close(&mut stream, &response).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_master::utils::{CacheStats, HistoryEntryType};

    fn test_stats() -> CacheStats {
        CacheStats {
            enabled: true,
            cache_dir: std::path::PathBuf::from("/tmp/cache"),
            search_count: 3,
            citation_count: 1,
            search_size_kb: 20,
            citation_size_kb: 5,
            total_size_kb: 25,
            ttl_search: Duration::from_secs(60),
            ttl_citations: Duration::from_secs(30),
        }
    }

    #[test]
    fn allowed_host_accepts_only_the_bound_authority() {
        let local: SocketAddr = "127.0.0.1:3001".parse().unwrap();
        let head = |host: &str| format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n");
        assert!(allowed_host(&head("127.0.0.1:3001"), local));
        assert!(allowed_host(&head("localhost:3001"), local));
        // A rebound attacker domain must not pass.
        assert!(!allowed_host(&head("attacker.example:3001"), local));
        // Wrong port, multiple hosts, or a missing host all fail closed.
        assert!(!allowed_host(&head("127.0.0.1:3002"), local));
        assert!(!allowed_host(
            "GET / HTTP/1.1\r\nHost: a:3001\r\nHost: b:3001\r\n\r\n",
            local
        ));
        assert!(!allowed_host("GET / HTTP/1.1\r\n\r\n", local));
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"a"), "YQ==");
        assert_eq!(base64_encode(b"ab"), "YWI=");
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn sha256_of_empty_matches_known_csp_value() {
        let digest = Sha256::digest(b"");
        assert_eq!(
            base64_encode(digest.as_slice()),
            "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        );
    }

    #[test]
    fn page_hashes_pin_the_exact_inline_blocks() {
        let page = build_page();
        let script = inline_block_hash(&page.html, "script").unwrap();
        let style = inline_block_hash(&page.html, "style").unwrap();
        assert_eq!(
            script,
            base64_encode(Sha256::digest(PANEL_JS.as_bytes()).as_slice())
        );
        assert_eq!(
            style,
            base64_encode(Sha256::digest(INDEX_CSS.as_bytes()).as_slice())
        );
        assert_ne!(script, style);
        assert!(page.csp.contains(&format!("script-src 'sha256-{script}'")));
        assert!(page.csp.contains(&format!("style-src 'sha256-{style}'")));
        assert!(page.csp.contains("default-src 'none'"));
    }

    #[test]
    fn page_uses_textcontent_and_never_html_strings() {
        let page = build_page();
        assert!(PANEL_JS.contains("textContent"));
        assert!(!page.html.contains("innerHTML"));
        assert!(!page.html.contains("document.write"));
        assert!(!page.html.contains("<iframe"));
    }

    #[test]
    fn page_text_has_no_en_or_em_dashes() {
        let page = build_page();
        assert!(!page.html.contains('\u{2013}'));
        assert!(!page.html.contains('\u{2014}'));
    }

    #[test]
    fn resolve_loopback_accepts_only_loopback_literals() {
        assert!(resolve_loopback("127.0.0.1", 3001).is_ok());
        assert!(resolve_loopback("::1", 3001).is_ok());
        for bad in ["0.0.0.0", "192.168.1.10", "8.8.8.8", "255.255.255.255"] {
            let err = resolve_loopback(bad, 3001).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "host {bad}");
        }
    }

    #[test]
    fn parse_request_extracts_method_and_strips_query() {
        let (method, path) =
            parse_request("GET /api/history?limit=1 HTTP/1.1\r\nHost: x\r\n").unwrap();
        assert_eq!(method, "GET");
        assert_eq!(path, "/api/history");
        assert!(parse_request("GARBAGE\r\n\r\n").is_none());
        assert!(parse_request("GET /\r\n\r\n").is_none());
    }

    #[test]
    fn route_rejects_unknown_paths_and_methods() {
        assert!(matches!(route("GET", "/"), Route::Index));
        assert!(matches!(route("GET", "/api/history"), Route::History));
        assert!(matches!(route("POST", "/api/history"), Route::NotAllowed));
        assert!(matches!(route("DELETE", "/"), Route::NotAllowed));
        assert!(matches!(route("GET", "/etc/passwd"), Route::NotFound));
        assert!(matches!(
            route("GET", "/api/history/extra"),
            Route::NotFound
        ));
    }

    #[test]
    fn render_json_exposes_entries_and_cache_but_no_html_context() {
        let entry = HistoryEntry {
            entry_type: HistoryEntryType::Search,
            timestamp: 1_700_000_000,
            query: "<script>alert(1)</script>".to_string(),
            source: Some("arxiv".to_string()),
            title: None,
            details: None,
        };
        let doc = render_json_doc(std::slice::from_ref(&entry), &test_stats());
        let body = serde_json::to_string(&doc).unwrap();
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["entries"][0]["query"], "<script>alert(1)</script>");
        assert_eq!(value["entries"][0]["entry_type"], "Search");
        assert_eq!(value["cache"]["enabled"], true);
        assert_eq!(value["cache"]["total_size_kb"], 25);
    }

    #[test]
    fn render_json_handles_empty_history() {
        let doc = render_json_doc(&[], &test_stats());
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(value["entries"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn http_response_sets_length_and_headers() {
        let response = http_response(
            "200 OK",
            "text/html; charset=utf-8",
            &[("Content-Security-Policy", "default-src 'none'")],
            "<!DOCTYPE html>",
        );
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Content-Type: text/html; charset=utf-8\r\n"));
        assert!(response.contains("Content-Length: 15\r\n"));
        assert!(response.contains("X-Content-Type-Options: nosniff\r\n"));
        assert!(response.contains("Connection: close\r\n"));
        assert!(response.contains("Content-Security-Policy: default-src 'none'\r\n"));
        assert!(response.ends_with("<!DOCTYPE html>"));
    }

    #[tokio::test]
    async fn read_head_parses_a_complete_get_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(addr).await.unwrap();
        let (mut server, _) = listener.accept().await.unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        match read_head(&mut server).await.unwrap() {
            HeadRead::Complete(head) => assert!(head.starts_with("GET / HTTP/1.1")),
            other => panic!("expected complete head, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_head_rejects_oversized_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(addr).await.unwrap();
        let (mut server, _) = listener.accept().await.unwrap();
        client
            .write_all(&vec![b'A'; MAX_HEAD_BYTES + 1])
            .await
            .unwrap();
        drop(client);
        assert!(matches!(
            read_head(&mut server).await.unwrap(),
            HeadRead::TooLarge
        ));
    }

    #[tokio::test]
    async fn read_head_reports_partial_input_at_eof() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(addr).await.unwrap();
        let (mut server, _) = listener.accept().await.unwrap();
        client.write_all(b"GET /").await.unwrap();
        client.shutdown().await.unwrap();
        assert!(matches!(
            read_head(&mut server).await.unwrap(),
            HeadRead::Partial
        ));
    }
}
