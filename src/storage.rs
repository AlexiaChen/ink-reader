use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A bookmark stores the logical reading position (chapter + block index),
/// NOT a page number (page numbers change on terminal resize).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    /// Canonical absolute path to the book file
    pub book_path: String,
    pub chapter: usize,
    /// Index of the first ContentBlock on the bookmarked page
    pub block_index: usize,
    pub chapter_title: String,
    /// ISO 8601 timestamp
    pub added_at: String,
}

impl Bookmark {
    pub fn new(
        book_path: impl Into<String>,
        chapter: usize,
        block_index: usize,
        chapter_title: impl Into<String>,
    ) -> Self {
        let now = chrono_like_now();
        Self {
            book_path: book_path.into(),
            chapter,
            block_index,
            chapter_title: chapter_title.into(),
            added_at: now,
        }
    }
}

/// Returns current UTC time in ISO 8601 format without pulling in chrono.
fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Simple formatting: YYYY-MM-DDTHH:MM:SSZ
    let s = secs;
    let sec = s % 60;
    let min = (s / 60) % 60;
    let hour = (s / 3600) % 24;
    let days = s / 86400;
    // Approx date from days since epoch (good enough for display)
    let year = 1970 + days / 365;
    let day_of_year = days % 365;
    let month = day_of_year / 30 + 1;
    let day = day_of_year % 30 + 1;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Manages bookmarks for all books, persisted to a JSON file.
pub struct BookmarkStore {
    path: PathBuf,
    bookmarks: Vec<Bookmark>,
}

impl BookmarkStore {
    /// Load (or create) the bookmark store at the default XDG location.
    pub fn load() -> Result<Self> {
        let path = bookmark_file_path()?;
        let bookmarks = if path.exists() {
            let data = std::fs::read_to_string(&path)
                .with_context(|| format!("reading bookmarks from {}", path.display()))?;
            serde_json::from_str::<Vec<Bookmark>>(&data).unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(Self { path, bookmarks })
    }

    /// Bookmarks for the given canonical book path.
    pub fn for_book(&self, book_path: &str) -> Vec<&Bookmark> {
        self.bookmarks
            .iter()
            .filter(|b| b.book_path == book_path)
            .collect()
    }

    /// Add a bookmark; deduplicate by (book_path, chapter, block_index).
    pub fn add(&mut self, bookmark: Bookmark) {
        let already = self.bookmarks.iter().any(|b| {
            b.book_path == bookmark.book_path
                && b.chapter == bookmark.chapter
                && b.block_index == bookmark.block_index
        });
        if !already {
            self.bookmarks.push(bookmark);
        }
    }

    /// Remove bookmark by index within the per-book list.
    pub fn remove_for_book(&mut self, book_path: &str, book_index: usize) {
        let global_indices: Vec<usize> = self
            .bookmarks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.book_path == book_path)
            .map(|(i, _)| i)
            .collect();
        if let Some(&global) = global_indices.get(book_index) {
            self.bookmarks.remove(global);
        }
    }

    /// Load from a specific path (used in tests).
    pub fn load_from(path: &PathBuf) -> Result<Self> {
        let bookmarks = if path.exists() {
            let data = std::fs::read_to_string(path)?;
            serde_json::from_str::<Vec<Bookmark>>(&data).unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(Self { path: path.clone(), bookmarks })
    }

    /// Persist to disk.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(&self.bookmarks)?;
        std::fs::write(&self.path, data)
            .with_context(|| format!("writing bookmarks to {}", self.path.display()))
    }
}

/// Returns the canonical path to the bookmark JSON file.
fn bookmark_file_path() -> Result<PathBuf> {
    let data_dir = dirs::data_local_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("share")))
        .context("cannot determine data directory")?;
    Ok(data_dir.join("ink-reader").join("bookmarks.json"))
}

/// Return a stable book identifier: canonical absolute path as a string.
pub fn book_id(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn make_bookmark(path: &str, chapter: usize, block: usize) -> Bookmark {
        Bookmark::new(path, chapter, block, "Chapter Title")
    }

    #[test]
    fn add_and_retrieve() {
        let mut store = BookmarkStore {
            path: PathBuf::from("/tmp/test_bookmarks.json"),
            bookmarks: Vec::new(),
        };
        store.add(make_bookmark("/books/a.epub", 1, 10));
        store.add(make_bookmark("/books/b.epub", 0, 0));
        let a_marks = store.for_book("/books/a.epub");
        assert_eq!(a_marks.len(), 1);
        assert_eq!(a_marks[0].chapter, 1);
    }

    #[test]
    fn no_duplicate_bookmarks() {
        let mut store = BookmarkStore {
            path: PathBuf::from("/tmp/test_bookmarks.json"),
            bookmarks: Vec::new(),
        };
        store.add(make_bookmark("/books/a.epub", 1, 10));
        store.add(make_bookmark("/books/a.epub", 1, 10)); // duplicate
        assert_eq!(store.for_book("/books/a.epub").len(), 1);
    }

    #[test]
    fn remove_bookmark() {
        let mut store = BookmarkStore {
            path: PathBuf::from("/tmp/test_bookmarks.json"),
            bookmarks: Vec::new(),
        };
        store.add(make_bookmark("/books/a.epub", 0, 0));
        store.add(make_bookmark("/books/a.epub", 1, 10));
        store.remove_for_book("/books/a.epub", 0);
        assert_eq!(store.for_book("/books/a.epub").len(), 1);
    }

    #[test]
    fn round_trip_serialization() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut store = BookmarkStore { path: path.clone(), bookmarks: Vec::new() };
        store.add(make_bookmark("/books/a.epub", 2, 5));
        store.save().unwrap();

        let loaded = BookmarkStore::load_from(&path).unwrap();
        assert_eq!(loaded.bookmarks.len(), 1);
        assert_eq!(loaded.bookmarks[0].chapter, 2);
    }
}
