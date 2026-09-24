//! Document extraction subsystem: chunking, embed policy, and format extractors (Markdown, HTML, Word, PDF).

pub mod chunker;
pub mod docx;
pub mod html;
pub mod markdown;
pub mod pdf;
pub mod policy;

pub use chunker::chunk_document;
pub use docx::DocxExtractor;
pub use html::HtmlDocExtractor;
pub use markdown::parse_document;
pub use pdf::PdfExtractor;
pub use policy::classify_document_chunk;

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
                // For native source/markdown, delegate to markdown parser
                let text = String::from_utf8_lossy(bytes);
                let doc = markdown::parse_document(path, &text)?;
                let mut metadata = std::collections::HashMap::new();
                if let Some(ref title) = doc.title {
                    metadata.insert("title".to_string(), title.clone());
                }
                Ok(ExtractedDocument {
                    title: doc.title,
                    metadata,
                    normalized_text: doc.content,
                    outbound_links: doc.links,
                })
            }
        }
    }
}
