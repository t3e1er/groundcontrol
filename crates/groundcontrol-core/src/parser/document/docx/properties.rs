//! Microsoft Word OpenXML core properties (`docProps/core.xml`) extraction.

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashMap;

/// Parse `docProps/core.xml` for title, creator, description, and keywords.
pub fn parse_core_properties(
    xml: &str,
    metadata: &mut HashMap<String, String>,
    title: &mut Option<String>,
) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut current_tag = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                current_tag = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
            }
            Ok(Event::Text(ref e)) => {
                let text_raw = std::str::from_utf8(e.as_ref()).unwrap_or("");
                let val = quick_xml::escape::unescape(text_raw)
                    .unwrap_or(std::borrow::Cow::Borrowed(text_raw))
                    .trim()
                    .to_string();
                if !val.is_empty() {
                    match current_tag.as_str() {
                        "title" => {
                            *title = Some(val.clone());
                            metadata.insert("title".to_string(), val);
                        }
                        "creator" => {
                            metadata.insert("author".to_string(), val);
                        }
                        "description" | "subject" => {
                            metadata.insert("description".to_string(), val);
                        }
                        "keywords" => {
                            metadata.insert("keywords".to_string(), val);
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(_)) => {
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
}
