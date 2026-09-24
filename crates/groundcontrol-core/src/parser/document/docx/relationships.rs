//! Microsoft Word OpenXML relationship extraction (`word/_rels/document.xml.rels`).

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashMap;

/// Parse `word/_rels/document.xml.rels` to map relationship Id to Target URI.
pub fn parse_relationships(xml: &str, rels: &mut HashMap<String, String>) {
    let mut reader = Reader::from_str(xml);

    loop {
        match reader.read_event() {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                if e.local_name().as_ref() == b"Relationship" {
                    let mut id = None;
                    let mut target = None;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"Id" {
                            id = String::from_utf8(attr.value.to_vec()).ok();
                        } else if attr.key.as_ref() == b"Target" {
                            target = String::from_utf8(attr.value.to_vec()).ok();
                        }
                    }
                    if let (Some(i), Some(t)) = (id, target) {
                        rels.insert(i, t);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
}
