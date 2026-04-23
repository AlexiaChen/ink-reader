use std::time::Instant;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Size;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

use crate::book::{paginate_blocks, BookReader, ContentBlock, Page, PaginationKey};
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

        let blocks = self
            .reader
            .chapter_blocks(chapter_idx)
            .unwrap_or_else(|_| vec![ContentBlock::Paragraph("[Error loading chapter]".to_string())]);

        self.pages = paginate_blocks(&blocks, size.width, size.height);
        if self.pages.is_empty() {
            self.pages = vec![Page { lines: vec!["[Empty chapter]".to_string()], ..Page::default() }];
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

        if let Ok(dyn_img) = image::load_from_memory(&bytes) {
            if let Some(picker) = &self.picker {
                self.current_image = Some(picker.new_resize_protocol(dyn_img));
            }
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

    fn handle_key_reading(&mut self, key: KeyEvent, size: Size) {
        // Quit always works, even during animation
        if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
            || (key.code == KeyCode::Char('c')
                && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            self.should_quit = true;
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
            KeyCode::Char('p') => {
                if self.current_chapter > 0 {
                    let prev = self.current_chapter - 1;
                    self.load_chapter(prev, size);
                }
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
            // Add bookmark at current position
            KeyCode::Char('a') => {
                let chapter_title = self
                    .reader
                    .meta()
                    .chapters
                    .get(self.current_chapter)
                    .map(|c| c.title.clone())
                    .unwrap_or_default();
                self.bookmarks.add(Bookmark::new(
                    self.book_path.clone(),
                    self.current_chapter,
                    0,
                    chapter_title,
                ));
                let _ = self.bookmarks.save();
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
        let book_key = self.book_path.clone();
        let bm_count = self.bookmarks.for_book(&book_key).len();

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
                let bmarks = self.bookmarks.for_book(&book_key);
                if let Some(idx) = self.bookmark_state.selected()
                    && let Some(bm) = bmarks.get(idx)
                {
                    let chapter = bm.chapter;
                    self.load_chapter(chapter, size);
                }
                self.mode = Mode::Reading;
            }
            KeyCode::Char('d') => {
                // Delete selected bookmark
                if let Some(idx) = self.bookmark_state.selected() {
                    self.bookmarks.remove_for_book(&book_key, idx);
                    let _ = self.bookmarks.save();
                }
                // Adjust selection
                let new_count = self.bookmarks.for_book(&book_key).len();
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

        let old_has_image =
            self.pages.get(self.current_page).map_or(false, |p| p.image.is_some());
        let old_lines =
            self.pages.get(self.current_page).map(|p| p.lines.clone()).unwrap_or_default();

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

        let old_has_image =
            self.pages.get(self.current_page).map_or(false, |p| p.image.is_some());
        let old_lines =
            self.pages.get(self.current_page).map(|p| p.lines.clone()).unwrap_or_default();

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
        if let Some(a) = &self.anim {
            if a.start.elapsed().as_millis() as u64 >= a.duration_ms {
                self.anim = None;
            }
        }
    }
}
