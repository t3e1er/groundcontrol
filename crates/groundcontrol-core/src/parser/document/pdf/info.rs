//! PDF `/Info` trailer dictionary metadata extraction.

use lopdf::{Document, Object};
use std::collections::HashMap;

/// Extract document title and properties from PDF `/Info` trailer dictionary.
pub fn extract_info_metadata(doc: &Document) -> (Option<String>, HashMap<String, String>) {
    let mut metadata = HashMap::new();
    let mut title = None;

    if let Ok(info_dict) =
        doc.trailer.get(b"Info").and_then(|obj| doc.get_object(obj.as_reference()?))
    {
        if let Ok(dict) = info_dict.as_dict() {
            if let Some(t) = dict.get(b"Title").ok().and_then(extract_string_value) {
                if !t.is_empty() {
                    title = Some(t.clone());
                    metadata.insert("title".to_string(), t);
                }
            }
            if let Some(a) = dict.get(b"Author").ok().and_then(extract_string_value) {
                if !a.is_empty() {
                    metadata.insert("author".to_string(), a);
                }
            }
            if let Some(s) = dict.get(b"Subject").ok().and_then(extract_string_value) {
                if !s.is_empty() {
                    metadata.insert("subject".to_string(), s);
                }
            }
            if let Some(k) = dict.get(b"Keywords").ok().and_then(extract_string_value) {
                if !k.is_empty() {
                    metadata.insert("keywords".to_string(), k);
                }
            }
        }
    }

    (title, metadata)
}

/// Extracts a UTF-8 string from a lopdf String or Name object.
pub fn extract_string_value(obj: &Object) -> Option<String> {
    match obj {
        Object::String(bytes, _) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Object::Name(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    }
}
