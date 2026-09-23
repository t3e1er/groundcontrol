//! HTML hyperlink and cross-reference extraction.

use groundcontrol_common::types::DocLink;

/// Try extracting a valid cross-reference [`DocLink`] from an `<a>` element's `href` attribute.
pub fn parse_anchor_link(href: &str, text: &str) -> Option<DocLink> {
    let clean_href = href.trim();
    if clean_href.is_empty() || clean_href.starts_with('#') || clean_href.starts_with("javascript:")
    {
        return None;
    }

    Some(DocLink {
        target: clean_href.to_string(),
        label: if text.is_empty() { None } else { Some(text.to_string()) },
    })
}
