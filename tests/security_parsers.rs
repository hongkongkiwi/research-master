//! Bounded parser security regressions and valid-document compatibility checks.

use lopdf::{dictionary, Document, Object, Stream};
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::reader::NsReader;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use zip::write::SimpleFileOptions;

#[test]
fn test_xml_rejects_excessive_namespace_declarations() {
    // RUSTSEC-2026-0195: exceed the declaration cap without a large allocation.
    for empty_element in [false, true] {
        let mut xml = String::from("<root");
        for index in 0..257 {
            xml.push_str(&format!(" xmlns:n{index}=\"urn:test:{index}\""));
        }
        xml.push_str(if empty_element { "/>" } else { "></root>" });

        let mut reader = NsReader::from_str(&xml);
        assert!(
            reader.read_event().is_err(),
            "257 namespace declarations must be rejected (empty_element={empty_element})"
        );
    }
}

#[test]
fn test_xml_resolves_valid_namespaces_and_restores_parent_scope() {
    let xml = r#"<root xmlns="urn:root" xmlns:p="urn:outer"><p:item xmlns:p="urn:inner"/><p:item/></root>"#;
    let mut reader = NsReader::from_str(xml);

    let (namespace, event) = reader.read_resolved_event().unwrap();
    assert_eq!(namespace, ResolveResult::Bound(Namespace(b"urn:root")));
    assert!(matches!(event, Event::Start(ref tag) if tag.local_name().as_ref() == b"root"));

    let (namespace, event) = reader.read_resolved_event().unwrap();
    assert_eq!(namespace, ResolveResult::Bound(Namespace(b"urn:inner")));
    assert!(matches!(event, Event::Empty(ref tag) if tag.local_name().as_ref() == b"item"));

    let (namespace, event) = reader.read_resolved_event().unwrap();
    assert_eq!(namespace, ResolveResult::Bound(Namespace(b"urn:outer")));
    assert!(matches!(event, Event::Empty(ref tag) if tag.local_name().as_ref() == b"item"));

    let (namespace, event) = reader.read_resolved_event().unwrap();
    assert_eq!(namespace, ResolveResult::Bound(Namespace(b"urn:root")));
    assert!(matches!(event, Event::End(ref tag) if tag.local_name().as_ref() == b"root"));
    assert!(matches!(reader.read_event().unwrap(), Event::Eof));
}

#[test]
fn test_pdf_parsers_preserve_text_in_valid_document() {
    let mut document = Document::with_version("1.4");
    let pages_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let content_id = document.add_object(Stream::new(
        dictionary! {},
        b"BT /F1 12 Tf 72 720 Td (Parser smoke test) Tj ET".to_vec(),
    ));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);

    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let parsed = Document::load_mem(&bytes).unwrap();
    assert_eq!(parsed.get_pages().len(), 1);
    assert_eq!(
        parsed.extract_text(&[1]).unwrap().trim(),
        "Parser smoke test"
    );
    assert_eq!(
        pdf_extract::extract_text_from_mem(&bytes).unwrap().trim(),
        "Parser smoke test"
    );
}

#[test]
fn test_zip_roundtrip_preserves_contents_and_rejects_traversal_path() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file("bin/research-master", options).unwrap();
    writer.write_all(b"archive smoke test").unwrap();
    writer.start_file("../outside", options).unwrap();
    writer.write_all(b"not extracted").unwrap();

    let mut archive = zip::ZipArchive::new(writer.finish().unwrap()).unwrap();
    assert_eq!(archive.len(), 2);
    {
        let mut entry = archive.by_index(0).unwrap();
        assert_eq!(
            entry.enclosed_name(),
            Some(PathBuf::from("bin/research-master"))
        );
        let mut contents = String::new();
        entry.read_to_string(&mut contents).unwrap();
        assert_eq!(contents, "archive smoke test");
    }
    assert!(archive.by_index(1).unwrap().enclosed_name().is_none());
}
