//! PDF link annotation (`/Annots`) cross-reference extraction.

use groundcontrol_common::types::DocLink;
use lopdf::{Document, Object};

use super::info::extract_string_value;

/// Extract URI cross-reference links from a PDF `/Annots` object.
pub fn extract_annots_links(doc: &Document, annots_obj: &Object, links: &mut Vec<DocLink>) {
    let annot_list = match annots_obj {
        Object::Array(arr) => arr.clone(),
        Object::Reference(r) => {
            if let Ok(Object::Array(arr)) = doc.get_object(*r) {
                arr.clone()
            } else {
                return;
            }
        }
        _ => return,
    };

    for item in annot_list {
        let dict = match item {
            Object::Dictionary(d) => Some(d),
            Object::Reference(r) => doc.get_object(r).ok().and_then(|o| o.as_dict().ok().cloned()),
            _ => None,
        };

        if let Some(d) = dict {
            // Check if Subtype is Link
            if let Some(subtype) = d.get(b"Subtype").ok().and_then(extract_string_value) {
                if subtype.eq_ignore_ascii_case("Link") {
                    // Check action dictionary /A
                    if let Ok(action_obj) = d.get(b"A") {
                        let action_dict = match action_obj {
                            Object::Dictionary(ad) => Some(ad.clone()),
                            Object::Reference(r) => {
                                doc.get_object(*r).ok().and_then(|o| o.as_dict().ok().cloned())
                            }
                            _ => None,
                        };

                        if let Some(ad) = action_dict {
                            if let Some(uri) = ad.get(b"URI").ok().and_then(extract_string_value) {
                                let clean_uri = uri.trim();
                                if !clean_uri.is_empty() {
                                    links.push(DocLink {
                                        target: clean_uri.to_string(),
                                        label: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
