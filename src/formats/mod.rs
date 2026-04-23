use std::path::Path;

use anyhow::{bail, Result};

use crate::book::BookReader;

mod epub;
mod mobi;
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
        Some("mobi") | Some("azw") | Some("azw3") | Some("prc") => {
            let reader = mobi::MobiReader::open(path)?;
            Ok(Box::new(reader))
        }
        Some("pdf") => {
            let reader = pdf::PdfReader::open(path)?;
            Ok(Box::new(reader))
        }
        Some("txt") | Some("md") => {
            let reader = txt::TxtReader::open(path)?;
            Ok(Box::new(reader))
        }
        other => bail!(
            "Unsupported file format: {}",
            other.unwrap_or("(no extension)")
        ),
    }
}
