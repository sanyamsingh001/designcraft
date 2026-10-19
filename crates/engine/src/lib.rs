//! The DesignCraft engine façade.
//!
//! Every user-visible action is a command with a stable id (`frame.create`, `text.insert`,
//! `layout.pages.insert`…) and JSON parameters. The egui UI, the CLI, the control channel and MCP
//! all go through [`Session::execute`]. Tools (pointer gestures) are hosted here and reduce to
//! commands, so every gesture is journaled and replayable.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod cmd;
pub mod dtd;
pub mod guard;
pub mod links;
pub mod math;
pub mod recovery;
pub mod sample;
pub mod script;
mod tooling;
pub mod ui_commands;

use std::sync::Arc;

use designcraft_compose::Cache;
use designcraft_doc::{Document, LayerId, Selection};
use serde_json::Value;

pub use cmd::{CommandInfo, CommandSpec, command_specs, find_command};
pub use designcraft_compose as compose;
pub use designcraft_doc as doc;
pub use designcraft_tools as tools;
pub use tooling::{SnapView, UiRequest, ViewInfo};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    #[error("no active document")]
    NoDocument,
    #[error("{0}")]
    Other(String),
    /// A command panicked; the guard kept the document (a bug: please report it).
    #[error("internal error in `{0}` (the document was kept as it was): {1}")]
    Internal(String, String),
}

impl From<designcraft_doc::DocError> for EngineError {
    fn from(e: designcraft_doc::DocError) -> Self {
        EngineError::Other(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub label: String,
    pub doc: Arc<Document>,
    pub selection: Selection,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    pub undo: Vec<HistoryEntry>,
    pub redo: Vec<HistoryEntry>,
    pub limit: usize,
}

#[derive(Clone, Debug)]
pub struct Interaction {
    pub label: String,
    pub doc: Arc<Document>,
    pub selection: Selection,
    pub preview: Option<(String, Value)>,
}

#[derive(Clone, Debug)]
pub struct DocState {
    pub doc: Arc<Document>,
    pub selection: Selection,
    pub history: History,
    pub path: Option<String>,
    pub revision: u64,
    pub saved_revision: u64,
    /// The document as last saved (dirty = the current document is a different allocation).
    pub saved_doc: Arc<Document>,
    pub active_layer: LayerId,
    pub interaction: Option<Interaction>,
    /// Editing parent spreads (Pages panel double-click on a parent).
    pub editing_parents: bool,
    pub uid: u64,
    /// Unfilled document while data-merge preview is showing a record. Not saved.
    pub preview_stash: Option<Arc<Document>>,
    /// 1-based record currently previewed.
    pub preview_record: Option<u32>,
    /// The fonts the document brought (`doc.font_scope`), found while it is open.
    pub fonts: Option<Arc<FontScope>>,
}

/// A document's font scope ([`designcraft_fonts::FontDb::load_document_fonts`]): its fonts are
/// found until the document's state (with every copy of it) is gone — closed, reverted, or its
/// session ended.
#[derive(Debug)]
pub struct FontScope(pub u32);

impl Drop for FontScope {
    fn drop(&mut self) {
        designcraft_fonts::FontDb::global().close_scope(self.0);
    }
}

static NEXT_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl DocState {
    pub fn new(doc: Document, path: Option<String>) -> Self {
        let active_layer = doc.default_layer();
        let doc = Arc::new(doc);
        DocState {
            saved_doc: doc.clone(),
            doc,
            selection: Selection::default(),
            history: History { limit: 1000, ..Default::default() },
            path,
            revision: 1,
            saved_revision: 1,
            active_layer,
            interaction: None,
            editing_parents: false,
            uid: NEXT_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            preview_stash: None,
            preview_record: None,
            fonts: None,
        }
    }
    pub fn is_dirty(&self) -> bool {
        !Arc::ptr_eq(&self.doc, &self.saved_doc)
    }
    pub fn title(&self) -> String {
        self.path
            .as_deref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.doc.title.clone())
    }
}

/// Preferences that the engine needs (UI keeps its own).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub show_hidden_characters: bool,
    pub typographers_quotes: bool,
    /// Polygon Settings (double-click the Polygon tool): number of sides, star inset (0–1).
    pub polygon_sides: u32,
    pub star_inset: f64,
    /// Scaling objects scales their stroke weights (Preferences › General › When Scaling).
    pub scale_strokes: bool,
    /// X/Y/W/H in the Control and Properties panels measure the stroke's outer edge.
    pub dimensions_include_stroke: bool,
    /// Preferences › General › Transformations are Totals: rotation, shear and scale of nested
    /// objects (a graphic in a frame, objects in groups) are measured on the pasteboard rather
    /// than relative to their container.
    pub transformations_are_totals: bool,
    /// Preferences › General › Page Numbering View: Absolute (1, 2, 3… from the first page) instead
    /// of section numbering (the page names, e.g. "iv", "A-3").
    pub absolute_page_numbers: bool,
    /// Preferences › Composition › Highlight: H&J violations, custom tracking/kerning, substituted
    /// fonts (screen only).
    pub highlight_hj: bool,
    pub highlight_keeps: bool,
    pub highlight_custom_tracking: bool,
    pub highlight_substituted_fonts: bool,
    /// Preferences › Appearance of Black › Printing / Exporting: 100% K as rich (pure) black in
    /// RGB output (PNG) instead of the accurate dark grey.
    pub rich_black_output: bool,
    /// Font menu favourites (family names).
    pub favorite_fonts: Vec<String>,
    /// Preferences › Type › Show Font Names in English: the font menus show CJK families by their
    /// English names instead of their native ones. Documents store the English name either way.
    pub show_font_names_in_english: bool,
    /// Preferences › Type › Smart Text Reflow: pages follow the primary text frame's story
    /// (added while it oversets, empty ones at the end removed).
    pub smart_text_reflow: bool,
    /// Preferences › Autocorrect: typing a space or punctuation after a listed word replaces it.
    pub autocorrect: bool,
    /// Misspelled word → correction (lowercase; the correction follows the typed capitalisation).
    pub autocorrect_list: Vec<(String, String)>,
    /// Preferences › Track Changes › Show: added text highlighted on screen.
    pub show_added_text: bool,
    /// Preferences › Notes: note anchors shown in layout view.
    pub show_note_anchors: bool,
    /// Preferences › File Handling: minutes between document recovery saves.
    pub recovery_minutes: f64,
}

/// A starter autocorrect list (common English typing slips).
pub fn default_autocorrect() -> Vec<(String, String)> {
    [
        ("teh", "the"),
        ("adn", "and"),
        ("taht", "that"),
        ("recieve", "receive"),
        ("seperate", "separate"),
        ("occured", "occurred"),
        ("untill", "until"),
        ("wich", "which"),
        ("becuase", "because"),
        ("definately", "definitely"),
        ("thier", "their"),
        ("alot", "a lot"),
    ]
    .iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect()
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            show_hidden_characters: false,
            typographers_quotes: true,
            polygon_sides: 6,
            star_inset: 0.0,
            scale_strokes: true,
            dimensions_include_stroke: true,
            transformations_are_totals: true,
            absolute_page_numbers: false,
            highlight_hj: false,
            highlight_keeps: false,
            highlight_custom_tracking: false,
            highlight_substituted_fonts: true,
            rich_black_output: false,
            favorite_fonts: Vec::new(),
            show_font_names_in_english: false,
            smart_text_reflow: true,
            autocorrect: false,
            autocorrect_list: default_autocorrect(),
            show_added_text: true,
            show_note_anchors: true,
            recovery_minutes: 0.5,
        }
    }
}

pub struct Session {
    docs: Vec<DocState>,
    active: Option<usize>,
    pub prefs: Prefs,
    pub cache: Arc<Cache>,
    pub(crate) tool: Box<dyn designcraft_tools::Tool>,
    pub journal: Vec<(String, Value)>,
    pub clipboard: Option<Arc<Document>>,
    /// Crash-recovery folder (the app sets it; saving or closing a document removes its entry).
    pub recovery_dir: Option<std::path::PathBuf>,
    /// Copied text with its formatting (a frameless story slice) and its plain text.
    pub text_clipboard: Option<(Arc<designcraft_doc::Story>, String)>,
    /// Exact system text emitted for the current object clipboard; never document state.
    pub(crate) object_clipboard_marker: Option<String>,
    /// Random namespace for object ownership, initialized before the first successful Copy/Cut.
    pub(crate) object_clipboard_nonce: Option<[u8; 16]>,
    /// Requests for the UI (dialogs, view changes) produced by tools/commands.
    pub ui_requests: Vec<UiRequest>,
    /// Graphic loaded in the place cursor: (asset, natural size in points).
    pub loaded: Option<(designcraft_doc::AssetId, (f64, f64))>,
    pub(crate) untitled: u32,
    /// Transforms applied to the current selection, oldest first (Transform Again / Sequence
    /// Again), with the selection they were applied to and whether the last one came from a
    /// tool interaction (its previews replace each other).
    pub(crate) transforms: (Vec<(String, Value)>, Vec<designcraft_doc::ItemId>, bool),
    /// Drag and drop text editing: the press landed in the selected text.
    pub(crate) text_drag: bool,
    /// The open Object Library (File › New / Open Library).
    pub library: Option<cmd::library::Library>,
    /// The open book (File › Open Book).
    pub book: Option<cmd::book::Book>,
    /// Content Collector conveyor: collected objects as snippets (name, bytes).
    pub conveyor: Vec<(String, Vec<u8>)>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        guard::install_panic_hook();
        Session {
            docs: vec![],
            active: None,
            prefs: Prefs::default(),
            cache: Arc::new(Cache::new()),
            tool: designcraft_tools::create("selection"),
            journal: vec![],
            clipboard: None,
            recovery_dir: None,
            text_clipboard: None,
            object_clipboard_marker: None,
            object_clipboard_nonce: None,
            ui_requests: vec![],
            loaded: None,
            transforms: Default::default(),
            text_drag: false,
            library: None,
            conveyor: Vec::new(),
            book: None,
            untitled: 0,
        }
    }

    pub fn documents(&self) -> &[DocState] {
        &self.docs
    }
    pub fn active_index(&self) -> Option<usize> {
        self.active
    }
    /// A page's label as the UI shows it (Page Numbering View).
    pub fn page_label(&self, abs: usize) -> String {
        match self.active() {
            Some(d) if !self.prefs.absolute_page_numbers => d.doc.page_name(abs),
            _ => (abs + 1).to_string(),
        }
    }

    /// The page a typed page reference means: `+n` is always absolute; otherwise a section page
    /// name (section numbering) or a position (absolute numbering, or no page has that name).
    pub fn resolve_page(&self, s: &str) -> Option<usize> {
        let d = self.active()?;
        let n = d.doc.page_count();
        let s = s.trim();
        let pos = |t: &str| t.parse::<usize>().ok().filter(|k| (1..=n).contains(k)).map(|k| k - 1);
        if let Some(t) = s.strip_prefix('+') {
            return pos(t);
        }
        if !self.prefs.absolute_page_numbers
            && let Some(i) = (0..n).find(|i| d.doc.page_name(*i).eq_ignore_ascii_case(s))
        {
            return Some(i);
        }
        pos(s)
    }

    pub fn active(&self) -> Option<&DocState> {
        self.active.and_then(|i| self.docs.get(i))
    }
    pub fn active_mut(&mut self) -> Option<&mut DocState> {
        self.active.and_then(|i| self.docs.get_mut(i))
    }
    pub fn doc(&self) -> Result<&DocState> {
        self.active().ok_or(EngineError::NoDocument)
    }
    pub fn doc_mut(&mut self) -> Result<&mut DocState> {
        self.active_mut().ok_or(EngineError::NoDocument)
    }
    pub fn set_active(&mut self, i: usize) {
        if i < self.docs.len() && self.active != Some(i) {
            self.stop_data_preview();
            self.active = Some(i);
        }
    }
    pub fn add_document(&mut self, d: DocState) -> usize {
        self.stop_data_preview();
        self.docs.push(d);
        self.active = Some(self.docs.len() - 1);
        self.docs.len() - 1
    }
    /// Put the unfilled template back. Preview is session state and must not be saved.
    pub fn stop_data_preview(&mut self) {
        let Some(st) = self.active_mut() else { return };
        st.preview_record = None;
        let Some(stash) = st.preview_stash.take() else { return };
        st.doc = stash;
        st.revision = st.revision.saturating_add(1);
    }
    pub fn close_document(&mut self, i: usize) {
        if self.active == Some(i) {
            self.stop_data_preview();
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let (Some(dir), Some(d)) = (&self.recovery_dir, self.docs.get(i)) {
            recovery::discard(dir, d.uid);
        }
        if i < self.docs.len() {
            self.docs.remove(i);
            self.active = if self.docs.is_empty() { None } else { Some(i.min(self.docs.len() - 1)) };
        }
    }

    /// Run a command by id. Edits push one undo step (unless inside an interaction).
    /// Run command `id`. A panic inside it becomes [`EngineError::Internal`] and leaves the
    /// document as it was (see [`guard`]).
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
        self.guarded(id, |s| s.execute_unguarded(id, params))
    }

    fn execute_unguarded(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.into()))?;
        if let Err(e) = (spec.enabled)(self) {
            // A command that takes `ids` acts on the objects the call names, so it needs no
            // selection: with a text caret or nothing selected, `{"ids": [5]}` still runs.
            let named = if e == cmd::NOTHING_SELECTED { cmd::named_targets(spec, params) } else { None };
            let Some(ids) = named else { return Err(EngineError::Disabled(id.into(), e)) };
            let st = self.doc()?;
            if let Some(gone) = ids.iter().find(|i| st.doc.item(**i).is_none()) {
                return Err(EngineError::Disabled(id.into(), format!("no object with id {}", gone.0)));
            }
        }
        // Preview is temporary. Save, merge, and every other edit restore the template first.
        // Queries stay read-only so the panel can poll fields during a preview. `data.preview`
        // itself is the one command that fills the live document.
        if id != "data.preview" && (spec.undoable || spec.journal) {
            self.stop_data_preview();
        }
        let before = self.active().map(|d| (d.uid, d.doc.clone()));
        // Retain the primary composition before the command, even when the cache is cold.
        // Each later command compares against its own input, including an explicit deletion.
        let composition_before = if self.prefs.smart_text_reflow && spec.undoable && self.active().is_some_and(|st| st.interaction.is_none()) {
            before.as_ref().and_then(|(_, doc)| doc.settings.primary_story.map(|sid| self.cache.get(doc, sid, None)))
        } else {
            None
        };
        let r = (spec.run)(self, params)?;
        // Undo, redo or deleting layers can take the active layer away: new objects would land on
        // a layer that isn't there (invisible, not exported). Fall back to the top layer.
        if let Some(st) = self.active_mut()
            && st.doc.layer(st.active_layer).is_none()
        {
            st.active_layer = st.doc.default_layer();
        }
        self.record_transform(id, params);
        // Live captions follow their sources.
        if spec.undoable
            && let Some(st) = self.active_mut()
            && st.interaction.is_none()
            && cmd::captions::has_live(&st.doc)
        {
            let mut d = (*st.doc).clone();
            if cmd::captions::refresh_live(&mut d) {
                st.doc = Arc::new(d);
            }
        }
        // History traversal also changes generated values without replacing source stories.
        // Gestures refresh after their original composition is retained at commit.
        if let Some((uid, old)) = &before
            && self.active().is_some_and(|st| st.uid == *uid && st.interaction.is_none() && !Arc::ptr_eq(old, &st.doc))
        {
            self.invalidate_primary_reference_cache(old);
        }
        // Include automatic story changes (live captions) in the same comparison and undo step.
        if self.prefs.smart_text_reflow && spec.undoable {
            let old = before.as_ref().filter(|(uid, _)| self.active().is_some_and(|st| st.uid == *uid)).map(|(_, doc)| doc.as_ref());
            self.smart_reflow(old, composition_before.as_ref());
        }
        // Record undo if the document changed (and we're not previewing an interaction).
        if let (Some((uid, old)), Some(st)) = (before, self.active_mut())
            && st.uid == uid
            && !Arc::ptr_eq(&old, &st.doc)
            && st.interaction.is_none()
            && spec.undoable
        {
            let entry = HistoryEntry { label: spec.label.to_string(), doc: old, selection: st.selection.clone() };
            push_undo(st, entry);
        }
        if spec.journal {
            self.journal.push((id.to_string(), params.clone()));
        }
        Ok(r)
    }

    // The outer cache signature can miss values consumed only by printed child stories.
    // Keep unchanged/unused references cached; changed source stories already invalidate themselves.
    fn invalidate_primary_reference_cache(&self, before: &Document) {
        let Some(st) = self.active() else { return };
        let Some(sid) = st.doc.settings.primary_story.filter(|sid| before.settings.primary_story == Some(*sid)) else { return };
        let (Some(old), Some(current)) = (before.stories.get(&sid), st.doc.stories.get(&sid)) else { return };
        if Arc::ptr_eq(old, current) && !reflow_reference_inputs_same(before, &st.doc, old, &self.cache, 0) {
            self.cache.clear();
        }
    }

    /// Smart Text Reflow after primary composition inputs change: add threaded pages while it
    /// oversets; remove trailing pages whose only object is an empty frame of it. An explicit
    /// removal from the primary chain is respected until a later content/layout change.
    fn smart_reflow(&mut self, before: Option<&Document>, composition_before: Option<&Arc<designcraft_compose::ComposedStory>>) {
        let Some(st) = self.active() else { return };
        if st.interaction.is_some() {
            return;
        }
        let Some(sid) = st.doc.settings.primary_story else { return };
        let Some(story) = st.doc.story(sid) else { return };
        if let Some(old) = before.filter(|doc| doc.settings.primary_story == Some(sid))
            && (old.page_count() > st.doc.page_count()
                || old.story(sid).is_some_and(|old_story| {
                    if old_story.frames == story.frames {
                        return false;
                    }
                    // Allocate at most one entry per current frame, only when the chain changed.
                    let frames: std::collections::BTreeSet<_> = story.frames.iter().copied().collect();
                    old_story.frames.iter().any(|frame| !frames.contains(frame))
                }))
        {
            // Clear, page deletion and object Cut can remove pages or reduce the chain without
            // deleting text. Do not turn that explicit removal straight back into new frames/pages.
            return;
        }
        let cs = self.cache.get(&st.doc, sid, None);
        if let Some(old) = before.filter(|doc| doc.settings.primary_story == Some(sid))
            && let Some(previous) = composition_before
            && reflow_inputs_same(old, &st.doc, sid, &self.cache)
            && (Arc::ptr_eq(previous, &cs) || reflow_composition_same(previous, &cs, 0))
        {
            // Cache churn alone is not a content/layout edit. Conversely, a cache hit
            // does not cover every nested variable input, so semantics must match first.
            return;
        }
        let overset = cs.overset_at.is_some();
        let empty_tail = story.frames.len() > 1 && story.frames.last().is_some_and(|f| cs.frame(*f).is_none_or(|ft| ft.range.is_empty()));
        if !overset && !empty_tail {
            return;
        }
        let mut d = (*st.doc).clone();
        if overset {
            let _ = cmd::place_text_autoflow(&mut d, sid, 500);
        } else {
            // Drop empty trailing frames on the last pages (one frame, nothing else there).
            while let Some(story) = d.story(sid) {
                if story.frames.len() < 2 {
                    break;
                }
                let Some(&last) = story.frames.last() else { break };
                let cs = self.cache.get(&d, sid, None);
                if cs.frame(last).is_some_and(|ft| !ft.range.is_empty()) {
                    break;
                }
                let Some(page) = d.page_of_item(last) else { break };
                // Only the empty frame itself may be the page's sole root, never its ancestor group.
                let alone = d
                    .spreads
                    .iter()
                    .flat_map(|sp| &sp.items)
                    .filter(|item| d.page_of_item(item.id) == Some(page))
                    .map(|item| item.id)
                    .eq(std::iter::once(last));
                if page + 1 != d.page_count() || !alone || d.page_count() < 2 {
                    break;
                }
                if d.remove_item(last).is_err() || d.delete_pages(&[page]).is_err() {
                    break;
                }
            }
        }
        if let Some(st) = self.active_mut() {
            st.doc = Arc::new(d);
            st.revision += 1;
        }
    }

    /// Remember a move/rotate/scale/shear/flip for Transform Again.
    fn record_transform(&mut self, id: &str, params: &Value) {
        if !matches!(id, "transform.move" | "transform.rotate" | "transform.scale" | "transform.shear" | "transform.flip") {
            return;
        }
        let Some(st) = self.active() else { return };
        let sel = st.selection.items.clone();
        let live = st.interaction.is_some();
        let mut p = params.clone();
        if let Some(o) = p.as_object_mut() {
            o.remove("ids");
            o.remove("id");
        }
        let (list, on, was_live) = &mut self.transforms;
        if *on != sel {
            list.clear();
            *on = sel;
        } else if *was_live && live && list.last().is_some_and(|l| l.0 == id) {
            list.pop();
        }
        list.push((id.to_string(), p));
        *was_live = live;
    }

    pub fn commands(&self) -> Vec<CommandInfo> {
        command_specs().iter().map(|c| c.info(self)).collect()
    }

    /// Mutate the active document (copy-on-write) and bump the revision.
    pub fn edit<T>(&mut self, f: impl FnOnce(&mut Document, &mut Selection) -> Result<T>) -> Result<T> {
        let st = self.doc_mut()?;
        let mut doc = (*st.doc).clone();
        let mut sel = st.selection.clone();
        let r = f(&mut doc, &mut sel)?;
        crate::cmd::sync_placeholders(&mut doc);
        doc.sync_endnote_story();
        st.doc = Arc::new(doc);
        st.selection = sel;
        st.revision += 1;
        Ok(r)
    }
}

// These comparisons belong to the reflow policy, not the conservative rendering cache.
// Ignore edit counters/link bookkeeping and unprinted editorial-note payloads.
// Their anchors remain in the text; printed notes and overset content still matter.
fn reflow_story_same(a: &designcraft_doc::Story, b: &designcraft_doc::Story, depth: usize) -> bool {
    if std::ptr::eq(a, b) {
        return true;
    }
    if depth >= 64 {
        return false;
    }
    if a.id != b.id
        || a.text != b.text
        || a.paras != b.paras
        || a.chars != b.chars
        || a.frames != b.frames
        || a.direction != b.direction
        || a.vertical != b.vertical
        || a.anchors != b.anchors
        || a.xrefs != b.xrefs
        || a.index_refs != b.index_refs
        || a.objects != b.objects
        || a.tables.len() != b.tables.len()
        || a.notes.len() != b.notes.len()
        || a.endnotes.len() != b.endnotes.len()
    {
        return false;
    }
    a.tables.iter().zip(&b.tables).all(|((ka, a), (kb, b))| {
        ka == kb
            && a.id == b.id
            && a.rows == b.rows
            && a.columns == b.columns
            && a.options == b.options
            && a.style == b.style
            && a.cells.len() == b.cells.len()
            && a.cells.iter().zip(&b.cells).all(|(a, b)| {
                (a.row_span, a.col_span, &a.fill, a.fill_tint, a.insets, a.vj, a.rotation, &a.strokes, a.border_overrides, &a.style, &a.graphic)
                    == (
                        b.row_span,
                        b.col_span,
                        &b.fill,
                        b.fill_tint,
                        b.insets,
                        b.vj,
                        b.rotation,
                        &b.strokes,
                        b.border_overrides,
                        &b.style,
                        &b.graphic,
                    )
                    && reflow_story_same(&a.text, &b.text, depth + 1)
            })
    }) && a
        .notes
        .iter()
        .chain(&a.endnotes)
        .zip(b.notes.iter().chain(&b.endnotes))
        .all(|(a, b)| a.id == b.id && reflow_story_same(&a.text, &b.text, depth + 1))
}

fn reflow_composite_same(a: &designcraft_doc::Styles, b: &designcraft_doc::Styles, chars: &designcraft_doc::CharProps) -> bool {
    let family = chars.font_family.trim_start_matches("CompositeFont/");
    a.composite_fonts.iter().find(|font| font.name == family) == b.composite_fonts.iter().find(|font| font.name == family)
}

// Resolve formatting used or referenced by the primary story, including its invisible tail.
// Walk paragraph/run boundaries together; an unused definition must not resume page generation.
fn reflow_formats_same(
    a: &Arc<designcraft_doc::Styles>,
    b: &Arc<designcraft_doc::Styles>,
    story: &designcraft_doc::Story,
    note_style: &str,
    depth: usize,
) -> bool {
    if Arc::ptr_eq(a, b) {
        return true;
    }
    if depth >= 64 {
        return false;
    }
    let mut runs = story.runs().peekable();
    for (range, para) in story.para_ranges().into_iter().zip(&story.paras) {
        let old = a.resolve_para(para);
        if old != b.resolve_para(para) || !reflow_composite_same(a, b, &old.1) {
            return false;
        }
        // These rules inject character styles without recording them in the story's runs.
        // Compare their resolved dependencies even in paragraphs not reached by composition.
        for name in old
            .0
            .nested_styles
            .iter()
            .map(|rule| rule.style.as_str())
            .chain(old.0.grep_styles.iter().map(|rule| rule.style.as_str()))
            .chain(old.0.nested_line_styles.iter().map(|rule| rule.style.as_str()))
            .chain((!story.notes.is_empty()).then_some(note_style))
            .filter(|name| !name.is_empty() && *name != designcraft_doc::NO_CHAR_STYLE)
        {
            let format = designcraft_doc::CharFormat { style: name.into(), ..Default::default() };
            let chars = a.resolve_char(&old.1, &format);
            if chars != b.resolve_char(&old.1, &format) || !reflow_composite_same(a, b, &chars) {
                return false;
            }
        }
        while let Some((run, format)) = runs.peek() {
            if run.start > range.end {
                break;
            }
            let chars = a.resolve_char(&old.1, format);
            if chars != b.resolve_char(&old.1, format) || !reflow_composite_same(a, b, &chars) {
                return false;
            }
            if run.end > range.end {
                break;
            }
            runs.next();
        }
    }
    story.tables.values().flat_map(|table| &table.cells).all(|cell| reflow_formats_same(a, b, &cell.text, note_style, depth + 1))
        && story.notes.iter().chain(&story.endnotes).all(|note| reflow_formats_same(a, b, &note.text, note_style, depth + 1))
}

// Notes, lists and variable/reference controls also apply entirely in the overset tail.
fn reflow_reference_inputs_same(a: &Document, b: &Document, story: &designcraft_doc::Story, cache: &Cache, depth: usize) -> bool {
    if depth >= 64 {
        return false;
    }
    if !story.notes.is_empty() && (a.footnote_options != b.footnote_options || a.footnote_start(story.id) != b.footnote_start(story.id)) {
        return false;
    }
    if !story.endnotes.is_empty()
        && ((a.endnote_options.style, &a.endnote_options.prefix, &a.endnote_options.suffix)
            != (b.endnote_options.style, &b.endnote_options.prefix, &b.endnote_options.suffix)
            || a.endnote_start(story.id) != b.endnote_start(story.id))
    {
        return false;
    }
    for para in &story.paras {
        let (props, _) = a.styles.resolve_para(para);
        if props.list_type == designcraft_doc::ListType::Numbers
            && !props.list_name.is_empty()
            && a.list_start(story.id, &props.list_name) != b.list_start(story.id, &props.list_name)
        {
            return false;
        }
    }
    let mut variables = [false; designcraft_doc::vars::VAR_MAX];
    let mut running = [false; designcraft_doc::vars::VAR_MAX];
    for index in story.text.chars().filter_map(designcraft_doc::vars::var_index) {
        if std::mem::replace(&mut variables[index], true) {
            continue;
        }
        let old = a.text_variables.get(index);
        let new = b.text_variables.get(index);
        use designcraft_doc::vars::VarKind;
        if matches!(old.map(|v| &v.kind), Some(VarKind::RunningHeader { .. })) && matches!(new.map(|v| &v.kind), Some(VarKind::RunningHeader { .. }))
        {
            // Header definitions are inputs to their expanded values, not printable text.
            running[index] = true;
            continue;
        }
        if old != new {
            return false;
        }
        let Some(variable) = old else { continue };
        let same = match &variable.kind {
            VarKind::Custom { .. } | VarKind::OutputDate { .. } => true,
            VarKind::LastPageNumber { .. } => a.page_count() == b.page_count() && a.sections == b.sections,
            VarKind::ChapterNumber => a.settings.chapter_number == b.settings.chapter_number,
            VarKind::FileName { .. } => a.title == b.title,
            VarKind::CreationDate { .. } => a.created == b.created,
            VarKind::ModificationDate { .. } => a.created == b.created && a.modified == b.modified,
            VarKind::RunningHeader { .. } => {
                running[index] = true;
                true
            }
        };
        if !same {
            return false;
        }
    }
    if !story.xrefs.is_empty() {
        // Rendering may invalidate its entire reference bank for an unrelated story edit.
        // Reflow follows only the text these references actually expand to, with the same
        // anchor-page resolver and format contract as composition. Check every reference,
        // including those in the overset tail that have not produced any visible glyphs.
        let old_index = cache.xref_index(a);
        let new_index = cache.xref_index(b);
        if story.xrefs.iter().any(|xref| compose::xref::xref_text(a, xref, Some(&old_index)) != compose::xref::xref_text(b, xref, Some(&new_index))) {
            return false;
        }
    }
    if running.iter().any(|used| *used) && !reflow_running_headers_same(a, b, &running, cache) {
        return false;
    }
    story.tables.values().flat_map(|table| &table.cells).all(|cell| reflow_reference_inputs_same(a, b, &cell.text, cache, depth + 1))
        && story.notes.iter().chain(&story.endnotes).all(|note| reflow_reference_inputs_same(a, b, &note.text, cache, depth + 1))
}

// Use the renderer's matching, cleaned text, page/order and carry-forward contract.
// Compare only header variables present in this story, across the document's possible
// page contexts and the value carried beyond its last page. This bounded dependency
// check also sees changes consumed by a wholly overset control, without treating an
// unrelated source allocation/edit as a primary edit or comparing every story's data.
fn reflow_running_headers_same(a: &Document, b: &Document, used: &[bool; designcraft_doc::vars::VAR_MAX], cache: &Cache) -> bool {
    let old = cache.running_index(a);
    let new = cache.running_index(b);
    let pages = a.page_count().max(b.page_count());
    // Compare definitions through the renderer's expansion, including wrappers and the
    // no-page context. The final page probe includes values carried beyond the document.
    for page in std::iter::once(None).chain((0..=pages).map(Some)) {
        let old_values = compose::vars::values(a, page, Some(&old));
        let new_values = compose::vars::values(b, page, Some(&new));
        if used.iter().enumerate().any(|(index, used)| *used && old_values.get(index) != new_values.get(index)) {
            return false;
        }
    }
    true
}

fn reflow_inputs_same(a: &Document, b: &Document, sid: designcraft_doc::StoryId, cache: &Cache) -> bool {
    let (Some(old), Some(story)) = (a.story(sid), b.story(sid)) else { return false };
    if a.hyphenation_exceptions != b.hyphenation_exceptions
        || compose::advanced_type_inputs(a, old) != compose::advanced_type_inputs(b, story)
        || a.settings.glyph_fallback != b.settings.glyph_fallback
        || a.font_scope != b.font_scope
        || a.sections != b.sections
        || !a.conditions.iter().filter(|condition| !condition.visible).map(|condition| &condition.name).eq(b
            .conditions
            .iter()
            .filter(|condition| !condition.visible)
            .map(|condition| &condition.name))
        || !reflow_story_same(old, story, 0)
        || !reflow_formats_same(&a.styles, &b.styles, old, &a.footnote_options.ref_char_style, 0)
        || !reflow_reference_inputs_same(a, b, old, cache, 0)
    {
        return false;
    }
    reflow_frames_same(a, b, sid)
}

fn reflow_frames_same(a: &Document, b: &Document, sid: designcraft_doc::StoryId) -> bool {
    let (a_story, b_story) = (a.story(sid), b.story(sid));
    let old = compose::frame_specs(a, sid);
    let new = compose::frame_specs(b, sid);
    // Notes and cells use grid-free synthetic frames; table paragraphs bypass line layout.
    let grid_used = a.story(sid).is_some_and(|story| {
        story
            .paras
            .iter()
            .enumerate()
            .any(|(pi, para)| story.para_table(pi).is_none() && a.styles.resolve_para(para).0.grid_align != designcraft_doc::GridAlign::None)
    });
    old.len() == new.len()
        && old.iter().zip(&new).all(|(a, b)| {
            a.id == b.id
                && a.area == b.area
                && a.opts == b.opts
                && a.vertical == b.vertical
                && a.exclusions == b.exclusions
                && a.page_name == b.page_name
                && a.page == b.page
                && (!grid_used || a.grid == b.grid)
                && a.left_page == b.left_page
                && match (a_story, b_story) {
                    (Some(old), Some(story)) => compose::page_anchor_inputs(old, a) == compose::page_anchor_inputs(story, b),
                    _ => false,
                }
        })
}

// Compare final composition values for indirect dependencies such as variables and note numbering.
// ComposedStory has no PartialEq; anchored-object staging values have already been consumed into the final origin.
fn reflow_composition_same(a: &compose::ComposedStory, b: &compose::ComposedStory, depth: usize) -> bool {
    if std::ptr::eq(a, b) {
        return true;
    }
    if depth >= 64
        || a.story != b.story
        || a.text_len != b.text_len
        || a.overset_at != b.overset_at
        || a.styles != b.styles
        || a.frames.len() != b.frames.len()
    {
        return false;
    }
    a.frames.iter().zip(&b.frames).all(|(a, b)| {
        a.frame == b.frame
            && a.vertical == b.vertical
            && a.range == b.range
            && a.columns == b.columns
            && a.content_height == b.content_height
            && a.decos == b.decos
            && a.lines.len() == b.lines.len()
            && a.lines.iter().zip(&b.lines).all(|(a, b)| {
                a.column == b.column
                    && a.baseline == b.baseline
                    && a.x0 == b.x0
                    && a.x1 == b.x1
                    && a.ascent == b.ascent
                    && a.descent == b.descent
                    && a.leading == b.leading
                    && a.range == b.range
                    && a.para == b.para
                    && a.hyphenated == b.hyphenated
                    && a.first_in_para == b.first_in_para
                    && a.last_in_para == b.last_in_para
                    && a.end_x == b.end_x
                    && a.spacing == b.spacing
                    && a.hj == b.hj
                    && a.keep_violation == b.keep_violation
                    && a.glyphs.len() == b.glyphs.len()
                    && a.glyphs.iter().zip(&b.glyphs).all(|(a, b)| {
                        (a.face, a.gid, a.x, a.y, a.adv, a.sx, a.sy, a.style, a.byte, a.len, a.visible, a.upright)
                            == (b.face, b.gid, b.x, b.y, b.adv, b.sx, b.sy, b.style, b.byte, b.len, b.visible, b.upright)
                            && a.tcy == b.tcy
                            && a.rtl == b.rtl
                    })
            })
            && a.objects.len() == b.objects.len()
            && a.objects.iter().zip(&b.objects).all(|(a, b)| (a.index, a.origin, a.size, a.line) == (b.index, b.origin, b.size, b.line))
            && a.notes.len() == b.notes.len()
            && a.notes.iter().zip(&b.notes).all(|(a, b)| {
                (a.id, a.index, a.column, &a.label, a.origin, a.rect, &a.source) == (b.id, b.index, b.column, &b.label, b.origin, b.rect, &b.source)
                    && reflow_composition_same(&a.text, &b.text, depth + 1)
            })
            && a.tables.len() == b.tables.len()
            && a.tables.iter().zip(&b.tables).all(|(a, b)| {
                (a.table, a.anchor, a.column, a.rect, a.line, a.first, a.last) == (b.table, b.anchor, b.column, b.rect, b.line, b.first, b.last)
                    && a.strokes == b.strokes
                    && a.cells.len() == b.cells.len()
                    && a.cells.iter().zip(&b.cells).all(|(a, b)| {
                        (a.row, a.col, a.rect, &a.fill, a.origin, &a.source, a.clip, a.overset, a.repeated, &a.graphic)
                            == (b.row, b.col, b.rect, &b.fill, b.origin, &b.source, b.clip, b.overset, b.repeated, &b.graphic)
                            && reflow_composition_same(&a.text, &b.text, depth + 1)
                    })
            })
    })
}

fn push_undo(st: &mut DocState, e: HistoryEntry) {
    st.history.undo.push(e);
    st.history.redo.clear();
    if st.history.undo.len() > st.history.limit.max(1) {
        st.history.undo.remove(0);
    }
}

pub(crate) use push_undo as record_undo;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod resize_scale_strokes_tests {
    use std::sync::Arc;

    use designcraft_doc::{ItemId, SpreadRef};
    use designcraft_geom::{Affine, Point, Rect, Unit};
    use designcraft_tools::{Mods, PointerEvent, PointerKind, SnapView};
    use serde_json::json;

    use super::{Session, ViewInfo};

    fn assert_rect(actual: Rect, expected: Rect) {
        for (have, want) in [(actual.x0, expected.x0), (actual.y0, expected.y0), (actual.x1, expected.x1), (actual.y1, expected.y1)] {
            assert!((have - want).abs() < 1e-6, "{actual:?}, want {expected:?}");
        }
    }

    fn expected_rect(horizontal: bool, alt: bool, shift: bool, size: f64) -> Rect {
        // The fixture starts at [100,100,200,150]. Its opposite midpoint is fixed
        // without Alt; Alt fixes [150,125]. Shift retains the original 2:1 aspect.
        let (w, h) = if horizontal { (size, if shift { size / 2.0 } else { 50.0 }) } else { (if shift { size * 2.0 } else { 100.0 }, size) };
        if alt {
            Rect::new(150.0 - w / 2.0, 125.0 - h / 2.0, 150.0 + w / 2.0, 125.0 + h / 2.0)
        } else if horizontal {
            Rect::new(100.0, 125.0 - h / 2.0, 100.0 + w, 125.0 + h / 2.0)
        } else {
            Rect::new(150.0 - w / 2.0, 100.0, 150.0 + w / 2.0, 100.0 + size)
        }
    }

    fn resize_case(horizontal: bool, reachable: bool, alt: bool, shift: bool, scale_strokes: bool) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let moving =
            ItemId(s.execute("frame.create", &json!({"rect": [100, 100, 200, 150], "content": "unassigned"})).unwrap()["id"].as_u64().unwrap());
        s.execute("object.stroke", &json!({"ids": [moving.0], "weight": 4, "swatch": "Black", "align": "center"})).unwrap();
        let mut references = Vec::new();
        for (x, size) in [(300.0, 3.0), (400.0, 8.0)] {
            if size == 8.0 && !reachable {
                continue;
            }
            let (w, h) = if horizontal { (size, 80.0) } else { (80.0, size) };
            let id = ItemId(
                s.execute("frame.create", &json!({"rect": [x, 300.0, x + w, 300.0 + h], "content": "unassigned"})).unwrap()["id"].as_u64().unwrap(),
            );
            s.execute("object.stroke", &json!({"ids": [id.0], "swatch": "None", "weight": 0})).unwrap();
            references.push(id);
        }
        s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
        s.execute("prefs.set", &json!({"scaleStrokes": scale_strokes})).unwrap();
        assert_eq!(s.prefs.scale_strokes, scale_strokes);
        s.set_tool("selection");
        // These immutable documents are the geometry/reference and Undo oracles.
        let before = s.doc().unwrap().doc.clone();
        let undo_count = s.doc().unwrap().history.undo.len();
        let view =
            ViewInfo { zoom: 1.0, snap: SnapView { smart_guides: true, smart_dimensions: true, zone_px: 6.0, ..SnapView::OFF }, unit: Unit::Points };
        let mods = Mods { cmd: true, alt, shift, ..Mods::default() };
        let down = if horizontal { Point::new(200.0, 125.0) } else { Point::new(150.0, 150.0) };
        let drag = if horizontal { Point::new(if alt { 150.5 } else { 101.0 }, 125.0) } else { Point::new(150.0, if alt { 125.5 } else { 101.0 }) };
        let layout = s.layout();
        for (kind, at) in [(PointerKind::Down, down), (PointerKind::Drag, drag), (PointerKind::Up, drag)] {
            let at = layout.to_canvas(SpreadRef::Doc(0), at);
            s.pointer(&PointerEvent::new(kind, at.x, at.y).with_mods(mods), view).unwrap();
        }

        // OFF rejects the unreachable 3 pt target; ON must actually produce
        // that visible size, using the stroke scale of the final path geometry.
        let (signed_size, weight) = if !scale_strokes {
            (if reachable { 4.0 } else { 1.0 }, 4.0)
        } else if shift {
            // Uniform scaling: visible target / original visible driver size.
            let k = 3.0 / if horizontal { 104.0 } else { 54.0 };
            (if horizontal { 100.0 * k } else { 50.0 * k }, 4.0 * k)
        } else if horizontal {
            (2.382_576_169_033_49, 0.617_423_830_966_508)
        } else {
            (2.167_226_108_320_73, 0.832_773_891_679_269)
        };
        let expected = expected_rect(horizontal, alt, shift, signed_size);
        let after = s.doc().unwrap().doc.clone();
        let item = after.item(moving).unwrap();
        assert_eq!(item.xf, Affine::IDENTITY);
        assert_rect(item.path.bounds().unwrap(), expected.abs());
        assert_rect(item.bounds(), expected.abs());
        assert_rect(item.visible_bounds(), expected.abs().inflate(weight / 2.0, weight / 2.0));
        assert!((item.stroke.weight - weight).abs() < 1e-6, "stroke {} want {weight}", item.stroke.weight);
        let visible_size = if horizontal { item.visible_bounds().width() } else { item.visible_bounds().height() };
        let want_visible = if scale_strokes {
            3.0
        } else if reachable {
            8.0
        } else {
            5.0
        };
        assert!((visible_size - want_visible).abs() < 1e-6, "visible size {visible_size} want {want_visible}");
        assert_eq!(item.path.subpaths.len(), 1);
        let path = &item.path.subpaths[0];
        assert!(path.closed);
        assert_eq!(path.anchors.len(), 4);
        // Bounds alone normalize a reflection. Check the committed path's
        // ordered corners as well so an unintended negative span cannot pass.
        for (anchor, want) in path.anchors.iter().zip([
            Point::new(expected.x0, expected.y0),
            Point::new(expected.x1, expected.y0),
            Point::new(expected.x1, expected.y1),
            Point::new(expected.x0, expected.y1),
        ]) {
            assert!((anchor.p - want).hypot() < 1e-6, "committed corner {:?} want {want:?}", anchor.p);
        }
        for id in references {
            assert_eq!(after.item(id), before.item(id), "resizing must leave the reference document items unchanged");
        }
        assert_eq!(s.doc().unwrap().selection.items, vec![moving]);
        assert!(s.doc().unwrap().interaction.is_none(), "Up must commit the actual tool gesture");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
        let (command, params) = s.journal.last().unwrap();
        assert_eq!(command, "transform.resize");
        assert_eq!(params["content"], true);
        assert_eq!(params["distribute"], false);
        assert!(s.overlays(view).is_empty(), "Up clears dimension feedback");
        after.check().unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before), "one Undo restores the complete original geometry");
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &after), "Redo restores the committed bounds and stroke");
    }

    #[test]
    fn command_resize_scale_strokes_off_chooses_reachable_dimensions() {
        // First case is R6: right midpoint [200,125] -> [101,125], with 3 and
        // 8 pt references. The candidate commits x1=99 instead of x1=104.
        for horizontal in [true, false] {
            for alt in [false, true] {
                for shift in [false, true] {
                    resize_case(horizontal, true, alt, shift, false);
                }
            }
        }
    }

    #[test]
    fn command_resize_scale_strokes_off_keeps_proposal_without_reachable_dimensions() {
        for horizontal in [true, false] {
            for alt in [false, true] {
                for shift in [false, true] {
                    resize_case(horizontal, false, alt, shift, false);
                }
            }
        }
    }

    #[test]
    fn command_resize_scale_strokes_on_matches_visible_dimensions() {
        for horizontal in [true, false] {
            for reachable in [false, true] {
                for alt in [false, true] {
                    for shift in [false, true] {
                        resize_case(horizontal, reachable, alt, shift, true);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod scaled_resize_pointer_tests {
    use std::sync::Arc;

    use designcraft_doc::{Item, ItemId, SpreadRef};
    use designcraft_geom::{Affine, Point, Rect, Unit, format_measure};
    use designcraft_tools::{Mods, Overlay, PointerEvent, PointerKind, SnapView};
    use serde_json::{Value, json};

    use super::{Session, ViewInfo};

    fn close(have: f64, want: f64) {
        assert!((have - want).abs() < 1e-6, "have {have}, want {want}");
    }

    fn rect(have: Rect, want: Rect) {
        for (a, b) in [(have.x0, want.x0), (have.y0, want.y0), (have.x1, want.x1), (have.y1, want.y1)] {
            close(a, b);
        }
    }

    fn frame(s: &mut Session, bounds: Rect, stroke: bool, align: &str) -> ItemId {
        // frame.create enforces a 0.5 pt initial side. Use the public resize
        // command to create smaller reference fixtures, before taking the oracle.
        let created = Rect::new(bounds.x0, bounds.y0, bounds.x1.max(bounds.x0 + 0.5), bounds.y1.max(bounds.y0 + 0.5));
        let id = ItemId(
            s.execute("frame.create", &json!({"rect": [created.x0, created.y0, created.x1, created.y1], "content": "unassigned"})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        s.execute(
            "object.stroke",
            &json!({"ids": [id.0], "swatch": if stroke { "Black" } else { "None" }, "weight": if stroke { 4 } else { 0 }, "align": align}),
        )
        .unwrap();
        if created != bounds {
            s.execute("transform.resize", &json!({"ids": [id.0], "from": [created.x0, created.y0, created.x1, created.y1], "to": [bounds.x0, bounds.y0, bounds.x1, bounds.y1], "content": false})).unwrap();
        }
        id
    }

    fn view() -> ViewInfo {
        ViewInfo { zoom: 1.0, snap: SnapView { smart_guides: true, smart_dimensions: true, zone_px: 6.0, ..SnapView::OFF }, unit: Unit::Points }
    }

    fn pointer(s: &mut Session, kind: PointerKind, at: Point, mods: Mods, view: ViewInfo) {
        let canvas = s.layout().to_canvas(SpreadRef::Doc(0), at);
        s.pointer(&PointerEvent::new(kind, canvas.x, canvas.y).with_mods(mods), view).unwrap();
    }

    #[derive(Clone, Copy)]
    struct Side {
        horizontal: bool,
        low: bool,
        alt: bool,
        shift: bool,
        reflected: bool,
    }

    impl Side {
        fn down(self) -> Point {
            if self.horizontal {
                Point::new(if self.low { 100.0 } else { 200.0 }, 125.0)
            } else {
                Point::new(150.0, if self.low { 100.0 } else { 150.0 })
            }
        }

        fn at(self, size: f64) -> Point {
            let value = if self.alt {
                let center = if self.horizontal { 150.0 } else { 125.0 };
                center + if self.low { -size / 2.0 } else { size / 2.0 }
            } else {
                let fixed = if self.low { if self.horizontal { 200.0 } else { 150.0 } } else { 100.0 };
                fixed + if self.low { -size } else { size }
            };
            if self.horizontal { Point::new(value, 125.0) } else { Point::new(150.0, value) }
        }

        fn expected(self, magnitude: f64) -> Rect {
            let signed = if self.reflected { -magnitude } else { magnitude };
            let (w, h) = if self.horizontal {
                (signed, if self.shift { signed / 2.0 } else { 50.0 })
            } else {
                (if self.shift { signed * 2.0 } else { 100.0 }, signed)
            };
            if self.alt {
                Rect::new(150.0 - w / 2.0, 125.0 - h / 2.0, 150.0 + w / 2.0, 125.0 + h / 2.0)
            } else if self.horizontal {
                let (x0, x1) = if self.low { (200.0 - w, 200.0) } else { (100.0, 100.0 + w) };
                Rect::new(x0, 125.0 - h / 2.0, x1, 125.0 + h / 2.0)
            } else {
                let (y0, y1) = if self.low { (150.0 - h, 150.0) } else { (100.0, 100.0 + h) };
                Rect::new(150.0 - w / 2.0, y0, 150.0 + w / 2.0, y1)
            }
        }
    }

    fn committed_geometry(item: &Item, expected: Rect, weight: f64, pad: f64) {
        assert_eq!(item.xf, Affine::IDENTITY);
        rect(item.path.bounds().unwrap(), expected.abs());
        rect(item.bounds(), expected.abs());
        rect(item.visible_bounds(), expected.abs().inflate(pad / 2.0, pad / 2.0));
        close(item.stroke.weight, weight);
        assert_eq!(item.path.subpaths.len(), 1);
        let path = &item.path.subpaths[0];
        assert!(path.closed);
        assert_eq!(path.anchors.len(), 4);
        for (anchor, expected) in path.anchors.iter().zip([
            Point::new(expected.x0, expected.y0),
            Point::new(expected.x1, expected.y0),
            Point::new(expected.x1, expected.y1),
            Point::new(expected.x0, expected.y1),
        ]) {
            assert!((anchor.p - expected).hypot() < 1e-6, "corner {:?}, want {expected:?}", anchor.p);
        }
    }

    // Oracle independent of tools' solver: uniform ratios or the positive root
    // of D + P*sqrt(D/B) = T, derived from the engine's documented stroke policy.
    fn oracle(side: Side, target: Option<f64>, original_pad: f64, on: bool) -> (f64, f64, f64) {
        let base = if side.horizontal { 100.0 } else { 50.0 };
        let magnitude = match target {
            None => 1.0,
            Some(t) if !on => t - original_pad,
            Some(t) if side.shift => t * base / (base + original_pad),
            Some(t) => {
                let a = original_pad / base.sqrt();
                let root = ((a * a + 4.0 * t).sqrt() - a) / 2.0;
                root * root
            }
        };
        let k = if !on {
            1.0
        } else if side.shift {
            magnitude / base
        } else {
            (magnitude / base).sqrt()
        };
        (magnitude, 4.0 * k, original_pad * k)
    }

    fn side_case(side: Side, references: &[f64], target: Option<f64>, many: bool, on: bool, align: &str) -> (Item, Value) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let moving = frame(&mut s, Rect::new(100.0, 100.0, 200.0, 150.0), true, align);
        let mut ids = Vec::new();
        for (i, size) in references.iter().enumerate() {
            let x = 300.0 + 100.0 * i as f64;
            let (w, h) = if side.horizontal { (*size, 80.0) } else { (80.0, *size) };
            ids.push(frame(&mut s, Rect::new(x, 300.0, x + w, 300.0 + h), false, "center"));
        }
        s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
        s.execute("prefs.set", &json!({"scaleStrokes": on})).unwrap();
        s.set_tool("selection");
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let mods = Mods { cmd: true, alt: side.alt, shift: side.shift, ..Mods::default() };
        let view = view();
        pointer(&mut s, PointerKind::Down, side.down(), mods, view);
        if many {
            for size in [40.0, -12.0, 6.0] {
                pointer(&mut s, PointerKind::Drag, side.at(size), mods, view);
            }
        }
        let at = side.at(if side.reflected { -1.0 } else { 1.0 });
        pointer(&mut s, PointerKind::Drag, at, mods, view);
        let measures: Vec<_> =
            s.overlays(view).into_iter().filter_map(|o| if let Overlay::Measure { text, .. } = o { Some(text) } else { None }).collect();
        if let Some(target) = target {
            assert!(measures.contains(&format_measure(target, Unit::Points)), "target {target} missing: {measures:?}");
        } else {
            assert!(measures.is_empty(), "unreachable dimensions must not advertise a match: {measures:?}");
        }
        pointer(&mut s, PointerKind::Up, at, mods, view);
        let original_pad = match align {
            "inside" => 0.0,
            "outside" => 8.0,
            _ => 4.0,
        };
        let (size, weight, pad) = oracle(side, target, original_pad, on);
        let expected = side.expected(size);
        let after = s.doc().unwrap().doc.clone();
        let item = after.item(moving).unwrap().clone();
        committed_geometry(&item, expected, weight, pad);
        let visible = if side.horizontal { item.visible_bounds().width() } else { item.visible_bounds().height() };
        close(visible, target.unwrap_or(size + pad));
        for id in ids {
            assert_eq!(before.item(id), after.item(id), "reference changed");
        }
        assert_eq!(s.doc().unwrap().selection.items, vec![moving]);
        assert!(s.doc().unwrap().interaction.is_none());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
        let (command, params) = s.journal.last().unwrap().clone();
        assert_eq!(command, "transform.resize");
        assert_eq!(params["content"], true);
        assert_eq!(params["distribute"], false);
        let values: Vec<_> = params["to"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        rect(Rect::new(values[0], values[1], values[2], values[3]), expected);
        assert!(s.overlays(view).is_empty());
        after.check().unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &after));
        (item, params)
    }

    #[test]
    fn scaled_visible_dimension_matches_preserve_axes_anchors_and_reflections() {
        for horizontal in [true, false] {
            for low in [false, true] {
                for alt in [false, true] {
                    for shift in [false, true] {
                        for reflected in [false, true] {
                            let side = Side { horizontal, low, alt, shift, reflected };
                            let once = side_case(side, &[3.0, 8.0], Some(3.0), false, true, "center");
                            let repeated = side_case(side, &[3.0, 8.0], Some(3.0), true, true, "center");
                            assert_eq!(once, repeated, "preview count must not change final geometry or stroke");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn scaled_dimensions_rank_final_visible_size_and_filter_scale_floor() {
        for horizontal in [true, false] {
            for alt in [false, true] {
                for shift in [false, true] {
                    for reflected in [false, true] {
                        let side = Side { horizontal, low: false, alt, shift, reflected };
                        // 0.05 is nearer than 3, but below the engine's 0.001
                        // per-axis floor. 3 is feasible with scaled strokes.
                        side_case(side, &[0.05, 3.0], Some(3.0), true, true, "center");
                        side_case(side, &[0.05], None, true, true, "center");
                        side_case(side, &[], None, false, true, "center");
                        // Old-outset request=5 selects 5; final visible proposal
                        // is around 1, so it should select the 1.5 pt alternative.
                        side_case(side, &[1.5, 5.0], Some(1.5), true, true, "center");
                        // Fixed-stroke control still rejects 3 and matches 8.
                        side_case(side, &[3.0, 8.0], Some(8.0), true, false, "center");
                    }
                }
            }
        }
    }

    #[test]
    fn scaled_dimensions_follow_inside_and_outside_stroke_outsets() {
        for align in ["inside", "outside"] {
            for horizontal in [true, false] {
                for shift in [false, true] {
                    let side = Side { horizontal, low: true, alt: true, shift, reflected: true };
                    side_case(side, &[3.0, 8.0], Some(3.0), true, true, align);
                }
            }
        }
    }

    #[test]
    fn scaled_resize_uses_current_policy_and_original_stroke_during_gesture() {
        for (initial_cmd, initial_pref, final_cmd, final_pref, target) in
            [(true, true, true, false, 8.0), (true, true, false, true, 8.0), (true, false, true, true, 3.0), (false, true, true, true, 3.0)]
        {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1})).unwrap();
            let moving = frame(&mut s, Rect::new(100.0, 100.0, 200.0, 150.0), true, "center");
            let r1 = frame(&mut s, Rect::new(300.0, 300.0, 303.0, 380.0), false, "center");
            let r2 = frame(&mut s, Rect::new(400.0, 300.0, 408.0, 380.0), false, "center");
            s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
            s.execute("prefs.set", &json!({"scaleStrokes": initial_pref})).unwrap();
            s.set_tool("selection");
            let before = s.doc().unwrap().doc.clone();
            let view = view();
            let initial = Mods { cmd: initial_cmd, ..Mods::default() };
            pointer(&mut s, PointerKind::Down, Point::new(200.0, 125.0), initial, view);
            pointer(&mut s, PointerKind::Drag, Point::new(140.0, 125.0), initial, view);
            s.execute("prefs.set", &json!({"scaleStrokes": final_pref})).unwrap();
            let final_mods = Mods { cmd: final_cmd, ..Mods::default() };
            pointer(&mut s, PointerKind::Drag, Point::new(101.0, 125.0), final_mods, view);
            pointer(&mut s, PointerKind::Up, Point::new(101.0, 125.0), final_mods, view);
            let side = Side { horizontal: true, low: false, alt: false, shift: false, reflected: false };
            let (size, weight, pad) = oracle(side, Some(target), 4.0, final_cmd && final_pref);
            let item = s.doc().unwrap().doc.item(moving).unwrap();
            committed_geometry(item, side.expected(size), weight, pad);
            close(item.visible_bounds().width(), target);
            assert_eq!(s.journal.last().unwrap().1["content"], final_cmd);
            assert_eq!(s.doc().unwrap().doc.item(r1), before.item(r1));
            assert_eq!(s.doc().unwrap().doc.item(r2), before.item(r2));
        }
    }

    #[test]
    fn scaled_uniform_dimension_at_engine_floor_remains_reachable() {
        for horizontal in [true, false] {
            let target = if horizontal { 0.104 } else { 0.054 };
            let side = Side { horizontal, low: false, alt: true, shift: true, reflected: true };
            side_case(side, &[0.01, target, 3.0], Some(target), true, true, "center");
        }
    }

    #[test]
    fn scaled_corner_dimension_accounts_for_other_axis_position_snap() {
        for alt in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1})).unwrap();
            let moving = frame(&mut s, Rect::new(100.0, 100.0, 200.0, 150.0), true, "center");
            let reference = frame(&mut s, Rect::new(300.0, 150.0, 303.0, 230.0), false, "center");
            s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
            s.execute("prefs.set", &json!({"scaleStrokes": true})).unwrap();
            s.set_tool("selection");
            let before = s.doc().unwrap().doc.clone();
            let mods = Mods { cmd: true, alt, ..Mods::default() };
            let view = ViewInfo { snap: SnapView { align_edges: true, ..view().snap }, ..view() };
            pointer(&mut s, PointerKind::Down, Point::new(200.0, 150.0), mods, view);
            pointer(&mut s, PointerKind::Drag, Point::new(180.0, 140.0), mods, view);
            let at = if alt { Point::new(150.5, 149.0) } else { Point::new(101.0, 149.0) };
            pointer(&mut s, PointerKind::Drag, at, mods, view);
            pointer(&mut s, PointerKind::Up, at, mods, view);
            // y=149 snaps to 150; Alt mirrors that position delta, restoring
            // the original 50 pt height. Solve the x target using that height.
            let d = 2.382_576_169_033_49;
            let weight = 0.617_423_830_966_508;
            let expected = if alt { Rect::new(150.0 - d / 2.0, 100.0, 150.0 + d / 2.0, 150.0) } else { Rect::new(100.0, 100.0, 100.0 + d, 150.0) };
            let item = s.doc().unwrap().doc.item(moving).unwrap();
            committed_geometry(item, expected, weight, weight);
            close(item.visible_bounds().width(), 3.0);
            assert_eq!(s.doc().unwrap().doc.item(reference), before.item(reference));
        }
    }

    fn fixed_floor_case(side: Side, references: &[f64], target: Option<f64>, content: bool, many: bool) -> (Item, Value) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages":1})).unwrap();
        let moving = frame(&mut s, Rect::new(100.0, 100.0, 200.0, 150.0), true, "center");
        let mut ids = Vec::new();
        for (i, size) in references.iter().enumerate() {
            let x = 300.0 + 100.0 * i as f64;
            let (w, h) = if side.horizontal { (*size, 80.0) } else { (80.0, *size) };
            ids.push(frame(&mut s, Rect::new(x, 300.0, x + w, 300.0 + h), false, "center"));
        }
        s.execute("selection.set", &json!({"ids":[moving.0]})).unwrap();
        // Cmd has Scale Strokes OFF; ordinary frame resizing has it ON to
        // verify that the preference alone never scales a frame-only stroke.
        s.execute("prefs.set", &json!({"scaleStrokes":!content})).unwrap();
        s.set_tool("selection");
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let mods = Mods { cmd: content, alt: side.alt, shift: side.shift, ..Mods::default() };
        let view = view();
        pointer(&mut s, PointerKind::Down, side.down(), mods, view);
        if many {
            for size in [40.0, -12.0, 6.0] {
                pointer(&mut s, PointerKind::Drag, side.at(size), mods, view);
            }
        }
        let at = side.at(if side.reflected { -1.0 } else { 1.0 });
        pointer(&mut s, PointerKind::Drag, at, mods, view);
        let measures: Vec<_> =
            s.overlays(view).into_iter().filter_map(|o| if let Overlay::Measure { text, .. } = o { Some(text) } else { None }).collect();
        if let Some(target) = target {
            assert!(measures.contains(&format_measure(target, Unit::Points)), "missing target {target}: {measures:?}");
        } else {
            assert!(measures.is_empty(), "unreachable target advertised: {measures:?}");
        }
        pointer(&mut s, PointerKind::Up, at, mods, view);
        let magnitude = target.map_or(1.0, |target| target - 4.0);
        let expected = side.expected(magnitude);
        let after = s.doc().unwrap().doc.clone();
        let item = after.item(moving).unwrap().clone();
        committed_geometry(&item, expected, 4.0, 4.0);
        close(if side.horizontal { item.visible_bounds().width() } else { item.visible_bounds().height() }, target.unwrap_or(5.0));
        for id in ids {
            assert_eq!(after.item(id), before.item(id));
        }
        let (command, params) = s.journal.last().unwrap().clone();
        assert_eq!(command, "transform.resize");
        assert_eq!(params["content"], content);
        assert_eq!(params["distribute"], false);
        let values: Vec<_> = params["to"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        rect(Rect::new(values[0], values[1], values[2], values[3]), expected);
        assert!(s.overlays(view).is_empty());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &after));
        (item, params)
    }

    #[test]
    fn scaled_fixed_stroke_floor_rejects_exact_outset_targets() {
        for horizontal in [true, false] {
            for alt in [false, true] {
                for shift in [false, true] {
                    for reflected in [false, true] {
                        for content in [true, false] {
                            let side = Side { horizontal, low: false, alt, shift, reflected };
                            for (references, target) in [(&[4.0, 8.0][..], Some(8.0)), (&[4.0][..], None)] {
                                let once = fixed_floor_case(side, references, target, content, false);
                                let repeated = fixed_floor_case(side, references, target, content, true);
                                assert_eq!(once, repeated, "fixed stroke must use pointer-down inputs");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn scaled_fixed_stroke_floor_boundary_remains_reachable() {
        for horizontal in [true, false] {
            for alt in [false, true] {
                for shift in [false, true] {
                    for content in [true, false] {
                        let side = Side { horizontal, low: false, alt, shift, reflected: true };
                        let target = if horizontal { 4.1 } else { 4.05 };
                        fixed_floor_case(side, &[4.0, target, 8.0], Some(target), content, true);
                    }
                }
            }
        }
    }

    #[test]
    fn scaled_zero_outset_text_group_multiple_and_nested_geometry_keep_dimension_matching() {
        for kind in ["text", "group", "multiple", "nested"] {
            for horizontal in [true, false] {
                for alt in [false, true] {
                    for shift in [false, true] {
                        for reflected in [false, true] {
                            let mut s = Session::new();
                            s.execute("file.new", &json!({"pages": 1})).unwrap();
                            let (selected, leaves) = if kind == "text" {
                                let id = ItemId(
                                    s.execute("frame.create", &json!({"rect": [100,100,200,150], "content": "text", "caret": false})).unwrap()["id"]
                                        .as_u64()
                                        .unwrap(),
                                );
                                s.execute("object.stroke", &json!({"ids": [id.0], "swatch": "None", "weight": 0})).unwrap();
                                (vec![id], vec![id])
                            } else {
                                let a_bounds =
                                    if kind == "nested" { Rect::new(100.0, 100.0, 200.0, 150.0) } else { Rect::new(100.0, 100.0, 145.0, 150.0) };
                                let b_bounds =
                                    if kind == "nested" { Rect::new(130.0, 110.0, 170.0, 140.0) } else { Rect::new(160.0, 100.0, 200.0, 150.0) };
                                let a = frame(&mut s, a_bounds, false, "center");
                                let b = frame(&mut s, b_bounds, false, "center");
                                if kind == "multiple" {
                                    (vec![a, b], vec![a, b])
                                } else {
                                    let group = ItemId(s.execute("object.group", &json!({"ids": [a.0,b.0]})).unwrap()["id"].as_u64().unwrap());
                                    // Selecting only a nested child with identity parents is
                                    // distinct from predicting strokes under arbitrary groups.
                                    if kind == "nested" { (vec![a], vec![a]) } else { (vec![group], vec![a, b]) }
                                }
                            };
                            let (w, h) = if horizontal { (3.0, 80.0) } else { (80.0, 3.0) };
                            let reference = frame(&mut s, Rect::new(300.0, 300.0, 300.0 + w, 300.0 + h), false, "center");
                            s.execute("selection.set", &json!({"ids": selected.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
                            s.execute("prefs.set", &json!({"scaleStrokes": true})).unwrap();
                            s.set_tool("selection");
                            let before = s.doc().unwrap().doc.clone();
                            let undo = s.doc().unwrap().history.undo.len();
                            let side = Side { horizontal, low: false, alt, shift, reflected };
                            let mods = Mods { cmd: true, alt, shift, ..Mods::default() };
                            let view = view();
                            pointer(&mut s, PointerKind::Down, side.down(), mods, view);
                            for size in [40.0, -12.0] {
                                pointer(&mut s, PointerKind::Drag, side.at(size), mods, view);
                            }
                            let at = side.at(if reflected { -1.0 } else { 1.0 });
                            pointer(&mut s, PointerKind::Drag, at, mods, view);
                            assert!(
                                s.overlays(view).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == "3 pt")),
                                "{kind}: missing dimension feedback"
                            );
                            pointer(&mut s, PointerKind::Up, at, mods, view);
                            let expected = side.expected(3.0);
                            let after = s.doc().unwrap().doc.clone();
                            let actual = selected
                                .iter()
                                .map(|id| {
                                    let loc = after.find(*id).unwrap();
                                    let item = after.item(*id).unwrap();
                                    let parent = after.parent_xf(&loc);
                                    assert_eq!(item.bounds(), item.visible_bounds(), "{kind}: zero-outset model must remain zero");
                                    parent.transform_rect_bbox(item.bounds())
                                })
                                .reduce(|a, b| a.union(b))
                                .unwrap();
                            rect(actual, expected.abs());
                            close(if horizontal { actual.width() } else { actual.height() }, 3.0);
                            // Check each committed leaf in spread space, including the group's
                            // transform branch and the individually baked multiple-selection branch.
                            let m = Affine::translate((expected.x0, expected.y0))
                                * Affine::scale_non_uniform(expected.width() / 100.0, expected.height() / 50.0)
                                * Affine::translate((-100.0, -100.0));
                            for id in leaves {
                                let old_loc = before.find(id).unwrap();
                                let new_loc = after.find(id).unwrap();
                                let old = before.parent_xf(&old_loc).transform_rect_bbox(before.item(id).unwrap().bounds());
                                let new = after.parent_xf(&new_loc).transform_rect_bbox(after.item(id).unwrap().bounds());
                                rect(new, m.transform_rect_bbox(old));
                                assert!(after.item(id).unwrap().stroke.is_none());
                            }
                            let (command, params) = s.journal.last().unwrap();
                            assert_eq!(command, "transform.resize");
                            assert_eq!(params["content"], true);
                            let values: Vec<_> = params["to"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
                            rect(Rect::new(values[0], values[1], values[2], values[3]), expected);
                            assert_eq!(after.item(reference), before.item(reference));
                            assert!(s.overlays(view).is_empty());
                            assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
                            s.execute("edit.undo", &json!({})).unwrap();
                            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
                            s.execute("edit.redo", &json!({})).unwrap();
                            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &after));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn scaled_dimensions_skip_text_transform_policy_without_false_feedback() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let moving = ItemId(
            s.execute("frame.create", &json!({"rect": [100,100,200,150], "content": "text", "caret": false})).unwrap()["id"].as_u64().unwrap(),
        );
        s.execute("object.stroke", &json!({"ids": [moving.0], "swatch": "Black", "weight": 4, "align": "center"})).unwrap();
        let reference = frame(&mut s, Rect::new(300.0, 300.0, 303.0, 380.0), false, "center");
        s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
        s.execute("prefs.set", &json!({"scaleStrokes": true})).unwrap();
        s.set_tool("selection");
        let before = s.doc().unwrap().doc.clone();
        let mods = Mods { cmd: true, ..Mods::default() };
        let view = view();
        pointer(&mut s, PointerKind::Down, Point::new(200.0, 125.0), mods, view);
        pointer(&mut s, PointerKind::Drag, Point::new(101.0, 125.0), mods, view);
        assert!(!s.overlays(view).iter().any(|o| matches!(o, Overlay::Measure { .. })));
        pointer(&mut s, PointerKind::Up, Point::new(101.0, 125.0), mods, view);
        rect(s.doc().unwrap().doc.item(moving).unwrap().bounds(), Rect::new(100.0, 100.0, 101.0, 150.0));
        close(s.doc().unwrap().doc.item(moving).unwrap().stroke.weight, 4.0);
        assert_eq!(s.doc().unwrap().doc.item(reference), before.item(reference));
    }

    #[test]
    fn scaled_graphic_frame_matches_visible_dimensions_from_original_stroke() {
        // A synthetic 1x1 RGBA PNG, loaded through the preexisting public
        // in-memory place API; no external asset or fixture mutation.
        const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR4nGP4DwQACfsD/fteaysAAAAASUVORK5CYII=";
        for horizontal in [true, false] {
            for shift in [false, true] {
                let mut s = Session::new();
                s.execute("file.new", &json!({"pages": 1})).unwrap();
                let moving = frame(&mut s, Rect::new(100.0, 100.0, 200.0, 150.0), true, "center");
                s.execute("place.load", &json!({"base64": PIXEL, "name": "resize-pixel.png"})).unwrap();
                s.execute("place.drop", &json!({"frame": moving.0})).unwrap();
                assert!(matches!(s.doc().unwrap().doc.item(moving).unwrap().content, designcraft_doc::Content::Graphic(_)));
                let reference = frame(&mut s, Rect::new(300.0, 300.0, 303.0, 380.0), false, "center");
                s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
                s.execute("prefs.set", &json!({"scaleStrokes": true})).unwrap();
                s.set_tool("selection");
                let before = s.doc().unwrap().doc.clone();
                let side = Side { horizontal, low: true, alt: true, shift, reflected: true };
                let mods = Mods { cmd: true, alt: true, shift, ..Mods::default() };
                let view = view();
                pointer(&mut s, PointerKind::Down, side.down(), mods, view);
                pointer(&mut s, PointerKind::Drag, side.at(40.0), mods, view);
                pointer(&mut s, PointerKind::Drag, side.at(-1.0), mods, view);
                pointer(&mut s, PointerKind::Up, side.at(-1.0), mods, view);
                let (size, weight, pad) = oracle(side, Some(3.0), 4.0, true);
                let item = s.doc().unwrap().doc.item(moving).unwrap();
                committed_geometry(item, side.expected(size), weight, pad);
                close(if horizontal { item.visible_bounds().width() } else { item.visible_bounds().height() }, 3.0);
                assert_eq!(s.doc().unwrap().doc.item(reference), before.item(reference));
                assert_eq!(s.doc().unwrap().doc.assets, before.assets);
            }
        }
    }

    #[test]
    fn scaled_corner_dimensions_solve_shared_stroke_scale() {
        for alt in [false, true] {
            for sign_x in [-1.0, 1.0] {
                for sign_y in [-1.0, 1.0] {
                    for many in [false, true] {
                        let mut s = Session::new();
                        s.execute("file.new", &json!({"pages": 1})).unwrap();
                        let moving = frame(&mut s, Rect::new(100.0, 100.0, 200.0, 150.0), true, "center");
                        let reference = frame(&mut s, Rect::new(300.0, 300.0, 303.0, 380.0), false, "center");
                        s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
                        s.execute("prefs.set", &json!({"scaleStrokes": true})).unwrap();
                        s.set_tool("selection");
                        let before = s.doc().unwrap().doc.clone();
                        let mods = Mods { cmd: true, alt, ..Mods::default() };
                        let view = view();
                        pointer(&mut s, PointerKind::Down, Point::new(200.0, 150.0), mods, view);
                        if many {
                            pointer(&mut s, PointerKind::Drag, Point::new(180.0, 140.0), mods, view);
                            pointer(&mut s, PointerKind::Drag, Point::new(120.0, 110.0), mods, view);
                        }
                        let at =
                            if alt { Point::new(150.0 + sign_x / 2.0, 125.0 + sign_y / 2.0) } else { Point::new(100.0 + sign_x, 100.0 + sign_y) };
                        pointer(&mut s, PointerKind::Drag, at, mods, view);
                        let labels = s.overlays(view).into_iter().filter(|o| matches!(o, Overlay::Measure { text, .. } if text == "3 pt")).count();
                        assert_eq!(labels, 2);
                        pointer(&mut s, PointerKind::Up, at, mods, view);
                        // Both final path magnitudes are D; mean scale is
                        // D/sqrt(100*50), so D*(1+4/sqrt(5000))=3.
                        let d = 3.0 / (1.0 + 4.0 / 5000.0_f64.sqrt());
                        let weight = 3.0 - d;
                        let (w, h) = (sign_x * d, sign_y * d);
                        let expected = if alt {
                            Rect::new(150.0 - w / 2.0, 125.0 - h / 2.0, 150.0 + w / 2.0, 125.0 + h / 2.0)
                        } else {
                            Rect::new(100.0, 100.0, 100.0 + w, 100.0 + h)
                        };
                        let item = s.doc().unwrap().doc.item(moving).unwrap();
                        committed_geometry(item, expected, weight, weight);
                        close(item.visible_bounds().width(), 3.0);
                        close(item.visible_bounds().height(), 3.0);
                        assert_eq!(s.doc().unwrap().doc.item(reference), before.item(reference));
                    }
                }
            }
        }
    }

    #[test]
    fn scaled_resize_keeps_position_deltas_in_spread_coordinates() {
        for horizontal in [true, false] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1})).unwrap();
            let moving = frame(&mut s, Rect::new(100.0, 100.0, 200.0, 150.0), true, "center");
            let bounds = if horizontal { Rect::new(97.0, 300.0, 177.0, 380.0) } else { Rect::new(300.0, 97.0, 380.0, 177.0) };
            let reference = frame(&mut s, bounds, false, "center");
            s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
            s.execute("prefs.set", &json!({"scaleStrokes": true})).unwrap();
            s.set_tool("selection");
            let before = s.doc().unwrap().doc.clone();
            let side = Side { horizontal, low: false, alt: false, shift: false, reflected: true };
            let mods = Mods { cmd: true, ..Mods::default() };
            let view = ViewInfo { snap: SnapView { align_edges: true, ..view().snap }, ..view() };
            pointer(&mut s, PointerKind::Down, side.down(), mods, view);
            pointer(&mut s, PointerKind::Drag, side.at(-1.0), mods, view);
            pointer(&mut s, PointerKind::Up, side.at(-1.0), mods, view);
            let expected = side.expected(3.0); // 99 -> 97 is a -2 position delta.
            let base: f64 = if horizontal { 100.0 } else { 50.0 };
            let weight = 4.0 * (3.0 / base).sqrt();
            committed_geometry(s.doc().unwrap().doc.item(moving).unwrap(), expected, weight, weight);
            assert_eq!(s.doc().unwrap().doc.item(reference), before.item(reference));
        }
    }

    #[derive(Clone, Copy)]
    enum R7aTextStroke {
        Centered,
        Inside,
        None,
    }

    impl R7aTextStroke {
        fn weight(self) -> f64 {
            if matches!(self, Self::None) { 0.0 } else { 4.0 }
        }

        fn pad(self) -> f64 {
            if matches!(self, Self::Centered) { 4.0 } else { 0.0 }
        }
    }

    #[derive(Clone, Copy)]
    struct R7aTextResizeCase<'a> {
        side: Side,
        references: &'a [f64],
        target: Option<f64>,
        content: bool,
        scale_strokes: bool,
        stroke: R7aTextStroke,
        many: bool,
    }

    fn r7a_text_frame(s: &mut Session, stroke: R7aTextStroke) -> ItemId {
        let id = ItemId(
            s.execute(
                "frame.create",
                &json!({
                    "rect": [100,100,200,150], "content": "text", "caret": false, "text": "seed"
                }),
            )
            .unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        s.execute(
            "object.stroke",
            &json!({
                "ids": [id.0], "swatch": if matches!(stroke, R7aTextStroke::None) { "None" } else { "Black" },
                "weight": stroke.weight(), "align": if matches!(stroke, R7aTextStroke::Inside) { "inside" } else { "center" }
            }),
        )
        .unwrap();
        id
    }

    fn r7a_sides() -> Vec<Side> {
        let mut sides = Vec::new();
        for horizontal in [true, false] {
            for low in [false, true] {
                for alt in [false, true] {
                    for shift in [false, true] {
                        for reflected in [false, true] {
                            sides.push(Side { horizontal, low, alt, shift, reflected });
                        }
                    }
                }
            }
        }
        sides
    }

    fn r7a_dimension_feedback(s: &mut Session, view: ViewInfo, target: Option<f64>) {
        let labels: Vec<_> =
            s.overlays(view).into_iter().filter_map(|overlay| if let Overlay::Measure { text, .. } = overlay { Some(text) } else { None }).collect();
        let expected: Vec<_> = target.into_iter().map(|value| format_measure(value, Unit::Points)).collect();
        assert_eq!(labels, expected, "dimension feedback must describe the final reachable geometry");
    }

    fn r7a_history(s: &mut Session, before: &Arc<designcraft_doc::Document>, after: &Arc<designcraft_doc::Document>, undo: usize) {
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, before));
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, after));
    }

    fn r7a_text_case(case: R7aTextResizeCase<'_>) -> (Item, Value) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let moving = r7a_text_frame(&mut s, case.stroke);
        let mut references = Vec::new();
        for (index, length) in case.references.iter().enumerate() {
            let x = 300.0 + 100.0 * index as f64;
            let (w, h) = if case.side.horizontal { (*length, 80.0) } else { (80.0, *length) };
            references.push(frame(&mut s, Rect::new(x, 300.0, x + w, 300.0 + h), false, "center"));
        }
        s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
        s.execute("prefs.set", &json!({"scaleStrokes": case.scale_strokes})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let journal = s.journal.clone();
        let mods = Mods { cmd: case.content, alt: case.side.alt, shift: case.side.shift, ..Mods::default() };
        let view = view();
        let at = case.side.at(if case.side.reflected { -1.0 } else { 1.0 });
        // Explicit engine cancellation is separate from the unchanged UI Escape route.
        for cancel in [true, false] {
            s.set_tool("selection");
            pointer(&mut s, PointerKind::Down, case.side.down(), mods, view);
            if case.many {
                for size in [40.0, -12.0, 6.0] {
                    pointer(&mut s, PointerKind::Drag, case.side.at(size), mods, view);
                }
            }
            pointer(&mut s, PointerKind::Drag, at, mods, view);
            if cancel {
                s.cancel_interaction();
                assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
                assert_eq!(s.doc().unwrap().selection.items, vec![moving]);
                assert!(s.doc().unwrap().interaction.is_none());
                assert_eq!(s.doc().unwrap().history.undo.len(), undo);
                assert_eq!(s.journal, journal);
                s.set_tool("directSelection");
                s.set_tool("selection");
                assert!(s.overlays(view).is_empty());
            }
        }
        r7a_dimension_feedback(&mut s, view, case.target);
        pointer(&mut s, PointerKind::Up, at, mods, view);
        let magnitude = case.target.map_or(1.0, |target| target - case.stroke.pad());
        let expected = case.side.expected(magnitude);
        let m = Affine::translate((expected.x0, expected.y0))
            * Affine::scale_non_uniform(expected.width() / 100.0, expected.height() / 50.0)
            * Affine::translate((-100.0, -100.0));
        let k = ((expected.width() / 100.0) * (expected.height() / 50.0)).abs().sqrt();
        let weight = if case.content && !case.scale_strokes { case.stroke.weight() / k } else { case.stroke.weight() };
        let pad = if matches!(case.stroke, R7aTextStroke::Centered) { weight } else { 0.0 };
        let after = s.doc().unwrap().doc.clone();
        let item = after.item(moving).unwrap().clone();
        if case.content {
            assert_eq!(item.path, before.item(moving).unwrap().path, "content scaling preserves the inner path");
            for (actual, wanted) in item.xf.as_coeffs().into_iter().zip(m.as_coeffs()) {
                close(actual, wanted);
            }
            rect(item.bounds(), expected.abs());
            rect(item.visible_bounds(), expected.abs().inflate(pad / 2.0, pad / 2.0));
            close(item.stroke.weight, weight);
        } else {
            committed_geometry(&item, expected, weight, pad);
        }
        assert_eq!(item.content, before.item(moving).unwrap().content);
        let sid = item.text_frame().unwrap().story;
        assert_eq!(after.story(sid), before.story(sid), "resizing must preserve text and formatting");
        for reference in references {
            assert_eq!(after.item(reference), before.item(reference));
        }
        if let Some(target) = case.target {
            close(if case.side.horizontal { item.visible_bounds().width() } else { item.visible_bounds().height() }, target);
        }
        assert_eq!(s.doc().unwrap().selection.items, vec![moving]);
        assert!(s.doc().unwrap().interaction.is_none());
        assert!(s.overlays(view).is_empty());
        after.check().unwrap();
        let (command, params) = s.journal.last().unwrap().clone();
        assert_eq!(command, "transform.resize");
        assert_eq!(params["from"], json!([100.0, 100.0, 200.0, 150.0]));
        assert_eq!(params["content"], case.content);
        assert_eq!(params["distribute"], false);
        let values: Vec<_> = params["to"].as_array().unwrap().iter().map(|value| value.as_f64().unwrap()).collect();
        rect(Rect::new(values[0], values[1], values[2], values[3]), expected);
        r7a_history(&mut s, &before, &after, undo);
        (item, params)
    }

    fn r7a_stroked_content_selection_case(kind: &str, scale_strokes: bool) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let a = frame(&mut s, Rect::new(100.0, 100.0, 145.0, 150.0), true, "center");
        let b = frame(&mut s, Rect::new(160.0, 100.0, 200.0, 150.0), true, "center");
        let selected = if kind == "multiple" {
            vec![a, b]
        } else {
            let group = ItemId(s.execute("object.group", &json!({"ids": [a.0,b.0]})).unwrap()["id"].as_u64().unwrap());
            if kind == "nested" { vec![a] } else { vec![group] }
        };
        let reference_a = frame(&mut s, Rect::new(300.0, 300.0, 303.0, 380.0), false, "center");
        let reference_b = frame(&mut s, Rect::new(400.0, 300.0, 408.0, 380.0), false, "center");
        s.execute("selection.set", &json!({"ids": selected.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
        s.execute("prefs.set", &json!({"scaleStrokes": scale_strokes})).unwrap();
        s.set_tool("selection");
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let from = selected.iter().map(|id| before.item(*id).unwrap().bounds()).reduce(|a, b| a.union(b)).unwrap();
        let expected = Rect::new(from.x0, from.y0, from.x0 + 1.0, from.y1);
        let m = Affine::translate((expected.x0, expected.y0))
            * Affine::scale_non_uniform(1.0 / from.width(), 1.0)
            * Affine::translate((-from.x0, -from.y0));
        let k = (1.0 / from.width()).sqrt();
        let mods = Mods { cmd: true, ..Mods::default() };
        let view = view();
        pointer(&mut s, PointerKind::Down, Point::new(from.x1, from.center().y), mods, view);
        for size in [20.0, -12.0, 1.0] {
            pointer(&mut s, PointerKind::Drag, Point::new(from.x0 + size, from.center().y), mods, view);
        }
        r7a_dimension_feedback(&mut s, view, None);
        pointer(&mut s, PointerKind::Up, Point::new(from.x0 + 1.0, from.center().y), mods, view);
        let after = s.doc().unwrap().doc.clone();
        let bounds = selected.iter().map(|id| after.item(*id).unwrap().bounds()).reduce(|a, b| a.union(b)).unwrap();
        rect(bounds, expected);
        for child in [a, b] {
            if kind == "nested" && child == b {
                assert_eq!(after.item(child), before.item(child));
                continue;
            }
            let old = before.item(child).unwrap();
            let item = after.item(child).unwrap();
            let loc = after.find(child).unwrap();
            rect(after.parent_xf(&loc).transform_rect_bbox(item.bounds()), m.transform_rect_bbox(old.bounds()));
            if kind == "group" {
                assert_eq!(item.path, old.path);
                assert_eq!(item.xf, old.xf);
                close(item.stroke.weight, if scale_strokes { 4.0 } else { 4.0 / k });
            } else {
                assert_eq!(item.xf, old.xf);
                close(item.stroke.weight, if scale_strokes { 4.0 * k } else { 4.0 });
            }
        }
        for reference in [reference_a, reference_b] {
            assert_eq!(after.item(reference), before.item(reference));
        }
        assert_eq!(s.doc().unwrap().selection.items, selected);
        assert!(s.overlays(view).is_empty());
        let (command, params) = s.journal.last().unwrap();
        assert_eq!(command, "transform.resize");
        assert_eq!(params["content"], true);
        let values: Vec<_> = params["to"].as_array().unwrap().iter().map(|value| value.as_f64().unwrap()).collect();
        rect(Rect::new(values[0], values[1], values[2], values[3]), expected);
        r7a_history(&mut s, &before, &after, undo);
    }

    #[test]
    fn r7a_text_content_resize_off_suppresses_false_dimensions() {
        // R7A-CORE-1 independently fails BEFORE: the candidate advertises 8 pt,
        // requests width 4, and commits stored stroke 20 / visible width 24.
        r7a_text_case(R7aTextResizeCase {
            side: Side { horizontal: true, low: false, alt: false, shift: false, reflected: false },
            references: &[8.0],
            target: None,
            content: true,
            scale_strokes: false,
            stroke: R7aTextStroke::Centered,
            many: false,
        });
        for side in r7a_sides() {
            for scale_strokes in [false, true] {
                let case = R7aTextResizeCase {
                    side,
                    references: &[3.0, 4.0, 8.0],
                    target: None,
                    content: true,
                    scale_strokes,
                    stroke: R7aTextStroke::Centered,
                    many: false,
                };
                let once = r7a_text_case(case);
                let repeated = r7a_text_case(R7aTextResizeCase { many: true, ..case });
                assert_eq!(once, repeated, "unsupported content policies must never gain fallback dimension feedback");
            }
        }
        // Unverified grouped, nested and multiple stroked selections also suppress matches.
        for kind in ["group", "nested", "multiple"] {
            for scale_strokes in [false, true] {
                r7a_stroked_content_selection_case(kind, scale_strokes);
            }
        }
    }

    #[test]
    fn r7a_text_frame_resize_filters_fixed_outset_floor() {
        // R7A-CORE-2 independently fails BEFORE: 4 pt is accepted instead of 8,
        // then the command's .001 floor commits visible width 4.1.
        for side in r7a_sides() {
            for scale_strokes in [true, false] {
                for (references, target) in [(&[4.0, 8.0][..], Some(8.0)), (&[3.0, 4.0][..], None)] {
                    let case =
                        R7aTextResizeCase { side, references, target, content: false, scale_strokes, stroke: R7aTextStroke::Centered, many: false };
                    let once = r7a_text_case(case);
                    let repeated = r7a_text_case(R7aTextResizeCase { many: true, ..case });
                    assert_eq!(once, repeated, "reject an unreachable closest target before considering an alternative");
                }
            }
        }
    }

    #[test]
    fn r7a_text_frame_resize_keeps_reachable_fixed_dimensions() {
        for side in r7a_sides() {
            for scale_strokes in [false, true] {
                r7a_text_case(R7aTextResizeCase {
                    side,
                    references: &[8.0],
                    target: Some(8.0),
                    content: false,
                    scale_strokes,
                    stroke: R7aTextStroke::Centered,
                    many: true,
                });
            }
        }
    }

    #[test]
    fn r7a_text_frame_resize_keeps_reachable_floor_boundary() {
        for side in r7a_sides() {
            let target = if side.horizontal { 4.1 } else { 4.05 };
            for scale_strokes in [false, true] {
                r7a_text_case(R7aTextResizeCase {
                    side,
                    references: &[target],
                    target: Some(target),
                    content: false,
                    scale_strokes,
                    stroke: R7aTextStroke::Centered,
                    many: true,
                });
            }
        }
    }

    #[test]
    fn r7a_zero_outset_resize_keeps_supported_dimensions() {
        let sides = [
            Side { horizontal: true, low: false, alt: false, shift: false, reflected: false },
            Side { horizontal: true, low: true, alt: true, shift: true, reflected: true },
            Side { horizontal: false, low: false, alt: true, shift: false, reflected: true },
            Side { horizontal: false, low: true, alt: false, shift: true, reflected: false },
        ];
        for side in sides {
            for scale_strokes in [false, true] {
                for content in [false, true] {
                    for stroke in [R7aTextStroke::None, R7aTextStroke::Inside] {
                        r7a_text_case(R7aTextResizeCase {
                            side,
                            references: &[0.025, 3.0],
                            target: Some(3.0),
                            content,
                            scale_strokes,
                            stroke,
                            many: true,
                        });
                    }
                }
            }
        }
        for kind in ["group", "multiple", "nested"] {
            for scale_strokes in [false, true] {
                let mut s = Session::new();
                s.execute("file.new", &json!({"pages": 1})).unwrap();
                let a_bounds = if kind == "nested" { Rect::new(100.0, 100.0, 200.0, 150.0) } else { Rect::new(100.0, 100.0, 145.0, 150.0) };
                let b_bounds = if kind == "nested" { Rect::new(130.0, 110.0, 170.0, 140.0) } else { Rect::new(160.0, 100.0, 200.0, 150.0) };
                let a = frame(&mut s, a_bounds, false, "center");
                let b = frame(&mut s, b_bounds, false, "center");
                let selected = if kind == "multiple" {
                    vec![a, b]
                } else {
                    let group = ItemId(s.execute("object.group", &json!({"ids": [a.0,b.0]})).unwrap()["id"].as_u64().unwrap());
                    if kind == "nested" { vec![a] } else { vec![group] }
                };
                let reference = frame(&mut s, Rect::new(300.0, 300.0, 303.0, 380.0), false, "center");
                s.execute("selection.set", &json!({"ids": selected.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
                s.execute("prefs.set", &json!({"scaleStrokes": scale_strokes})).unwrap();
                s.set_tool("selection");
                let before = s.doc().unwrap().doc.clone();
                let undo = s.doc().unwrap().history.undo.len();
                let side = Side { horizontal: true, low: false, alt: true, shift: true, reflected: true };
                let mods = Mods { cmd: true, alt: true, shift: true, ..Mods::default() };
                let view = view();
                pointer(&mut s, PointerKind::Down, side.down(), mods, view);
                for size in [40.0, -12.0, -1.0] {
                    pointer(&mut s, PointerKind::Drag, side.at(size), mods, view);
                }
                r7a_dimension_feedback(&mut s, view, Some(3.0));
                pointer(&mut s, PointerKind::Up, side.at(-1.0), mods, view);
                let expected = side.expected(3.0);
                let m = Affine::translate((expected.x0, expected.y0))
                    * Affine::scale_non_uniform(expected.width() / 100.0, expected.height() / 50.0)
                    * Affine::translate((-100.0, -100.0));
                let after = s.doc().unwrap().doc.clone();
                for child in [a, b] {
                    if kind == "nested" && child == b {
                        assert_eq!(after.item(child), before.item(child));
                        continue;
                    }
                    let loc = after.find(child).unwrap();
                    let old = before.item(child).unwrap().bounds();
                    let item = after.item(child).unwrap();
                    rect(after.parent_xf(&loc).transform_rect_bbox(item.bounds()), m.transform_rect_bbox(old));
                    assert!(item.stroke.is_none());
                }
                assert_eq!(after.item(reference), before.item(reference));
                assert_eq!(s.doc().unwrap().selection.items, selected);
                assert!(s.overlays(view).is_empty());
                r7a_history(&mut s, &before, &after, undo);
            }
        }
    }

    #[test]
    fn r7a_text_content_resize_keeps_position_snapping() {
        for scale_strokes in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1})).unwrap();
            let moving = r7a_text_frame(&mut s, R7aTextStroke::Centered);
            let aligned = frame(&mut s, Rect::new(97.0, 300.0, 177.0, 380.0), false, "center");
            let dimension = frame(&mut s, Rect::new(300.0, 300.0, 303.0, 380.0), false, "center");
            s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
            s.execute("prefs.set", &json!({"scaleStrokes": scale_strokes})).unwrap();
            s.set_tool("selection");
            let before = s.doc().unwrap().doc.clone();
            let undo = s.doc().unwrap().history.undo.len();
            let side = Side { horizontal: true, low: false, alt: false, shift: false, reflected: true };
            let mods = Mods { cmd: true, ..Mods::default() };
            let view = ViewInfo { snap: SnapView { align_edges: true, ..view().snap }, ..view() };
            pointer(&mut s, PointerKind::Down, side.down(), mods, view);
            for size in [40.0, -12.0, -1.0] {
                pointer(&mut s, PointerKind::Drag, side.at(size), mods, view);
            }
            r7a_dimension_feedback(&mut s, view, None);
            assert!(s.overlays(view).iter().any(|o| matches!(o, Overlay::Guide { .. })), "position snap must remain active");
            pointer(&mut s, PointerKind::Up, side.at(-1.0), mods, view);
            let after = s.doc().unwrap().doc.clone();
            let item = after.item(moving).unwrap();
            rect(item.bounds(), side.expected(3.0).abs());
            assert_eq!(item.path, before.item(moving).unwrap().path);
            close(item.xf.as_coeffs()[0], -0.03);
            close(item.xf.as_coeffs()[3], 1.0);
            let weight = if scale_strokes { 4.0 } else { 4.0 / 0.03_f64.sqrt() };
            close(item.stroke.weight, weight);
            close(item.visible_bounds().width(), 3.0 + weight);
            assert_eq!(after.item(aligned), before.item(aligned));
            assert_eq!(after.item(dimension), before.item(dimension));
            assert!(s.overlays(view).is_empty());
            r7a_history(&mut s, &before, &after, undo);
        }
    }

    #[test]
    fn r7a_text_resize_policy_changes_keep_original_inputs() {
        // Final ordinary resizing works after either content stroke policy preview.
        // The inverse transition finishes with the already supported safe ON suppression.
        for (initial_cmd, initial_scale, final_cmd, final_scale, target) in [
            (true, false, false, true, Some(8.0)),
            (true, true, false, false, Some(8.0)),
            (false, false, true, true, None),
            (true, false, true, true, None),
        ] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1})).unwrap();
            let moving = r7a_text_frame(&mut s, R7aTextStroke::Centered);
            let reference = frame(&mut s, Rect::new(300.0, 300.0, 308.0, 380.0), false, "center");
            s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
            s.execute("prefs.set", &json!({"scaleStrokes": initial_scale})).unwrap();
            s.set_tool("selection");
            let before = s.doc().unwrap().doc.clone();
            let undo = s.doc().unwrap().history.undo.len();
            let side = Side { horizontal: true, low: false, alt: false, shift: false, reflected: false };
            let initial = Mods { cmd: initial_cmd, ..Mods::default() };
            let final_mods = Mods { cmd: final_cmd, ..Mods::default() };
            let view = view();
            pointer(&mut s, PointerKind::Down, side.down(), initial, view);
            pointer(&mut s, PointerKind::Drag, side.at(40.0), initial, view);
            s.execute("prefs.set", &json!({"scaleStrokes": final_scale})).unwrap();
            pointer(&mut s, PointerKind::Drag, side.at(1.0), final_mods, view);
            r7a_dimension_feedback(&mut s, view, target);
            pointer(&mut s, PointerKind::Up, side.at(1.0), final_mods, view);
            let after = s.doc().unwrap().doc.clone();
            let item = after.item(moving).unwrap();
            let size = if final_cmd { 1.0 } else { 4.0 };
            rect(item.bounds(), side.expected(size));
            close(item.stroke.weight, 4.0);
            close(item.visible_bounds().width(), size + 4.0);
            if final_cmd {
                assert_eq!(item.path, before.item(moving).unwrap().path);
            } else {
                assert_eq!(item.xf, Affine::IDENTITY);
            }
            assert_eq!(item.content, before.item(moving).unwrap().content);
            assert_eq!(after.item(reference), before.item(reference));
            assert_eq!(s.journal.last().unwrap().1["content"], final_cmd);
            assert!(s.overlays(view).is_empty());
            r7a_history(&mut s, &before, &after, undo);
        }
    }

    #[test]
    fn r7a_text_resize_cancel_preserves_original_document() {
        for scale_strokes in [false, true] {
            for content in [false, true] {
                let mut s = Session::new();
                s.execute("file.new", &json!({"pages": 1})).unwrap();
                let moving = r7a_text_frame(&mut s, R7aTextStroke::Centered);
                let reference = frame(&mut s, Rect::new(300.0, 300.0, 308.0, 380.0), false, "center");
                s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
                s.execute("prefs.set", &json!({"scaleStrokes": scale_strokes})).unwrap();
                s.set_tool("selection");
                let before = s.doc().unwrap().doc.clone();
                let journal = s.journal.clone();
                let undo = s.doc().unwrap().history.undo.len();
                let side = Side { horizontal: true, low: false, alt: true, shift: true, reflected: true };
                let mods = Mods { cmd: content, alt: true, shift: true, ..Mods::default() };
                let view = view();
                pointer(&mut s, PointerKind::Down, side.down(), mods, view);
                for size in [40.0, -12.0, -1.0] {
                    pointer(&mut s, PointerKind::Drag, side.at(size), mods, view);
                }
                assert!(!Arc::ptr_eq(&s.doc().unwrap().doc, &before));
                s.cancel_interaction();
                assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
                assert_eq!(s.doc().unwrap().selection.items, vec![moving]);
                assert_eq!(s.doc().unwrap().doc.item(reference), before.item(reference));
                assert_eq!(s.doc().unwrap().history.undo.len(), undo);
                assert_eq!(s.journal, journal);
                assert!(s.doc().unwrap().interaction.is_none());
                s.set_tool("directSelection");
                s.set_tool("selection");
                assert!(s.overlays(view).is_empty());
            }
        }
    }

    #[derive(Clone, Copy)]
    struct R7aRefinementCase<'a> {
        kind: &'a str,
        side: Side,
        references: &'a [f64],
        stroked: bool,
        scale_strokes: bool,
        dimension: Option<f64>,
        position: bool,
        many: bool,
    }

    fn r7a_refinement_sides() -> [Side; 4] {
        [
            Side { horizontal: true, low: false, alt: false, shift: false, reflected: false },
            Side { horizontal: true, low: true, alt: true, shift: true, reflected: true },
            Side { horizontal: false, low: false, alt: true, shift: false, reflected: true },
            Side { horizontal: false, low: true, alt: false, shift: true, reflected: false },
        ]
    }

    fn r7a_refinement_ordinary_case(case: R7aRefinementCase<'_>) -> (Vec<Item>, Value) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let nested = case.kind == "nested";
        let a_bounds = if nested { Rect::new(100.0, 100.0, 200.0, 150.0) } else { Rect::new(100.0, 100.0, 145.0, 150.0) };
        let b_bounds = if nested { Rect::new(130.0, 110.0, 170.0, 140.0) } else { Rect::new(160.0, 100.0, 200.0, 150.0) };
        let a = frame(&mut s, a_bounds, case.stroked, "center");
        let b = frame(&mut s, b_bounds, case.stroked, "center");
        let group = (case.kind != "multiple").then(|| ItemId(s.execute("object.group", &json!({"ids": [a.0,b.0]})).unwrap()["id"].as_u64().unwrap()));
        let selected = if nested { vec![a] } else { group.map_or_else(|| vec![a, b], |id| vec![id]) };
        let mut references = Vec::new();
        for (index, length) in case.references.iter().enumerate() {
            let x = 300.0 + 100.0 * index as f64;
            let reference = if *length == 0.0 {
                // frame.create cannot create a zero side; a public no-stroke line can.
                let id = ItemId(s.execute("line.create", &json!({"a": [x,300.0], "b": [x,380.0]})).unwrap()["id"].as_u64().unwrap());
                s.execute("object.stroke", &json!({"ids": [id.0], "swatch": "None", "weight": 0})).unwrap();
                id
            } else {
                let (w, h) = if case.side.horizontal { (*length, 80.0) } else { (80.0, *length) };
                frame(&mut s, Rect::new(x, 300.0, x + w, 300.0 + h), false, "center")
            };
            references.push(reference);
        }
        let magnitude = if case.position { 3.0 } else { case.dimension.unwrap_or(1.0) };
        let expected = case.side.expected(magnitude);
        if case.position {
            let at = if case.side.horizontal {
                if case.side.low { expected.x0 } else { expected.x1 }
            } else if case.side.low {
                expected.y0
            } else {
                expected.y1
            };
            let aligned = if case.side.horizontal { Rect::new(at, 300.0, at + 80.0, 380.0) } else { Rect::new(300.0, at, 380.0, at + 80.0) };
            references.push(frame(&mut s, aligned, false, "center"));
        }
        s.execute("selection.set", &json!({"ids": selected.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
        s.execute("prefs.set", &json!({"scaleStrokes": case.scale_strokes, "dimensionsIncludeStroke": true})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let journal = s.journal.clone();
        let mods = Mods { alt: case.side.alt, shift: case.side.shift, ..Mods::default() };
        let view = ViewInfo { snap: SnapView { align_edges: case.position, ..view().snap }, ..view() };
        let at = case.side.at(if case.side.reflected { -1.0 } else { 1.0 });
        let mut labels = Vec::new();
        let mut position_guide = false;
        for cancel in [true, false] {
            s.set_tool("selection");
            pointer(&mut s, PointerKind::Down, case.side.down(), mods, view);
            if case.many {
                for size in [40.0, -12.0, 6.0] {
                    pointer(&mut s, PointerKind::Drag, case.side.at(size), mods, view);
                }
            }
            pointer(&mut s, PointerKind::Drag, at, mods, view);
            if cancel {
                assert!(!Arc::ptr_eq(&s.doc().unwrap().doc, &before));
                s.cancel_interaction();
                assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
                assert_eq!(s.doc().unwrap().selection.items, selected);
                assert_eq!(s.doc().unwrap().history.undo.len(), undo);
                assert_eq!(s.journal, journal);
                assert!(s.doc().unwrap().interaction.is_none());
                s.set_tool("directSelection");
                s.set_tool("selection");
                assert!(s.overlays(view).is_empty());
            } else {
                let overlays = s.overlays(view);
                position_guide = overlays.iter().any(|overlay| matches!(overlay, Overlay::Guide { .. }));
                labels =
                    overlays.into_iter().filter_map(|overlay| if let Overlay::Measure { text, .. } = overlay { Some(text) } else { None }).collect();
                pointer(&mut s, PointerKind::Up, at, mods, view);
            }
        }
        let after = s.doc().unwrap().doc.clone();
        // This is the committed Doc/panel measurement, including each selected item's
        // stroke. A normal group measures its own bounds, not its descendants' outset.
        let measured = selected.iter().map(|id| after.item(*id).unwrap().visible_bounds()).reduce(|a, b| a.union(b)).unwrap();
        for reference in references {
            assert_eq!(after.item(reference), before.item(reference));
        }
        for id in [a, b] {
            close(after.item(id).unwrap().stroke.weight, if case.stroked { 4.0 } else { 0.0 });
        }
        if case.kind == "group" {
            assert_eq!(after.item(a), before.item(a));
            assert_eq!(after.item(b), before.item(b));
        } else if nested {
            assert_eq!(after.item(b), before.item(b));
            assert_eq!(after.item(group.unwrap()).unwrap().xf, Affine::IDENTITY);
        }
        assert_eq!(s.doc().unwrap().selection.items, selected);
        assert!(s.doc().unwrap().interaction.is_none());
        assert!(s.overlays(view).is_empty());
        after.check().unwrap();
        let (command, params) = s.journal.last().unwrap().clone();
        assert_eq!(command, "transform.resize");
        assert_eq!(params["from"], json!([100.0, 100.0, 200.0, 150.0]));
        assert_eq!(params["content"], false);
        assert_eq!(params["distribute"], false);
        r7a_history(&mut s, &before, &after, undo);
        let actual_length = if case.side.horizontal { measured.width() } else { measured.height() };
        let actual_label = format_measure(actual_length, Unit::Points);
        for label in &labels {
            // Independently fails BEFORE after inspecting the actual commit and history:
            // group .025 vs .1, nested 4 vs 4.1, multiple 3 vs 7.
            assert_eq!(
                label, &actual_label,
                "{} ordinary feedback disagrees with committed visible size, scaleStrokes={}",
                case.kind, case.scale_strokes
            );
        }
        let wanted: Vec<_> = case.dimension.into_iter().map(|value| format_measure(value, Unit::Points)).collect();
        assert_eq!(labels, wanted, "unmodeled ordinary policies must not advertise dimension matches");
        if case.position {
            assert!(position_guide, "position snapping must survive suppression");
        }
        let pad = if case.stroked && case.kind != "group" { 4.0 } else { 0.0 };
        rect(measured, expected.abs().inflate(pad / 2.0, pad / 2.0));
        close(actual_length, magnitude + pad);
        let m = Affine::translate((expected.x0, expected.y0))
            * Affine::scale_non_uniform(expected.width() / 100.0, expected.height() / 50.0)
            * Affine::translate((-100.0, -100.0));
        for id in [a, b] {
            if nested && id == b {
                continue;
            }
            let old = before.item(id).unwrap();
            let item = after.item(id).unwrap();
            let loc = after.find(id).unwrap();
            rect(after.parent_xf(&loc).transform_rect_bbox(item.bounds()), m.transform_rect_bbox(old.bounds()));
            assert_eq!(item.xf, old.xf);
            if case.kind != "group" {
                assert_eq!(item.path.subpaths.len(), 1);
                let expected_corners = old.path.subpaths[0].anchors.iter().map(|anchor| m * anchor.p);
                for (actual, expected) in item.path.subpaths[0].anchors.iter().zip(expected_corners) {
                    assert!((actual.p - expected).hypot() < 1e-6);
                }
            }
        }
        if case.kind == "group" {
            let item = after.item(group.unwrap()).unwrap();
            assert!(item.stroke.is_none());
            for (actual, expected) in item.xf.as_coeffs().into_iter().zip(m.as_coeffs()) {
                close(actual, expected);
            }
        }
        let values: Vec<_> = params["to"].as_array().unwrap().iter().map(|value| value.as_f64().unwrap()).collect();
        rect(Rect::new(values[0], values[1], values[2], values[3]), expected);
        let items = [Some(a), Some(b), group].into_iter().flatten().map(|id| after.item(id).unwrap().clone()).collect();
        (items, params)
    }

    #[test]
    fn r7a_refinement_ordinary_group_floor_feedback() {
        for side in r7a_refinement_sides() {
            for scale_strokes in [false, true] {
                for references in [&[0.025][..], &[0.0][..]] {
                    let case = R7aRefinementCase {
                        kind: "group",
                        side,
                        references,
                        stroked: true,
                        scale_strokes,
                        dimension: None,
                        position: false,
                        many: false,
                    };
                    let once = r7a_refinement_ordinary_case(case);
                    let repeated = r7a_refinement_ordinary_case(R7aRefinementCase { many: true, ..case });
                    assert_eq!(once, repeated);
                }
            }
        }
    }

    #[test]
    fn r7a_refinement_ordinary_nested_floor_feedback() {
        for side in r7a_refinement_sides() {
            for scale_strokes in [false, true] {
                for references in [&[4.0][..], &[4.0, 8.0][..]] {
                    let case = R7aRefinementCase {
                        kind: "nested",
                        side,
                        references,
                        stroked: true,
                        scale_strokes,
                        dimension: None,
                        position: false,
                        many: false,
                    };
                    let once = r7a_refinement_ordinary_case(case);
                    let repeated = r7a_refinement_ordinary_case(R7aRefinementCase { many: true, ..case });
                    assert_eq!(once, repeated);
                }
            }
        }
    }

    #[test]
    fn r7a_refinement_ordinary_multiple_dimension_feedback() {
        for side in r7a_refinement_sides() {
            for scale_strokes in [false, true] {
                for references in [&[3.0][..], &[0.025][..], &[0.0][..]] {
                    let case = R7aRefinementCase {
                        kind: "multiple",
                        side,
                        references,
                        stroked: true,
                        scale_strokes,
                        dimension: None,
                        position: false,
                        many: false,
                    };
                    let once = r7a_refinement_ordinary_case(case);
                    let repeated = r7a_refinement_ordinary_case(R7aRefinementCase { many: true, ..case });
                    assert_eq!(once, repeated);
                }
            }
        }
    }

    #[test]
    fn r7a_refinement_ordinary_keeps_position_snapping() {
        for kind in ["group", "nested", "multiple"] {
            for side in r7a_refinement_sides() {
                for scale_strokes in [false, true] {
                    let case = R7aRefinementCase {
                        kind,
                        side,
                        references: &[3.0],
                        stroked: true,
                        scale_strokes,
                        dimension: None,
                        position: true,
                        many: false,
                    };
                    let once = r7a_refinement_ordinary_case(case);
                    let repeated = r7a_refinement_ordinary_case(R7aRefinementCase { many: true, ..case });
                    assert_eq!(once, repeated);
                }
            }
        }
    }

    #[test]
    fn r7a_refinement_ordinary_keeps_zero_outset_dimensions() {
        for kind in ["group", "nested", "multiple"] {
            for side in r7a_refinement_sides() {
                for scale_strokes in [false, true] {
                    let case = R7aRefinementCase {
                        kind,
                        side,
                        references: &[0.025, 3.0],
                        stroked: false,
                        scale_strokes,
                        dimension: Some(3.0),
                        position: false,
                        many: false,
                    };
                    let once = r7a_refinement_ordinary_case(case);
                    let repeated = r7a_refinement_ordinary_case(R7aRefinementCase { many: true, ..case });
                    assert_eq!(once, repeated);
                }
            }
        }
    }

    #[test]
    fn r7a_refinement_ordinary_keeps_supported_fixed_text() {
        for side in r7a_refinement_sides() {
            for scale_strokes in [false, true] {
                r7a_text_case(R7aTextResizeCase {
                    side,
                    references: &[4.0, 8.0],
                    target: Some(8.0),
                    content: false,
                    scale_strokes,
                    stroke: R7aTextStroke::Centered,
                    many: true,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests_idml;
#[cfg(test)]
mod tests_table;

#[cfg(test)]
mod smart_reflow_tests {
    use std::sync::Arc;

    use designcraft_doc::{Document, ItemId, StoryId};
    use serde_json::json;

    use super::{Session, compose};

    #[derive(Clone, Copy, Debug)]
    enum Removal {
        Frame,
        Page,
        Cut,
    }

    fn primary_chain() -> (Session, StoryId, ItemId) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1, "primaryTextFrame": true})).unwrap();
        let sid = s.doc().unwrap().doc.settings.primary_story.unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("text.insert", &json!({"text": "Words keep coming and the page fills up quickly with them. ".repeat(250)})).unwrap();
        let d = s.doc().unwrap().doc.clone();
        let last = *d.story(sid).unwrap().frames.last().unwrap();
        let cs = s.cache.get(&d, sid, None);
        assert!(d.page_count() >= 3);
        assert!(!cs.is_overset());
        assert!(cs.frame(last).is_some_and(|f| !f.range.is_empty()));
        let mut probe = (*d).clone();
        probe.remove_item(last).unwrap();
        assert!(compose::compose_story(&probe, sid, &Default::default()).is_overset(), "removal must reduce occupied flow capacity");
        (s, sid, last)
    }

    fn remove(s: &mut Session, frame: ItemId, kind: Removal) {
        match kind {
            Removal::Frame => {
                s.execute("edit.clear", &json!({"ids": [frame.0]})).unwrap();
            }
            Removal::Page => {
                let page = s.doc().unwrap().doc.page_of_item(frame).unwrap();
                s.execute("layout.pages.delete", &json!({"pages": [page]})).unwrap();
            }
            Removal::Cut => {
                s.execute("selection.set", &json!({"ids": [frame.0]})).unwrap();
                s.execute("edit.cut", &json!({})).unwrap();
            }
        }
    }

    fn assert_removed(s: &Session, before: &Document, sid: StoryId, frame: ItemId, pages: usize) {
        let d = &s.doc().unwrap().doc;
        assert!(d.item(frame).is_none());
        assert_eq!(d.page_count(), pages);
        assert_eq!(d.settings.primary_story, Some(sid));
        let story = d.story(sid).unwrap();
        let old = before.story(sid).unwrap();
        assert_eq!(story.text, old.text, "removing a frame must retain its story text");
        let frames: Vec<_> = old.frames.iter().copied().filter(|f| *f != frame).collect();
        assert_eq!(story.frames, frames, "no replacement frame is added");
        assert!(s.cache.get(d, sid, None).is_overset());
        d.check().unwrap();
    }

    #[test]
    fn explicit_primary_removal_survives_unrelated_edits_and_undo_redo() {
        for kind in [Removal::Frame, Removal::Page, Removal::Cut] {
            let (mut s, sid, last) = primary_chain();
            let before = s.doc().unwrap().doc.clone();
            let pages = before.page_count() - usize::from(matches!(kind, Removal::Page));
            remove(&mut s, last, kind);
            assert_removed(&s, &before, sid, last, pages);
            assert!(s.prefs.smart_text_reflow);
            let removed = s.doc().unwrap().doc.clone();
            s.execute("edit.undo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before), "{kind:?}: undo restores the deleted frame/page");
            s.execute("edit.redo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &removed), "{kind:?}: redo does not reflow");

            // A cold cache must compare against the post-deletion document, not revive autoflow.
            s.cache.clear();
            s.execute("frame.create", &json!({"rect": [10, 10, 20, 20], "content": "unassigned"})).unwrap();
            assert_removed(&s, &before, sid, last, pages);
            s.execute("frame.create", &json!({"rect": [10, 30, 100, 60], "content": "text", "text": "Other story", "caret": false})).unwrap();
            assert_removed(&s, &before, sid, last, pages);
            let first = s.doc().unwrap().doc.story(sid).unwrap().frames[0];
            s.execute("object.primaryTextFrame", &json!({"ids": [first.0], "on": true})).unwrap();
            assert_removed(&s, &before, sid, last, pages);
            assert!(s.prefs.smart_text_reflow);
        }
    }

    #[test]
    fn deleting_a_blank_page_does_not_reflow_primary_page_variables() {
        let (mut s, sid, _) = primary_chain();
        let end = s.doc().unwrap().doc.story(sid).unwrap().len();
        s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
        // A Last Page Number variable makes composition depend on the document page count.
        let index = s.doc().unwrap().doc.text_variables.iter().position(|v| v.name == "Last Page Number").unwrap();
        let marker = designcraft_doc::vars::var_char(index).unwrap().to_string();
        s.execute("text.insert", &json!({"text": marker})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let last = *before.story(sid).unwrap().frames.last().unwrap();
        assert_eq!(before.page_of_item(last), Some(before.page_count() - 1));
        remove(&mut s, last, Removal::Frame);
        assert_removed(&s, &before, sid, last, before.page_count());
        s.execute("layout.pages.delete", &json!({"pages": [before.page_count() - 1]})).unwrap();
        assert_removed(&s, &before, sid, last, before.page_count() - 1);
        assert!(s.prefs.smart_text_reflow);
        s.execute("frame.create", &json!({"rect": [10, 10, 20, 20], "content": "unassigned"})).unwrap();
        assert_removed(&s, &before, sid, last, before.page_count() - 1);
    }

    #[test]
    fn primary_content_edit_resumes_reflow_after_explicit_removal() {
        for kind in [Removal::Frame, Removal::Page, Removal::Cut] {
            let (mut s, sid, last) = primary_chain();
            remove(&mut s, last, kind);
            let removed = s.doc().unwrap().doc.clone();
            let end = removed.story(sid).unwrap().len();
            s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
            s.execute("text.insert", &json!({"text": " More text."})).unwrap();
            let flowed = s.doc().unwrap().doc.clone();
            assert_eq!(flowed.settings.primary_story, Some(sid));
            assert!(s.prefs.smart_text_reflow);
            assert!(flowed.page_count() > removed.page_count());
            assert!(!s.cache.get(&flowed, sid, None).is_overset());
            assert!(flowed.item(last).is_none());
            assert!(flowed.story(sid).unwrap().frames.iter().any(|f| !removed.story(sid).unwrap().frames.contains(f)));
            assert_eq!(flowed.story(sid).unwrap().text, removed.story(sid).unwrap().text.clone() + " More text.");
            flowed.check().unwrap();
            s.execute("edit.undo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &removed), "one undo restores text and the explicitly reduced chain");
            s.execute("edit.redo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &flowed));
        }
    }

    #[test]
    fn primary_frame_layout_edit_resumes_reflow_after_explicit_removal() {
        let (mut s, sid, last) = primary_chain();
        remove(&mut s, last, Removal::Page);
        let removed = s.doc().unwrap().doc.clone();
        let first = removed.story(sid).unwrap().frames[0];
        let inset = removed.item(first).unwrap().text_frame().unwrap().options.inset.map(|v| v + 1.0);
        s.execute("object.textFrameOptions", &json!({"ids": [first.0], "inset": inset})).unwrap();
        let flowed = s.doc().unwrap().doc.clone();
        assert!(flowed.page_count() > removed.page_count());
        assert!(!s.cache.get(&flowed, sid, None).is_overset());
        assert_eq!(flowed.settings.primary_story, Some(sid));
        assert!(s.prefs.smart_text_reflow);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &removed));
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &flowed));
        flowed.check().unwrap();
    }

    #[test]
    fn deleting_primary_text_still_prunes_empty_pages_including_clear_and_cut() {
        for command in ["text.delete", "edit.clear", "edit.cut"] {
            let (mut s, sid, _) = primary_chain();
            let before = s.doc().unwrap().doc.clone();
            let end = before.story(sid).unwrap().len();
            s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": end})).unwrap();
            let result = s.execute(command, &json!({})).unwrap();
            let empty = s.doc().unwrap().doc.clone();
            assert_eq!(empty.page_count(), 1, "{command}: empty trailing pages are pruned");
            assert_eq!(empty.settings.primary_story, Some(sid));
            assert_eq!(empty.story(sid).unwrap().frames.len(), 1);
            assert!(empty.story(sid).unwrap().text.is_empty());
            assert!(s.prefs.smart_text_reflow);
            if command == "edit.cut" {
                assert_eq!(result["text"], before.story(sid).unwrap().text);
            }
            empty.check().unwrap();
            s.execute("edit.undo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
            s.execute("edit.redo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &empty));
        }
    }

    #[test]
    fn reflow_off_preserves_removal_and_does_not_resume_on_primary_edits() {
        for kind in [Removal::Frame, Removal::Page, Removal::Cut] {
            let (mut s, sid, last) = primary_chain();
            s.execute("prefs.set", &json!({"smartTextReflow": false})).unwrap();
            let before = s.doc().unwrap().doc.clone();
            let pages = before.page_count() - usize::from(matches!(kind, Removal::Page));
            remove(&mut s, last, kind);
            assert_removed(&s, &before, sid, last, pages);
            let frames = s.doc().unwrap().doc.story(sid).unwrap().frames.clone();
            let end = s.doc().unwrap().doc.story(sid).unwrap().len();
            s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
            s.execute("text.insert", &json!({"text": " More text."})).unwrap();
            assert_eq!(s.doc().unwrap().doc.page_count(), pages);
            assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().frames, frames);
            assert!(s.cache.get(&s.doc().unwrap().doc, sid, None).is_overset());
            let end = s.doc().unwrap().doc.story(sid).unwrap().len();
            s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": end})).unwrap();
            s.execute("text.delete", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc.page_count(), pages, "OFF does not prune empty pages");
            assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().frames, frames);
            assert_eq!(s.doc().unwrap().doc.settings.primary_story, Some(sid));
            assert!(!s.prefs.smart_text_reflow);
            s.doc().unwrap().doc.check().unwrap();
        }
    }

    #[test]
    fn primary_live_caption_changes_reflow_in_the_same_history_step() {
        for reflow in [true, false] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1, "width": 612, "height": 792})).unwrap();
            let source =
                ItemId(s.execute("frame.create", &json!({"rect": [72, 36, 540, 72], "content": "unassigned"})).unwrap()["id"].as_u64().unwrap());
            s.execute("object.altText", &json!({"ids": [source.0], "text": "First"})).unwrap();
            let caption = s.execute("object.caption", &json!({"ids": [source.0], "text": "Photo: {altText}", "live": true, "height": 648})).unwrap();
            let frame = ItemId(caption["ids"][0].as_u64().unwrap());
            let sid = s.doc().unwrap().doc.item(frame).unwrap().text_frame().unwrap().story;
            s.execute("object.primaryTextFrame", &json!({"ids": [frame.0], "on": true})).unwrap();
            s.execute("prefs.set", &json!({"smartTextReflow": reflow})).unwrap();
            let before = s.doc().unwrap().doc.clone();
            assert_eq!(before.page_count(), 1);
            assert_eq!(before.story(sid).unwrap().text, "Photo: First");
            assert!(!s.cache.get(&before, sid, None).is_overset());

            let text = "Words keep coming and the page fills up quickly with them. ".repeat(250);
            // Changing a non-wrapping source's metadata leaves primary composition unchanged.
            // The primary story changes only in the secondary live-caption refresh.
            let mut probe = (*before).clone();
            probe.item_mut(source).unwrap().alt_text = text.clone();
            assert!(Arc::ptr_eq(&s.cache.get(&before, sid, None), &s.cache.get(&probe, sid, None)));

            s.execute("object.altText", &json!({"ids": [source.0], "text": text})).unwrap();
            let grown = s.doc().unwrap().doc.clone();
            assert_eq!(grown.story(sid).unwrap().text, format!("Photo: {text}"));
            assert_eq!(grown.settings.primary_story, Some(sid));
            assert_eq!(s.prefs.smart_text_reflow, reflow);
            if reflow {
                assert!(grown.page_count() > 1, "caption growth must flow in the source edit itself");
                assert!(grown.story(sid).unwrap().frames.len() > 1);
                assert!(!s.cache.get(&grown, sid, None).is_overset());
            } else {
                assert_eq!(grown.page_count(), 1);
                assert_eq!(grown.story(sid).unwrap().frames, vec![frame]);
                assert!(s.cache.get(&grown, sid, None).is_overset());
            }
            grown.check().unwrap();
            s.execute("edit.undo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before), "one undo restores source, caption and pages");
            s.execute("edit.redo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &grown));

            s.execute("object.label", &json!({"ids": [source.0], "label": "Unrelated"})).unwrap();
            let before_shrink = s.doc().unwrap().doc.clone();
            assert_eq!(before_shrink.page_count(), grown.page_count());
            assert_eq!(before_shrink.story(sid).unwrap().frames, grown.story(sid).unwrap().frames);
            assert_eq!(before_shrink.story(sid).unwrap().text, grown.story(sid).unwrap().text);

            s.execute("object.altText", &json!({"ids": [source.0], "text": "Second"})).unwrap();
            let shrunk = s.doc().unwrap().doc.clone();
            assert_eq!(shrunk.story(sid).unwrap().text, "Photo: Second");
            assert_eq!(shrunk.page_count(), 1, "caption shrinkage prunes empty tail pages in the source edit");
            assert_eq!(shrunk.story(sid).unwrap().frames, vec![frame]);
            assert_eq!(shrunk.settings.primary_story, Some(sid));
            assert_eq!(s.prefs.smart_text_reflow, reflow);
            assert!(!s.cache.get(&shrunk, sid, None).is_overset());
            shrunk.check().unwrap();
            s.execute("edit.undo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before_shrink));
            s.execute("edit.redo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &shrunk));
        }
    }

    #[test]
    fn primary_layout_gesture_resumes_reflow_only_on_commit() {
        for command in ["object.textFrameOptions", "transform.set"] {
            for cold in [false, true] {
                let (mut s, sid, last) = primary_chain();
                remove(&mut s, last, Removal::Page);
                let removed = s.doc().unwrap().doc.clone();
                let first = removed.story(sid).unwrap().frames[0];
                s.execute("selection.set", &json!({"ids": [first.0]})).unwrap();
                let selection = s.doc().unwrap().selection.clone();
                let undo_count = s.doc().unwrap().history.undo.len();
                let journal = s.journal.clone();
                let params = if command == "object.textFrameOptions" {
                    let inset = removed.item(first).unwrap().text_frame().unwrap().options.inset.map(|v| v + 1.0);
                    json!({"ids": [first.0], "inset": inset})
                } else {
                    json!({"ids": [first.0], "width": removed.item(first).unwrap().bounds().width() - 1.0})
                };

                s.begin_interaction("Primary layout gesture").unwrap();
                // Repeated previews replace one another and never add pages.
                for _ in 0..2 {
                    s.preview(command, params.clone()).unwrap();
                    let preview = &s.doc().unwrap().doc;
                    assert_eq!(preview.page_count(), removed.page_count());
                    assert_eq!(preview.story(sid).unwrap().frames, removed.story(sid).unwrap().frames);
                    assert!(s.cache.get(preview, sid, None).is_overset());
                    assert_eq!(s.doc().unwrap().history.undo.len(), undo_count);
                    assert_eq!(s.journal, journal);
                }
                s.cancel_interaction();
                assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &removed));
                assert_eq!(s.doc().unwrap().selection, selection);
                assert_eq!(s.doc().unwrap().history.undo.len(), undo_count);
                assert_eq!(s.journal, journal);

                s.begin_interaction("Primary layout gesture").unwrap();
                s.preview(command, params.clone()).unwrap();
                let preview_selection = s.doc().unwrap().selection.clone();
                if cold {
                    s.cache.clear();
                }
                s.commit_interaction().unwrap();
                let flowed = s.doc().unwrap().doc.clone();
                assert!(s.doc().unwrap().interaction.is_none());
                assert!(flowed.page_count() > removed.page_count(), "{command}, cold={cold}");
                assert!(!s.cache.get(&flowed, sid, None).is_overset());
                assert!(flowed.item(last).is_none());
                assert_eq!(flowed.settings.primary_story, Some(sid));
                assert!(s.prefs.smart_text_reflow);
                assert_eq!(s.doc().unwrap().selection, preview_selection);
                assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
                assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Primary layout gesture");
                assert_eq!(s.journal.len(), journal.len() + 1);
                assert_eq!(s.journal.last(), Some(&(command.to_string(), params)));
                flowed.check().unwrap();
                s.execute("edit.undo", &json!({})).unwrap();
                assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &removed), "one undo restores layout and generated pages");
                assert_eq!(s.doc().unwrap().selection, selection);
                s.execute("edit.redo", &json!({})).unwrap();
                assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &flowed));
                assert_eq!(s.doc().unwrap().selection, preview_selection);
            }
        }
    }

    #[test]
    fn unrelated_gesture_and_empty_commit_do_not_resume_primary_reflow() {
        for cold in [false, true] {
            let (mut s, sid, last) = primary_chain();
            let original = s.doc().unwrap().doc.clone();
            remove(&mut s, last, Removal::Page);
            let pages = original.page_count() - 1;
            let other = s.execute("frame.create", &json!({"rect": [10, 10, 20, 20], "content": "unassigned"})).unwrap()["id"].as_u64().unwrap();
            s.execute("selection.set", &json!({"ids": [other]})).unwrap();
            let before = s.doc().unwrap().doc.clone();
            let selection = s.doc().unwrap().selection.clone();
            let undo_count = s.doc().unwrap().history.undo.len();
            let journal = s.journal.clone();
            let params = json!({"ids": [other], "width": 40});
            s.begin_interaction("Unrelated layout gesture").unwrap();
            s.preview("transform.set", params.clone()).unwrap();
            let preview = s.doc().unwrap().doc.clone();
            assert_removed(&s, &original, sid, last, pages);
            assert_eq!(s.doc().unwrap().history.undo.len(), undo_count);
            assert_eq!(s.journal, journal);
            if cold {
                s.cache.clear();
            }
            s.commit_interaction().unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &preview), "an unrelated commit must not restore primary capacity");
            assert_removed(&s, &original, sid, last, pages);
            assert!(s.prefs.smart_text_reflow);
            assert_eq!(s.doc().unwrap().selection, selection);
            assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
            assert_eq!(s.journal.len(), journal.len() + 1);
            assert_eq!(s.journal.last(), Some(&("transform.set".to_string(), params)));
            s.execute("edit.undo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
            s.execute("edit.redo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &preview));
            assert_eq!(s.doc().unwrap().selection, selection);

            s.cache.clear();
            let undo_count = s.doc().unwrap().history.undo.len();
            let journal = s.journal.clone();
            s.begin_interaction("No change").unwrap();
            s.commit_interaction().unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &preview));
            assert_removed(&s, &original, sid, last, pages);
            assert_eq!(s.doc().unwrap().history.undo.len(), undo_count);
            assert_eq!(s.journal, journal);
        }
    }

    fn assert_same_visible_flow(a: &compose::ComposedStory, b: &compose::ComposedStory) {
        assert_eq!(a.overset_at, b.overset_at);
        assert_eq!(a.frames.len(), b.frames.len());
        for (a, b) in a.frames.iter().zip(&b.frames) {
            assert_eq!((a.frame, a.range.clone(), &a.columns, a.content_height), (b.frame, b.range.clone(), &b.columns, b.content_height));
            assert_eq!(a.lines.len(), b.lines.len());
            for (a, b) in a.lines.iter().zip(&b.lines) {
                assert_eq!((a.range.clone(), a.baseline, a.x0, a.x1, a.end_x), (b.range.clone(), b.baseline, b.x0, b.x1, b.end_x));
                assert_eq!(a.glyphs.len(), b.glyphs.len());
                for (a, b) in a.glyphs.iter().zip(&b.glyphs) {
                    assert_eq!((a.face, a.gid, a.x, a.y, a.adv, a.byte, a.len), (b.face, b.gid, b.x, b.y, b.adv, b.byte, b.len));
                }
            }
        }
    }

    fn assert_reflow_history(s: &mut Session, before: &Arc<Document>, after: &Arc<Document>) {
        after.check().unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, before));
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, after));
    }

    #[test]
    fn editorial_note_metadata_preserves_explicit_primary_removal() {
        for kind in [Removal::Frame, Removal::Page, Removal::Cut] {
            for field in ["author", "text"] {
                for cold in [false, true] {
                    let (mut s, sid, _) = primary_chain();
                    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
                    let note = s.execute("note.new", &json!({"text": "Check wording", "author": "Ed"})).unwrap()["id"].as_u64().unwrap();
                    let original = s.doc().unwrap().doc.clone();
                    let last = *original.story(sid).unwrap().frames.last().unwrap();
                    let pages = original.page_count() - usize::from(matches!(kind, Removal::Page));
                    remove(&mut s, last, kind);
                    let before = s.doc().unwrap().doc.clone();
                    let old_flow = s.cache.get(&before, sid, None);
                    if cold {
                        s.cache.clear();
                    }
                    let undo_count = s.doc().unwrap().history.undo.len();
                    let params = if field == "author" {
                        json!({"story": sid.0, "id": note, "author": "Editor"})
                    } else {
                        json!({"story": sid.0, "id": note, "text": "Checked wording"})
                    };
                    s.execute("note.edit", &params).unwrap();
                    let after = s.doc().unwrap().doc.clone();
                    assert_removed(&s, &original, sid, last, pages);
                    assert_eq!(after.story(sid).unwrap().paras, before.story(sid).unwrap().paras);
                    assert_eq!(after.story(sid).unwrap().chars, before.story(sid).unwrap().chars);
                    let edited = after.story(sid).unwrap().editorial.iter().find(|n| n.id == note).unwrap();
                    let expected = if field == "author" { ("Check wording", "Editor") } else { ("Checked wording", "Ed") };
                    assert_eq!((edited.text.as_str(), edited.author.as_str()), expected);
                    assert_same_visible_flow(&old_flow, &s.cache.get(&after, sid, None));
                    assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
                    assert_reflow_history(&mut s, &before, &after);
                }
            }
        }
    }

    #[test]
    fn converting_editorial_note_to_printable_text_resumes_primary_reflow() {
        let (mut s, sid, _) = primary_chain();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        let text = "Printed note text fills more pages. ".repeat(250);
        let note = s.execute("note.new", &json!({"text": text, "author": "Ed"})).unwrap()["id"].as_u64().unwrap();
        let original = s.doc().unwrap().doc.clone();
        let last = *original.story(sid).unwrap().frames.last().unwrap();
        remove(&mut s, last, Removal::Page);
        let before = s.doc().unwrap().doc.clone();
        let undo_count = s.doc().unwrap().history.undo.len();
        s.execute("note.convertToText", &json!({"story": sid.0, "id": note})).unwrap();
        let after = s.doc().unwrap().doc.clone();
        assert!(after.story(sid).unwrap().text.starts_with(&text));
        assert!(after.story(sid).unwrap().editorial.is_empty());
        assert!(after.item(last).is_none());
        assert!(after.page_count() > before.page_count());
        assert!(!s.cache.get(&after, sid, None).is_overset());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
        assert_reflow_history(&mut s, &before, &after);
    }

    #[test]
    fn unused_paragraph_style_preserves_explicit_primary_removal() {
        for kind in [Removal::Frame, Removal::Page, Removal::Cut] {
            for cold in [false, true] {
                let (mut s, sid, last) = primary_chain();
                let original = s.doc().unwrap().doc.clone();
                let pages = original.page_count() - usize::from(matches!(kind, Removal::Page));
                remove(&mut s, last, kind);
                if cold {
                    s.cache.clear();
                }
                let before = s.doc().unwrap().doc.clone();
                let selection = s.doc().unwrap().selection.clone();
                let undo_count = s.doc().unwrap().history.undo.len();
                s.execute("style.paragraph.create", &json!({"name": "Unused R5"})).unwrap();
                let after = s.doc().unwrap().doc.clone();
                assert!(after.styles.para("Unused R5").is_some());
                assert_removed(&s, &original, sid, last, pages);
                assert_eq!(s.doc().unwrap().selection, selection);
                assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
                assert_reflow_history(&mut s, &before, &after);
            }
        }
    }

    #[test]
    fn primary_script_label_preserves_explicit_removal() {
        for kind in [Removal::Frame, Removal::Page, Removal::Cut] {
            for cold in [false, true] {
                let (mut s, sid, last) = primary_chain();
                let original = s.doc().unwrap().doc.clone();
                let pages = original.page_count() - usize::from(matches!(kind, Removal::Page));
                remove(&mut s, last, kind);
                let before = s.doc().unwrap().doc.clone();
                let first = before.story(sid).unwrap().frames[0];
                let selection = s.doc().unwrap().selection.clone();
                let undo_count = s.doc().unwrap().history.undo.len();
                if cold {
                    s.cache.clear();
                }
                s.execute("object.label", &json!({"ids": [first.0], "label": "chapter"})).unwrap();
                let after = s.doc().unwrap().doc.clone();
                assert_eq!(after.item(first).unwrap().label, "chapter");
                assert_removed(&s, &original, sid, last, pages);
                assert_eq!(s.doc().unwrap().selection, selection);
                assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
                assert_reflow_history(&mut s, &before, &after);
            }
        }
    }

    #[test]
    fn zero_net_primary_preview_preserves_removal_on_commit_and_cancel() {
        for cold in [false, true] {
            let (mut s, sid, last) = primary_chain();
            let original = s.doc().unwrap().doc.clone();
            remove(&mut s, last, Removal::Page);
            let pages = original.page_count() - 1;
            let first = s.doc().unwrap().doc.story(sid).unwrap().frames[0];
            s.execute("selection.set", &json!({"ids": [first.0]})).unwrap();
            let before = s.doc().unwrap().doc.clone();
            let selection = s.doc().unwrap().selection.clone();
            let width = before.item(first).unwrap().bounds().width();
            let undo_count = s.doc().unwrap().history.undo.len();
            let journal = s.journal.clone();
            let final_params = json!({"ids": [first.0], "width": width});
            for commit in [false, true] {
                s.begin_interaction("Return primary width").unwrap();
                s.preview("transform.set", json!({"ids": [first.0], "width": width - 1.0})).unwrap();
                s.preview("transform.set", final_params.clone()).unwrap();
                let preview = s.doc().unwrap().doc.clone();
                assert_eq!(preview.item(first).unwrap().bounds(), before.item(first).unwrap().bounds());
                assert_same_visible_flow(&s.cache.get(&before, sid, None), &s.cache.get(&preview, sid, None));
                assert_removed(&s, &original, sid, last, pages);
                assert_eq!(s.doc().unwrap().history.undo.len(), undo_count);
                assert_eq!(s.journal, journal);
                if cold {
                    s.cache.clear();
                }
                if commit {
                    s.commit_interaction().unwrap();
                    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &preview), "zero-net commit must not generate pages");
                    assert_removed(&s, &original, sid, last, pages);
                    assert_eq!(s.doc().unwrap().selection, selection);
                    assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
                    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Return primary width");
                    assert_eq!(s.journal.len(), journal.len() + 1);
                    assert_eq!(s.journal.last(), Some(&("transform.set".to_string(), final_params.clone())));
                    assert_reflow_history(&mut s, &before, &preview);
                    assert_eq!(s.doc().unwrap().selection, selection);
                } else {
                    s.cancel_interaction();
                    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
                    assert_eq!(s.doc().unwrap().selection, selection);
                    assert_eq!(s.doc().unwrap().history.undo.len(), undo_count);
                    assert_eq!(s.journal, journal);
                }
            }
        }
    }

    #[test]
    fn editing_an_overset_tail_resumes_reflow_with_identical_visible_layout() {
        let (mut s, sid, _) = primary_chain();
        let end = s.doc().unwrap().doc.story(sid).unwrap().len();
        s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
        s.execute("text.insert", &json!({"text": "\nTail paragraph"})).unwrap();
        let full = s.doc().unwrap().doc.clone();
        let last = *full.story(sid).unwrap().frames.last().unwrap();
        remove(&mut s, last, Removal::Page);
        let before = s.doc().unwrap().doc.clone();
        let old_flow = compose::compose_story(&before, sid, &Default::default());
        let tail_start = before.story(sid).unwrap().text.rfind('\n').unwrap() + 1;
        assert!(old_flow.overset_at.is_some_and(|at| at < tail_start));
        let mut probe = (*before).clone();
        let end = before.story(sid).unwrap().len();
        probe.story_mut(sid).unwrap().insert(end, " More invisible text.");
        assert_same_visible_flow(&old_flow, &compose::compose_story(&probe, sid, &Default::default()));
        s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
        let undo_count = s.doc().unwrap().history.undo.len();
        s.execute("text.insert", &json!({"text": " More invisible text."})).unwrap();
        let after = s.doc().unwrap().doc.clone();
        assert!(after.page_count() > before.page_count());
        assert!(!s.cache.get(&after, sid, None).is_overset());
        assert_eq!(after.story(sid).unwrap().text, before.story(sid).unwrap().text.clone() + " More invisible text.");
        assert!(after.item(last).is_none());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
        assert_reflow_history(&mut s, &before, &after);
    }

    #[test]
    fn a_used_style_in_the_overset_tail_resumes_primary_reflow() {
        let (mut s, sid, _) = primary_chain();
        s.execute("style.paragraph.create", &json!({"name": "Used Tail", "chars": {"size": 12}})).unwrap();
        let end = s.doc().unwrap().doc.story(sid).unwrap().len();
        s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
        s.execute("text.insert", &json!({"text": "\nStyled tail"})).unwrap();
        let end = s.doc().unwrap().doc.story(sid).unwrap().len();
        s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
        s.execute("style.paragraph.apply", &json!({"name": "Used Tail"})).unwrap();
        let full = s.doc().unwrap().doc.clone();
        let last = *full.story(sid).unwrap().frames.last().unwrap();
        remove(&mut s, last, Removal::Page);
        let before = s.doc().unwrap().doc.clone();
        let tail_start = before.story(sid).unwrap().text.rfind('\n').unwrap() + 1;
        let old_flow = compose::compose_story(&before, sid, &Default::default());
        assert!(old_flow.overset_at.is_some_and(|at| at < tail_start));
        let mut probe = (*before).clone();
        probe.styles_mut().para_mut("Used Tail").unwrap().chars.size = Some(14.0);
        assert_same_visible_flow(&old_flow, &compose::compose_story(&probe, sid, &Default::default()));
        let undo_count = s.doc().unwrap().history.undo.len();
        s.execute("style.paragraph.edit", &json!({"name": "Used Tail", "chars": {"size": 14}})).unwrap();
        let after = s.doc().unwrap().doc.clone();
        let old_story = before.story(sid).unwrap();
        let new_story = after.story(sid).unwrap();
        assert_eq!((&old_story.text, &old_story.paras, &old_story.chars), (&new_story.text, &new_story.paras, &new_story.chars));
        assert!(after.page_count() > before.page_count());
        assert!(!s.cache.get(&after, sid, None).is_overset());
        assert!(after.item(last).is_none());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
        assert_reflow_history(&mut s, &before, &after);
    }

    #[test]
    fn used_paragraph_style_changes_grow_and_prune_primary_pages() {
        let (mut s, sid, last) = primary_chain();
        remove(&mut s, last, Removal::Page);
        let before = s.doc().unwrap().doc.clone();
        let style = before.story(sid).unwrap().paras[0].style.clone();
        let size = before.styles.resolve_para(&before.story(sid).unwrap().paras[0]).1.size;
        let undo_count = s.doc().unwrap().history.undo.len();
        s.execute("style.paragraph.edit", &json!({"name": style, "chars": {"size": size + 1.0}})).unwrap();
        let grown = s.doc().unwrap().doc.clone();
        assert!(grown.page_count() > before.page_count());
        assert!(!s.cache.get(&grown, sid, None).is_overset());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
        assert_reflow_history(&mut s, &before, &grown);
        let undo_count = s.doc().unwrap().history.undo.len();
        s.execute("style.paragraph.edit", &json!({"name": style, "chars": {"size": size * 0.25}})).unwrap();
        let shrunk = s.doc().unwrap().doc.clone();
        assert!(shrunk.page_count() < grown.page_count(), "used font metrics must prune empty tail pages");
        assert!(!s.cache.get(&shrunk, sid, None).is_overset());
        assert_eq!(shrunk.settings.primary_story, Some(sid));
        assert!(shrunk.item(last).is_none());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
        assert_reflow_history(&mut s, &grown, &shrunk);
    }

    #[test]
    fn relevant_wrap_changes_resume_reflow_and_prune_on_removal() {
        let (mut s, sid, last) = primary_chain();
        let original = s.doc().unwrap().doc.clone();
        remove(&mut s, last, Removal::Page);
        let first = s.doc().unwrap().doc.story(sid).unwrap().frames[0];
        let rect = s.doc().unwrap().doc.item(first).unwrap().bounds();
        let wrap = s.execute("frame.create", &json!({"rect": [rect.x0, rect.y0, rect.x1, rect.y1], "content": "unassigned"})).unwrap()["id"]
            .as_u64()
            .unwrap();
        assert_removed(&s, &original, sid, last, original.page_count() - 1);
        let before = s.doc().unwrap().doc.clone();
        let undo_count = s.doc().unwrap().history.undo.len();
        s.execute("object.textWrap", &json!({"ids": [wrap], "mode": "boundingBox"})).unwrap();
        let grown = s.doc().unwrap().doc.clone();
        assert!(grown.page_count() > before.page_count());
        let cs = s.cache.get(&grown, sid, None);
        assert!(!cs.is_overset());
        assert!(cs.frame(first).unwrap().range.is_empty(), "the wrap actually removes primary flow capacity");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
        assert_reflow_history(&mut s, &before, &grown);
        let undo_count = s.doc().unwrap().history.undo.len();
        s.execute("object.textWrap", &json!({"ids": [wrap], "mode": "none"})).unwrap();
        let shrunk = s.doc().unwrap().doc.clone();
        assert!(shrunk.page_count() < grown.page_count());
        assert!(!s.cache.get(&shrunk, sid, None).is_overset());
        assert_eq!(shrunk.settings.primary_story, Some(sid));
        assert!(shrunk.item(last).is_none());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
        assert_reflow_history(&mut s, &grown, &shrunk);
    }

    #[test]
    fn indirect_character_styles_in_an_overset_tail_resume_reflow() {
        for rule in ["nestedStyles", "grepStyles", "nestedLineStyles"] {
            let (mut s, sid, _) = primary_chain();
            s.execute("style.character.create", &json!({"name": "Tail Overlay", "chars": {"size": 12}})).unwrap();
            let rules = match rule {
                "grepStyles" => json!({"grepStyles": [{"style": "Tail Overlay", "pattern": "."}]}),
                "nestedLineStyles" => json!({"nestedLineStyles": [{"style": "Tail Overlay", "lines": 1}]}),
                _ => json!({"nestedStyles": [{"style": "Tail Overlay"}]}),
            };
            s.execute("style.paragraph.create", &json!({"name": "Tail Rules", "para": rules})).unwrap();
            let end = s.doc().unwrap().doc.story(sid).unwrap().len();
            s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
            s.execute("text.insert", &json!({"text": "\nStyled tail"})).unwrap();
            let end = s.doc().unwrap().doc.story(sid).unwrap().len();
            s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
            s.execute("style.paragraph.apply", &json!({"name": "Tail Rules"})).unwrap();
            let full = s.doc().unwrap().doc.clone();
            let last = *full.story(sid).unwrap().frames.last().unwrap();
            remove(&mut s, last, Removal::Page);
            let before = s.doc().unwrap().doc.clone();
            let old_flow = compose::compose_story(&before, sid, &Default::default());
            let tail_start = before.story(sid).unwrap().text.rfind('\n').unwrap() + 1;
            assert!(old_flow.overset_at.is_some_and(|at| at < tail_start));
            let mut probe = (*before).clone();
            probe.styles_mut().char_style_mut("Tail Overlay").unwrap().chars.size = Some(14.0);
            assert_same_visible_flow(&old_flow, &compose::compose_story(&probe, sid, &Default::default()));
            let undo_count = s.doc().unwrap().history.undo.len();
            s.execute("style.character.edit", &json!({"name": "Tail Overlay", "chars": {"size": 14}})).unwrap();
            let after = s.doc().unwrap().doc.clone();
            assert!(after.page_count() > before.page_count(), "{rule} is a used style dependency");
            assert!(!s.cache.get(&after, sid, None).is_overset());
            assert!(after.item(last).is_none());
            assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
            assert_reflow_history(&mut s, &before, &after);
        }
    }

    #[test]
    fn overset_note_text_options_and_reference_style_resume_reflow() {
        for change in ["text", "options", "style"] {
            let (mut s, sid, _) = primary_chain();
            s.execute("style.character.create", &json!({"name": "Tail Reference", "chars": {"size": 12}})).unwrap();
            s.execute("footnote.options", &json!({"refCharStyle": "Tail Reference"})).unwrap();
            let end = s.doc().unwrap().doc.story(sid).unwrap().len();
            s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
            s.execute("text.insert", &json!({"text": "\nTail paragraph"})).unwrap();
            let end = s.doc().unwrap().doc.story(sid).unwrap().len();
            s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
            let note = s.execute("footnote.insert", &json!({"text": "Hidden note"})).unwrap()["id"].as_u64().unwrap();
            let full = s.doc().unwrap().doc.clone();
            let last = *full.story(sid).unwrap().frames.last().unwrap();
            remove(&mut s, last, Removal::Page);
            let before = s.doc().unwrap().doc.clone();
            let old_flow = compose::compose_story(&before, sid, &Default::default());
            let tail_start = before.story(sid).unwrap().text.rfind('\n').unwrap() + 1;
            assert!(old_flow.overset_at.is_some_and(|at| at < tail_start));
            assert!(old_flow.frames.iter().all(|frame| frame.notes.is_empty()));
            let mut probe = (*before).clone();
            let (command, params) = match change {
                "text" => {
                    let text = &mut probe.story_mut(sid).unwrap().note_mut(note).unwrap().text;
                    text.replace(0..text.len(), "Edited hidden note");
                    ("footnote.setText", json!({"story": sid.0, "id": note, "text": "Edited hidden note"}))
                }
                "options" => {
                    probe.footnote_options.space_before = 3.0;
                    ("footnote.options", json!({"spaceBefore": 3}))
                }
                _ => {
                    probe.styles_mut().char_style_mut("Tail Reference").unwrap().chars.size = Some(14.0);
                    ("style.character.edit", json!({"name": "Tail Reference", "chars": {"size": 14}}))
                }
            };
            assert_same_visible_flow(&old_flow, &compose::compose_story(&probe, sid, &Default::default()));
            let undo_count = s.doc().unwrap().history.undo.len();
            s.execute(command, &params).unwrap();
            let after = s.doc().unwrap().doc.clone();
            assert!(after.page_count() > before.page_count(), "overset note {change} is a primary input change");
            assert!(!s.cache.get(&after, sid, None).is_overset());
            assert!(after.item(last).is_none());
            assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
            assert_reflow_history(&mut s, &before, &after);
        }
    }

    #[test]
    fn unused_cell_and_table_style_edits_preserve_primary_removal() {
        for kind in ["cell", "table"] {
            let (mut s, sid, last) = primary_chain();
            s.execute(&format!("style.{kind}.create"), &json!({"name": "Unused nested style"})).unwrap();
            let original = s.doc().unwrap().doc.clone();
            remove(&mut s, last, Removal::Page);
            let before = s.doc().unwrap().doc.clone();
            let undo_count = s.doc().unwrap().history.undo.len();
            let params = if kind == "cell" {
                json!({"name": "Unused nested style", "fill": "[Black]"})
            } else {
                json!({"name": "Unused nested style", "spaceBefore": 6})
            };
            s.execute(&format!("style.{kind}.edit"), &params).unwrap();
            let after = s.doc().unwrap().doc.clone();
            assert_ne!(before.story(sid).unwrap().rev, after.story(sid).unwrap().rev, "the existing command bumps unrelated story revisions");
            assert_removed(&s, &original, sid, last, original.page_count() - 1);
            assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
            assert_reflow_history(&mut s, &before, &after);
        }
    }

    #[test]
    fn overset_dictionary_and_variable_inputs_resume_primary_reflow() {
        for change in ["dictionary", "variable"] {
            let (mut s, sid, _) = primary_chain();
            s.execute("variables.define", &json!({"name": "Tail Value", "type": "custom", "text": "short"})).unwrap();
            let end = s.doc().unwrap().doc.story(sid).unwrap().len();
            s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
            s.execute("text.insert", &json!({"text": "\nTail counterrevolutionary "})).unwrap();
            s.execute("variables.insert", &json!({"name": "Tail Value"})).unwrap();
            let full = s.doc().unwrap().doc.clone();
            let last = *full.story(sid).unwrap().frames.last().unwrap();
            remove(&mut s, last, Removal::Page);
            let before = s.doc().unwrap().doc.clone();
            let old_flow = compose::compose_story(&before, sid, &Default::default());
            let tail_start = before.story(sid).unwrap().text.rfind('\n').unwrap() + 1;
            assert!(old_flow.overset_at.is_some_and(|at| at < tail_start));
            let mut probe = (*before).clone();
            let (command, params) = if change == "dictionary" {
                probe.hyphenation_exceptions.push("counter~revolutionary".into());
                ("hyphenation.addException", json!({"word": "counter~revolutionary"}))
            } else {
                let index = probe.variable("Tail Value").unwrap();
                probe.text_variables[index].kind = designcraft_doc::vars::VarKind::Custom { text: "Changed invisible value".into() };
                ("variables.define", json!({"name": "Tail Value", "type": "custom", "text": "Changed invisible value"}))
            };
            assert_eq!(probe.story(sid).unwrap(), before.story(sid).unwrap(), "only a document-level input changes");
            assert_same_visible_flow(&old_flow, &compose::compose_story(&probe, sid, &Default::default()));
            let undo_count = s.doc().unwrap().history.undo.len();
            s.execute(command, &params).unwrap();
            let after = s.doc().unwrap().doc.clone();
            assert_eq!(after.story(sid).unwrap().text, before.story(sid).unwrap().text);
            assert!(after.page_count() > before.page_count(), "overset {change} input must resume reflow");
            assert!(!s.cache.get(&after, sid, None).is_overset());
            assert!(after.item(last).is_none());
            assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
            assert_reflow_history(&mut s, &before, &after);
        }
    }

    #[test]
    fn an_unused_variable_definition_preserves_primary_removal() {
        let (mut s, sid, _) = primary_chain();
        s.execute("variables.define", &json!({"name": "Used Value", "type": "custom", "text": "short"})).unwrap();
        let end = s.doc().unwrap().doc.story(sid).unwrap().len();
        s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
        s.execute("text.insert", &json!({"text": "\nTail "})).unwrap();
        s.execute("variables.insert", &json!({"name": "Used Value"})).unwrap();
        let original = s.doc().unwrap().doc.clone();
        let last = *original.story(sid).unwrap().frames.last().unwrap();
        remove(&mut s, last, Removal::Page);
        let before = s.doc().unwrap().doc.clone();
        let undo_count = s.doc().unwrap().history.undo.len();
        s.execute("variables.define", &json!({"name": "Unused Value", "type": "custom", "text": "unrelated"})).unwrap();
        let after = s.doc().unwrap().doc.clone();
        assert!(after.variable("Unused Value").is_some());
        assert_removed(&s, &original, sid, last, original.page_count() - 1);
        assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
        assert_reflow_history(&mut s, &before, &after);
    }

    #[test]
    fn endnote_label_options_refresh_cached_primary_layout() {
        for params in [json!({"prefix": "["}), json!({"suffix": "]"}), json!({"style": "upperRoman"})] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1, "primaryTextFrame": true})).unwrap();
            let sid = s.doc().unwrap().doc.settings.primary_story.unwrap();
            s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
            s.execute("text.insert", &json!({"text": "Source "})).unwrap();
            s.execute("endnote.insert", &json!({"text": "Printed endnote"})).unwrap();
            let before = s.doc().unwrap().doc.clone();
            let old_flow = s.cache.get(&before, sid, None);
            let undo_count = s.doc().unwrap().history.undo.len();
            s.execute("endnote.options", &params).unwrap();
            let after = s.doc().unwrap().doc.clone();
            assert_eq!(before.story(sid), after.story(sid), "only document label controls and the separate bank change");
            let fresh = compose::compose_story(&after, sid, &Default::default());
            let glyphs = |cs: &compose::ComposedStory| {
                cs.frames
                    .iter()
                    .flat_map(|f| &f.lines)
                    .flat_map(|l| &l.glyphs)
                    .map(|g| (g.face, g.gid, g.x, g.y, g.adv, g.visible))
                    .collect::<Vec<_>>()
            };
            assert_ne!(glyphs(&old_flow), glyphs(&fresh), "the label control must affect real glyphs");
            assert_same_visible_flow(&fresh, &s.cache.get(&after, sid, None));
            assert_eq!(after.page_count(), before.page_count());
            assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
            after.check().unwrap();
            s.execute("edit.undo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
            assert_same_visible_flow(&old_flow, &s.cache.get(&before, sid, None));
            s.execute("edit.redo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &after));
            assert_same_visible_flow(&fresh, &s.cache.get(&after, sid, None));
        }
    }

    #[test]
    fn overset_endnote_label_options_resume_primary_reflow() {
        for field in ["prefix", "suffix", "style"] {
            for cold in [false, true] {
                let (mut s, sid, _) = primary_chain();
                let end = s.doc().unwrap().doc.story(sid).unwrap().len();
                s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
                s.execute("text.insert", &json!({"text": "\nTail endnote "})).unwrap();
                let bank = s.execute("endnote.insert", &json!({"text": "Printed endnote bank"})).unwrap()["frame"].as_u64().unwrap();
                let original = s.doc().unwrap().doc.clone();
                let last = *original.story(sid).unwrap().frames.last().unwrap();
                assert_ne!(original.page_of_item(last), original.page_of_item(ItemId(bank)));
                remove(&mut s, last, Removal::Page);
                let before = s.doc().unwrap().doc.clone();
                let old_flow = s.cache.get(&before, sid, None);
                let reference = before.story(sid).unwrap().text.find(designcraft_doc::ENDNOTE_REF).unwrap();
                assert!(old_flow.overset_at.is_some_and(|at| at < reference));
                assert!(before.item(ItemId(bank)).is_some());
                let mut probe = (*before).clone();
                let params = match field {
                    "prefix" => {
                        probe.endnote_options.prefix = "[".into();
                        json!({"prefix": "["})
                    }
                    "suffix" => {
                        probe.endnote_options.suffix = "]".into();
                        json!({"suffix": "]"})
                    }
                    _ => {
                        probe.endnote_options.style = designcraft_doc::NumberStyle::UpperRoman;
                        json!({"style": "upperRoman"})
                    }
                };
                assert_same_visible_flow(&old_flow, &compose::compose_story(&probe, sid, &Default::default()));
                if cold {
                    s.cache.clear();
                }
                let undo_count = s.doc().unwrap().history.undo.len();
                s.execute("endnote.options", &params).unwrap();
                let after = s.doc().unwrap().doc.clone();
                assert!(after.page_count() > before.page_count(), "overset endnote {field}, cold={cold}");
                assert!(after.item(last).is_none());
                assert!(after.item(ItemId(bank)).is_some());
                assert_eq!(after.story(sid).unwrap().text, before.story(sid).unwrap().text);
                assert!(!s.cache.get(&after, sid, None).is_overset());
                assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
                assert_reflow_history(&mut s, &before, &after);
            }
        }
    }

    #[test]
    fn unused_endnote_label_options_preserve_primary_removal() {
        let (mut s, sid, last) = primary_chain();
        let original = s.doc().unwrap().doc.clone();
        remove(&mut s, last, Removal::Page);
        let before = s.doc().unwrap().doc.clone();
        assert!(before.story(sid).unwrap().endnotes.is_empty());
        let undo_count = s.doc().unwrap().history.undo.len();
        s.execute("endnote.options", &json!({"prefix": "Unused label "})).unwrap();
        let after = s.doc().unwrap().doc.clone();
        assert_eq!(after.endnote_options.prefix, "Unused label ");
        assert_removed(&s, &original, sid, last, original.page_count() - 1);
        assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
        assert_reflow_history(&mut s, &before, &after);
    }

    #[test]
    fn content_scale_gesture_refreshes_live_ppi_caption_in_one_undo_step() {
        for reflow in [true, false] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1})).unwrap();
            let png = designcraft_render::Rendered { width: 40, height: 20, pixels: vec![200; 40 * 20 * 4] }.to_png();
            let source =
                ItemId(s.execute("frame.create", &json!({"rect": [100, 100, 140, 120], "content": "graphic"})).unwrap()["id"].as_u64().unwrap());
            s.execute("file.place", &json!({"base64": super::cmd::base64_encode(&png), "name": "a.png", "frame": source.0})).unwrap();
            s.execute("object.fittingOptions", &json!({"ids": [source.0], "fitting": "fitProportionally", "align": 0})).unwrap();
            let frame = ItemId(
                s.execute("object.caption", &json!({"ids": [source.0], "text": "PPI {ppi}", "live": true})).unwrap()["ids"][0].as_u64().unwrap(),
            );
            let sid = s.doc().unwrap().doc.item(frame).unwrap().text_frame().unwrap().story;
            s.execute("object.primaryTextFrame", &json!({"ids": [frame.0], "on": true})).unwrap();
            s.execute("prefs.set", &json!({"smartTextReflow": reflow})).unwrap();
            s.execute("selection.set", &json!({"ids": [source.0]})).unwrap();
            let before = s.doc().unwrap().doc.clone();
            assert_eq!(before.story(sid).unwrap().text, "PPI 72");
            let selection = s.doc().unwrap().selection.clone();
            let undo_count = s.doc().unwrap().history.undo.len();
            let journal = s.journal.clone();
            let params = json!({"from": [100, 100, 140, 120], "to": [100, 100, 180, 140], "content": true});

            s.begin_interaction("Scale image").unwrap();
            s.preview("transform.resize", params.clone()).unwrap();
            assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "PPI 72", "captions are deferred during preview");
            s.cancel_interaction();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
            assert_eq!(s.doc().unwrap().selection, selection);
            assert_eq!(s.doc().unwrap().history.undo.len(), undo_count);
            assert_eq!(s.journal, journal);

            s.begin_interaction("Scale image").unwrap();
            s.preview("transform.resize", json!({"from": [100, 100, 140, 120], "to": [100, 100, 160, 130], "content": true})).unwrap();
            s.preview("transform.resize", params.clone()).unwrap();
            assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "PPI 72");
            assert_eq!(s.doc().unwrap().history.undo.len(), undo_count);
            assert_eq!(s.journal, journal);
            s.cache.clear();
            s.commit_interaction().unwrap();
            let after = s.doc().unwrap().doc.clone();
            assert_eq!(after.story(sid).unwrap().text, "PPI 36", "the final content scale must refresh its caption");
            assert!((after.item(source).unwrap().bounds().width() - 2.0 * before.item(source).unwrap().bounds().width()).abs() < 1e-9);
            assert_eq!(s.doc().unwrap().selection, selection);
            assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
            assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Scale image");
            assert_eq!(s.journal.len(), journal.len() + 1);
            assert_eq!(s.journal.last(), Some(&("transform.resize".to_string(), params)));
            assert_reflow_history(&mut s, &before, &after);
        }
    }

    fn grouped_live_ppi_caption(reflow: bool, nested: bool) -> (Session, ItemId, ItemId, StoryId) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        s.execute("prefs.set", &json!({"smartTextReflow": reflow})).unwrap();
        let png = designcraft_render::Rendered { width: 40, height: 20, pixels: vec![200; 40 * 20 * 4] }.to_png();
        let source = ItemId(s.execute("frame.create", &json!({"rect": [100, 100, 140, 120], "content": "graphic"})).unwrap()["id"].as_u64().unwrap());
        s.execute("file.place", &json!({"base64": super::cmd::base64_encode(&png), "name": "grouped.png", "frame": source.0})).unwrap();
        s.execute("object.fittingOptions", &json!({"ids": [source.0], "fitting": "fitProportionally", "align": 0})).unwrap();
        let caption =
            ItemId(s.execute("object.caption", &json!({"ids": [source.0], "text": "PPI {ppi}", "live": true})).unwrap()["ids"][0].as_u64().unwrap());
        let sid = s.doc().unwrap().doc.item(caption).unwrap().text_frame().unwrap().story;
        let mut group = ItemId(s.execute("object.group", &json!({"ids": [source.0, caption.0]})).unwrap()["id"].as_u64().unwrap());
        if nested {
            let other = s.execute("frame.create", &json!({"rect": [160, 100, 180, 120], "content": "unassigned"})).unwrap()["id"].as_u64().unwrap();
            group = ItemId(s.execute("object.group", &json!({"ids": [group.0, other]})).unwrap()["id"].as_u64().unwrap());
        }
        let d = &s.doc().unwrap().doc;
        assert!(d.settings.primary_story.is_none());
        assert!(d.spreads.iter().all(|sp| sp.items.iter().all(|item| item.live_caption.is_none())), "no top-level caption may mask the guard");
        assert_eq!(d.top_level_of(source), Some(group));
        assert_eq!(d.top_level_of(caption), Some(group));
        assert_eq!(d.find(caption).unwrap().path.len(), if nested { 3 } else { 2 });
        assert_eq!(d.item(caption).unwrap().live_caption.as_ref().unwrap().source, source);
        assert_eq!(d.story(sid).unwrap().text, "PPI 72");
        d.check().unwrap();
        (s, source, group, sid)
    }

    // Independent oracle: project the image's horizontal extent through all parent transforms.
    // Do not use the caption formatter or refresh routine to compute the expected text.
    fn grouped_image_ppi(d: &Document, source: ItemId, expected_width: f64) -> u32 {
        let loc = d.find(source).unwrap();
        let item = d.item(source).unwrap();
        let graphic = item.graphic().unwrap();
        let pixels = d.assets.get(&graphic.asset).unwrap().pixels.unwrap().0;
        let xf = d.parent_xf(&loc) * item.xf * graphic.xf;
        let left = xf * designcraft_geom::Point::new(0.0, 0.0);
        let right = xf * designcraft_geom::Point::new(graphic.size.0, 0.0);
        let width = (right - left).hypot();
        assert!((width - expected_width).abs() < 1e-9, "effective image width: {width}");
        assert_eq!(pixels, 40);
        (f64::from(pixels) * 72.0 / width).round() as u32
    }

    fn grouped_resize_params(d: &Document, group: ItemId, scale: f64) -> serde_json::Value {
        let b = d.item(group).unwrap().bounds();
        json!({
            "from": [b.x0, b.y0, b.x1, b.y1],
            "to": [b.x0, b.y0, b.x0 + scale * b.width(), b.y0 + scale * b.height()],
            "content": true
        })
    }

    #[test]
    fn grouped_live_ppi_caption_gesture_refreshes_in_one_history_step() {
        for reflow in [true, false] {
            for nested in [false, true] {
                let (mut s, source, group, sid) = grouped_live_ppi_caption(reflow, nested);
                let before = s.doc().unwrap().doc.clone();
                let selection = s.doc().unwrap().selection.clone();
                let undo_count = s.doc().unwrap().history.undo.len();
                let journal = s.journal.clone();
                let params = grouped_resize_params(&before, group, 2.0);
                assert_eq!(grouped_image_ppi(&before, source, 40.0), 72);

                s.begin_interaction("Scale grouped image").unwrap();
                s.preview("transform.resize", params.clone()).unwrap();
                assert_eq!(grouped_image_ppi(&s.doc().unwrap().doc, source, 80.0), 36);
                assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "PPI 72", "preview defers captions");
                assert_eq!(s.doc().unwrap().history.undo.len(), undo_count);
                assert_eq!(s.journal, journal);
                s.cancel_interaction();
                assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
                assert_eq!(s.doc().unwrap().selection, selection);
                assert_eq!(s.doc().unwrap().history.undo.len(), undo_count);
                assert_eq!(s.journal, journal);
                assert_eq!(grouped_image_ppi(&s.doc().unwrap().doc, source, 40.0), 72);
                assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "PPI 72");

                s.begin_interaction("Scale grouped image").unwrap();
                s.preview("transform.resize", grouped_resize_params(&before, group, 1.5)).unwrap();
                assert_eq!(grouped_image_ppi(&s.doc().unwrap().doc, source, 60.0), 48);
                s.preview("transform.resize", params.clone()).unwrap();
                assert_eq!(grouped_image_ppi(&s.doc().unwrap().doc, source, 80.0), 36);
                assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "PPI 72");
                assert_eq!(s.doc().unwrap().history.undo.len(), undo_count);
                assert_eq!(s.journal, journal);
                s.commit_interaction().unwrap();
                let after = s.doc().unwrap().doc.clone();
                let expected = grouped_image_ppi(&after, source, 80.0);
                assert_eq!(expected, 36);
                assert_eq!(after.story(sid).unwrap().text, format!("PPI {expected}"), "reflow={reflow}, nested={nested}");
                assert_eq!(after.page_count(), before.page_count());
                assert_eq!(s.prefs.smart_text_reflow, reflow);
                assert_eq!(s.doc().unwrap().selection, selection);
                assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
                assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Scale grouped image");
                assert_eq!(s.journal.len(), journal.len() + 1);
                assert_eq!(s.journal.last(), Some(&("transform.resize".to_string(), params)));
                after.check().unwrap();

                s.execute("edit.undo", &json!({})).unwrap();
                assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
                assert_eq!(s.doc().unwrap().selection, selection);
                assert_eq!(grouped_image_ppi(&s.doc().unwrap().doc, source, 40.0), 72);
                assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "PPI 72");
                s.execute("edit.redo", &json!({})).unwrap();
                assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &after));
                assert_eq!(s.doc().unwrap().selection, selection);
                assert_eq!(grouped_image_ppi(&s.doc().unwrap().doc, source, 80.0), 36);
                assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "PPI 36");
            }
        }
    }

    #[test]
    fn grouped_live_ppi_caption_ordinary_transforms_refresh_in_one_history_step() {
        for reflow in [true, false] {
            for nested in [false, true] {
                for command in ["transform.resize", "transform.scale"] {
                    let (mut s, source, group, sid) = grouped_live_ppi_caption(reflow, nested);
                    let before = s.doc().unwrap().doc.clone();
                    let selection = s.doc().unwrap().selection.clone();
                    let undo_count = s.doc().unwrap().history.undo.len();
                    let journal = s.journal.clone();
                    let params = if command == "transform.resize" {
                        grouped_resize_params(&before, group, 2.0)
                    } else {
                        json!({"ids": [group.0], "sx": 2.0, "sy": 2.0})
                    };
                    assert_eq!(grouped_image_ppi(&before, source, 40.0), 72);
                    s.execute(command, &params).unwrap();
                    let after = s.doc().unwrap().doc.clone();
                    let expected = grouped_image_ppi(&after, source, 80.0);
                    assert_eq!(expected, 36);
                    assert_eq!(after.story(sid).unwrap().text, format!("PPI {expected}"), "{command}, reflow={reflow}, nested={nested}");
                    assert_eq!(after.page_count(), before.page_count());
                    assert_eq!(s.prefs.smart_text_reflow, reflow);
                    assert_eq!(s.doc().unwrap().selection, selection);
                    assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
                    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, if command == "transform.resize" { "Resize" } else { "Scale" });
                    assert_eq!(s.journal.len(), journal.len() + 1);
                    assert_eq!(s.journal.last(), Some(&(command.to_string(), params)));
                    after.check().unwrap();

                    s.execute("edit.undo", &json!({})).unwrap();
                    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
                    assert_eq!(s.doc().unwrap().selection, selection);
                    assert_eq!(grouped_image_ppi(&s.doc().unwrap().doc, source, 40.0), 72);
                    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "PPI 72");
                    s.execute("edit.redo", &json!({})).unwrap();
                    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &after));
                    assert_eq!(s.doc().unwrap().selection, selection);
                    assert_eq!(grouped_image_ppi(&s.doc().unwrap().doc, source, 80.0), 36);
                    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "PPI 36");
                }
            }
        }
    }

    #[test]
    fn primary_live_caption_gestures_refresh_before_reflow_and_history() {
        for reflow in [true, false] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1, "width": 612, "height": 792})).unwrap();
            let source =
                ItemId(s.execute("frame.create", &json!({"rect": [72, 36, 540, 72], "content": "unassigned"})).unwrap()["id"].as_u64().unwrap());
            s.execute("object.altText", &json!({"ids": [source.0], "text": "First"})).unwrap();
            let frame = ItemId(
                s.execute("object.caption", &json!({"ids": [source.0], "text": "Photo: {altText}", "live": true, "height": 648})).unwrap()["ids"][0]
                    .as_u64()
                    .unwrap(),
            );
            let sid = s.doc().unwrap().doc.item(frame).unwrap().text_frame().unwrap().story;
            s.execute("object.primaryTextFrame", &json!({"ids": [frame.0], "on": true})).unwrap();
            s.execute("prefs.set", &json!({"smartTextReflow": reflow})).unwrap();
            s.execute("selection.set", &json!({"ids": [source.0]})).unwrap();
            for (text, growth) in [("Words keep coming and the page fills up quickly with them. ".repeat(250), true), ("Second".into(), false)] {
                let before = s.doc().unwrap().doc.clone();
                let undo_count = s.doc().unwrap().history.undo.len();
                let journal = s.journal.clone();
                let params = json!({"ids": [source.0], "text": text});
                s.begin_interaction("Caption source gesture").unwrap();
                s.preview("object.altText", params.clone()).unwrap();
                assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, before.story(sid).unwrap().text);
                assert_eq!(s.doc().unwrap().doc.page_count(), before.page_count());
                assert_eq!(s.doc().unwrap().history.undo.len(), undo_count);
                assert_eq!(s.journal, journal);
                s.cache.clear();
                s.commit_interaction().unwrap();
                let after = s.doc().unwrap().doc.clone();
                assert_eq!(after.story(sid).unwrap().text, format!("Photo: {text}"));
                assert_eq!(after.settings.primary_story, Some(sid));
                if growth && reflow {
                    assert!(after.page_count() > before.page_count());
                    assert!(!s.cache.get(&after, sid, None).is_overset());
                } else if growth {
                    assert_eq!(after.page_count(), 1);
                    assert!(s.cache.get(&after, sid, None).is_overset());
                } else {
                    assert_eq!(after.page_count(), 1);
                    assert!(!s.cache.get(&after, sid, None).is_overset());
                }
                assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
                assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Caption source gesture");
                assert_eq!(s.journal.len(), journal.len() + 1);
                assert_eq!(s.journal.last(), Some(&("object.altText".to_string(), params)));
                assert_reflow_history(&mut s, &before, &after);
            }
        }
    }

    #[test]
    fn primary_layout_gesture_with_reflow_off_preserves_reduced_chain() {
        let (mut s, sid, last) = primary_chain();
        s.execute("prefs.set", &json!({"smartTextReflow": false})).unwrap();
        remove(&mut s, last, Removal::Page);
        let before = s.doc().unwrap().doc.clone();
        let first = before.story(sid).unwrap().frames[0];
        let inset = before.item(first).unwrap().text_frame().unwrap().options.inset.map(|v| v + 1.0);
        let undo_count = s.doc().unwrap().history.undo.len();
        s.begin_interaction("Primary layout, reflow off").unwrap();
        s.preview("object.textFrameOptions", json!({"ids": [first.0], "inset": inset})).unwrap();
        let preview = s.doc().unwrap().doc.clone();
        s.cache.clear();
        s.commit_interaction().unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &preview));
        assert_eq!(preview.page_count(), before.page_count());
        assert_eq!(preview.story(sid).unwrap().frames, before.story(sid).unwrap().frames);
        assert_eq!(preview.settings.primary_story, Some(sid));
        assert!(!s.prefs.smart_text_reflow);
        assert!(s.cache.get(&preview, sid, None).is_overset());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo_count + 1);
        preview.check().unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &preview));
    }
    // REVIEW7A: reference dependency regressions use only the existing document/type contract.
    struct R7aXrefCase {
        session: Session,
        primary: StoryId,
        target: StoryId,
        target_frame: ItemId,
        anchor: u64,
        other: StoryId,
        other_frame: ItemId,
        removed: ItemId,
        tail: bool,
    }

    fn r7a_xref_case(self_target: bool, tail: bool, format: &str) -> R7aXrefCase {
        let (mut session, primary, _) = primary_chain();
        session.execute("text.select", &json!({"story": primary.0, "anchor": 0, "focus": 0})).unwrap();
        session.execute("text.insert", &json!({"text": "Target: original.\n"})).unwrap();
        // Keep a spare last page, so moving an external anchor from page 2 to page 3
        // changes no primary frame geometry even after deleting its occupied last page.
        session.execute("layout.pages.insert", &json!({"count": 1})).unwrap();
        let (target, target_frame) = if self_target {
            let frame = session.doc().unwrap().doc.story(primary).unwrap().frames[0];
            (primary, frame)
        } else {
            let doc = &session.doc().unwrap().doc;
            let (si, pi) = doc.page_loc(1).unwrap();
            let x = doc.spreads[si].pages[pi].bounds().x0 + 10.0;
            let made = session
                .execute(
                    "frame.create",
                    &json!({
                        "spread": si, "rect": [x, 10.0, x + 200.0, 30.0],
                        "content": "text", "text": "Target: original.\nUnused target paragraph", "caret": false
                    }),
                )
                .unwrap();
            (StoryId(made["story"].as_u64().unwrap()), ItemId(made["id"].as_u64().unwrap()))
        };
        session.execute("text.select", &json!({"story": target.0, "anchor": 0, "focus": 0})).unwrap();
        let anchor = session.execute("anchor.create", &json!({"name": "Stable"})).unwrap()["id"].as_u64().unwrap();
        let at = if tail {
            let end = session.doc().unwrap().doc.story(primary).unwrap().len();
            session.execute("text.select", &json!({"story": primary.0, "anchor": end, "focus": end})).unwrap();
            session.execute("text.insert", &json!({"text": "\nTail reference "})).unwrap();
            session.doc().unwrap().doc.story(primary).unwrap().len()
        } else {
            session.doc().unwrap().doc.story(primary).unwrap().text.find('\n').unwrap() + 1
        };
        session.execute("text.select", &json!({"story": primary.0, "anchor": at, "focus": at})).unwrap();
        session.execute("xref.insert", &json!({"anchor": anchor, "format": format})).unwrap();
        let made = session
            .execute(
                "frame.create",
                &json!({
                    "spread": 0, "rect": [10, 10, 200, 30], "content": "text", "text": "Unrelated", "caret": false
                }),
            )
            .unwrap();
        let (other, other_frame) = (StoryId(made["story"].as_u64().unwrap()), ItemId(made["id"].as_u64().unwrap()));
        let full = session.doc().unwrap().doc.clone();
        let removed = *full.story(primary).unwrap().frames.last().unwrap();
        remove(&mut session, removed, Removal::Page);
        assert_removed(&session, &full, primary, removed, full.page_count() - 1);
        let flow = session.cache.get(&session.doc().unwrap().doc, primary, None);
        let mark = session.doc().unwrap().doc.story(primary).unwrap().text.find(designcraft_doc::XREF_MARK).unwrap();
        if tail {
            assert!(flow.overset_at.is_some_and(|at| at < mark), "the reference must be wholly overset");
        } else {
            assert!(flow.overset_at.is_some_and(|at| mark < at), "the reference must actually be visible");
        }
        assert_eq!(session.doc().unwrap().doc.page_of_item(target_frame), Some(if self_target { 0 } else { 1 }));
        R7aXrefCase { session, primary, target, target_frame, anchor, other, other_frame, removed, tail }
    }

    fn r7a_xref_text(session: &Session, doc: &Document, primary: StoryId) -> String {
        let index = session.cache.xref_index(doc);
        compose::xref::xref_text(doc, &doc.story(primary).unwrap().xrefs[0], Some(&index))
    }

    fn r7a_xref_before(case: &R7aXrefCase) -> (Arc<Document>, Arc<compose::ComposedStory>, usize) {
        let before = case.session.doc().unwrap().doc.clone();
        let flow = case.session.cache.get(&before, case.primary, None);
        assert!(flow.is_overset());
        assert!(case.session.prefs.smart_text_reflow);
        let undo = case.session.doc().unwrap().history.undo.len();
        (before, flow, undo)
    }

    fn r7a_xref_clear_if_cold(case: &R7aXrefCase, cold: bool) {
        if cold {
            case.session.cache.clear();
        }
    }

    fn r7a_xref_preserved(case: &mut R7aXrefCase, before: &Arc<Document>, undo: usize) {
        let after = case.session.doc().unwrap().doc.clone();
        assert_eq!(after.page_count(), before.page_count(), "an unrelated reference input must preserve the deleted primary capacity");
        assert_eq!(after.story(case.primary), before.story(case.primary), "primary text, references and frame chain must stay intact");
        assert_eq!(after.settings.primary_story, Some(case.primary));
        assert!(after.item(case.removed).is_none());
        assert!(case.session.cache.get(&after, case.primary, None).is_overset());
        assert_eq!(case.session.doc().unwrap().history.undo.len(), undo + 1);
        assert_reflow_history(&mut case.session, before, &after);
    }

    fn r7a_xref_resumed(case: &mut R7aXrefCase, before: &Arc<Document>, undo: usize) {
        let after = case.session.doc().unwrap().doc.clone();
        assert!(after.page_count() > before.page_count(), "a real reference/primary edit must resume reflow, including overset dependencies");
        assert!(after.story(case.primary).unwrap().frames.len() > before.story(case.primary).unwrap().frames.len());
        assert_eq!(after.settings.primary_story, Some(case.primary));
        assert!(after.item(case.removed).is_none());
        assert!(!case.session.cache.get(&after, case.primary, None).is_overset());
        assert_eq!(case.session.doc().unwrap().history.undo.len(), undo + 1);
        assert_reflow_history(&mut case.session, before, &after);
    }

    fn r7a_xref_changed_probe(case: &R7aXrefCase, before: &Document, old: &compose::ComposedStory, probe: &Document) {
        assert_ne!(r7a_xref_text(&case.session, before, case.primary), r7a_xref_text(&case.session, probe, case.primary));
        if case.tail && case.target != case.primary {
            assert_same_visible_flow(old, &case.session.cache.get(probe, case.primary, None));
        }
    }

    fn r7a_xref_rename(doc: &mut Document, target: StoryId, anchor: u64) {
        let story = doc.story_mut(target).unwrap();
        let anchor = story.anchors.iter_mut().find(|a| a.id == anchor).unwrap();
        Arc::make_mut(anchor).name = "Renamed target".into();
        story.rev = story.rev.wrapping_add(1);
    }

    fn r7a_xref_page_delta(case: &R7aXrefCase, doc: &Document) -> f64 {
        let page = doc.page_of_item(case.target_frame).unwrap();
        assert_eq!(page, 1);
        let (si, pi) = doc.page_loc(page).unwrap();
        let (other_si, other_pi) = doc.page_loc(page + 1).unwrap();
        assert_eq!(si, other_si, "external target moves between the two pages of the same spread");
        doc.spreads[other_si].pages[other_pi].bounds().x0 - doc.spreads[si].pages[pi].bounds().x0
    }

    #[test]
    fn r7a_xref_unrelated_story_creation_preserves_deleted_primary_capacity() {
        for tail in [false, true] {
            for cold in [false, true] {
                let mut case = r7a_xref_case(true, tail, "Text Anchor Name");
                let (before, old, undo) = r7a_xref_before(&case);
                let mut probe = (*before).clone();
                let layer = probe.default_layer();
                let para = designcraft_doc::ParaFormat { style: probe.styles.default_paragraph.clone(), ..Default::default() };
                probe
                    .add_text_frame(
                        designcraft_doc::SpreadRef::Doc(0),
                        designcraft_geom::Rect::new(10.0, 10.0, 190.0, 30.0),
                        layer,
                        "New unrelated story",
                        para,
                    )
                    .unwrap();
                assert_eq!(probe.story(case.primary), before.story(case.primary));
                assert_eq!(r7a_xref_text(&case.session, &before, case.primary), r7a_xref_text(&case.session, &probe, case.primary));
                assert_same_visible_flow(&old, &case.session.cache.get(&probe, case.primary, None));
                r7a_xref_clear_if_cold(&case, cold);
                let made = case
                    .session
                    .execute(
                        "frame.create",
                        &json!({
                            "spread": 0, "rect": [10, 10, 190, 30], "content": "text", "text": "New unrelated story", "caret": false
                        }),
                    )
                    .unwrap();
                assert!(case.session.doc().unwrap().doc.item(ItemId(made["id"].as_u64().unwrap())).is_some());
                r7a_xref_preserved(&mut case, &before, undo);
            }
        }
    }

    #[test]
    fn r7a_xref_unrelated_story_edit_preserves_deleted_primary_capacity() {
        for tail in [false, true] {
            for cold in [false, true] {
                let mut case = r7a_xref_case(false, tail, "Paragraph Text");
                case.session.execute("text.select", &json!({"story": case.other.0, "anchor": 0, "focus": 0})).unwrap();
                let (before, old, undo) = r7a_xref_before(&case);
                let mut probe = (*before).clone();
                probe.story_mut(case.other).unwrap().insert(0, "Edited ");
                assert_eq!(r7a_xref_text(&case.session, &before, case.primary), r7a_xref_text(&case.session, &probe, case.primary));
                assert_same_visible_flow(&old, &case.session.cache.get(&probe, case.primary, None));
                r7a_xref_clear_if_cold(&case, cold);
                case.session.execute("text.insert", &json!({"text": "Edited "})).unwrap();
                assert_eq!(case.session.doc().unwrap().doc.story(case.other).unwrap().text, "Edited Unrelated");
                r7a_xref_preserved(&mut case, &before, undo);
            }
        }
    }

    #[test]
    fn r7a_xref_unrelated_story_deletion_preserves_deleted_primary_capacity() {
        for tail in [false, true] {
            for cold in [false, true] {
                let mut case = r7a_xref_case(false, tail, "Page Number");
                let (before, old, undo) = r7a_xref_before(&case);
                let mut probe = (*before).clone();
                probe.remove_item(case.other_frame).unwrap();
                assert_eq!(r7a_xref_text(&case.session, &before, case.primary), r7a_xref_text(&case.session, &probe, case.primary));
                assert_same_visible_flow(&old, &case.session.cache.get(&probe, case.primary, None));
                r7a_xref_clear_if_cold(&case, cold);
                case.session.execute("edit.clear", &json!({"ids": [case.other_frame.0]})).unwrap();
                assert!(case.session.doc().unwrap().doc.story(case.other).is_none());
                r7a_xref_preserved(&mut case, &before, undo);
            }
        }
    }

    #[test]
    fn r7a_xref_unused_target_paragraph_change_preserves_deleted_primary_capacity() {
        for tail in [false, true] {
            for cold in [false, true] {
                let mut case = r7a_xref_case(false, tail, "Text Anchor Name");
                let end = case.session.doc().unwrap().doc.story(case.target).unwrap().text.find('\n').unwrap();
                let start = designcraft_doc::ANCHOR_MARK.len_utf8();
                case.session.execute("text.select", &json!({"story": case.target.0, "anchor": start, "focus": end})).unwrap();
                let (before, old, undo) = r7a_xref_before(&case);
                let mut probe = (*before).clone();
                probe.story_mut(case.target).unwrap().replace(start..end, "Target: revised.");
                assert_ne!(before.xref_values(case.anchor), probe.xref_values(case.anchor), "unused resolver fields really change");
                assert_eq!(r7a_xref_text(&case.session, &before, case.primary), r7a_xref_text(&case.session, &probe, case.primary));
                assert_same_visible_flow(&old, &case.session.cache.get(&probe, case.primary, None));
                r7a_xref_clear_if_cold(&case, cold);
                case.session.execute("text.insert", &json!({"text": "Target: revised."})).unwrap();
                r7a_xref_preserved(&mut case, &before, undo);
            }
        }
    }

    #[test]
    fn r7a_xref_unused_format_definition_preserves_deleted_primary_capacity() {
        for tail in [false, true] {
            for cold in [false, true] {
                let mut case = r7a_xref_case(true, tail, "Text Anchor Name");
                let (before, _, undo) = r7a_xref_before(&case);
                r7a_xref_clear_if_cold(&case, cold);
                case.session.execute("xref.defineFormat", &json!({"name": "Unused R7A format", "definition": "<pageNum />"})).unwrap();
                assert!(case.session.doc().unwrap().doc.xref_format("Unused R7A format").is_some());
                r7a_xref_preserved(&mut case, &before, undo);
            }
        }
    }

    #[test]
    fn r7a_xref_target_page_move_unused_by_name_preserves_deleted_primary_capacity() {
        for tail in [false, true] {
            for cold in [false, true] {
                let mut case = r7a_xref_case(false, tail, "Text Anchor Name");
                let (before, old, undo) = r7a_xref_before(&case);
                let dx = r7a_xref_page_delta(&case, &before);
                let mut probe = (*before).clone();
                let item = probe.item_mut(case.target_frame).unwrap();
                item.xf = designcraft_geom::Affine::translate((dx, 0.0)) * item.xf;
                assert_eq!(probe.page_of_item(case.target_frame), Some(2));
                assert_eq!(r7a_xref_text(&case.session, &before, case.primary), r7a_xref_text(&case.session, &probe, case.primary));
                assert_same_visible_flow(&old, &case.session.cache.get(&probe, case.primary, None));
                r7a_xref_clear_if_cold(&case, cold);
                case.session.execute("transform.move", &json!({"ids": [case.target_frame.0], "dx": dx, "dy": 0})).unwrap();
                assert_eq!(case.session.doc().unwrap().doc.page_of_item(case.target_frame), Some(2));
                r7a_xref_preserved(&mut case, &before, undo);
            }
        }
    }

    #[test]
    fn r7a_xref_used_anchor_name_change_resumes_primary_reflow() {
        for self_target in [false, true] {
            for tail in [false, true] {
                for cold in [false, true] {
                    let mut case = r7a_xref_case(self_target, tail, "Text Anchor Name");
                    let (before, old, undo) = r7a_xref_before(&case);
                    let mut probe = (*before).clone();
                    r7a_xref_rename(&mut probe, case.target, case.anchor);
                    r7a_xref_changed_probe(&case, &before, &old, &probe);
                    let (target, anchor) = (case.target, case.anchor);
                    // No anchor-rename command exists; use the actual engine commit path
                    // around this synthetic document edit, rather than call private reflow.
                    r7a_xref_clear_if_cold(&case, cold);
                    case.session.begin_interaction("Rename reference target").unwrap();
                    case.session
                        .edit(|doc, _| {
                            r7a_xref_rename(doc, target, anchor);
                            Ok(())
                        })
                        .unwrap();
                    assert_eq!(case.session.doc().unwrap().doc.page_count(), before.page_count(), "preview defers reflow");
                    r7a_xref_clear_if_cold(&case, cold);
                    case.session.commit_interaction().unwrap();
                    r7a_xref_resumed(&mut case, &before, undo);
                }
            }
        }
    }

    #[test]
    fn r7a_xref_used_target_paragraph_change_resumes_primary_reflow() {
        for self_target in [false, true] {
            for tail in [false, true] {
                for cold in [false, true] {
                    let mut case = r7a_xref_case(self_target, tail, "Paragraph Text");
                    let end = case.session.doc().unwrap().doc.story(case.target).unwrap().text.find('\n').unwrap();
                    let start = designcraft_doc::ANCHOR_MARK.len_utf8();
                    case.session.execute("text.select", &json!({"story": case.target.0, "anchor": start, "focus": end})).unwrap();
                    let (before, old, undo) = r7a_xref_before(&case);
                    let mut probe = (*before).clone();
                    probe.story_mut(case.target).unwrap().replace(start..end, "Target: revised.");
                    r7a_xref_changed_probe(&case, &before, &old, &probe);
                    r7a_xref_clear_if_cold(&case, cold);
                    case.session.execute("text.insert", &json!({"text": "Target: revised."})).unwrap();
                    r7a_xref_resumed(&mut case, &before, undo);
                }
            }
        }
    }

    #[test]
    fn r7a_xref_used_target_page_move_resumes_primary_reflow() {
        for tail in [false, true] {
            for cold in [false, true] {
                let mut case = r7a_xref_case(false, tail, "Page Number");
                let (before, old, undo) = r7a_xref_before(&case);
                let dx = r7a_xref_page_delta(&case, &before);
                let mut probe = (*before).clone();
                let item = probe.item_mut(case.target_frame).unwrap();
                item.xf = designcraft_geom::Affine::translate((dx, 0.0)) * item.xf;
                assert_eq!(probe.page_of_item(case.target_frame), Some(2));
                assert_eq!(probe.story(case.primary), before.story(case.primary), "only the external target frame moves");
                r7a_xref_changed_probe(&case, &before, &old, &probe);
                r7a_xref_clear_if_cold(&case, cold);
                case.session.execute("transform.move", &json!({"ids": [case.target_frame.0], "dx": dx, "dy": 0})).unwrap();
                r7a_xref_resumed(&mut case, &before, undo);
            }
        }
    }

    #[test]
    fn r7a_xref_used_reference_format_changes_resume_primary_reflow() {
        for definition in [false, true] {
            for self_target in [false, true] {
                for tail in [false, true] {
                    for cold in [false, true] {
                        let mut case = r7a_xref_case(self_target, tail, "Text Anchor Name");
                        let (before, old, undo) = r7a_xref_before(&case);
                        let mut probe = (*before).clone();
                        let (command, params) = if definition {
                            probe.xref_formats.iter_mut().find(|f| f.name == "Text Anchor Name").unwrap().definition =
                                "Changed <txtAnchrName />".into();
                            ("xref.defineFormat", json!({"name": "Text Anchor Name", "definition": "Changed <txtAnchrName />"}))
                        } else {
                            let story = probe.story_mut(case.primary).unwrap();
                            Arc::make_mut(&mut story.xrefs[0]).format = "Page Number".into();
                            story.rev = story.rev.wrapping_add(1);
                            ("xref.setFormat", json!({"story": case.primary.0, "index": 0, "format": "Page Number"}))
                        };
                        r7a_xref_changed_probe(&case, &before, &old, &probe);
                        r7a_xref_clear_if_cold(&case, cold);
                        case.session.execute(command, &params).unwrap();
                        r7a_xref_resumed(&mut case, &before, undo);
                    }
                }
            }
        }
    }

    #[test]
    fn r7a_xref_overset_primary_text_edit_resumes_primary_reflow() {
        for cold in [false, true] {
            let mut case = r7a_xref_case(true, true, "Text Anchor Name");
            let end = case.session.doc().unwrap().doc.story(case.primary).unwrap().len();
            case.session.execute("text.select", &json!({"story": case.primary.0, "anchor": end, "focus": end})).unwrap();
            let (before, old, undo) = r7a_xref_before(&case);
            let mut probe = (*before).clone();
            probe.story_mut(case.primary).unwrap().insert(end, " More entirely overset text.");
            assert_same_visible_flow(&old, &case.session.cache.get(&probe, case.primary, None));
            r7a_xref_clear_if_cold(&case, cold);
            case.session.execute("text.insert", &json!({"text": " More entirely overset text."})).unwrap();
            r7a_xref_resumed(&mut case, &before, undo);
        }
    }

    #[test]
    fn r7a_xref_target_deletion_resumes_primary_reflow() {
        for tail in [false, true] {
            for cold in [false, true] {
                let mut case = r7a_xref_case(false, tail, "Text Anchor Name");
                let (before, old, undo) = r7a_xref_before(&case);
                let mut probe = (*before).clone();
                probe.remove_item(case.target_frame).unwrap();
                assert_eq!(r7a_xref_text(&case.session, &probe, case.primary), "??");
                r7a_xref_changed_probe(&case, &before, &old, &probe);
                r7a_xref_clear_if_cold(&case, cold);
                case.session.execute("edit.clear", &json!({"ids": [case.target_frame.0]})).unwrap();
                assert!(case.session.doc().unwrap().doc.story(case.target).is_none());
                r7a_xref_resumed(&mut case, &before, undo);
            }
        }
    }

    // REVIEW7A incremental running-header regressions. Every probe is the actual
    // public command in an interaction, cancelled before the ordinary command/commit.
    struct R7aRunningCase {
        session: Session,
        primary: StoryId,
        heading: StoryId,
        heading_frame: ItemId,
        second: Option<(StoryId, ItemId)>,
        other: StoryId,
        other_frame: ItemId,
        removed: ItemId,
        index: usize,
        tail: bool,
    }

    fn r7a_running_select(s: &mut Session, story: StoryId) {
        let text = &s.doc().unwrap().doc.story(story).unwrap().text;
        let start = if text.starts_with(designcraft_doc::ANCHOR_MARK) { designcraft_doc::ANCHOR_MARK.len_utf8() } else { 0 };
        let end = text.len();
        s.execute("text.select", &json!({"story": story.0, "anchor": start, "focus": end})).unwrap();
    }

    fn r7a_running_source(s: &mut Session, text: &str, y: f64, character: bool) -> (StoryId, ItemId) {
        let made = s
            .execute(
                "frame.create",
                &json!({
                    "spread": 0, "rect": [10.0, y, 310.0, y + 24.0],
                    "content": "text", "text": text, "caret": false
                }),
            )
            .unwrap();
        let story = StoryId(made["story"].as_u64().unwrap());
        let frame = ItemId(made["id"].as_u64().unwrap());
        r7a_running_select(s, story);
        s.execute(if character { "style.character.apply" } else { "style.paragraph.apply" }, &json!({"name": "R7a Heading"})).unwrap();
        (story, frame)
    }

    fn r7a_running_case(character: bool, last: bool, second: bool, tail: bool) -> R7aRunningCase {
        let (mut session, primary, _) = primary_chain();
        session.execute("style.paragraph.create", &json!({"name": "R7a Heading"})).unwrap();
        session.execute("style.character.create", &json!({"name": "R7a Heading"})).unwrap();
        let (heading, heading_frame) = r7a_running_source(&mut session, "Stable heading", 100.0, character);
        session.execute("text.select", &json!({"story": heading.0, "anchor": 0, "focus": 0})).unwrap();
        let anchor = session.execute("anchor.create", &json!({"name": "Stable"})).unwrap()["id"].as_u64().unwrap();
        let second = second.then(|| r7a_running_source(&mut session, "Second heading", 120.0, character));
        // Replace the built-in definition. An extra Heading variable would leave
        // Basic Paragraph in the renderer's wanted set and invalidate the oracle.
        let index = session
            .execute(
                "variables.define",
                &json!({
                    "name": "Running Header", "type": "runningHeader", "style": "R7a Heading",
                    "character": character, "use": if last { "lastOnPage" } else { "firstOnPage" },
                    "before": "Head[", "after": "]"
                }),
            )
            .unwrap()["index"]
            .as_u64()
            .unwrap() as usize;
        let at = if tail { session.doc().unwrap().doc.story(primary).unwrap().len() } else { 0 };
        session.execute("text.select", &json!({"story": primary.0, "anchor": at, "focus": at})).unwrap();
        if tail {
            session.execute("text.insert", &json!({"text": "\n"})).unwrap();
        }
        session.execute("variables.insert", &json!({"name": "Running Header"})).unwrap();
        session.execute("text.insert", &json!({"text": "\n"})).unwrap();
        session.execute("xref.insert", &json!({"anchor": anchor, "format": "Text Anchor Name"})).unwrap();
        let mark = session.doc().unwrap().doc.story(primary).unwrap().text.find(designcraft_doc::XREF_MARK).unwrap();
        session.execute("text.select", &json!({"story": primary.0, "anchor": mark, "focus": mark + designcraft_doc::XREF_MARK.len_utf8()})).unwrap();
        session.execute("type.char", &json!({"attrs": {"size": 16, "underline": true}})).unwrap();
        let made = session
            .execute(
                "frame.create",
                &json!({
                    "spread": 0, "rect": [10, 170, 210, 194], "content": "text", "text": "Unrelated", "caret": false
                }),
            )
            .unwrap();
        let (other, other_frame) = (StoryId(made["story"].as_u64().unwrap()), ItemId(made["id"].as_u64().unwrap()));
        let full = session.doc().unwrap().doc.clone();
        let removed = *full.story(primary).unwrap().frames.last().unwrap();
        assert!(session.cache.get(&full, primary, None).frame(removed).is_some_and(|f| !f.range.is_empty()));
        remove(&mut session, removed, Removal::Page);
        assert_removed(&session, &full, primary, removed, full.page_count() - 1);
        let before = session.doc().unwrap().doc.clone();
        let old = session.cache.get(&before, primary, None);
        assert!(session.prefs.smart_text_reflow && old.is_overset());
        assert!(compose::compose_story(&before, primary, &Default::default()).is_overset(), "autoflow's own fit must need capacity");
        let text = &before.story(primary).unwrap().text;
        let header_at = text.find(designcraft_doc::vars::var_char(index).unwrap()).unwrap();
        let xref_at = text.find(designcraft_doc::XREF_MARK).unwrap();
        assert!(old.overset_at.is_some_and(|o| if tail { o < header_at && o < xref_at } else { header_at < o && xref_at < o }));
        assert_eq!(before.page_of_item(heading_frame), Some(0));
        r7a_sidecar_nonmatching(&before, other);
        assert_eq!(before.item(other_frame).unwrap().wrap.mode, designcraft_doc::WrapMode::None);
        let values = r7a_sidecar_header_values(&session, &before, index);
        let expected: Vec<String> = (0..before.page_count())
            .map(|page| if second.is_some() && (page > 0 || last) { "Head[Second heading]" } else { "Head[Stable heading]" }.to_string())
            .collect();
        assert_eq!(values, expected);
        assert_eq!(r7a_sidecar_xrefs(&session, &before, primary), vec!["“Stable”".to_string()]);
        R7aRunningCase { session, primary, heading, heading_frame, second, other, other_frame, removed, index, tail }
    }

    fn r7a_sidecar_header_values(s: &Session, d: &Document, index: usize) -> Vec<String> {
        let running = s.cache.running_index(d);
        (0..d.page_count()).map(|page| compose::vars::values(d, Some(page), Some(&running))[index].clone()).collect()
    }

    fn r7a_sidecar_xrefs(s: &Session, d: &Document, primary: StoryId) -> Vec<String> {
        let index = s.cache.xref_index(d);
        d.story(primary).unwrap().xrefs.iter().map(|x| compose::xref::xref_text(d, x, Some(&index))).collect()
    }

    fn r7a_sidecar_nonmatching(d: &Document, sid: StoryId) {
        use designcraft_doc::vars::VarKind;
        let story = d.story(sid).unwrap();
        assert!(!compose::vars::has_running(d, story));
        for variable in &d.text_variables {
            if let VarKind::RunningHeader { style, character, .. } = &variable.kind {
                if *character {
                    assert!(story.runs().all(|(_, f)| f.style != *style));
                } else {
                    assert!(story.paras.iter().all(|f| f.style != *style));
                }
            }
        }
    }

    fn r7a_running_preview(
        case: &mut R7aRunningCase,
        command: &str,
        params: serde_json::Value,
    ) -> (Arc<Document>, Arc<compose::ComposedStory>, Arc<Document>, usize) {
        let before = case.session.doc().unwrap().doc.clone();
        let old = case.session.cache.get(&before, case.primary, None);
        let undo = case.session.doc().unwrap().history.undo.len();
        case.session.begin_interaction("Independent dependency probe").unwrap();
        case.session.preview(command, params).unwrap();
        let probe = case.session.doc().unwrap().doc.clone();
        assert_eq!(probe.page_count(), before.page_count(), "preview defers reflow");
        assert_eq!(probe.story(case.primary).unwrap().frames, before.story(case.primary).unwrap().frames);
        assert_eq!(case.session.doc().unwrap().history.undo.len(), undo);
        probe.check().unwrap();
        (before, old, probe, undo)
    }

    fn r7a_running_cancel(case: &mut R7aRunningCase, before: &Arc<Document>, undo: usize, cold: bool) {
        case.session.cancel_interaction();
        assert!(Arc::ptr_eq(&case.session.doc().unwrap().doc, before));
        assert_eq!(case.session.doc().unwrap().history.undo.len(), undo);
        if cold {
            case.session.cache.clear();
        }
    }

    fn r7a_running_equal_probe(case: &R7aRunningCase, before: &Document, old: &compose::ComposedStory, probe: &Document) {
        assert_eq!(probe.story(case.primary), before.story(case.primary));
        assert_eq!(r7a_sidecar_header_values(&case.session, probe, case.index), r7a_sidecar_header_values(&case.session, before, case.index));
        assert_eq!(r7a_sidecar_xrefs(&case.session, probe, case.primary), r7a_sidecar_xrefs(&case.session, before, case.primary));
        let flow = case.session.cache.get(probe, case.primary, None);
        assert_eq!(format!("{old:?}"), format!("{flow:?}"), "all primary composition fields are unchanged");
        assert!(!std::ptr::eq(old, &*flow), "cache churn must be exercised");
        // Explicitly include the carried body after the document's last page.
        let carry = |d: &Document| {
            let index = case.session.cache.running_index(d);
            compose::vars::values(d, Some(d.page_count()), Some(&index))[case.index].clone()
        };
        assert_eq!(carry(before), carry(probe));
    }

    fn r7a_running_changed_probe(case: &R7aRunningCase, before: &Document, old: &compose::ComposedStory, probe: &Document) {
        assert_eq!(probe.story(case.primary), before.story(case.primary));
        assert_ne!(
            r7a_sidecar_header_values(&case.session, before, case.index),
            r7a_sidecar_header_values(&case.session, probe, case.index),
            "the consumed header output must actually change"
        );
        assert_eq!(
            r7a_sidecar_xrefs(&case.session, before, case.primary),
            r7a_sidecar_xrefs(&case.session, probe, case.primary),
            "these controls isolate header dependencies"
        );
        if case.tail {
            assert_eq!(
                format!("{old:?}"),
                format!("{:?}", case.session.cache.get(probe, case.primary, None)),
                "a wholly overset header change has identical visible composition"
            );
        }
    }

    fn r7a_running_preserved(case: &mut R7aRunningCase, before: &Arc<Document>, undo: usize) {
        let after = case.session.doc().unwrap().doc.clone();
        assert_eq!(after.page_count(), before.page_count(), "equal consumed header/reference values must preserve deleted primary capacity");
        assert_eq!(after.story(case.primary), before.story(case.primary));
        assert_eq!(after.settings.primary_story, Some(case.primary));
        assert!(after.item(case.removed).is_none());
        assert!(case.session.cache.get(&after, case.primary, None).is_overset());
        assert_eq!(case.session.doc().unwrap().history.undo.len(), undo + 1);
        assert_reflow_history(&mut case.session, before, &after);
    }

    fn r7a_running_resumed(case: &mut R7aRunningCase, before: &Arc<Document>, undo: usize) {
        let after = case.session.doc().unwrap().doc.clone();
        assert!(after.page_count() > before.page_count(), "changed consumed header/primary values must resume reflow");
        assert!(after.story(case.primary).unwrap().frames.len() > before.story(case.primary).unwrap().frames.len());
        assert_eq!(after.settings.primary_story, Some(case.primary));
        assert!(after.item(case.removed).is_none());
        assert!(!case.session.cache.get(&after, case.primary, None).is_overset());
        assert_eq!(case.session.doc().unwrap().history.undo.len(), undo + 1);
        assert_reflow_history(&mut case.session, before, &after);
    }

    #[test]
    fn r7a_running_header_nonmatching_story_preserves_deleted_primary_capacity() {
        for operation in ["create", "edit", "delete"] {
            for tail in [false, true] {
                for cold in [false, true] {
                    let mut case = r7a_running_case(false, false, false, tail);
                    if operation == "edit" {
                        case.session.execute("text.select", &json!({"story": case.other.0, "anchor": 0, "focus": 0})).unwrap();
                    }
                    let (command, params) = match operation {
                        "create" => (
                            "frame.create",
                            json!({"spread": 0, "rect": [10, 100, 210, 124], "content": "text", "text": "New unrelated story", "caret": false}),
                        ),
                        "edit" => ("text.insert", json!({"text": "Edited "})),
                        _ => ("edit.clear", json!({"ids": [case.other_frame.0]})),
                    };
                    let (before, old, probe, undo) = r7a_running_preview(&mut case, command, params.clone());
                    assert_eq!(probe.story(case.heading), before.story(case.heading));
                    assert_eq!(format!("{:?}", probe.item(case.heading_frame)), format!("{:?}", before.item(case.heading_frame)));
                    r7a_running_equal_probe(&case, &before, &old, &probe);
                    match operation {
                        "create" => {
                            assert_eq!(probe.stories.len(), before.stories.len() + 1);
                            let added = *probe.stories.keys().find(|sid| !before.stories.contains_key(*sid)).unwrap();
                            r7a_sidecar_nonmatching(&probe, added);
                            let frame = probe.story(added).unwrap().frames[0];
                            assert_eq!(probe.item(frame).unwrap().wrap.mode, designcraft_doc::WrapMode::None);
                        }
                        "edit" => {
                            assert_eq!(probe.story(case.other).unwrap().text, "Edited Unrelated");
                            r7a_sidecar_nonmatching(&probe, case.other);
                        }
                        _ => {
                            assert!(probe.story(case.other).is_none() && probe.item(case.other_frame).is_none());
                            assert_eq!(probe.stories.len() + 1, before.stories.len());
                        }
                    }
                    r7a_running_cancel(&mut case, &before, undo, cold);
                    case.session.execute(command, &params).unwrap();
                    r7a_running_preserved(&mut case, &before, undo);
                }
            }
        }
    }

    #[test]
    fn r7a_running_header_equal_cleaned_source_preserves_deleted_primary_capacity() {
        for character in [false, true] {
            for tail in [false, true] {
                for cold in [false, true] {
                    let mut case = r7a_running_case(character, false, false, tail);
                    r7a_running_select(&mut case.session, case.heading);
                    let params = json!({"text": "Stable\theading"});
                    let (before, old, probe, undo) = r7a_running_preview(&mut case, "text.insert", params.clone());
                    assert_ne!(probe.story(case.heading).unwrap().text, before.story(case.heading).unwrap().text);
                    r7a_running_equal_probe(&case, &before, &old, &probe);
                    r7a_running_cancel(&mut case, &before, undo, cold);
                    case.session.execute("text.insert", &params).unwrap();
                    r7a_running_preserved(&mut case, &before, undo);
                }
            }
        }
    }

    #[test]
    fn r7a_xref_equal_expanded_used_definition_preserves_deleted_primary_capacity() {
        for tail in [false, true] {
            for cold in [false, true] {
                let mut case = r7a_xref_case(false, tail, "Text Anchor Name");
                let (before, old, undo) = r7a_xref_before(&case);
                let command = "xref.defineFormat";
                let params = json!({"name": "Text Anchor Name", "definition": "“<txtAnchrName />”"});
                case.session.begin_interaction("Equal expanded reference probe").unwrap();
                case.session.preview(command, params.clone()).unwrap();
                let probe = case.session.doc().unwrap().doc.clone();
                assert_ne!(before.xref_format("Text Anchor Name"), probe.xref_format("Text Anchor Name"));
                assert_eq!(probe.story(case.primary), before.story(case.primary));
                assert_eq!(r7a_sidecar_xrefs(&case.session, &before, case.primary), r7a_sidecar_xrefs(&case.session, &probe, case.primary));
                assert_eq!(format!("{old:?}"), format!("{:?}", case.session.cache.get(&probe, case.primary, None)));
                assert_eq!(case.session.doc().unwrap().history.undo.len(), undo);
                probe.check().unwrap();
                case.session.cancel_interaction();
                assert!(Arc::ptr_eq(&case.session.doc().unwrap().doc, &before));
                r7a_xref_clear_if_cold(&case, cold);
                case.session.execute(command, &params).unwrap();
                r7a_xref_preserved(&mut case, &before, undo);
            }
        }
    }

    #[test]
    fn r7a_running_header_matching_source_and_style_changes_resume_primary_reflow() {
        for operation in ["create", "text", "delete", "removeStyle", "addStyle"] {
            for character in [false, true] {
                for tail in [false, true] {
                    for cold in [false, true] {
                        let mut case = r7a_running_case(character, operation != "create", matches!(operation, "create" | "delete"), tail);
                        let (command, params) = match operation {
                            // The public Copy/Move command allocates a complete matching
                            // story, preserving its paragraph/character styles. Put its
                            // different text before the current first heading on page 1.
                            "create" => ("transform.move", json!({"ids": [case.second.unwrap().1.0], "dx": 0, "dy": -45, "copy": true})),
                            "text" => {
                                r7a_running_select(&mut case.session, case.heading);
                                ("text.insert", json!({"text": "Changed heading"}))
                            }
                            "delete" => ("edit.clear", json!({"ids": [case.second.unwrap().1.0]})),
                            "removeStyle" => {
                                r7a_running_select(&mut case.session, case.heading);
                                let name = if character {
                                    designcraft_doc::story::NO_CHAR_STYLE.to_string()
                                } else {
                                    case.session.doc().unwrap().doc.styles.default_paragraph.clone()
                                };
                                (if character { "style.character.apply" } else { "style.paragraph.apply" }, json!({"name": name}))
                            }
                            _ => {
                                r7a_running_select(&mut case.session, case.other);
                                (if character { "style.character.apply" } else { "style.paragraph.apply" }, json!({"name": "R7a Heading"}))
                            }
                        };
                        let (before, old, probe, undo) = r7a_running_preview(&mut case, command, params.clone());
                        if operation == "create" {
                            assert_eq!(probe.stories.len(), before.stories.len() + 1);
                            let added = *probe.stories.keys().find(|sid| !before.stories.contains_key(*sid)).unwrap();
                            let story = probe.story(added).unwrap();
                            assert!(!compose::vars::has_running(&probe, story));
                            if character {
                                assert!(story.runs().any(|(_, f)| f.style == "R7a Heading"));
                            } else {
                                assert!(story.paras.iter().any(|f| f.style == "R7a Heading"));
                            }
                        }
                        r7a_running_changed_probe(&case, &before, &old, &probe);
                        r7a_running_cancel(&mut case, &before, undo, cold);
                        case.session.execute(command, &params).unwrap();
                        r7a_running_resumed(&mut case, &before, undo);
                    }
                }
            }
        }
    }

    #[test]
    fn r7a_running_header_source_page_and_order_changes_resume_primary_reflow() {
        for page_move in [false, true] {
            for character in [false, true] {
                for last in [false, true] {
                    for tail in [false, true] {
                        for cold in [false, true] {
                            let mut case = r7a_running_case(character, last, true, tail);
                            let doc = case.session.doc().unwrap().doc.clone();
                            let frame = if page_move && last { case.second.unwrap().1 } else { case.heading_frame };
                            let params = if page_move {
                                let (si, pi) = doc.page_loc(1).unwrap();
                                let x = doc.spreads[si].pages[pi].bounds().x0 + 10.0;
                                let dx = x - doc.item(frame).unwrap().bounds().x0;
                                json!({"ids": [frame.0], "dx": dx, "dy": 0, "toSpread": designcraft_doc::SpreadRef::Doc(si)})
                            } else {
                                json!({"ids": [frame.0], "dx": 0, "dy": 60})
                            };
                            let (before, old, probe, undo) = r7a_running_preview(&mut case, "transform.move", params.clone());
                            if page_move {
                                assert_ne!(probe.find(frame).unwrap().spread, before.find(frame).unwrap().spread);
                                assert_eq!(probe.page_of_item(frame), Some(1));
                            } else {
                                assert_ne!(probe.item(frame).unwrap().bounds(), before.item(frame).unwrap().bounds());
                            }
                            r7a_running_changed_probe(&case, &before, &old, &probe);
                            r7a_running_cancel(&mut case, &before, undo, cold);
                            case.session.execute("transform.move", &params).unwrap();
                            r7a_running_resumed(&mut case, &before, undo);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn r7a_running_header_used_definition_changes_resume_primary_reflow() {
        for operation in ["wrapper", "style", "character", "use"] {
            for character in [false, true] {
                for tail in [false, true] {
                    for cold in [false, true] {
                        let mut case = r7a_running_case(character, false, true, tail);
                        let params = json!({
                            "name": "Running Header", "type": "runningHeader",
                            "style": if operation == "style" { "R7a Missing" } else { "R7a Heading" },
                            "character": if operation == "character" { !character } else { character },
                            "use": if operation == "use" { "lastOnPage" } else { "firstOnPage" },
                            "before": if operation == "wrapper" { "Changed[" } else { "Head[" }, "after": "]"
                        });
                        let (before, old, probe, undo) = r7a_running_preview(&mut case, "variables.define", params.clone());
                        assert_ne!(before.text_variables[case.index], probe.text_variables[case.index]);
                        r7a_running_changed_probe(&case, &before, &old, &probe);
                        r7a_running_cancel(&mut case, &before, undo, cold);
                        case.session.begin_interaction("Change consumed header definition").unwrap();
                        case.session.preview("variables.define", params.clone()).unwrap();
                        assert_eq!(case.session.doc().unwrap().doc.page_count(), before.page_count());
                        assert_eq!(case.session.doc().unwrap().history.undo.len(), undo);
                        if cold {
                            case.session.cache.clear();
                        }
                        case.session.commit_interaction().unwrap();
                        r7a_running_resumed(&mut case, &before, undo);
                    }
                }
            }
        }
    }

    #[test]
    fn r7a_running_header_overset_primary_text_edit_resumes_primary_reflow() {
        for character in [false, true] {
            for cold in [false, true] {
                let mut case = r7a_running_case(character, false, false, true);
                let end = case.session.doc().unwrap().doc.story(case.primary).unwrap().len();
                case.session.execute("text.select", &json!({"story": case.primary.0, "anchor": end, "focus": end})).unwrap();
                let params = json!({"text": " More entirely overset primary text."});
                let (before, old, probe, undo) = r7a_running_preview(&mut case, "text.insert", params.clone());
                assert_same_visible_flow(&old, &case.session.cache.get(&probe, case.primary, None));
                assert_ne!(probe.story(case.primary).unwrap().text, before.story(case.primary).unwrap().text);
                assert_eq!(
                    r7a_sidecar_header_values(&case.session, &probe, case.index),
                    r7a_sidecar_header_values(&case.session, &before, case.index)
                );
                assert_eq!(r7a_sidecar_xrefs(&case.session, &probe, case.primary), r7a_sidecar_xrefs(&case.session, &before, case.primary));
                r7a_running_cancel(&mut case, &before, undo, cold);
                case.session.execute("text.insert", &params).unwrap();
                r7a_running_resumed(&mut case, &before, undo);
            }
        }
    }

    #[test]
    fn r7a_running_header_equal_used_xref_definition_preserves_deleted_primary_capacity() {
        for character in [false, true] {
            for tail in [false, true] {
                for cold in [false, true] {
                    let mut case = r7a_running_case(character, false, false, tail);
                    let params = json!({"name": "Text Anchor Name", "definition": "“<txtAnchrName />”"});
                    let (before, old, probe, undo) = r7a_running_preview(&mut case, "xref.defineFormat", params.clone());
                    assert_ne!(before.xref_format("Text Anchor Name"), probe.xref_format("Text Anchor Name"));
                    r7a_running_equal_probe(&case, &before, &old, &probe);
                    r7a_running_cancel(&mut case, &before, undo, cold);
                    case.session.execute("xref.defineFormat", &params).unwrap();
                    r7a_running_preserved(&mut case, &before, undo);
                }
            }
        }
    }
}

#[cfg(test)]
mod reflow_review8_tests;
