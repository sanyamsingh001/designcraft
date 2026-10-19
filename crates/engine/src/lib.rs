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
        let r = (spec.run)(self, params)?;
        // Undo, redo or deleting layers can take the active layer away: new objects would land on
        // a layer that isn't there (invisible, not exported). Fall back to the top layer.
        if let Some(st) = self.active_mut()
            && st.doc.layer(st.active_layer).is_none()
        {
            st.active_layer = st.doc.default_layer();
        }
        self.record_transform(id, params);
        if self.prefs.smart_text_reflow && spec.undoable {
            self.smart_reflow();
        }
        // Live captions follow their sources.
        if spec.undoable
            && let Some(st) = self.active_mut()
            && st.interaction.is_none()
            && st.doc.spreads.iter().any(|sp| sp.items.iter().any(|i| i.live_caption.is_some()))
        {
            let mut d = (*st.doc).clone();
            if cmd::captions::refresh_live(&mut d) {
                st.doc = Arc::new(d);
            }
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

    /// Smart Text Reflow for the primary story: add threaded pages while it oversets; remove
    /// trailing pages whose only object is an empty frame of it.
    fn smart_reflow(&mut self) {
        let Some(st) = self.active() else { return };
        if st.interaction.is_some() {
            return;
        }
        let Some(sid) = st.doc.settings.primary_story else { return };
        let Some(story) = st.doc.story(sid) else { return };
        let cs = self.cache.get(&st.doc, sid, None);
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
                let cs = designcraft_compose::compose_story(&d, sid, &Default::default());
                if cs.frame(last).is_some_and(|ft| !ft.range.is_empty()) {
                    break;
                }
                let Some(page) = d.page_of_item(last) else { break };
                let alone = d.spreads.iter().flat_map(|sp| sp.items.iter()).filter(|i| d.page_of_item(i.id) == Some(page)).count() == 1;
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
            &json!({"ids": [id.0], "swatch": if stroke { "[Black]" } else { "None" }, "weight": if stroke { 4 } else { 0 }, "align": align}),
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

    fn r35_transformed_stroke_case(position_snap: bool, many: bool) -> (Item, Value) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let moving = frame(&mut s, Rect::new(0.0, 0.0, 100.0, 50.0), true, "center");
        let reference = frame(&mut s, Rect::new(400.0, 300.0, 454.0, 380.0), false, "center");
        s.execute("object.fill", &json!({"ids": [moving.0, reference.0], "swatch": "None"})).unwrap();
        let original_xf = Affine::new([2.0, 0.0, 0.0, 2.0, 100.0, 100.0]);
        s.edit(|doc, _| {
            doc.item_mut(moving).unwrap().xf = original_xf;
            if position_snap {
                Arc::make_mut(&mut doc.spreads[0]).pages[0].guides.push(designcraft_doc::Guide {
                    orientation: designcraft_doc::Orientation::Vertical,
                    position: 150.0,
                    spread: true,
                    locked: false,
                    layer: None,
                    liquid: false,
                });
            }
            Ok(json!({}))
        })
        .unwrap();
        s.execute("selection.set", &json!({"ids": [moving.0]})).unwrap();
        s.execute("prefs.set", &json!({"scaleStrokes": true})).unwrap();
        s.set_tool("selection");
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let mods = Mods { cmd: true, ..Mods::default() };
        let mut view = view();
        view.snap.snap_to_guides = position_snap;
        let at = Point::new(if position_snap { 152.0 } else { 150.0 }, 150.0);
        pointer(&mut s, PointerKind::Down, Point::new(300.0, 150.0), mods, view);
        if many {
            for x in [220.0, 180.0] {
                pointer(&mut s, PointerKind::Drag, Point::new(x, 150.0), mods, view);
            }
        }
        let mut previous = None;
        for _ in 0..if many { 3 } else { 1 } {
            pointer(&mut s, PointerKind::Drag, at, mods, view);
            let item = s.doc().unwrap().doc.item(moving).unwrap().clone();
            assert_eq!(item.xf, original_xf);
            rect(item.path.bounds().unwrap(), Rect::new(0.0, 0.0, 25.0, 50.0));
            rect(item.bounds(), Rect::new(100.0, 100.0, 150.0, 200.0));
            close(item.stroke.weight, 2.0);
            assert!(s.overlays(view).iter().all(|o| !matches!(o, Overlay::Measure { .. })), "no false 54 pt feedback");
            if let Some(previous) = &previous {
                assert_eq!(&item, previous, "identical pointer samples must not compound the stroke or geometry");
            }
            previous = Some(item);
        }
        pointer(&mut s, PointerKind::Up, at, mods, view);
        let after = s.doc().unwrap().doc.clone();
        let item = after.item(moving).unwrap();
        assert_eq!(Some(item), previous.as_ref(), "commit must retain the final preview");
        assert_eq!(after.item(reference), before.item(reference));
        assert_eq!(s.doc().unwrap().selection.items, vec![moving]);
        assert!(s.doc().unwrap().interaction.is_none());
        assert!(s.overlays(view).is_empty());
        // For this axis-aligned rectangle the renderer strokes in local space,
        // then applies xf. Do not use Item::visible_bounds as the render oracle.
        let local_ink = item.path.bounds().unwrap().inflate(item.stroke.weight / 2.0, item.stroke.weight / 2.0);
        let rendered_bounds = item.xf.transform_rect_bbox(local_ink);
        rect(rendered_bounds, Rect::new(98.0, 98.0, 152.0, 202.0));
        close(rendered_bounds.width(), 54.0);
        if !many {
            let mut renderer = designcraft_render::Renderer::new();
            renderer.threads = 0;
            let image = renderer.render_page(&after, &s.cache, 0, 1.0, false, &Default::default()).unwrap();
            let ink: Vec<_> = (90..160)
                .filter(|&x| {
                    let [r, g, b, a] = image.pixel(x, 150);
                    a > 128 && r < 128 && g < 128 && b < 128
                })
                .collect();
            assert_eq!((ink.first().copied(), ink.last().copied()), (Some(98), Some(151)), "54 rendered pixels, not about 56");
            assert_eq!(ink.len(), 8, "two 4-pixel strokes, with an unfilled interior");
        }
        let (command, params) = s.journal.last().unwrap();
        assert_eq!(command, "transform.resize");
        assert_eq!(params["to"], json!([100.0, 100.0, 150.0, 200.0]));
        let result = (item.clone(), params.clone());
        r7a_history(&mut s, &before, &after, undo);
        result
    }

    #[test]
    fn r35_transformed_stroke_resize_suppresses_false_dimensions_and_keeps_rendered_size() {
        for position_snap in [false, true] {
            let single = r35_transformed_stroke_case(position_snap, false);
            let repeated = r35_transformed_stroke_case(position_snap, true);
            assert_eq!(single, repeated, "position snap {position_snap}: previews must always use the pointer-down document");
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
