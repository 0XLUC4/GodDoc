//! Application state and window chrome: tab row, formatting bar, page, status bar.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use egui::text::{CCursor, CCursorRange};
use egui::{
    Align, Align2, Button, CentralPanel, Color32, ComboBox, Context, CornerRadius, FontId, Frame,
    Id, Key, KeyboardShortcut, Layout, Margin, Modifiers, Panel, Popup, Rect, Response, RichText,
    ScrollArea, Sense, Stroke, TextEdit, TextureHandle, TextureId, Ui, ViewportCommand, pos2, vec2,
};
use goddoc_core::{
    Align as ParaAlign, Document, Font, Format, Image, ImageKind, Kind, List, MAX_INDENT, Margins,
    Script, Spacing, docx,
};

use crate::theme::{self, Palette};
use crate::toasts::Toasts;
use crate::widgets::{self, icon, with_icon};
use crate::{APP_NAME, branding, editor, layout, platform};

const TAB_HEIGHT: f32 = 38.0;
const BAR_HEIGHT: f32 = 62.0;
/// Height of the row of ribbon tabs.
const RIBBON_TABS_HEIGHT: f32 = 30.0;
/// Longest document name shown in a tab before "…".
const TAB_TEXT_MAX: f32 = 180.0;
/// Highlighter colours (content, like the text colours).
const HIGHLIGHTS: [([u8; 3], &str); 4] = [
    ([0xFF, 0xF1, 0x76], "Yellow"),
    ([0xB9, 0xF6, 0xCA], "Green"),
    ([0xB3, 0xE5, 0xFC], "Blue"),
    ([0xF8, 0xBB, 0xD0], "Pink"),
];

/// Characters offered by Insert > Special character.
const SYMBOLS: [(&str, &str); 16] = [
    ("—", "Em dash"),
    ("–", "En dash"),
    ("…", "Ellipsis"),
    ("«", "Left quote"),
    ("»", "Right quote"),
    ("•", "Bullet"),
    ("→", "Right arrow"),
    ("←", "Left arrow"),
    ("©", "Copyright"),
    ("®", "Registered"),
    ("™", "Trademark"),
    ("€", "Euro"),
    ("£", "Pound"),
    ("°", "Degree"),
    ("±", "Plus-minus"),
    ("×", "Multiplication"),
];

/// Documents remembered on the home screen.
const MAX_RECENT: usize = 10;
const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.5..=3.0;
const SIZE_RANGE: std::ops::RangeInclusive<f32> = 6.0..=96.0;
/// Typing within this many seconds of the last edit joins its undo step.
const UNDO_GROUP_SECONDS: f64 = 1.0;
/// Characters kept across all undo steps of a document.
const UNDO_BUDGET: usize = 4_000_000;

/// Text colors (content, the only colors the UI shows besides greys).
const SWATCHES: [([u8; 3], &str); 5] = [
    ([0x00, 0x00, 0x00], "Black"),
    ([0x55, 0x55, 0x55], "Dark grey"),
    ([0xD9, 0x2D, 0x20], "Red"),
    ([0x1F, 0x5F, 0xD6], "Blue"),
    ([0x1E, 0x8E, 0x3E], "Green"),
];

#[derive(Clone, Copy)]
enum Flag {
    Bold,
    Italic,
    Underline,
    Strike,
}

impl Flag {
    fn of(self, format: &mut Format) -> &mut bool {
        match self {
            Self::Bold => &mut format.bold,
            Self::Italic => &mut format.italic,
            Self::Underline => &mut format.underline,
            Self::Strike => &mut format.strike,
        }
    }
}

/// A formatting change to the selection.
#[derive(Clone, Copy)]
enum Edit {
    Toggle(Flag),
    Font(Font),
    Size(f32),
    Color([u8; 3]),
    Kind(Kind),
    Align(ParaAlign),
    /// Turn the list on, or off when the whole selection already has it.
    List(List),
    /// Highlighter colour, `None` to remove it.
    Highlight(Option<[u8; 3]>),
    /// Raise or lower the text; the same again puts it back on the line.
    Script(Script),
    Spacing(Spacing),
    /// Indent levels to add (negative: remove).
    Indent(i8),
    /// Move headings and list items one level down (true) or up their hierarchy.
    Nest(bool),
    /// Show or hide the line under the paragraph.
    Rule,
    ClearFormat,
    Case(Case),
}

#[derive(Clone, Copy)]
enum Case {
    Upper,
    Lower,
    /// First letter of each word in capitals.
    Title,
}

#[derive(Clone, Copy)]
enum Cmd {
    New,
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    ToggleTheme,
    Zoom(f32),
    Edit(Edit),
    /// Insert a table of (rows, columns).
    Table(u16, u16),
    DeleteTable,
    PageBreak,
    /// Move the caret to the next table cell.
    NextCell,
    InsertImage,
    /// Remove the selected picture.
    DeleteImage,
    /// Show or hide page numbers.
    PageNumbers,
    /// Show or hide the navigation pane.
    Outline,
    /// Put the caret at a character and bring it into view.
    Goto(usize),
    Cut,
    Copy,
    Paste,
    SelectAll,
    CloseTab,
    Quit,
    /// Add a table row below (true) or above the caret's.
    TableRow(bool),
    /// Add a table column right (true) or left of the caret's.
    TableCol(bool),
    /// Delete the caret's table row (true) or column.
    DeleteLine(bool),
    Landscape(bool),
    PageMargins(Margins),
    InsertText(&'static str),
    InsertDate,
    /// Pick up the look of the selection, to give it to what is selected next.
    FormatPainter,
    /// Show or hide the formatting marks.
    Marks,
    /// Insert a table of contents where the caret is.
    Contents,
    Export(Export),
    /// Open the find and replace bar.
    Find,
    CloseFind,
    /// Go to the next (true) or previous match.
    FindNext(bool),
    ReplaceOne,
    ReplaceAll,
}

/// A format a copy of the document can be written in.
#[derive(Clone, Copy)]
enum Export {
    Markdown,
    Html,
}

/// A table of contents just inserted, whose page numbers wait for the pages to be laid out
/// again (the table itself pushes the headings down).
struct Contents {
    /// (paragraph of the line, paragraph of its heading).
    lines: Vec<(usize, usize)>,
    /// Layouts left to settle on.
    tries: u8,
}

/// Tab of the ribbon: which set of tools shows under the tab row.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Ribbon {
    File,
    #[default]
    Home,
    Insert,
    Layout,
    Table,
    View,
}

impl Ribbon {
    const ALL: [(Self, &str); 6] = [
        (Self::File, "File"),
        (Self::Home, "Home"),
        (Self::Insert, "Insert"),
        (Self::Layout, "Layout"),
        (Self::Table, "Table"),
        (Self::View, "View"),
    ];
}

/// The find and replace bar.
#[derive(Default)]
struct Find {
    open: bool,
    query: String,
    with: String,
    match_case: bool,
    /// Give the query field the keyboard on the next frame.
    focus: bool,
}

struct Snapshot {
    doc: Document,
    selection: Range<usize>,
}

struct DocTab {
    id: u64,
    name: String,
    path: Option<PathBuf>,
    doc: Document,
    dirty: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Time of the last typed edit, to group typing into undo steps.
    last_edit: f64,
    /// Selected characters (empty at the caret).
    selection: Range<usize>,
    /// Format chosen at the caret for the next typed text.
    pending: Option<Format>,
    /// Number of pages, as last laid out.
    pages: usize,
    /// Page (0 for the first) each paragraph starts on, as last laid out.
    para_pages: Vec<usize>,
    /// Format painter: the look picked up, and the selection it was picked from.
    painter: Option<(Format, Range<usize>)>,
    contents: Option<Contents>,
    view: editor::View,
}

impl DocTab {
    fn new(id: u64, name: String, path: Option<PathBuf>, doc: Document) -> Self {
        Self {
            id,
            name,
            path,
            doc,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: f64::NEG_INFINITY,
            selection: 0..0,
            pending: None,
            pages: 1,
            para_pages: Vec::new(),
            painter: None,
            contents: None,
            view: editor::View::default(),
        }
    }

    fn editor_id(&self) -> Id {
        Id::new(("editor", self.id))
    }

    /// Untouched new document: opening a file may take its place.
    fn is_blank(&self) -> bool {
        self.path.is_none() && !self.dirty && self.doc.is_empty()
    }

    /// Format of the next typed character.
    fn typing(&self) -> Format {
        self.pending
            .unwrap_or_else(|| self.doc.format_at(self.selection.start))
    }

    /// Format the toolbar shows: the start of the selection, or the typing format.
    fn current(&self) -> Format {
        match self.doc.formats().get(self.selection.start) {
            Some(format) if !self.selection.is_empty() => *format,
            _ => self.typing(),
        }
    }

    /// Remember `doc` (the state before a change) as an undo step.
    fn checkpoint(&mut self, doc: Document) {
        self.undo.push(Snapshot {
            doc,
            selection: self.selection.clone(),
        });
        self.redo.clear();
        let keep = (UNDO_BUDGET / (self.doc.len() + 1)).clamp(10, 200);
        if self.undo.len() > keep {
            self.undo.drain(..self.undo.len() - keep);
        }
    }

    fn apply(&mut self, edit: Edit) {
        let chars = self.selection.clone();
        let change: Box<dyn Fn(&mut Format)> = match edit {
            Edit::Kind(kind) => {
                self.checkpoint(self.doc.clone());
                self.doc.set_kind(self.doc.paras_in(chars), kind);
                return self.touched();
            }
            Edit::Align(align) => {
                self.checkpoint(self.doc.clone());
                self.doc.set_align(self.doc.paras_in(chars), align);
                return self.touched();
            }
            Edit::List(list) => {
                self.checkpoint(self.doc.clone());
                let paras = self.doc.paras_in(chars);
                let all = self.doc.paras()[paras.clone()]
                    .iter()
                    .all(|para| para.list == list);
                self.doc
                    .set_list(paras, if all { List::None } else { list });
                return self.touched();
            }
            Edit::Spacing(spacing) => return self.edit_paras(|para| para.spacing = spacing),
            Edit::Indent(by) => {
                return self.edit_paras(|para| {
                    para.indent = para.indent.saturating_add_signed(by).min(MAX_INDENT);
                });
            }
            Edit::Nest(deeper) => {
                let before = self.doc.clone();
                if self.doc.nest(self.doc.paras_in(chars), deeper) {
                    self.checkpoint(before);
                    self.touched();
                }
                return;
            }
            Edit::Rule => {
                let on = !self.para().rule;
                return self.edit_paras(|para| para.rule = on);
            }
            Edit::ClearFormat | Edit::Case(_) if chars.is_empty() => return,
            Edit::ClearFormat => {
                self.checkpoint(self.doc.clone());
                self.doc.clear_format(chars);
                return self.touched();
            }
            Edit::Case(case) => {
                self.checkpoint(self.doc.clone());
                let upper = |ch: char| ch.to_uppercase().next().unwrap_or(ch);
                let lower = |ch: char| ch.to_lowercase().next().unwrap_or(ch);
                // For title case: the next letter starts a word.
                let starts_word = std::cell::Cell::new(true);
                self.doc.map_chars(chars, |ch| match case {
                    Case::Upper => upper(ch),
                    Case::Lower => lower(ch),
                    Case::Title => {
                        let first = starts_word.replace(!ch.is_alphanumeric());
                        if first { upper(ch) } else { lower(ch) }
                    }
                });
                return self.touched();
            }
            Edit::Highlight(color) => Box::new(move |format| format.highlight = color),
            Edit::Script(script) => {
                let script = if self.current().script == script {
                    Script::Normal
                } else {
                    script
                };
                Box::new(move |format| format.script = script)
            }
            Edit::Toggle(flag) => {
                let on = !*flag.of(&mut self.current());
                Box::new(move |format| *flag.of(format) = on)
            }
            Edit::Font(font) => Box::new(move |format| format.font = font),
            Edit::Size(size) => Box::new(move |format| format.size = size),
            Edit::Color(color) => Box::new(move |format| format.color = color),
        };
        if chars.is_empty() {
            // Nothing selected: the change applies to what is typed next.
            let mut format = self.typing();
            change(&mut format);
            self.pending = Some(format);
        } else {
            self.checkpoint(self.doc.clone());
            self.doc.restyle(chars, change);
            self.touched();
        }
    }

    /// Change the paragraphs of the selection.
    fn edit_paras(&mut self, change: impl Fn(&mut goddoc_core::Para)) {
        self.checkpoint(self.doc.clone());
        let paras = self.doc.paras_in(self.selection.clone());
        self.doc.edit_paras(paras, change);
        self.touched();
    }

    /// Type `text` over the selection.
    fn insert_text(&mut self, ctx: &Context, text: &str) {
        let range = self.selection.clone();
        let typing = self.typing();
        let caret = range.start + text.chars().count();
        self.restructure(|doc| {
            let mut edited = doc.clone();
            edited.replace(range, text, typing);
            // Not over a selection that would break a table apart.
            if !edited.tables_intact(doc) {
                return None;
            }
            *doc = edited;
            Some(caret)
        });
        set_cursor(ctx, self.editor_id(), &self.selection);
    }

    /// Insert a table of contents in front of the caret's paragraph: a line per heading,
    /// with its number and its page. False when there is no heading, or in a table.
    fn insert_contents(&mut self) -> bool {
        let doc = &self.doc;
        let markers = doc.markers();
        let para = doc.para_at(self.selection.start);
        let here = doc.paras()[para];
        // (paragraph of the heading, its level, what its line reads).
        let mut entries = Vec::new();
        for (index, (text, heading)) in doc.text().split('\n').zip(doc.paras()).enumerate() {
            let level = heading
                .kind
                .heading_level()
                .filter(|_| !text.trim().is_empty());
            let Some(level) = level else {
                continue;
            };
            let page = self.para_pages.get(index).map_or(1, |page| page + 1);
            let number = markers[index].as_ref().map(|number| format!("{number} "));
            let line = format!("{}{text}\t{page}", number.unwrap_or_default());
            entries.push((index, level as u8, line));
        }
        if entries.is_empty() || here.cell.is_some() {
            return false;
        }
        let at = doc.para_chars(para).start;
        let lines: Vec<&str> = entries.iter().map(|entry| entry.2.as_str()).collect();
        let title = "Contents";
        let text = format!("{title}\n{}\n", lines.join("\n"));
        let count = entries.len() + 1;
        let done = self.restructure(|doc| {
            doc.replace(at..at, &text, Kind::Normal.format());
            // The lines are plain text; the paragraph they push down stays as it was.
            doc.edit_paras(para..para + count, |line| *line = Default::default());
            doc.edit_paras(para + count..para + count + 1, |pushed| *pushed = here);
            for (offset, entry) in entries.iter().enumerate() {
                let line = para + 1 + offset;
                doc.edit_paras(line..line + 1, |line| line.indent = entry.1);
            }
            doc.restyle(at..at + title.chars().count(), |format| {
                format.bold = true;
                format.size = 16.0;
            });
            Some(at)
        });
        // Headings from the caret on are `count` paragraphs further.
        let moved = |heading: usize| heading + if heading >= para { count } else { 0 };
        let lines = entries.iter().enumerate();
        self.contents = Some(Contents {
            lines: lines
                .map(|(offset, entry)| (para + 1 + offset, moved(entry.0)))
                .collect(),
            tries: 4,
        });
        done
    }

    /// Give the lines of the table of contents just inserted the pages of the new layout.
    fn settle_contents(&mut self) {
        let Some(mut contents) = self.contents.take() else {
            return;
        };
        if self.para_pages.len() != self.doc.paras().len() || contents.tries == 0 {
            return;
        }
        let mut changed = false;
        // From the last line up: a number that changes length moves what follows it.
        for (line, heading) in contents.lines.iter().rev() {
            let chars = self.doc.para_chars(*line);
            let text: String = self
                .doc
                .text()
                .chars()
                .skip(chars.start)
                .take(chars.len())
                .collect();
            let (Some(tab), Some(page)) = (text.rfind('\t'), self.para_pages.get(*heading)) else {
                return;
            };
            let page = (page + 1).to_string();
            if text[tab + 1..] != page {
                let from = chars.start + text[..=tab].chars().count();
                let format = self.doc.format_at(from);
                self.doc.replace(from..chars.end, &page, format);
                changed = true;
            }
        }
        if changed {
            contents.tries -= 1;
            self.contents = Some(contents);
        }
    }

    /// Mark the match after (or before) the current one, wrapping around.
    fn find_next(&mut self, find: &Find, forward: bool) {
        let found = self.doc.find(&find.query, find.match_case);
        let from = self.view.found.clone().unwrap_or(self.selection.clone());
        let next = if forward {
            let after = found.iter().find(|range| range.start >= from.end);
            after.or(found.first())
        } else {
            let before = found.iter().rev().find(|range| range.end <= from.start);
            before.or(found.last())
        };
        self.view.found = next.cloned();
        if let Some(range) = next {
            self.view.scroll_to = Some(range.start);
        }
    }

    /// Paragraph the caret is in.
    fn para(&self) -> goddoc_core::Para {
        self.doc.paras()[self.doc.para_at(self.selection.start)]
    }

    /// Run a change to the document's structure that says where the caret goes, or `None`
    /// when it does not apply here. True when it was done.
    fn restructure(&mut self, change: impl FnOnce(&mut Document) -> Option<usize>) -> bool {
        let before = self.doc.clone();
        let Some(caret) = change(&mut self.doc) else {
            return false;
        };
        self.checkpoint(before);
        self.selection = caret..caret;
        self.touched();
        true
    }

    /// Start of the cell after the caret's, or of the paragraph after the table.
    fn next_cell(&self) -> Option<usize> {
        let current = self.doc.para_at(self.selection.start);
        let cell = self.doc.paras()[current].cell?;
        let table = self.doc.table_paras(cell.table);
        let next = (current + 1..table.end)
            .find(|para| self.doc.paras()[*para].cell != Some(cell))
            .unwrap_or(table.end);
        (next < self.doc.paras().len()).then(|| self.doc.para_chars(next).start)
    }

    /// After a formatting change: unsaved, and the next typing starts a new undo step.
    fn touched(&mut self) {
        self.dirty = true;
        self.pending = None;
        self.last_edit = f64::NEG_INFINITY;
    }

    /// Swap the document with the latest step of `from`, pushing the current one onto `to`.
    fn step(&mut self, undo: bool) -> bool {
        let (from, to) = if undo {
            (&mut self.undo, &mut self.redo)
        } else {
            (&mut self.redo, &mut self.undo)
        };
        let Some(snapshot) = from.pop() else {
            return false;
        };
        to.push(Snapshot {
            doc: std::mem::replace(&mut self.doc, snapshot.doc),
            selection: std::mem::replace(&mut self.selection, snapshot.selection),
        });
        self.touched();
        true
    }
}

/// What a pending "unsaved changes" question is about.
#[derive(Clone, Copy)]
enum Closing {
    Tab(usize),
    Window,
}

pub struct GodApp {
    updates: crate::updates::Updates,
    tabs: Vec<DocTab>,
    active: usize,
    next_id: u64,
    dark: bool,
    zoom: f32,
    logo: Option<TextureHandle>,
    toasts: Toasts,
    closing: Option<Closing>,
    /// The user agreed to close the window with unsaved documents.
    quit: bool,
    title: String,
    /// The navigation pane (headings) is shown.
    show_outline: bool,
    /// Spaces, tabs and ends of paragraphs are shown.
    show_marks: bool,
    /// The home screen is shown instead of the active document.
    home: bool,
    /// Documents opened or saved lately, latest first.
    recent: Vec<PathBuf>,
    find: Find,
    ribbon: Ribbon,
    /// Decoded pictures, by the address of their image's bytes; `None` when undecodable.
    textures: HashMap<usize, Option<TextureHandle>>,
}

impl GodApp {
    /// `files` are opened at startup (command line / "Open with").
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        let get = |key: &str| cc.storage.and_then(|storage| storage.get_string(key));
        let dark = get("dark_mode").is_some_and(|value| value == "1");
        let zoom = get("zoom")
            .and_then(|value| value.parse::<f32>().ok())
            .map_or(1.25, |zoom| {
                zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end())
            });
        theme::install(&cc.egui_ctx, dark);
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        layout::install_fonts(&mut fonts);
        crate::updates::add_bold_font(&mut fonts);
        cc.egui_ctx.set_fonts(fonts);
        let mut app = Self {
            updates: crate::updates::Updates::default(),
            tabs: Vec::new(),
            active: 0,
            next_id: 0,
            dark,
            zoom,
            logo: branding::logo_texture(&cc.egui_ctx),
            toasts: Toasts::default(),
            closing: None,
            quit: false,
            title: String::new(),
            show_outline: get("outline").is_some_and(|value| value == "1"),
            show_marks: get("marks").is_some_and(|value| value == "1"),
            home: files.is_empty(),
            find: Find::default(),
            ribbon: Ribbon::default(),
            recent: get("recent")
                .map(|list| {
                    list.lines()
                        .filter(|line| !line.is_empty())
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or_default(),
            textures: HashMap::new(),
        };
        app.updates.start(&cc.egui_ctx);
        app.new_tab();
        for path in files {
            app.open_path(&path);
        }
        app
    }

    fn new_tab(&mut self) {
        self.push_tab("Untitled".to_owned(), None, Document::default());
    }

    fn push_tab(&mut self, name: String, path: Option<PathBuf>, doc: Document) {
        self.next_id += 1;
        let tab = DocTab::new(self.next_id, name, path, doc);
        // A file opened from the untouched start document replaces it.
        if self.tabs.get(self.active).is_some_and(DocTab::is_blank) {
            self.tabs[self.active] = tab;
        } else {
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
        }
    }

    /// Put `path` first among the recent documents.
    fn remember(&mut self, path: &Path) {
        self.recent.retain(|known| known != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(MAX_RECENT);
    }

    fn open_path(&mut self, path: &Path) {
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.path.as_deref() == Some(path))
        {
            self.active = index;
            self.home = false;
            return;
        }
        let loaded = std::fs::read(path)
            .map_err(|err| err.to_string())
            .and_then(|bytes| {
                if is_text(path) {
                    let mut doc = Document::default();
                    let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
                    doc.replace(0..0, &text, Format::default());
                    Ok(doc)
                } else {
                    docx::read(&bytes).map_err(|err| err.to_string())
                }
            });
        match loaded {
            Ok(doc) => {
                self.push_tab(file_name(path), Some(path.to_path_buf()), doc);
                self.remember(path);
                self.home = false;
            }
            Err(err) => {
                // A file that is gone or unreadable leaves the recent list.
                self.recent.retain(|known| known != path);
                self.toasts
                    .error(format!("Cannot open {}: {err}", path.display()));
            }
        }
    }

    /// Save tab `index`; asks for a place when it has none or `save_as`. True when written.
    fn save_tab(&mut self, index: usize, save_as: bool) -> bool {
        let tab = &self.tabs[index];
        let known = tab.path.clone().filter(|_| !save_as);
        let Some(path) = known.or_else(|| platform::pick_save(&tab.name)) else {
            return false;
        };
        let bytes = if is_text(&path) {
            Ok(tab.doc.text().as_bytes().to_vec())
        } else {
            docx::write(&tab.doc).map_err(|err| err.to_string())
        };
        match bytes.and_then(|bytes| platform::write_atomic(&path, &bytes)) {
            Ok(()) => {
                let tab = &mut self.tabs[index];
                tab.name = file_name(&path);
                tab.path = Some(path.clone());
                tab.dirty = false;
                self.toasts.success(format!("Saved {}", tab.name));
                self.remember(&path);
                true
            }
            Err(err) => {
                self.toasts.error(err);
                false
            }
        }
    }

    /// Put the image file `path` in the document, where the caret is.
    fn insert_image(&mut self, ctx: &Context, path: &Path) {
        let kind = path
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(ImageKind::of_extension);
        let loaded = kind
            .ok_or_else(|| "only PNG and JPEG images are supported".to_owned())
            .and_then(|kind| {
                let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
                let decoded = image::load_from_memory(&bytes).map_err(|err| err.to_string())?;
                Ok((kind, bytes, decoded.width(), decoded.height()))
            });
        let (kind, bytes, width, height) = match loaded {
            Ok(loaded) => loaded,
            Err(err) => {
                let name = file_name(path);
                self.toasts.error(format!("Cannot insert {name}: {err}"));
                return;
            }
        };
        // Pixels at 96 per inch, shrunk to fit between the margins.
        let points = vec2(width as f32, height as f32) * 0.75;
        let fit = layout::content_size(self.tabs[self.active].doc.page, 1.0).x;
        let size = points * (fit / points.x.max(1.0)).min(1.0);
        let image = Image {
            bytes: bytes.into(),
            kind,
        };
        let tab = &mut self.tabs[self.active];
        let at = tab.selection.start;
        let (width, height) = (
            size.x.round().max(1.0) as u16,
            size.y.round().max(1.0) as u16,
        );
        tab.restructure(|doc| Some(doc.insert_image(at, image, width, height)));
        tab.view.selected_picture = Some(tab.doc.para_at(tab.selection.start));
        tab.view.scroll_to = Some(tab.selection.start);
        set_cursor(ctx, tab.editor_id(), &tab.selection);
    }

    fn close_tab(&mut self, index: usize) {
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.new_tab();
        }
        if self.active > index || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
    }

    fn run(&mut self, ctx: &Context, cmd: Cmd) {
        let tab = &mut self.tabs[self.active];
        let editor_id = tab.editor_id();
        match cmd {
            Cmd::New => {
                // From the home screen, the untouched start document will do.
                let reuse = self.home && tab.is_blank();
                self.home = false;
                if reuse {
                    return;
                }
                self.next_id += 1;
                let tab = DocTab::new(
                    self.next_id,
                    "Untitled".to_owned(),
                    None,
                    Document::default(),
                );
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
                return;
            }
            Cmd::Open => {
                for path in platform::pick_documents() {
                    self.open_path(&path);
                }
                return;
            }
            Cmd::Save | Cmd::SaveAs => {
                self.save_tab(self.active, matches!(cmd, Cmd::SaveAs));
            }
            Cmd::Undo | Cmd::Redo => {
                if tab.step(matches!(cmd, Cmd::Undo)) {
                    set_cursor(ctx, editor_id, &tab.selection);
                }
            }
            Cmd::ToggleTheme => {
                self.dark = !self.dark;
                theme::set_dark(ctx, self.dark);
            }
            Cmd::Zoom(zoom) => self.zoom = zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end()),
            Cmd::Edit(edit) => tab.apply(edit),
            Cmd::Table(rows, cols) => {
                let at = tab.selection.start;
                if !tab.restructure(|doc| doc.insert_table(at, rows, cols)) {
                    self.toasts.error("A table cannot go inside another table");
                }
                set_cursor(ctx, editor_id, &tab.selection);
            }
            Cmd::DeleteTable => {
                if let Some(cell) = tab.para().cell {
                    tab.restructure(|doc| Some(doc.delete_table(cell.table)));
                    set_cursor(ctx, editor_id, &tab.selection);
                }
            }
            Cmd::PageBreak => {
                let at = tab.selection.start;
                if !tab.restructure(|doc| doc.insert_page_break(at)) {
                    self.toasts.error("A page break cannot go inside a table");
                }
                set_cursor(ctx, editor_id, &tab.selection);
            }
            Cmd::InsertImage => {
                for path in platform::pick_images() {
                    self.insert_image(ctx, &path);
                }
            }
            Cmd::DeleteImage => {
                if let Some(para) = tab.view.selected_picture.take() {
                    tab.restructure(|doc| {
                        doc.set_picture(para, None);
                        Some(doc.para_chars(para).start)
                    });
                    set_cursor(ctx, editor_id, &tab.selection);
                }
            }
            Cmd::PageNumbers => {
                tab.checkpoint(tab.doc.clone());
                tab.doc.page_numbers = !tab.doc.page_numbers;
                tab.touched();
            }
            Cmd::Outline => self.show_outline = !self.show_outline,
            Cmd::Goto(index) => {
                tab.selection = index..index;
                tab.view.scroll_to = Some(index);
                set_cursor(ctx, editor_id, &tab.selection);
            }
            Cmd::Cut => ctx.send_viewport_cmd(ViewportCommand::RequestCut),
            Cmd::Copy => ctx.send_viewport_cmd(ViewportCommand::RequestCopy),
            Cmd::Paste => ctx.send_viewport_cmd(ViewportCommand::RequestPaste),
            Cmd::SelectAll => {
                tab.selection = 0..tab.doc.len();
                set_cursor(ctx, editor_id, &tab.selection);
            }
            Cmd::CloseTab => {
                let index = self.active;
                if self.tabs[index].dirty {
                    self.closing = Some(Closing::Tab(index));
                } else {
                    self.close_tab(index);
                }
                return;
            }
            Cmd::Quit => {
                ctx.send_viewport_cmd(ViewportCommand::Close);
                return;
            }
            Cmd::TableRow(below) => {
                if let Some(cell) = tab.para().cell {
                    tab.restructure(|doc| Some(doc.insert_table_row(cell, below)));
                    set_cursor(ctx, editor_id, &tab.selection);
                }
            }
            Cmd::TableCol(right) => {
                if let Some(cell) = tab.para().cell {
                    tab.restructure(|doc| Some(doc.insert_table_col(cell, right)));
                    set_cursor(ctx, editor_id, &tab.selection);
                }
            }
            Cmd::DeleteLine(row) => {
                if let Some(cell) = tab.para().cell {
                    tab.restructure(|doc| Some(doc.delete_table_line(cell, row)));
                    set_cursor(ctx, editor_id, &tab.selection);
                }
            }
            Cmd::Landscape(on) => {
                tab.checkpoint(tab.doc.clone());
                tab.doc.page.landscape = on;
                tab.touched();
            }
            Cmd::PageMargins(margins) => {
                tab.checkpoint(tab.doc.clone());
                tab.doc.page.margins = margins;
                tab.touched();
            }
            Cmd::InsertText(text) => tab.insert_text(ctx, text),
            Cmd::InsertDate => tab.insert_text(ctx, &today()),
            Cmd::FormatPainter => {
                tab.painter = match tab.painter.take() {
                    Some(_) => None,
                    None => Some((tab.current(), tab.selection.clone())),
                };
            }
            Cmd::Marks => self.show_marks = !self.show_marks,
            Cmd::Contents => {
                if !tab.insert_contents() {
                    let why = "Give the document headings first, and put the caret outside tables";
                    self.toasts.error(why);
                }
                set_cursor(ctx, editor_id, &tab.selection);
            }
            Cmd::Export(export) => {
                let (label, extension) = match export {
                    Export::Markdown => ("Markdown", "md"),
                    Export::Html => ("Web page", "html"),
                };
                if let Some(path) = platform::pick_export(&tab.name, label, extension) {
                    let text = match export {
                        Export::Markdown => goddoc_core::export::markdown(&tab.doc),
                        Export::Html => goddoc_core::export::html(&tab.doc, &tab.name),
                    };
                    match platform::write_atomic(&path, text.as_bytes()) {
                        Ok(()) => self
                            .toasts
                            .success(format!("Exported {}", file_name(&path))),
                        Err(err) => self.toasts.error(err),
                    }
                }
            }
            // The find bar keeps the keyboard: these do not hand it back to the page.
            Cmd::Find => {
                self.find.open = true;
                self.find.focus = true;
                return;
            }
            Cmd::CloseFind => {
                self.find.open = false;
                tab.view.found = None;
            }
            Cmd::FindNext(forward) => {
                tab.find_next(&self.find, forward);
                return;
            }
            Cmd::ReplaceOne => {
                let found = tab.doc.find(&self.find.query, self.find.match_case);
                let current = tab.view.found.take().filter(|range| found.contains(range));
                if let Some(range) = current {
                    let format = tab.doc.formats()[range.start];
                    let with = self.find.with.clone();
                    let caret = range.start + with.chars().count();
                    tab.restructure(|doc| {
                        doc.replace(range, &with, format);
                        Some(caret)
                    });
                    set_cursor(ctx, editor_id, &tab.selection);
                }
                tab.find_next(&self.find, true);
                return;
            }
            Cmd::ReplaceAll => {
                let before = tab.doc.clone();
                let find = &self.find;
                let count = tab
                    .doc
                    .replace_all(&find.query, &find.with, find.match_case);
                if count > 0 {
                    tab.checkpoint(before);
                    tab.selection = 0..0;
                    tab.touched();
                    set_cursor(ctx, editor_id, &tab.selection);
                }
                tab.view.found = None;
                self.toasts.success(format!("Replaced {count}"));
                return;
            }
            Cmd::NextCell => {
                if let Some(caret) = tab.next_cell() {
                    tab.selection = caret..caret;
                    set_cursor(ctx, editor_id, &tab.selection);
                }
            }
        }
        // Buttons take the keyboard focus: give it back to the page.
        ctx.memory_mut(|memory| memory.request_focus(editor_id));
    }

    fn shortcuts(&mut self, ctx: &Context, clicked: &mut Vec<Cmd>) {
        let ctrl = Modifiers::COMMAND;
        let shift = Modifiers::COMMAND | Modifiers::SHIFT;
        // More specific (shifted) shortcuts first.
        let table = [
            (shift, Key::S, Cmd::SaveAs),
            (shift, Key::Z, Cmd::Redo),
            (ctrl, Key::N, Cmd::New),
            (ctrl, Key::O, Cmd::Open),
            (ctrl, Key::W, Cmd::CloseTab),
            (ctrl, Key::F, Cmd::Find),
            (ctrl, Key::H, Cmd::Find),
            (ctrl, Key::L, Cmd::Edit(Edit::Align(ParaAlign::Left))),
            (ctrl, Key::E, Cmd::Edit(Edit::Align(ParaAlign::Center))),
            (ctrl, Key::R, Cmd::Edit(Edit::Align(ParaAlign::Right))),
            (ctrl, Key::J, Cmd::Edit(Edit::Align(ParaAlign::Justify))),
            (ctrl, Key::S, Cmd::Save),
            (ctrl, Key::Z, Cmd::Undo),
            (ctrl, Key::Y, Cmd::Redo),
            (ctrl, Key::B, Cmd::Edit(Edit::Toggle(Flag::Bold))),
            (ctrl, Key::I, Cmd::Edit(Edit::Toggle(Flag::Italic))),
            (ctrl, Key::U, Cmd::Edit(Edit::Toggle(Flag::Underline))),
            (ctrl, Key::Enter, Cmd::PageBreak),
        ];
        ctx.input_mut(|input| {
            for (modifiers, key, cmd) in table {
                if input.consume_shortcut(&KeyboardShortcut::new(modifiers, key)) {
                    clicked.push(cmd);
                }
            }
        });
        // Tab walks through table cells instead of typing a tab.
        let tab = &self.tabs[self.active];
        // Delete removes the selected picture, not the text next to the caret.
        if tab.view.selected_picture.is_some()
            && ctx.input_mut(|input| {
                input.consume_key(Modifiers::NONE, Key::Delete)
                    || input.consume_key(Modifiers::NONE, Key::Backspace)
            })
        {
            clicked.push(Cmd::DeleteImage);
        }
        let para = tab.para();
        let focused = ctx.memory(|memory| memory.has_focus(tab.editor_id()));
        // In a heading or a list, Tab goes a level down and Shift+Tab a level up.
        let nests = para.kind.heading_level().is_some() || para.list != List::None;
        if para.cell.is_some() {
            if focused && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab)) {
                clicked.push(Cmd::NextCell);
            }
        } else if focused && nests {
            // Shift first: a bare Tab would answer for it too.
            let key = |modifiers| ctx.input_mut(|input| input.consume_key(modifiers, Key::Tab));
            if key(Modifiers::SHIFT) {
                clicked.push(Cmd::Edit(Edit::Nest(false)));
            } else if key(Modifiers::NONE) {
                clicked.push(Cmd::Edit(Edit::Nest(true)));
            }
        }
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        for path in dropped.into_iter().filter_map(|file| file.path) {
            let extension = path.extension().and_then(|extension| extension.to_str());
            if extension.and_then(ImageKind::of_extension).is_some() {
                self.insert_image(ctx, &path);
            } else {
                self.open_path(&path);
            }
        }
    }

    /// Hold the window open while documents are unsaved, until the user decides.
    fn guard_close(&mut self, ctx: &Context) {
        let requested = ctx.input(|input| input.viewport().close_requested());
        if requested && !self.quit && self.tabs.iter().any(|tab| tab.dirty) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.closing = Some(Closing::Window);
        } else if requested {
            self.updates.relaunch();
        }
    }

    fn update_title(&mut self, ctx: &Context) {
        let tab = &self.tabs[self.active];
        let star = if tab.dirty { "*" } else { "" };
        let title = if self.home {
            APP_NAME.to_owned()
        } else {
            format!("{}{star} - {APP_NAME}", tab.name)
        };
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    // ------------------------------------------------------------ tab row

    fn tab_row(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let palette = Palette::of(ui);
        let mut select = None;
        let mut close = None;
        let row = vec2(ui.available_width(), TAB_HEIGHT);
        ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(2.0);
            let home = tab_shape(ui, &palette, self.home, vec2(40.0, TAB_HEIGHT));
            ui.painter().text(
                home.rect.center() + vec2(0.0, 2.0),
                Align2::CENTER_CENTER,
                icon::HOUSE,
                FontId::proportional(17.0),
                palette.text,
            );
            if home.on_hover_text("Home").clicked() {
                self.home = true;
            }
            // Share what is left between the tabs; names shrink before anything overflows.
            let reserved = widgets::ICON_BUTTON.x + 140.0;
            let chrome = 12.0 + 16.0 + 8.0 + 8.0 + widgets::SMALL_ICON_BUTTON.x + 22.0;
            let per_tab = (ui.available_width() - reserved) / self.tabs.len() as f32;
            let text_max = (per_tab - chrome).clamp(40.0, TAB_TEXT_MAX);
            for (index, tab) in self.tabs.iter().enumerate() {
                let active = !self.home && index == self.active;
                let (select_it, close_it) =
                    doc_tab(ui, &palette, &tab.name, tab.dirty, active, text_max);
                if select_it {
                    select = Some(index);
                }
                if close_it {
                    close = Some(index);
                }
            }
            ui.add_space(6.0);
            let new = Button::new(RichText::new(with_icon(icon::PLUS, "New")).size(14.0))
                .frame_when_inactive(false)
                .stroke(egui::Stroke::NONE);
            if ui.add(new).on_hover_text("New document (Ctrl+N)").clicked() {
                clicked.push(Cmd::New);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (glyph, tip) = if self.dark {
                    (icon::SUN, "Light mode")
                } else {
                    (icon::MOON, "Dark mode")
                };
                if widgets::icon_button(ui, glyph, tip).clicked() {
                    clicked.push(Cmd::ToggleTheme);
                }
                self.updates.button(ui);
            });
        });
        if let Some(index) = select {
            self.active = index;
            self.home = false;
        }
        if let Some(index) = close {
            if self.tabs[index].dirty {
                self.closing = Some(Closing::Tab(index));
            } else {
                self.close_tab(index);
            }
        }
    }

    // ------------------------------------------------------------ formatting bar

    fn format_bar(&self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let palette = Palette::of(ui);
        let tab = &self.tabs[self.active];
        let format = tab.current();
        let para = tab.doc.paras()[tab.doc.para_at(tab.selection.start)];
        // In a bar that scrolls sideways the width on offer has no end: the tools then
        // take what they need.
        let wide = Some(ui.available_width()).filter(|wide| wide.is_finite());
        let row = vec2(wide.unwrap_or(0.0), BAR_HEIGHT);
        ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(6.0);
            let quick = [
                (icon::CLIPBOARD, "Paste (Ctrl+V)", Cmd::Paste),
                (icon::SCISSORS, "Cut (Ctrl+X)", Cmd::Cut),
                (icon::COPY, "Copy (Ctrl+C)", Cmd::Copy),
                (
                    icon::MAGNIFYING_GLASS,
                    "Find and replace (Ctrl+F)",
                    Cmd::Find,
                ),
            ];
            for (glyph, tip, cmd) in quick {
                if widgets::icon_button(ui, glyph, tip).clicked() {
                    clicked.push(cmd);
                }
            }
            divider(ui, &palette);
            let undo = ui.add_enabled_ui(!tab.undo.is_empty(), |ui| {
                widgets::icon_button(ui, icon::ARROW_U_UP_LEFT, "Undo (Ctrl+Z)")
            });
            if undo.inner.clicked() {
                clicked.push(Cmd::Undo);
            }
            let redo = ui.add_enabled_ui(!tab.redo.is_empty(), |ui| {
                widgets::icon_button(ui, icon::ARROW_U_UP_RIGHT, "Redo (Ctrl+Y)")
            });
            if redo.inner.clicked() {
                clicked.push(Cmd::Redo);
            }
            divider(ui, &palette);

            ComboBox::from_id_salt("kind")
                .width(104.0)
                .selected_text(para.kind.name())
                .show_ui(ui, |ui| {
                    for kind in Kind::ALL {
                        let look = kind.format();
                        let label = RichText::new(kind.name()).size(look.size.min(18.0));
                        if ui.selectable_label(kind == para.kind, label).clicked() {
                            clicked.push(Cmd::Edit(Edit::Kind(kind)));
                        }
                    }
                })
                .response
                .on_hover_text("Paragraph style");
            ui.add_space(4.0);
            ComboBox::from_id_salt("font")
                .width(138.0)
                .selected_text(format.font.family())
                .show_ui(ui, |ui| {
                    for font in Font::ALL {
                        let chosen = ui.selectable_label(font == format.font, font.family());
                        if chosen.clicked() {
                            clicked.push(Cmd::Edit(Edit::Font(font)));
                        }
                    }
                })
                .response
                .on_hover_text("Font");
            ui.add_space(4.0);
            let resize = |by: f32| {
                let size = (format.size + by).clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end());
                Cmd::Edit(Edit::Size(size))
            };
            if widgets::small_icon_button(ui, icon::MINUS, "Smaller text").clicked() {
                clicked.push(resize(-1.0));
            }
            let size = RichText::new(format!("{}", format.size)).size(14.0);
            ui.add_sized(vec2(34.0, 22.0), egui::Label::new(size).selectable(false))
                .on_hover_text("Font size (points)");
            if widgets::small_icon_button(ui, icon::PLUS, "Larger text").clicked() {
                clicked.push(resize(1.0));
            }
            divider(ui, &palette);

            let toggles = [
                (icon::TEXT_B, "Bold (Ctrl+B)", Flag::Bold, format.bold),
                (
                    icon::TEXT_ITALIC,
                    "Italic (Ctrl+I)",
                    Flag::Italic,
                    format.italic,
                ),
                (
                    icon::TEXT_UNDERLINE,
                    "Underline (Ctrl+U)",
                    Flag::Underline,
                    format.underline,
                ),
                (
                    icon::TEXT_STRIKETHROUGH,
                    "Strikethrough",
                    Flag::Strike,
                    format.strike,
                ),
            ];
            for (glyph, tip, flag, on) in toggles {
                if widgets::toggle_button(ui, glyph, tip, on).clicked() {
                    clicked.push(Cmd::Edit(Edit::Toggle(flag)));
                }
            }
            divider(ui, &palette);
            for (color, name) in SWATCHES {
                if swatch(ui, color, format.color == color, &palette)
                    .on_hover_text(name)
                    .clicked()
                {
                    clicked.push(Cmd::Edit(Edit::Color(color)));
                }
            }
            divider(ui, &palette);
            let aligns = [
                (icon::TEXT_ALIGN_LEFT, "Align left", ParaAlign::Left),
                (icon::TEXT_ALIGN_CENTER, "Center", ParaAlign::Center),
                (icon::TEXT_ALIGN_RIGHT, "Align right", ParaAlign::Right),
                (icon::TEXT_ALIGN_JUSTIFY, "Justify", ParaAlign::Justify),
            ];
            for (glyph, tip, align) in aligns {
                if widgets::toggle_button(ui, glyph, tip, para.align == align).clicked() {
                    clicked.push(Cmd::Edit(Edit::Align(align)));
                }
            }
            divider(ui, &palette);
            let lists = [
                (icon::LIST_BULLETS, "Bullet list", List::Bullet),
                (icon::LIST_NUMBERS, "Numbered list", List::Number),
                (icon::LIST_DASHES, "Dash list", List::Dash),
            ];
            for (glyph, tip, list) in lists {
                if widgets::toggle_button(ui, glyph, tip, para.list == list).clicked() {
                    clicked.push(Cmd::Edit(Edit::List(list)));
                }
            }
            divider(ui, &palette);
            let marker = RichText::new(icon::HIGHLIGHTER).size(16.0);
            ui.label(marker.color(palette.text_dim))
                .on_hover_text("Highlight colour");
            highlight_swatches(ui, format.highlight, &palette, clicked);
            divider(ui, &palette);
            let scripts = [
                (icon::TEXT_SUPERSCRIPT, "Superscript", Script::Super),
                (icon::TEXT_SUBSCRIPT, "Subscript", Script::Sub),
            ];
            for (glyph, tip, script) in scripts {
                if widgets::toggle_button(ui, glyph, tip, format.script == script).clicked() {
                    clicked.push(Cmd::Edit(Edit::Script(script)));
                }
            }
            let case = widgets::icon_button(ui, icon::TEXT_AA, "Change case");
            Popup::menu(&case).show(|ui| {
                let cases = [
                    ("UPPERCASE", Case::Upper),
                    ("lowercase", Case::Lower),
                    ("Title Case", Case::Title),
                ];
                for (label, case) in cases {
                    if ui.button(label).clicked() {
                        clicked.push(Cmd::Edit(Edit::Case(case)));
                    }
                }
            });
            let painting = self.tabs[self.active].painter.is_some();
            let tip = "Format painter: click, then select the text that takes this look";
            if widgets::toggle_button(ui, icon::PAINT_BRUSH, tip, painting).clicked() {
                clicked.push(Cmd::FormatPainter);
            }
            if widgets::icon_button(ui, icon::ERASER, "Clear formatting").clicked() {
                clicked.push(Cmd::Edit(Edit::ClearFormat));
            }
            divider(ui, &palette);
            if widgets::icon_button(ui, icon::TEXT_OUTDENT, "Decrease indent").clicked() {
                clicked.push(Cmd::Edit(Edit::Indent(-1)));
            }
            if widgets::icon_button(ui, icon::TEXT_INDENT, "Increase indent").clicked() {
                clicked.push(Cmd::Edit(Edit::Indent(1)));
            }
            divider(ui, &palette);
            if widgets::icon_button(ui, icon::IMAGE, "Insert an image").clicked() {
                clicked.push(Cmd::InsertImage);
            }
            let table = widgets::icon_button(ui, icon::TABLE, "Insert a table");
            Popup::menu(&table).show(|ui| {
                if let Some((rows, cols)) = table_picker(ui, &palette) {
                    clicked.push(Cmd::Table(rows, cols));
                }
            });
        });
    }

    fn status_bar(&self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let palette = Palette::of(ui);
        let tab = &self.tabs[self.active];
        ui.horizontal(|ui| {
            let pages = if tab.pages == 1 { "page" } else { "pages" };
            let counts = format!(
                "{} {pages} · {} words · {} characters",
                tab.pages,
                tab.doc.word_count(),
                tab.doc.len()
            );
            ui.label(RichText::new(counts).size(13.0).color(palette.text_dim));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if widgets::small_icon_button(ui, icon::PLUS, "Zoom in").clicked() {
                    clicked.push(Cmd::Zoom(self.zoom + 0.1));
                }
                let zoom = RichText::new(format!("{:.0}%", self.zoom * 100.0)).size(13.0);
                ui.add_sized(vec2(44.0, 22.0), egui::Label::new(zoom).selectable(false));
                if widgets::small_icon_button(ui, icon::MINUS, "Zoom out").clicked() {
                    clicked.push(Cmd::Zoom(self.zoom - 0.1));
                }
            });
        });
    }

    // ------------------------------------------------------------ page

    // ------------------------------------------------------------ home screen

    /// Start here: a new document, an existing one, or one worked on lately.
    fn welcome(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let palette = Palette::of(ui);
        let mut open_recent = None;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.set_max_width(560.0);
                ui.add_space(48.0);
                if let Some(logo) = &self.logo {
                    ui.add(
                        egui::Image::new(logo)
                            .fit_to_exact_size(vec2(112.0, 112.0))
                            .corner_radius(CornerRadius::same(24)),
                    );
                }
                ui.add_space(10.0);
                ui.label(RichText::new(APP_NAME).size(40.0).strong());
                let tagline = "Write, lay out, share. Local. Pure Rust.";
                ui.label(RichText::new(tagline).size(17.0).color(palette.text_dim));
                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    // Center the two primary buttons.
                    let width = 2.0 * 190.0 + ui.spacing().item_spacing.x;
                    ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                    let label = with_icon(icon::FILE_PLUS, "New document");
                    let new = Button::new(RichText::new(label).size(16.0).color(palette.bg))
                        .fill(palette.accent)
                        .corner_radius(CornerRadius::same(10))
                        .min_size(vec2(190.0, 44.0));
                    if ui.add(new).clicked() {
                        clicked.push(Cmd::New);
                    }
                    let label = with_icon(icon::FOLDER_OPEN, "Open…");
                    let open = Button::new(RichText::new(label).size(16.0))
                        .stroke(Stroke::new(1.0, palette.accent))
                        .corner_radius(CornerRadius::same(10))
                        .min_size(vec2(190.0, 44.0));
                    if ui.add(open).clicked() {
                        clicked.push(Cmd::Open);
                    }
                });
                let hint = "…or drop a document or an image anywhere on this window";
                ui.label(RichText::new(hint).color(palette.text_dim));
                ui.add_space(30.0);
                ui.label(RichText::new("RECENT").size(12.0).color(palette.text_dim));
                ui.add_space(4.0);
                if self.recent.is_empty() {
                    let none = "Documents you open or save will be listed here.";
                    ui.label(RichText::new(none).color(palette.text_dim));
                }
                for path in &self.recent {
                    let folder = path
                        .parent()
                        .map(|folder| folder.display().to_string())
                        .unwrap_or_default();
                    let name = with_icon(icon::FILE_DOC, &file_name(path));
                    let row = Button::new(RichText::new(name).size(15.0))
                        .frame_when_inactive(false)
                        .stroke(egui::Stroke::NONE)
                        .right_text(RichText::new(folder).size(12.0).color(palette.text_dim))
                        .wrap_mode(egui::TextWrapMode::Truncate)
                        .min_size(vec2(ui.available_width(), 34.0));
                    if ui
                        .add(row)
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        open_recent = Some(path.clone());
                    }
                }
                ui.add_space(30.0);
            });
        });
        if let Some(path) = open_recent {
            self.open_path(&path);
        }
    }

    // ------------------------------------------------------------ menu bar

    fn ribbon_tabs(&mut self, ui: &mut Ui) {
        let palette = Palette::of(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (ribbon, label) in Ribbon::ALL {
                if ribbon_tab(ui, &palette, label, self.ribbon == ribbon).clicked() {
                    self.ribbon = ribbon;
                }
            }
        });
    }

    /// Tools of the ribbon tab that is open.
    fn ribbon(&self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        if self.ribbon == Ribbon::Home {
            return self.format_bar(ui, clicked);
        }
        let palette = Palette::of(ui);
        let tab = &self.tabs[self.active];
        let para = tab.para();
        let page = tab.doc.page;
        let edit = |edit: Edit| Cmd::Edit(edit);
        // In a bar that scrolls sideways the width on offer has no end: the tools then
        // take what they need.
        let wide = Some(ui.available_width()).filter(|wide| wide.is_finite());
        let row = vec2(wide.unwrap_or(0.0), BAR_HEIGHT);
        ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let p = &palette;
            match self.ribbon {
                Ribbon::Home => {}
                Ribbon::File => {
                    tool(ui, p, clicked, icon::FILE_PLUS, "New", false, Cmd::New);
                    tool(ui, p, clicked, icon::FOLDER_OPEN, "Open", false, Cmd::Open);
                    tool(ui, p, clicked, icon::FLOPPY_DISK, "Save", false, Cmd::Save);
                    tool(
                        ui,
                        p,
                        clicked,
                        icon::FLOPPY_DISK,
                        "Save as",
                        false,
                        Cmd::SaveAs,
                    );
                    divider(ui, p);
                    let markdown = Cmd::Export(Export::Markdown);
                    tool(ui, p, clicked, icon::FILE_MD, "Markdown", false, markdown);
                    let page = Cmd::Export(Export::Html);
                    tool(ui, p, clicked, icon::FILE_HTML, "Web page", false, page);
                    divider(ui, p);
                    tool(ui, p, clicked, icon::X, "Close", false, Cmd::CloseTab);
                    tool(ui, p, clicked, icon::SIGN_OUT, "Quit", false, Cmd::Quit);
                }
                Ribbon::Insert => {
                    let contents = icon::LIST_MAGNIFYING_GLASS;
                    tool(ui, p, clicked, contents, "Contents", false, Cmd::Contents);
                    divider(ui, p);
                    tool(
                        ui,
                        p,
                        clicked,
                        icon::IMAGE,
                        "Image",
                        false,
                        Cmd::InsertImage,
                    );
                    let table = ribbon_button(ui, p, icon::TABLE, "Table", false);
                    Popup::menu(&table).show(|ui| {
                        if let Some((rows, cols)) = table_picker(ui, p) {
                            clicked.push(Cmd::Table(rows, cols));
                        }
                    });
                    divider(ui, p);
                    tool(
                        ui,
                        p,
                        clicked,
                        icon::FILE_DASHED,
                        "Page break",
                        false,
                        Cmd::PageBreak,
                    );
                    tool(
                        ui,
                        p,
                        clicked,
                        icon::MINUS,
                        "Line",
                        para.rule,
                        edit(Edit::Rule),
                    );
                    divider(ui, p);
                    tool(
                        ui,
                        p,
                        clicked,
                        icon::CALENDAR_BLANK,
                        "Date",
                        false,
                        Cmd::InsertDate,
                    );
                    let symbol = ribbon_button(ui, p, icon::HASH, "Symbol", false);
                    Popup::menu(&symbol).show(|ui| {
                        for (symbol, name) in SYMBOLS {
                            menu_item(ui, clicked, symbol, name, "", Cmd::InsertText(symbol));
                        }
                    });
                    divider(ui, p);
                    let numbered = tab.doc.page_numbers;
                    let numbers = icon::LIST_NUMBERS;
                    tool(
                        ui,
                        p,
                        clicked,
                        numbers,
                        "Page numbers",
                        numbered,
                        Cmd::PageNumbers,
                    );
                }
                Ribbon::Layout => {
                    let portrait = !page.landscape;
                    tool(
                        ui,
                        p,
                        clicked,
                        icon::FILE_TEXT,
                        "Portrait",
                        portrait,
                        Cmd::Landscape(false),
                    );
                    let landscape = icon::DEVICE_TABLET;
                    tool(
                        ui,
                        p,
                        clicked,
                        landscape,
                        "Landscape",
                        !portrait,
                        Cmd::Landscape(true),
                    );
                    divider(ui, p);
                    let margins = [
                        (icon::ARROWS_HORIZONTAL, "Normal margins", Margins::Normal),
                        (
                            icon::ARROWS_OUT_LINE_HORIZONTAL,
                            "Narrow margins",
                            Margins::Narrow,
                        ),
                        (
                            icon::ARROWS_IN_LINE_HORIZONTAL,
                            "Wide margins",
                            Margins::Wide,
                        ),
                    ];
                    for (glyph, label, kind) in margins {
                        let on = page.margins == kind;
                        tool(ui, p, clicked, glyph, label, on, Cmd::PageMargins(kind));
                    }
                    divider(ui, p);
                    for spacing in Spacing::ALL {
                        let on = para.spacing == spacing;
                        let cmd = edit(Edit::Spacing(spacing));
                        tool(ui, p, clicked, icon::LIST_DASHES, spacing.name(), on, cmd);
                    }
                    divider(ui, p);
                    let numbered = tab.doc.page_numbers;
                    let numbers = icon::LIST_NUMBERS;
                    tool(
                        ui,
                        p,
                        clicked,
                        numbers,
                        "Page numbers",
                        numbered,
                        Cmd::PageNumbers,
                    );
                }
                Ribbon::Table => {
                    let table = ribbon_button(ui, p, icon::TABLE, "New table", false);
                    Popup::menu(&table).show(|ui| {
                        if let Some((rows, cols)) = table_picker(ui, p) {
                            clicked.push(Cmd::Table(rows, cols));
                        }
                    });
                    divider(ui, p);
                    // The rest acts on the table the caret is in.
                    ui.add_enabled_ui(para.cell.is_some(), |ui| {
                        let tools = [
                            (icon::ARROW_LINE_UP, "Row above", Cmd::TableRow(false)),
                            (icon::ARROW_LINE_DOWN, "Row below", Cmd::TableRow(true)),
                            (icon::ARROW_LINE_LEFT, "Column left", Cmd::TableCol(false)),
                            (icon::ARROW_LINE_RIGHT, "Column right", Cmd::TableCol(true)),
                            (icon::ROWS, "Delete row", Cmd::DeleteLine(true)),
                            (icon::COLUMNS, "Delete column", Cmd::DeleteLine(false)),
                            (icon::TRASH, "Delete table", Cmd::DeleteTable),
                        ];
                        for (glyph, label, cmd) in tools {
                            tool(ui, p, clicked, glyph, label, false, cmd);
                        }
                    });
                }
                Ribbon::View => {
                    let pane = icon::SIDEBAR_SIMPLE;
                    tool(
                        ui,
                        p,
                        clicked,
                        pane,
                        "Navigation",
                        self.show_outline,
                        Cmd::Outline,
                    );
                    let marks = self.show_marks;
                    tool(ui, p, clicked, icon::PARAGRAPH, "Marks", marks, Cmd::Marks);
                    divider(ui, p);
                    let zoom_out = icon::MAGNIFYING_GLASS_MINUS;
                    tool(
                        ui,
                        p,
                        clicked,
                        zoom_out,
                        "Zoom out",
                        false,
                        Cmd::Zoom(self.zoom - 0.1),
                    );
                    let actual = icon::MAGNIFYING_GLASS;
                    tool(ui, p, clicked, actual, "100%", false, Cmd::Zoom(1.0));
                    let zoom_in = icon::MAGNIFYING_GLASS_PLUS;
                    tool(
                        ui,
                        p,
                        clicked,
                        zoom_in,
                        "Zoom in",
                        false,
                        Cmd::Zoom(self.zoom + 0.1),
                    );
                    divider(ui, p);
                    let (glyph, label) = if self.dark {
                        (icon::SUN, "Light mode")
                    } else {
                        (icon::MOON, "Dark mode")
                    };
                    tool(ui, p, clicked, glyph, label, false, Cmd::ToggleTheme);
                }
            }
        });
    }

    // ------------------------------------------------------------ find and replace

    fn find_bar(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let palette = Palette::of(ui);
        let find = &mut self.find;
        let count = self.tabs[self.active]
            .doc
            .find(&find.query, find.match_case)
            .len();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new(icon::MAGNIFYING_GLASS).size(15.0));
            let query = TextEdit::singleline(&mut find.query)
                .margin(crate::widgets::FIELD_MARGIN)
                .hint_text("Find")
                .desired_width(200.0);
            let query = ui.add(query);
            if std::mem::take(&mut find.focus) {
                query.request_focus();
            }
            // Enter goes to the next match and stays in the field.
            if query.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                clicked.push(Cmd::FindNext(true));
                find.focus = true;
            }
            if widgets::toggle_button(ui, icon::TEXT_AA, "Match case", find.match_case).clicked() {
                find.match_case = !find.match_case;
            }
            if widgets::icon_button(ui, icon::CARET_UP, "Previous").clicked() {
                clicked.push(Cmd::FindNext(false));
            }
            if widgets::icon_button(ui, icon::CARET_DOWN, "Next (Enter)").clicked() {
                clicked.push(Cmd::FindNext(true));
            }
            let found = RichText::new(format!("{count} found")).size(13.0);
            ui.label(found.color(palette.text_dim));
            ui.add_space(12.0);
            let with = TextEdit::singleline(&mut find.with)
                .margin(crate::widgets::FIELD_MARGIN)
                .hint_text("Replace with")
                .desired_width(200.0);
            ui.add(with);
            if ui.button("Replace").clicked() {
                clicked.push(Cmd::ReplaceOne);
            }
            if ui.button("Replace all").clicked() {
                clicked.push(Cmd::ReplaceAll);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_button(ui, icon::X, "Close (Esc)").clicked() {
                    clicked.push(Cmd::CloseFind);
                }
            });
        });
        if ui.input(|input| input.key_pressed(Key::Escape)) {
            clicked.push(Cmd::CloseFind);
        }
    }

    // ------------------------------------------------------------ navigation pane

    /// Headings of the document, indented by level; a click jumps to one.
    fn outline(&self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let palette = Palette::of(ui);
        let doc = &self.tabs[self.active].doc;
        ui.label(
            RichText::new("Navigation")
                .size(13.0)
                .color(palette.text_dim),
        );
        ui.add_space(4.0);
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let mut start = 0;
            let mut any = false;
            let markers = doc.markers();
            let paras = doc.paras().iter().zip(&markers);
            for (text, (para, marker)) in doc.text().split('\n').zip(paras) {
                let here = start;
                start += text.chars().count() + 1;
                let level = match para.kind {
                    Kind::Normal => continue,
                    Kind::Title | Kind::Heading1 => 0.0,
                    Kind::Heading2 => 1.0,
                    Kind::Heading3 => 2.0,
                };
                if text.trim().is_empty() {
                    continue;
                }
                any = true;
                ui.horizontal(|ui| {
                    ui.add_space(level * 14.0);
                    // A numbered heading shows with its number.
                    let text = match marker {
                        Some(number) => format!("{number} {text}"),
                        None => text.to_owned(),
                    };
                    let mut label = RichText::new(text).size(14.0 - level);
                    if level == 0.0 {
                        label = label.strong();
                    }
                    let heading = Button::new(label)
                        .frame_when_inactive(false)
                        .stroke(egui::Stroke::NONE)
                        .wrap_mode(egui::TextWrapMode::Truncate);
                    if ui.add(heading).clicked() {
                        clicked.push(Cmd::Goto(here));
                    }
                });
            }
            if !any {
                let hint = "Titles and headings of the document show up here.";
                ui.label(RichText::new(hint).size(13.0).color(palette.text_dim));
            }
        });
    }

    // ------------------------------------------------------------ page

    fn page(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let zoom_delta = ui.input(|input| input.zoom_delta());
        if zoom_delta != 1.0 && ui.rect_contains_pointer(ui.max_rect()) {
            self.zoom = (self.zoom * zoom_delta).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
        }
        let now = ui.input(|input| input.time);
        let textures = &mut self.textures;
        let tab = &mut self.tabs[self.active];
        let typing = tab.typing();
        tab.view.marks = self.show_marks;
        // What the right-click menu shows, as things are before this frame's edits.
        let palette = Palette::of(ui);
        let format = tab.current();
        let in_table = tab.para().cell.is_some();
        let picture = tab.view.selected_picture.is_some();
        let out = editor::show(
            ui,
            tab.editor_id(),
            &mut tab.doc,
            typing,
            self.zoom,
            &mut tab.view,
            editor::Hooks {
                textures: &mut |ctx, image| texture_of(textures, ctx, image),
                menu: &mut |ui| quick_menu(ui, &palette, format, in_table, picture, clicked),
            },
        );
        tab.pages = out.pages;
        tab.para_pages = out.para_pages;
        // Typing over a table of contents that waits for its pages leaves it as typed.
        if out.before.is_some() {
            tab.contents = None;
        } else {
            tab.settle_contents();
        }
        if tab.contents.is_some() {
            ui.ctx().request_repaint();
        }
        if let Some(before) = out.before {
            // Typing joins the step in progress; a move or a resize is a step of its own.
            if out.structural || now - tab.last_edit > UNDO_GROUP_SECONDS {
                tab.checkpoint(before);
            }
            tab.redo.clear();
            tab.last_edit = if out.structural {
                f64::NEG_INFINITY
            } else {
                now
            };
            tab.dirty = true;
        }
        if let Some(selection) = out.selection
            && selection != tab.selection
        {
            // The caret moved: a format picked for the old place no longer applies.
            tab.selection = selection;
            tab.pending = None;
        }
        // Format painter: the look picked up goes to what is selected next, once it is.
        let released = !ui.input(|input| input.pointer.any_down());
        let target = tab.selection.clone();
        let brush = tab.painter.clone().filter(|(_, from)| *from != target);
        if let Some((look, _)) = brush.filter(|_| released && !target.is_empty()) {
            tab.painter = None;
            tab.checkpoint(tab.doc.clone());
            tab.doc.restyle(target, |format| *format = look);
            tab.touched();
        }
    }

    // ------------------------------------------------------------ unsaved changes

    fn closing_dialog(&mut self, ctx: &Context) {
        let Some(closing) = self.closing else {
            return;
        };
        let question = match closing {
            Closing::Tab(index) => format!("Save changes to {}?", self.tabs[index].name),
            Closing::Window => "Save changes before closing?".to_owned(),
        };
        let mut choice = None;
        // A modal greys out the window behind it until the question is answered.
        let response = egui::Modal::new(Id::new("unsaved-changes")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("Unsaved changes");
            ui.add_space(6.0);
            ui.label(question);
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(Some(true));
                }
                if ui.button("Don't save").clicked() {
                    choice = Some(Some(false));
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(None);
                }
            });
        });
        // Escape or a click outside cancels.
        if choice.is_none() && response.should_close() {
            choice = Some(None);
        }
        let Some(choice) = choice else {
            return;
        };
        self.closing = None;
        let Some(save) = choice else {
            return;
        };
        match closing {
            Closing::Tab(index) => {
                if !save || self.save_tab(index, false) {
                    self.close_tab(index);
                }
            }
            Closing::Window => {
                let dirty: Vec<usize> = (0..self.tabs.len())
                    .filter(|index| self.tabs[*index].dirty)
                    .collect();
                if !save || dirty.into_iter().all(|index| self.save_tab(index, false)) {
                    self.quit = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
    }
}

impl eframe::App for GodApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let mut clicked: Vec<Cmd> = Vec::new();
        self.shortcuts(&ctx, &mut clicked);
        self.guard_close(&ctx);
        self.update_title(&ctx);
        let palette = Palette::of(ui);

        Panel::top("tab-row")
            .frame(Frame::new().fill(palette.surface).inner_margin(Margin {
                left: 8,
                right: 8,
                top: 4,
                bottom: 0,
            }))
            .show_separator_line(false)
            .show(ui, |ui| self.tab_row(ui, &mut clicked));
        if self.home {
            CentralPanel::default()
                .frame(Frame::new().fill(palette.bg))
                .show(ui, |ui| self.welcome(ui, &mut clicked));
        } else {
            Panel::top("ribbon-tabs")
                .frame(
                    Frame::new()
                        .fill(palette.bg)
                        .inner_margin(Margin::symmetric(8, 0)),
                )
                .show_separator_line(false)
                .show(ui, |ui| self.ribbon_tabs(ui));
            Panel::top("ribbon")
                .frame(
                    Frame::new()
                        .fill(palette.bg)
                        .inner_margin(Margin::symmetric(10, 0)),
                )
                .show(ui, |ui| {
                    // A window too narrow for all the tools scrolls them sideways.
                    // Its bar shows as soon as there is something to scroll to, not only under
                    // the pointer.
                    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                    ui.spacing_mut().scroll.bar_width = 6.0;
                    let bar = ScrollArea::horizontal().id_salt("ribbon");
                    bar.auto_shrink([false, true])
                        .show(ui, |ui| self.ribbon(ui, &mut clicked));
                });
            if self.find.open {
                Panel::top("find-bar")
                    .frame(
                        Frame::new()
                            .fill(palette.surface)
                            .inner_margin(Margin::symmetric(12, 5)),
                    )
                    .show(ui, |ui| self.find_bar(ui, &mut clicked));
            }
            Panel::bottom("status")
                .frame(
                    Frame::new()
                        .fill(palette.bg)
                        .inner_margin(Margin::symmetric(12, 3)),
                )
                .show(ui, |ui| self.status_bar(ui, &mut clicked));
            if self.show_outline {
                Panel::left("outline")
                    .resizable(true)
                    .default_size(220.0)
                    .size_range(160.0..=380.0)
                    .frame(Frame::new().fill(palette.bg).inner_margin(Margin::same(10)))
                    .show(ui, |ui| self.outline(ui, &mut clicked));
            }
            CentralPanel::default()
                .frame(Frame::new().fill(palette.canvas))
                .show(ui, |ui| self.page(ui, &mut clicked));
        }

        for cmd in clicked {
            self.run(&ctx, cmd);
        }
        self.closing_dialog(&ctx);
        self.updates.show(&ctx);
        self.toasts.show(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("dark_mode", if self.dark { "1" } else { "0" }.to_owned());
        storage.set_string("zoom", self.zoom.to_string());
        let outline = if self.show_outline { "1" } else { "0" };
        storage.set_string("outline", outline.to_owned());
        let marks = if self.show_marks { "1" } else { "0" };
        storage.set_string("marks", marks.to_owned());
        let recent: Vec<String> = self
            .recent
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        storage.set_string("recent", recent.join("\n"));
    }
}

fn is_text(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || "Untitled".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Put the page's caret / selection at `range`.
fn set_cursor(ctx: &Context, id: Id, range: &Range<usize>) {
    if let Some(mut state) = TextEdit::load_state(ctx, id) {
        let range = CCursorRange::two(CCursor::new(range.start), CCursor::new(range.end));
        state.cursor.set_char_range(Some(range));
        state.store(ctx, id);
    }
}

fn divider(ui: &mut Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(13.0, 22.0), Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        Stroke::new(1.0, palette.border),
    );
}

/// Round color dot; the current one gets a ring.
fn swatch(ui: &mut Ui, color: [u8; 3], current: bool, palette: &Palette) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    let painter = ui.painter();
    let center = rect.center();
    if current {
        painter.circle_stroke(center, 10.0, Stroke::new(1.5, palette.text));
    }
    let [r, g, b] = color;
    painter.circle(
        center,
        7.0,
        Color32::from_rgb(r, g, b),
        Stroke::new(1.0, palette.border),
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One document tab; returns (selected, close requested).
fn doc_tab(
    ui: &mut Ui,
    palette: &Palette,
    title: &str,
    dirty: bool,
    active: bool,
    text_max: f32,
) -> (bool, bool) {
    let text_color = if active {
        palette.text
    } else {
        palette.text_dim
    };
    let mut job = egui::text::LayoutJob::simple_singleline(
        title.to_owned(),
        FontId::proportional(14.0),
        text_color,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_max);
    let galley = ui.painter().layout_job(job);
    let star = if dirty { 14.0 } else { 0.0 };
    let close_size = widgets::SMALL_ICON_BUTTON;
    let width = 12.0 + 16.0 + 8.0 + galley.size().x + star + 8.0 + close_size.x + 8.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, TAB_HEIGHT), Sense::click());
    let painter = ui.painter();
    // The active tab reaches the bottom edge so it merges with the bar below.
    if active {
        let radius = CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        };
        painter.rect_filled(rect.with_min_y(rect.min.y + 4.0), radius, palette.bg);
    } else if response.hovered() {
        let shape = rect.shrink2(vec2(0.0, 5.0));
        painter.rect_filled(shape, CornerRadius::same(8), palette.hover);
    }
    let mid = rect.center().y + 2.0;
    painter.text(
        pos2(rect.min.x + 20.0, mid),
        Align2::CENTER_CENTER,
        icon::FILE_DOC,
        FontId::proportional(16.0),
        text_color,
    );
    let text_x = rect.min.x + 36.0;
    painter.galley(
        pos2(text_x, mid - galley.size().y / 2.0),
        galley.clone(),
        text_color,
    );
    if dirty {
        // Unsaved changes: a plain asterisk after the name.
        painter.text(
            pos2(text_x + galley.size().x + 4.0, mid),
            Align2::LEFT_CENTER,
            "*",
            FontId::proportional(15.0),
            text_color,
        );
    }
    let close_rect =
        Rect::from_center_size(pos2(rect.max.x - 8.0 - close_size.x / 2.0, mid), close_size);
    let close = ui
        .put(close_rect, widgets::icon_widget(icon::X, 13.0))
        .on_hover_text("Close");
    let response = response.on_hover_text(title);
    (
        response.clicked(),
        close.clicked() || response.middle_clicked(),
    )
}

/// Grid to pick a table size by pointing, as in Word. Returns (rows, columns) when clicked.
fn table_picker(ui: &mut Ui, palette: &Palette) -> Option<(u16, u16)> {
    const ROWS: u16 = 6;
    const COLS: u16 = 8;
    const SQUARE: f32 = 16.0;
    const PITCH: f32 = 20.0;
    let size = vec2(f32::from(COLS) * PITCH, f32::from(ROWS) * PITCH);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    // Square under the pointer: the table spans from the top-left corner to it.
    let picked = response.hover_pos().map(|pos| {
        let at = (pos - rect.min) / PITCH;
        (
            (at.y as u16).min(ROWS - 1) + 1,
            (at.x as u16).min(COLS - 1) + 1,
        )
    });
    for row in 0..ROWS {
        for col in 0..COLS {
            let corner = rect.min + vec2(f32::from(col), f32::from(row)) * PITCH;
            let square = Rect::from_min_size(corner, vec2(SQUARE, SQUARE));
            let inside = picked.is_some_and(|(rows, cols)| row < rows && col < cols);
            let fill = if inside {
                palette.text
            } else {
                Color32::TRANSPARENT
            };
            let border = Stroke::new(1.0, palette.border);
            ui.painter()
                .rect(square, 2.0, fill, border, egui::StrokeKind::Inside);
        }
    }
    let label = picked.map_or_else(
        || "Pick a size".to_owned(),
        |(rows, cols)| format!("{rows} × {cols}"),
    );
    ui.label(RichText::new(label).size(13.0).color(palette.text_dim));
    picked.filter(|_| response.clicked())
}

/// Text tab of the ribbon, underlined when open.
fn ribbon_tab(ui: &mut Ui, palette: &Palette, label: &str, active: bool) -> Response {
    let color = if active {
        palette.text
    } else {
        palette.text_dim
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(14.5), color);
    let size = vec2(galley.size().x + 22.0, RIBBON_TABS_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    if !active && response.hovered() {
        let shape = rect.shrink2(vec2(0.0, 3.0));
        painter.rect_filled(shape, CornerRadius::same(6), palette.hover);
    }
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    if active {
        let line = Rect::from_min_max(
            pos2(rect.min.x + 11.0, rect.max.y - 2.0),
            pos2(rect.max.x - 11.0, rect.max.y),
        );
        painter.rect_filled(line, CornerRadius::same(1), palette.text);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Large ribbon button: an icon over its label; `on` shows it pressed.
fn ribbon_button(ui: &mut Ui, palette: &Palette, glyph: &str, label: &str, on: bool) -> Response {
    let color = if ui.is_enabled() {
        palette.text
    } else {
        palette.text_dim.gamma_multiply(0.5)
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(11.5), color);
    let size = vec2((galley.size().x + 14.0).max(50.0), BAR_HEIGHT - 8.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let fill = if on {
        palette.selected
    } else if response.hovered() && ui.is_enabled() {
        palette.hover
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, CornerRadius::same(7), fill);
    painter.text(
        pos2(rect.center().x, rect.min.y + 19.0),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(21.0),
        color,
    );
    let text_at = pos2(rect.center().x - galley.size().x / 2.0, rect.max.y - 19.0);
    painter.galley(text_at, galley, color);
    response
}

/// A ribbon button that queues `cmd` when clicked.
fn tool(
    ui: &mut Ui,
    palette: &Palette,
    clicked: &mut Vec<Cmd>,
    glyph: &str,
    label: &str,
    on: bool,
    cmd: Cmd,
) {
    if ribbon_button(ui, palette, glyph, label, on).clicked() {
        clicked.push(cmd);
    }
}

/// Highlighter colours to pick from; the first dot removes the highlight.
fn highlight_swatches(
    ui: &mut Ui,
    current: Option<[u8; 3]>,
    palette: &Palette,
    clicked: &mut Vec<Cmd>,
) {
    let none = swatch(ui, [0xFF, 0xFF, 0xFF], current.is_none(), palette);
    // A stroke across the white dot: no colour.
    let center = none.rect.center();
    let slash = [center + vec2(-5.0, 5.0), center + vec2(5.0, -5.0)];
    ui.painter()
        .line_segment(slash, Stroke::new(1.0, palette.text_dim));
    if none.on_hover_text("No highlight").clicked() {
        clicked.push(Cmd::Edit(Edit::Highlight(None)));
    }
    for (color, name) in HIGHLIGHTS {
        let dot = swatch(ui, color, current == Some(color), palette);
        if dot.on_hover_text(name).clicked() {
            clicked.push(Cmd::Edit(Edit::Highlight(Some(color))));
        }
    }
}

/// Quick options on a right click in the page.
fn quick_menu(
    ui: &mut Ui,
    palette: &Palette,
    format: Format,
    in_table: bool,
    picture: bool,
    clicked: &mut Vec<Cmd>,
) {
    ui.set_min_width(220.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let toggles = [
            (icon::TEXT_B, "Bold", Flag::Bold, format.bold),
            (icon::TEXT_ITALIC, "Italic", Flag::Italic, format.italic),
            (
                icon::TEXT_UNDERLINE,
                "Underline",
                Flag::Underline,
                format.underline,
            ),
            (
                icon::TEXT_STRIKETHROUGH,
                "Strikethrough",
                Flag::Strike,
                format.strike,
            ),
        ];
        for (glyph, tip, flag, on) in toggles {
            if widgets::toggle_button(ui, glyph, tip, on).clicked() {
                clicked.push(Cmd::Edit(Edit::Toggle(flag)));
            }
        }
        let scripts = [
            (icon::TEXT_SUPERSCRIPT, "Superscript", Script::Super),
            (icon::TEXT_SUBSCRIPT, "Subscript", Script::Sub),
        ];
        for (glyph, tip, script) in scripts {
            if widgets::toggle_button(ui, glyph, tip, format.script == script).clicked() {
                clicked.push(Cmd::Edit(Edit::Script(script)));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let label = RichText::new(icon::TEXT_A_UNDERLINE).size(16.0);
        ui.label(label.color(palette.text_dim))
            .on_hover_text("Text colour");
        for (color, name) in SWATCHES {
            let dot = swatch(ui, color, format.color == color, palette);
            if dot.on_hover_text(name).clicked() {
                clicked.push(Cmd::Edit(Edit::Color(color)));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let label = RichText::new(icon::HIGHLIGHTER).size(16.0);
        ui.label(label.color(palette.text_dim))
            .on_hover_text("Highlight colour");
        highlight_swatches(ui, format.highlight, palette, clicked);
    });
    ui.separator();
    menu_item(ui, clicked, icon::SCISSORS, "Cut", "Ctrl+X", Cmd::Cut);
    menu_item(ui, clicked, icon::COPY, "Copy", "Ctrl+C", Cmd::Copy);
    menu_item(ui, clicked, icon::CLIPBOARD, "Paste", "Ctrl+V", Cmd::Paste);
    let all = icon::SELECTION_ALL;
    menu_item(ui, clicked, all, "Select all", "Ctrl+A", Cmd::SelectAll);
    let clear = Cmd::Edit(Edit::ClearFormat);
    menu_item(ui, clicked, icon::ERASER, "Clear formatting", "", clear);
    if in_table {
        ui.separator();
        menu_item(
            ui,
            clicked,
            icon::ARROW_LINE_UP,
            "Insert row above",
            "",
            Cmd::TableRow(false),
        );
        menu_item(
            ui,
            clicked,
            icon::ARROW_LINE_DOWN,
            "Insert row below",
            "",
            Cmd::TableRow(true),
        );
        let left = icon::ARROW_LINE_LEFT;
        menu_item(
            ui,
            clicked,
            left,
            "Insert column left",
            "",
            Cmd::TableCol(false),
        );
        let right = icon::ARROW_LINE_RIGHT;
        menu_item(
            ui,
            clicked,
            right,
            "Insert column right",
            "",
            Cmd::TableCol(true),
        );
        menu_item(
            ui,
            clicked,
            icon::TRASH,
            "Delete row",
            "",
            Cmd::DeleteLine(true),
        );
        menu_item(
            ui,
            clicked,
            icon::TRASH,
            "Delete column",
            "",
            Cmd::DeleteLine(false),
        );
    }
    if picture {
        ui.separator();
        menu_item(
            ui,
            clicked,
            icon::TRASH,
            "Delete image",
            "Del",
            Cmd::DeleteImage,
        );
    }
}

/// One entry of a menu: icon, label and shortcut hint. A click queues `cmd`.
fn menu_item(
    ui: &mut Ui,
    clicked: &mut Vec<Cmd>,
    glyph: &str,
    label: &str,
    shortcut: &str,
    cmd: Cmd,
) {
    let item = Button::new(with_icon(glyph, label)).shortcut_text(shortcut.to_owned());
    if ui.add(item).clicked() {
        clicked.push(cmd);
    }
}

/// Texture of `image`, decoded (and scaled down when huge) the first time it is shown.
fn texture_of(
    cache: &mut HashMap<usize, Option<TextureHandle>>,
    ctx: &Context,
    image: &Image,
) -> Option<TextureId> {
    /// Longest side of a picture's texture, in pixels.
    const MAX_SIDE: u32 = 2048;
    let key = image.bytes.as_ptr() as usize;
    if !cache.contains_key(&key) {
        // Images that are gone (closed documents) leave their textures behind.
        if cache.len() > 128 {
            cache.clear();
        }
        let texture = image::load_from_memory(&image.bytes).ok().map(|decoded| {
            let decoded = if decoded.width().max(decoded.height()) > MAX_SIDE {
                decoded.thumbnail(MAX_SIDE, MAX_SIDE)
            } else {
                decoded
            };
            let rgba = decoded.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
            ctx.load_texture(
                format!("picture-{key}"),
                pixels,
                egui::TextureOptions::LINEAR,
            )
        });
        cache.insert(key, texture);
    }
    cache.get(&key)?.as_ref().map(TextureHandle::id)
}

/// Tab-shaped hit area: filled with rounded top corners when active, joining the bar below.
fn tab_shape(ui: &mut Ui, palette: &Palette, active: bool, size: egui::Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    if active {
        let radius = CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        };
        painter.rect_filled(rect.with_min_y(rect.min.y + 4.0), radius, palette.bg);
    } else if response.hovered() {
        let shape = rect.shrink2(vec2(0.0, 5.0));
        painter.rect_filled(shape, CornerRadius::same(8), palette.hover);
    }
    response
}

/// Today's date written out ("10 October 2026"), from the system clock (UTC).
fn today() -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (year, month, day) = civil_date((seconds / 86_400) as i64);
    format!("{day} {} {year}", MONTHS[month])
}

/// (year, month from 0, day) of a number of days since 1 January 1970.
fn civil_date(days: i64) -> (i64, usize, i64) {
    // Count from 1 March 0000, so that leap days end a year, in 400-year eras.
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = ((month_from_march + 2) % 12) as usize;
    let year = year_of_era + era * 400 + i64::from(month < 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::civil_date;

    #[test]
    fn days_since_1970_become_dates() {
        assert_eq!(civil_date(0), (1970, 0, 1));
        assert_eq!(civil_date(59), (1970, 2, 1));
        // 29 February 2024, then the day after.
        assert_eq!(civil_date(19_782), (2024, 1, 29));
        assert_eq!(civil_date(19_783), (2024, 2, 1));
        assert_eq!(civil_date(20_736), (2026, 9, 10));
    }
}
