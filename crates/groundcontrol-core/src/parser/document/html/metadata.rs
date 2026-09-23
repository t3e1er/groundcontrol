//! HTML metadata and title extraction.

use scraper::{Html, Selector};
use std::collections::HashMap;

/// Extract document title and `<meta>` tags from parsed HTML.
pub fn extract_metadata(document: &Html) -> (Option<String>, HashMap<String, String>) {
    let mut metadata = HashMap::new();
    let mut title = None;

    // 1. Extract <title>
    if let Ok(title_sel) = Selector::parse("title") {
        if let Some(elem) = document.select(&title_sel).next() {
            let t = elem.text().collect::<Vec<_>>().join(" ").trim().to_string();
            if !t.is_empty() {
                title = Some(t.clone());
                metadata.insert("title".to_string(), t);
            }
        }
    }

    // 2. Extract <meta> tags
    if let Ok(meta_sel) = Selector::parse("meta") {
        for elem in document.select(&meta_sel) {
            let name = elem.value().attr("name").or_else(|| elem.value().attr("property"));
            let content = elem.value().attr("content");
            if let (Some(n), Some(c)) = (name, content) {
                if !n.is_empty() && !c.is_empty() {
                    metadata.insert(n.to_string(), c.to_string());
                }
            }
        }
    }

    (title, metadata)
}
