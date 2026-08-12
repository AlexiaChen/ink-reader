use std::path::Path;

use anyhow::{Result, bail};

use crate::book::BookReader;

mod epub;
mod pdf;
mod txt;

/// Detect format from file extension and return the appropriate reader.
pub fn load_reader(path: &Path) -> Result<Box<dyn BookReader>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase());

    match ext.as_deref() {
        Some("epub") => {
            let reader = epub::EpubReader::open(path)?;
            Ok(Box::new(reader))
        }
        Some("txt") => {
            let reader = txt::TxtReader::open(path)?;
            Ok(Box::new(reader))
        }
        Some("pdf") => {
            let reader = pdf::PdfReader::open(path)?;
            Ok(Box::new(reader))
        }
        other => bail!(
            "Unsupported file format: {}",
            other.unwrap_or("(no extension)")
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use pdf_oxide::api::Pdf;

    use super::*;

    #[test]
    fn unsupported_extensions_are_rejected() {
        for ext in ["mobi", "azw", "azw3", "prc", "md"] {
            let mut file = tempfile::Builder::new()
                .suffix(&format!(".{ext}"))
                .tempfile()
                .unwrap();
            writeln!(file, "placeholder").unwrap();

            let err = match load_reader(file.path()) {
                Ok(_) => panic!("expected .{ext} to be unsupported"),
                Err(err) => err,
            };
            assert_eq!(err.to_string(), format!("Unsupported file format: {ext}"));
        }
    }

    #[test]
    fn pdf_extension_dispatches_to_pdf_reader() {
        let file = tempfile::Builder::new().suffix(".pdf").tempfile().unwrap();
        let mut pdf = Pdf::from_text("PDF dispatch works").unwrap();
        pdf.save(file.path()).unwrap();

        let reader = load_reader(file.path()).unwrap();
        assert_eq!(reader.meta().chapters.len(), 1);
        assert!(!reader.chapter_blocks(0).unwrap().is_empty());
    }
}
