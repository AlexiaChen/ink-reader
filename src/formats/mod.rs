use std::path::Path;

use anyhow::{Result, bail};

use crate::book::BookReader;

mod epub;
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
        other => bail!(
            "Unsupported file format: {}",
            other.unwrap_or("(no extension)")
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn only_epub_and_text_extensions_are_supported() {
        for ext in ["mobi", "azw", "azw3", "prc", "pdf", "md"] {
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
}
