//! Compatibility checks for XML text events across the quick-xml upgrade.

#[path = "../src/utils/xml.rs"]
mod xml;

use quick_xml::events::Event;
use xml::{XmlEvent, XmlReader};

#[derive(Debug, PartialEq)]
enum Observed {
    Start(String),
    End(String),
    Empty(String),
    Text(String),
    TextError,
    CData(String),
    Comment(String),
    Eof,
}

fn events(input: &str) -> Vec<Observed> {
    let mut reader = XmlReader::new(input);
    let mut observed = Vec::new();
    loop {
        let event = match reader.read_event().expect("XML event must remain readable") {
            XmlEvent::Text(Ok(text)) => Observed::Text(text),
            XmlEvent::Text(Err(_)) => Observed::TextError,
            XmlEvent::Event(Event::Start(tag)) => {
                Observed::Start(String::from_utf8(tag.name().as_ref().to_vec()).unwrap())
            }
            XmlEvent::Event(Event::End(tag)) => {
                Observed::End(String::from_utf8(tag.name().as_ref().to_vec()).unwrap())
            }
            XmlEvent::Event(Event::Empty(tag)) => {
                Observed::Empty(String::from_utf8(tag.name().as_ref().to_vec()).unwrap())
            }
            XmlEvent::Event(Event::CData(text)) => {
                Observed::CData(String::from_utf8(text.as_ref().to_vec()).unwrap())
            }
            XmlEvent::Event(Event::Comment(text)) => {
                Observed::Comment(String::from_utf8(text.as_ref().to_vec()).unwrap())
            }
            XmlEvent::Event(Event::Eof) => Observed::Eof,
            XmlEvent::Event(other) => panic!("unexpected XML event: {other:?}"),
        };
        let eof = event == Observed::Eof;
        observed.push(event);
        if eof {
            return observed;
        }
    }
}

#[test]
fn test_xml_decodes_contiguous_text_once_without_trimming() {
    for (encoded, decoded) in [
        ("ordinary text", "ordinary text"),
        (" \t\n ", " \t\n "),
        ("  A &amp; B \t\n", "  A & B \t\n"),
        (
            "before &lt;x&gt; &quot;y&quot; &apos;z&apos; after",
            "before <x> \"y\" 'z' after",
        ),
        ("&amp;&lt;&gt;&quot;&apos;", "&<>\"'"),
        ("&amp;", "&"),
        ("&#65;", "A"),
        ("&#x41;", "A"),
        ("&#65;&#x42;&#233;&#x1D11E;", "ABé\u{1d11e}"),
        ("café &#233; &amp; &#x3B2; fin", "café é & β fin"),
        ("&amp;lt;", "&lt;"),
        ("&#38;lt;", "&lt;"),
        ("&amp;amp;lt;", "&amp;lt;"),
    ] {
        let input = format!("<root>{encoded}</root>");
        assert_eq!(
            events(&input),
            vec![
                Observed::Start("root".into()),
                Observed::Text(decoded.into()),
                Observed::End("root".into()),
                Observed::Eof,
            ],
            "input: {input:?}"
        );
    }
}

#[test]
fn test_xml_preserves_markup_boundaries_and_event_order() {
    let input =
        "<root>A&amp;<em>B&#x3C;</em>C&apos;<![CDATA[&amp;]]><!--keep-->&gt;<br/>&quot;Z</root>";
    assert_eq!(
        events(input),
        vec![
            Observed::Start("root".into()),
            Observed::Text("A&".into()),
            Observed::Start("em".into()),
            Observed::Text("B<".into()),
            Observed::End("em".into()),
            Observed::Text("C'".into()),
            Observed::CData("&amp;".into()),
            Observed::Comment("keep".into()),
            Observed::Text(">".into()),
            Observed::Empty("br".into()),
            Observed::Text("\"Z".into()),
            Observed::End("root".into()),
            Observed::Eof,
        ]
    );
}

#[test]
fn test_xml_unknown_reference_errors_the_whole_text_run_and_preserves_next_element() {
    for invalid in [
        "&unknown;",
        "before &unknown; after",
        "before &amp;&unknown;&#65; after",
        "&unknown;&amp;after",
    ] {
        let input = format!("<root><bad>{invalid}</bad><good>retained &amp; sound</good></root>");
        assert_eq!(
            events(&input),
            vec![
                Observed::Start("root".into()),
                Observed::Start("bad".into()),
                Observed::TextError,
                Observed::End("bad".into()),
                Observed::Start("good".into()),
                Observed::Text("retained & sound".into()),
                Observed::End("good".into()),
                Observed::End("root".into()),
                Observed::Eof,
            ],
            "input: {input:?}"
        );
    }
}
