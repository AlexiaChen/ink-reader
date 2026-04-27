use std::time::Instant;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Size;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

use crate::book::{BookReader, ContentBlock, Page, PaginationKey, paginate_blocks};
use crate::storage::{Bookmark, BookmarkStore};

/// State for a running page-flip animation.
pub struct AnimState {
    pub old_lines: Vec<String>,
    pub start: Instant,
    pub duration_ms: u64,
    pub forward: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Reading,
    TocOverlay,
    BookmarkOverlay,
}

pub struct App {
    pub reader: Box<dyn BookReader>,
    pub mode: Mode,
    pub current_chapter: usize,
    pub current_page: usize,
    pub pages: Vec<Page>,
    pub pagination_key: Option<PaginationKey>,
    pub toc_state: ratatui::widgets::ListState,
    pub bookmark_state: ratatui::widgets::ListState,
    pub bookmarks: BookmarkStore,
    pub picker: Option<Picker>,
    pub book_path: String,
    pub should_quit: bool,
    pending_error: Option<anyhow::Error>,
    pub anim: Option<AnimState>,
    /// Whether the book cover is currently being displayed.
    pub showing_cover: bool,
    /// Raw bytes of the cover image, if the format provided one.
    pub cover_bytes: Option<Vec<u8>>,
    /// Active image protocol for the current cover or in-chapter image page.
    pub current_image: Option<StatefulProtocol>,
}

impl App {
    pub fn new(reader: Box<dyn BookReader>, book_path: String) -> Result<Self> {
        let bookmarks = BookmarkStore::load()?;
        let picker = Picker::from_query_stdio()
            .ok()
            .or_else(|| Some(Picker::halfblocks()));

        // Extract cover bytes before moving `reader` into the struct.
        let cover_bytes = reader.cover_image().map(|(d, _)| d.to_vec());
        let showing_cover = cover_bytes.is_some();

        let mut app = Self {
            reader,
            mode: Mode::Reading,
            current_chapter: 0,
            current_page: 0,
            pages: vec![],
            pagination_key: None,
            toc_state: ratatui::widgets::ListState::default(),
            bookmark_state: ratatui::widgets::ListState::default(),
            bookmarks,
            picker,
            book_path,
            should_quit: false,
            pending_error: None,
            anim: None,
            showing_cover,
            cover_bytes,
            current_image: None,
        };

        app.toc_state.select(Some(0));
        // Build the cover image protocol if a cover is available.
        if app.showing_cover {
            app.refresh_current_image();
        }
        Ok(app)
    }

    /// (Re-)paginate the current chapter for the given terminal size.
    pub fn load_chapter(&mut self, chapter_idx: usize, size: Size) {
        self.anim = None; // cancel any in-flight animation
        let key = PaginationKey {
            chapter: chapter_idx,
            width: size.width,
            height: size.height,
        };

        // Avoid re-paginating if nothing changed
        if self.pagination_key.as_ref() == Some(&key) && self.current_chapter == chapter_idx {
            return;
        }

        let blocks = self.reader.chapter_blocks(chapter_idx).unwrap_or_else(|_| {
            vec![ContentBlock::Paragraph(
                "[Error loading chapter]".to_string(),
            )]
        });

        self.pages = paginate_blocks(&blocks, size.width, size.height);
        if self.pages.is_empty() {
            self.pages = vec![Page {
                lines: vec!["[Empty chapter]".to_string()],
                ..Page::default()
            }];
        }

        self.current_chapter = chapter_idx;
        self.current_page = 0;
        self.pagination_key = Some(key);
        self.refresh_current_image();
    }

    /// Rebuild `current_image` from whatever is currently being displayed:
    /// the book cover (when `showing_cover`) or an image embedded in the
    /// current page.  Clears `current_image` if there is nothing to show.
    pub fn refresh_current_image(&mut self) {
        self.current_image = None;

        let bytes: Vec<u8> = if self.showing_cover {
            match &self.cover_bytes {
                Some(b) => b.clone(),
                None => return,
            }
        } else if let Some(img) = self
            .pages
            .get(self.current_page)
            .and_then(|p| p.image.as_ref())
        {
            img.data.clone()
        } else {
            return;
        };

        if let Ok(dyn_img) = image::load_from_memory(&bytes)
            && let Some(picker) = &self.picker
        {
            self.current_image = Some(picker.new_resize_protocol(dyn_img));
        }
    }

    /// Called on terminal resize.
    pub fn on_resize(&mut self, size: Size) {
        self.pagination_key = None; // force re-paginate
        self.load_chapter(self.current_chapter, size);
    }

    pub fn handle_key(&mut self, key: KeyEvent, size: Size) {
        match self.mode {
            Mode::Reading => self.handle_key_reading(key, size),
            Mode::TocOverlay => self.handle_key_toc(key, size),
            Mode::BookmarkOverlay => self.handle_key_bookmarks(key, size),
        }
    }

    pub fn take_pending_error(&mut self) -> Option<anyhow::Error> {
        self.pending_error.take()
    }

    pub(crate) fn current_location_title(&self) -> &str {
        if let Some(title) = self
            .pages
            .get(self.current_page)
            .and_then(|page| page.section_title.as_deref())
        {
            return title;
        }

        self.reader
            .meta()
            .chapters
            .get(self.current_chapter)
            .map(|c| c.title.as_str())
            .unwrap_or("")
    }

    pub(crate) fn bookmarks_for_current_book(&self) -> Vec<&Bookmark> {
        self.bookmarks.for_book(&self.book_path)
    }

    fn handle_key_reading(&mut self, key: KeyEvent, size: Size) {
        // Quit always works, even during animation
        if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            match self.save_current_bookmark() {
                Ok(()) => self.should_quit = true,
                Err(err) => self.pending_error = Some(err),
            }
            return;
        }

        // Any other key during animation: cancel it and eat the keypress
        if self.anim.take().is_some() {
            return;
        }

        match key.code {
            // Next page
            KeyCode::Down | KeyCode::Char(' ') => {
                self.next_page(size);
            }
            // Previous page
            KeyCode::Up => {
                self.prev_page(size);
            }
            // Next chapter
            KeyCode::Char('n') => {
                let next = self.current_chapter + 1;
                if next < self.reader.meta().chapters.len() {
                    self.load_chapter(next, size);
                }
            }
            // Previous chapter
            KeyCode::Char('p') if self.current_chapter > 0 => {
                let prev = self.current_chapter - 1;
                self.load_chapter(prev, size);
            }
            // Toggle ToC
            KeyCode::Char('t') => {
                self.mode = Mode::TocOverlay;
                self.toc_state.select(Some(self.current_chapter));
            }
            // Toggle bookmarks
            KeyCode::Char('b') => {
                self.mode = Mode::BookmarkOverlay;
                self.bookmark_state.select(Some(0));
            }
            // Save bookmark at current position
            KeyCode::Char('s') => {
                if let Err(err) = self.save_current_bookmark() {
                    self.pending_error = Some(err);
                }
            }
            _ => {}
        }
    }

    fn handle_key_toc(&mut self, key: KeyEvent, size: Size) {
        let chapter_count = self.reader.meta().chapters.len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('t') => {
                self.mode = Mode::Reading;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let next = self
                    .toc_state
                    .selected()
                    .map(|i| (i + 1).min(chapter_count.saturating_sub(1)))
                    .unwrap_or(0);
                self.toc_state.select(Some(next));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let prev = self
                    .toc_state
                    .selected()
                    .map(|i| i.saturating_sub(1))
                    .unwrap_or(0);
                self.toc_state.select(Some(prev));
            }
            KeyCode::Enter => {
                if let Some(idx) = self.toc_state.selected() {
                    self.load_chapter(idx, size);
                }
                self.mode = Mode::Reading;
            }
            _ => {}
        }
    }

    fn handle_key_bookmarks(&mut self, key: KeyEvent, size: Size) {
        let bm_count = self.bookmarks_for_current_book().len();

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('b') => {
                self.mode = Mode::Reading;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let next = self
                    .bookmark_state
                    .selected()
                    .map(|i| (i + 1).min(bm_count.saturating_sub(1)))
                    .unwrap_or(0);
                self.bookmark_state.select(Some(next));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let prev = self
                    .bookmark_state
                    .selected()
                    .map(|i| i.saturating_sub(1))
                    .unwrap_or(0);
                self.bookmark_state.select(Some(prev));
            }
            KeyCode::Enter => {
                if let Some(idx) = self.bookmark_state.selected() {
                    let target = self
                        .bookmarks_for_current_book()
                        .get(idx)
                        .map(|bm| (bm.chapter, bm.block_index, bm.chapter_title.clone()));
                    if let Some((chapter, block_index, chapter_title)) = target {
                        self.jump_to_bookmark(chapter, block_index, &chapter_title, size);
                    }
                }
                self.mode = Mode::Reading;
            }
            KeyCode::Char('d') => {
                // Delete selected bookmark
                if let Some(idx) = self.bookmark_state.selected() {
                    self.bookmarks.remove_for_book(&self.book_path, idx);
                    if let Err(err) = self.bookmarks.save() {
                        self.pending_error = Some(err);
                    }
                }
                // Adjust selection
                let new_count = self.bookmarks_for_current_book().len();
                if new_count == 0 {
                    self.bookmark_state.select(None);
                } else {
                    let sel = self
                        .bookmark_state
                        .selected()
                        .unwrap_or(0)
                        .min(new_count - 1);
                    self.bookmark_state.select(Some(sel));
                }
            }
            _ => {}
        }
    }

    fn next_page(&mut self, size: Size) {
        // Dismiss cover: transition to chapter 0 page 0 without animation.
        if self.showing_cover {
            self.showing_cover = false;
            self.refresh_current_image();
            return;
        }

        if self.pages.is_empty() {
            self.load_chapter(self.current_chapter, size);
            return;
        }

        let old_has_image = self
            .pages
            .get(self.current_page)
            .is_some_and(|p| p.image.is_some());
        let old_lines = self
            .pages
            .get(self.current_page)
            .map(|p| p.lines.clone())
            .unwrap_or_default();

        let moved = if self.current_page + 1 < self.pages.len() {
            self.current_page += 1;
            true
        } else {
            let next = self.current_chapter + 1;
            if next < self.reader.meta().chapters.len() {
                self.load_chapter(next, size);
                true
            } else {
                false
            }
        };

        if moved {
            self.refresh_current_image();
            // Skip page-flip animation when either the old or new page has an image.
            let new_has_image = self.current_image.is_some();
            if !old_has_image && !new_has_image {
                self.anim = Some(AnimState {
                    old_lines,
                    start: Instant::now(),
                    duration_ms: 300,
                    forward: true,
                });
            }
        }
    }

    fn prev_page(&mut self, size: Size) {
        // If we're at the very start of the book and a cover exists, go back to it.
        if !self.showing_cover
            && self.current_chapter == 0
            && self.current_page == 0
            && self.cover_bytes.is_some()
        {
            self.showing_cover = true;
            self.refresh_current_image();
            return;
        }

        if self.showing_cover {
            // Already at cover; nothing further back.
            return;
        }

        let old_has_image = self
            .pages
            .get(self.current_page)
            .is_some_and(|p| p.image.is_some());
        let old_lines = self
            .pages
            .get(self.current_page)
            .map(|p| p.lines.clone())
            .unwrap_or_default();

        let moved = if self.current_page > 0 {
            self.current_page -= 1;
            true
        } else if self.current_chapter > 0 {
            let prev = self.current_chapter - 1;
            self.load_chapter(prev, size);
            self.current_page = self.pages.len().saturating_sub(1);
            true
        } else {
            false
        };

        if moved {
            self.refresh_current_image();
            let new_has_image = self.current_image.is_some();
            if !old_has_image && !new_has_image {
                self.anim = Some(AnimState {
                    old_lines,
                    start: Instant::now(),
                    duration_ms: 300,
                    forward: false,
                });
            }
        }
    }

    /// Expire completed animations (call once per event-loop iteration).
    pub fn tick_anim(&mut self) {
        if let Some(a) = &self.anim
            && a.start.elapsed().as_millis() as u64 >= a.duration_ms
        {
            self.anim = None;
        }
    }

    fn save_current_bookmark(&mut self) -> Result<()> {
        let block_index = self
            .pages
            .get(self.current_page)
            .map(|page| page.first_block)
            .unwrap_or(0);

        self.bookmarks.add(Bookmark::new(
            self.book_path.clone(),
            self.current_chapter,
            block_index,
            self.current_location_title(),
        ));
        self.bookmarks.save()
    }

    fn resolve_bookmark_chapter(&self, chapter: usize, chapter_title: &str) -> usize {
        if self
            .reader
            .meta()
            .chapters
            .get(chapter)
            .is_some_and(|current| current.title == chapter_title)
        {
            return chapter;
        }

        self.reader
            .meta()
            .chapters
            .iter()
            .position(|candidate| candidate.title == chapter_title)
            .unwrap_or_else(|| chapter.min(self.reader.meta().chapters.len().saturating_sub(1)))
    }

    fn jump_to_bookmark(
        &mut self,
        chapter: usize,
        block_index: usize,
        chapter_title: &str,
        size: Size,
    ) {
        self.showing_cover = false;
        self.load_chapter(self.resolve_bookmark_chapter(chapter, chapter_title), size);
        self.current_page = self
            .pages
            .iter()
            .rposition(|page| page.first_block <= block_index)
            .unwrap_or(0);
        self.refresh_current_image();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent};
    use ratatui::layout::Size;
    use tempfile::NamedTempFile;

    use super::*;
    use crate::book::{BookMeta, Chapter};

    const BOOK_PATH: &str = "/books/test.epub";

    struct DummyReader {
        meta: BookMeta,
        chapters: Vec<Vec<ContentBlock>>,
    }

    impl DummyReader {
        fn new() -> Self {
            Self {
                meta: BookMeta {
                    title: "Test Book".to_string(),
                    author: None,
                    chapters: vec![Chapter {
                        index: 0,
                        title: "Chapter 1".to_string(),
                        resource_id: "chapter-1".to_string(),
                    }],
                },
                chapters: vec![vec![
                    ContentBlock::Paragraph("Page zero".to_string()),
                    ContentBlock::PageBreak,
                    ContentBlock::Paragraph("Page one".to_string()),
                    ContentBlock::PageBreak,
                    ContentBlock::Paragraph("Page two".to_string()),
                ]],
            }
        }
    }

    impl BookReader for DummyReader {
        fn meta(&self) -> &BookMeta {
            &self.meta
        }

        fn chapter_blocks(&self, chapter_idx: usize) -> Result<Vec<ContentBlock>> {
            Ok(self.chapters[chapter_idx].clone())
        }
    }

    fn make_store(path: &PathBuf) -> BookmarkStore {
        BookmarkStore::load_from(path).unwrap()
    }

    fn make_app(store: BookmarkStore) -> App {
        App {
            reader: Box::new(DummyReader::new()),
            mode: Mode::Reading,
            current_chapter: 0,
            current_page: 0,
            pages: Vec::new(),
            pagination_key: None,
            toc_state: ratatui::widgets::ListState::default(),
            bookmark_state: ratatui::widgets::ListState::default(),
            bookmarks: store,
            picker: None,
            book_path: BOOK_PATH.to_string(),
            should_quit: false,
            pending_error: None,
            anim: None,
            showing_cover: false,
            cover_bytes: None,
            current_image: None,
        }
    }

    #[test]
    fn bookmark_jump_restores_saved_page() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut app = make_app(make_store(&path));
        let size = Size::new(40, 8);
        app.load_chapter(0, size);

        let saved_block = app.pages[1].first_block;
        app.bookmarks
            .add(Bookmark::new(BOOK_PATH, 0, saved_block, "Chapter 1"));
        app.mode = Mode::BookmarkOverlay;
        app.bookmark_state.select(Some(0));

        app.handle_key(KeyEvent::from(KeyCode::Enter), size);

        assert_eq!(app.current_page, 1);
        assert_eq!(app.mode, Mode::Reading);
    }

    #[test]
    fn save_shortcut_overwrites_single_bookmark_with_current_page_position() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut app = make_app(make_store(&path));
        let size = Size::new(40, 8);
        app.load_chapter(0, size);

        app.current_page = 1;
        app.handle_key(KeyEvent::from(KeyCode::Char('s')), size);
        app.current_page = 2;
        app.handle_key(KeyEvent::from(KeyCode::Char('s')), size);

        let saved = app.bookmarks.for_book(BOOK_PATH);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].chapter, 0);
        assert_eq!(saved[0].block_index, app.pages[2].first_block);
    }

    #[test]
    fn quitting_persists_current_page_as_bookmark() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut app = make_app(make_store(&path));
        let size = Size::new(40, 8);
        app.load_chapter(0, size);
        app.current_page = 2;

        app.handle_key(KeyEvent::from(KeyCode::Char('q')), size);

        let saved = app.bookmarks.for_book(BOOK_PATH);
        assert!(app.should_quit);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].block_index, app.pages[2].first_block);
    }

    #[test]
    fn current_location_title_prefers_page_section_title() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut app = make_app(make_store(&path));
        let size = Size::new(40, 8);
        app.load_chapter(0, size);
        app.pages[0].section_title = Some("第一章".to_string());

        assert_eq!(app.current_location_title(), "第一章");
    }

    #[test]
    fn bookmark_uses_page_section_title_when_available() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut app = make_app(make_store(&path));
        let size = Size::new(40, 8);
        app.load_chapter(0, size);
        app.pages[0].section_title = Some("第一章".to_string());

        app.handle_key(KeyEvent::from(KeyCode::Char('s')), size);

        let saved = app.bookmarks.for_book(BOOK_PATH);
        assert_eq!(saved[0].chapter_title, "第一章");
    }

    #[test]
    fn bookmark_jump_prefers_saved_title_when_indices_shift() {
        struct ShiftedReader {
            meta: BookMeta,
            chapters: Vec<Vec<ContentBlock>>,
        }

        impl BookReader for ShiftedReader {
            fn meta(&self) -> &BookMeta {
                &self.meta
            }

            fn chapter_blocks(&self, chapter_idx: usize) -> Result<Vec<ContentBlock>> {
                Ok(self.chapters[chapter_idx].clone())
            }
        }

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let store = make_store(&path);
        let mut app = App {
            reader: Box::new(ShiftedReader {
                meta: BookMeta {
                    title: "Shifted".to_string(),
                    author: None,
                    chapters: vec![
                        Chapter {
                            index: 0,
                            title: "序章".to_string(),
                            resource_id: "book#preface".to_string(),
                        },
                        Chapter {
                            index: 1,
                            title: "第一章".to_string(),
                            resource_id: "book#chapter-1".to_string(),
                        },
                        Chapter {
                            index: 2,
                            title: "第二章".to_string(),
                            resource_id: "book#chapter-2".to_string(),
                        },
                    ],
                },
                chapters: vec![
                    vec![ContentBlock::Paragraph("preface".to_string())],
                    vec![ContentBlock::Paragraph("chapter one".to_string())],
                    vec![ContentBlock::Paragraph("chapter two".to_string())],
                ],
            }),
            mode: Mode::BookmarkOverlay,
            current_chapter: 0,
            current_page: 0,
            pages: Vec::new(),
            pagination_key: None,
            toc_state: ratatui::widgets::ListState::default(),
            bookmark_state: ratatui::widgets::ListState::default(),
            bookmarks: store,
            picker: None,
            book_path: BOOK_PATH.to_string(),
            should_quit: false,
            pending_error: None,
            anim: None,
            showing_cover: false,
            cover_bytes: None,
            current_image: None,
        };
        let size = Size::new(40, 8);
        app.bookmarks
            .add(Bookmark::new(BOOK_PATH, 1, 0, "第二章".to_string()));
        app.bookmark_state.select(Some(0));

        app.handle_key(KeyEvent::from(KeyCode::Enter), size);

        assert_eq!(app.current_chapter, 2);
        assert_eq!(app.mode, Mode::Reading);
    }
}
