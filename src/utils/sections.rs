//! Heading-heuristic section outlining for extracted paper text.

/// A contiguous byte-offset span of `text` belonging to one section.
#[derive(Debug, Clone)]
pub struct SectionSpan {
    pub title: String,
    pub start: usize,
    pub end: usize,
}

/// Section boundaries detected in a document, in document order.
#[derive(Debug, Clone)]
pub struct SectionOutline {
    pub sections: Vec<SectionSpan>,
}

/// Lines longer than this are treated as body text, never headings.
const MAX_HEADING_CHARS: usize = 100;

/// Upper length limit for ALL-CAPS and Title Case heading lines.
const MAX_CASE_HEADING_CHARS: usize = 60;

const KNOWN_SECTION_WORDS: [&str; 10] = [
    "abstract",
    "introduction",
    "background",
    "related work",
    "methods",
    "results",
    "discussion",
    "conclusion",
    "conclusions",
    "references",
];

/// Split `text` into contiguous byte ranges, one per line, excluding terminators.
fn line_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let end = match text[start..].find('\n') {
            Some(nl) => start + nl,
            None => text.len(),
        };
        spans.push((start, end));
        start = end + 1;
    }
    spans
}

/// Drop a leading section number like `2`, `3.1`, or `4.2.`, requiring
/// whitespace before the remainder.
fn strip_numbering(content: &str) -> Option<&str> {
    let bytes = content.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() {
        return None;
    }
    let mut i = 0;
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
        i += 1;
    }
    if i >= bytes.len() || !bytes[i].is_ascii_whitespace() {
        return None;
    }
    let rest = content[i..].trim_start();
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

/// Match a known section word, optionally after numbering and trailing
/// punctuation; the next character must not extend the word.
fn known_word_heading(content: &str) -> bool {
    let stripped = strip_numbering(content).unwrap_or(content);
    let lower = stripped.trim_end_matches(['.', ':']).to_lowercase();
    KNOWN_SECTION_WORDS.iter().any(|word| {
        lower == *word
            || (lower.starts_with(word)
                && lower[word.len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| !c.is_alphabetic()))
    })
}

fn numbered_heading(content: &str) -> bool {
    strip_numbering(content).is_some()
}

fn is_all_caps_heading(content: &str) -> bool {
    let chars = content.chars().count();
    if chars == 0 || chars > MAX_CASE_HEADING_CHARS {
        return false;
    }
    let letters: Vec<char> = content.chars().filter(|c| c.is_alphabetic()).collect();
    letters.len() >= 2 && letters.iter().all(|c| c.is_uppercase())
}

fn is_title_case_heading(content: &str) -> bool {
    let chars = content.chars().count();
    if chars == 0 || chars > MAX_CASE_HEADING_CHARS || content.ends_with('.') {
        return false;
    }
    let words: Vec<&str> = content.split_whitespace().collect();
    if words.len() < 2 {
        return false;
    }
    words.iter().all(|word| {
        word.chars()
            .find(|c| c.is_alphabetic())
            .is_some_and(|c| c.is_uppercase())
    })
}

/// Decide whether one trimmed line is a heading; a Title Case line only
/// counts when body text follows it.
fn is_heading_line(content: &str, has_body_after: bool) -> Option<String> {
    if content.is_empty() || content.chars().count() > MAX_HEADING_CHARS {
        return None;
    }
    if numbered_heading(content) || known_word_heading(content) || is_all_caps_heading(content) {
        return Some(content.to_string());
    }
    if has_body_after && is_title_case_heading(content) {
        return Some(content.to_string());
    }
    None
}

/// Detect section headings with text heuristics only: short lines that are
/// numbered, start with a known section word, are ALL-CAPS, or are Title
/// Case lines followed by body text. The first span always covers the text
/// before the first heading (or the whole text when none is found); spans
/// are contiguous and cover the entire input.
pub fn outline(text: &str) -> SectionOutline {
    let spans = line_spans(text);

    // Index of the next non-empty line after each line, computed once.
    let mut next_body: Vec<Option<usize>> = vec![None; spans.len()];
    let mut next = None;
    for i in (0..spans.len()).rev() {
        next_body[i] = next;
        if !text[spans[i].0..spans[i].1].trim().is_empty() {
            next = Some(i);
        }
    }

    let mut headings: Vec<(usize, String)> = Vec::new();
    for (idx, &(start, end)) in spans.iter().enumerate() {
        let content = text[start..end].trim();
        if let Some(title) = is_heading_line(content, next_body[idx].is_some()) {
            headings.push((idx, title));
        }
    }

    let first_start = headings
        .first()
        .map(|&(idx, _)| spans[idx].0)
        .unwrap_or(text.len());

    let mut sections = Vec::with_capacity(headings.len() + 1);
    sections.push(SectionSpan {
        title: "Front matter".to_string(),
        start: 0,
        end: first_start,
    });
    for (i, (idx, title)) in headings.iter().enumerate() {
        let start = spans[*idx].0;
        let end = headings
            .get(i + 1)
            .map(|(next_idx, _)| spans[*next_idx].0)
            .unwrap_or(text.len());
        sections.push(SectionSpan {
            title: title.clone(),
            start,
            end,
        });
    }
    SectionOutline { sections }
}

/// Text of `span`, truncated to at most `max_chars` characters without
/// splitting one.
pub fn read_section(text: &str, span: &SectionSpan, max_chars: usize) -> String {
    let slice = text.get(span.start..span.end).unwrap_or("");
    if slice.chars().count() <= max_chars {
        slice.to_string()
    } else {
        slice.chars().take(max_chars).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_text() -> String {
        String::from(
            "We study attention models for machine translation.\n\n\
             1 Introduction\n\
             Sequence models dominate modern NLP work.\n\n\
             2 Methods\n\
             We use transformer layers throughout.\n\n\
             RESULTS\n\
             Accuracy improved on every benchmark task.\n",
        )
    }

    #[test]
    fn detects_numbered_and_caps_headings_with_contiguous_offsets() {
        let text = sample_text();
        let outline = outline(&text);
        let sections = &outline.sections;

        assert_eq!(sections.len(), 4);
        assert_eq!(sections[0].title, "Front matter");
        assert_eq!(sections[1].title, "1 Introduction");
        assert_eq!(sections[2].title, "2 Methods");
        assert_eq!(sections[3].title, "RESULTS");

        assert_eq!(sections[0].start, 0);
        assert_eq!(sections.last().expect("last span").end, text.len());
        for pair in sections.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }

        for span in sections {
            let slice = &text[span.start..span.end];
            assert!(!slice.is_empty());
            if span.title != "Front matter" {
                assert!(slice.trim_start().starts_with(span.title.as_str()));
            }
        }

        assert_eq!(
            read_section(&text, &sections[0], usize::MAX),
            "We study attention models for machine translation.\n\n"
        );
    }

    #[test]
    fn text_without_headings_yields_single_front_matter_span() {
        let text = String::from("A plain paragraph about birds.\nIt mentions 3 species total.\n");
        let outline = outline(&text);
        assert_eq!(outline.sections.len(), 1);
        assert_eq!(outline.sections[0].title, "Front matter");
        assert_eq!(outline.sections[0].start, 0);
        assert_eq!(outline.sections[0].end, text.len());
    }

    #[test]
    fn read_section_truncates_on_char_boundary() {
        let text = String::from("héllo wörld");
        let span = SectionSpan {
            title: "all".to_string(),
            start: 0,
            end: text.len(),
        };
        assert_eq!(read_section(&text, &span, 5), "héllo");
        assert_eq!(read_section(&text, &span, 100), text);
    }
}
