//! Rich document extraction subsystem for Word, PDF, and HTML.

pub mod docx;
pub mod html;
pub mod pdf;

pub use docx::DocxExtractor;
pub use html::HtmlDocExtractor;
pub use pdf::PdfExtractor;

use std::path::Path;

use groundcontrol_common::ports::{DocumentExtractor, ExtractedDocument};
use groundcontrol_common::types::FileFormat;
use groundcontrol_common::Result;

/// Default composite document extractor registry for groundcontrol.
#[derive(Clone, Default)]
pub struct DocumentExtractorRegistry {
    docx: DocxExtractor,
    html: HtmlDocExtractor,
    pdf: PdfExtractor,
}

impl DocumentExtractorRegistry {
    /// Create a new document extractor registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Extract structured text, title, metadata, and outbound links from raw file bytes.
    pub fn extract(
        &self,
        path: &Path,
        format: FileFormat,
        bytes: &[u8],
    ) -> Result<ExtractedDocument> {
        match format {
            FileFormat::HtmlDoc => self.html.extract(path, bytes),
            FileFormat::Docx => self.docx.extract(path, bytes),
            FileFormat::Pdf => self.pdf.extract(path, bytes),
            FileFormat::Source => {
                // For native source/markdown, wrap into an ExtractedDocument
                let text = String::from_utf8_lossy(bytes).into_owned();
                Ok(ExtractedDocument {
                    title: None,
                    metadata: std::collections::HashMap::new(),
                    normalized_text: text,
                    outbound_links: Vec::new(),
                })
            }
        }
    }
}
