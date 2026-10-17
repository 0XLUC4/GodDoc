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
    Align as ParaAlign, Change, Comment, Document, Font, Format, Image, ImageKind, Kind, List,
    MAX_INDENT, Margins, Paper, Script, Spacing, Wrap, docx,
};

use crate::theme::{self, Palette};
use crate::toasts::Toasts;
use crate::widgets::{self, icon, with_icon};
use crate::{APP_NAME, branding, editor, layout, platform};

const TAB_HEIGHT: f32 = 38.0;
const BAR_HEIGHT: f32 = 90.0;
/// Height of a large ribbon button.
const TOOL_HEIGHT: f32 = 54.0;
/// Height of the tools of a ribbon group, and of its name under them.
const GROUP_TOOLS: f32 = 60.0;
const GROUP_NAME: f32 = 18.0;
/// Font sizes offered, in points.
const SIZES: [f32; 16] = [
    8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0,
];
/// Text colours to pick from: greys and deep colours, then bright ones.
const TEXT_COLORS: [([u8; 3], &str); 20] = [
    ([0x00, 0x00, 0x00], "Black"),
    ([0x40, 0x40, 0x40], "Dark grey"),
    ([0x7F, 0x7F, 0x7F], "Grey"),
    ([0xBF, 0xBF, 0xBF], "Light grey"),
    ([0xFF, 0xFF, 0xFF], "White"),
    ([0x1F, 0x38, 0x64], "Navy"),
    ([0x2F, 0x54, 0x96], "Steel blue"),
    ([0x83, 0x3C, 0x0B], "Brown"),
    ([0x38, 0x57, 0x23], "Dark green"),
    ([0x7F, 0x60, 0x00], "Olive"),
    ([0xC0, 0x00, 0x00], "Dark red"),
    ([0xFF, 0x00, 0x00], "Red"),
    ([0xFF, 0xC0, 0x00], "Orange"),
    ([0xFF, 0xFF, 0x00], "Yellow"),
    ([0x92, 0xD0, 0x50], "Light green"),
    ([0x00, 0xB0, 0x50], "Green"),
    ([0x00, 0xB0, 0xF0], "Light blue"),
    ([0x00, 0x70, 0xC0], "Blue"),
    ([0x00, 0x20, 0x60], "Dark blue"),
    ([0x70, 0x30, 0xA0], "Purple"),
];
/// Height of the row of ribbon tabs.
const RIBBON_TABS_HEIGHT: f32 = 30.0;
/// Longest document name shown in a tab before "…".
const TAB_TEXT_MAX: f32 = 180.0;
/// Highlighter colours (content, like the text colours).
const HIGHLIGHTS: [([u8; 3], &str); 8] = [
    ([0xFF, 0xF1, 0x76], "Yellow"),
    ([0xB9, 0xF6, 0xCA], "Green"),
    ([0xB3, 0xE5, 0xFC], "Blue"),
    ([0xF8, 0xBB, 0xD0], "Pink"),
    ([0xFF, 0xD8, 0xA8], "Orange"),
    ([0xE1, 0xBE, 0xE7], "Purple"),
    ([0xFF, 0xCD, 0xD2], "Red"),
    ([0xE0, 0xE0, 0xE0], "Grey"),
];

/// Characters offered by Insert > Special character.
const SYMBOLS: [(&str, &str); 24] = [
    ("§", "Section"),
    ("¶", "Paragraph"),
    ("≤", "Less or equal"),
    ("≥", "Greater or equal"),
    ("≠", "Not equal"),
    ("≈", "About equal"),
    ("½", "One half"),
    ("✓", "Check mark"),
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
    /// Room left and right of the paragraphs, in twentieths of a point.
    Left(u16),
    Right(u16),
    /// How far their first line starts right of the others (left when negative).
    FirstLine(i16),
    /// Room above and under them, in twentieths of a point.
    Before(u16),
    After(u16),
    /// Height of their lines, in hundredths of single spacing.
    Line(u16),
    ClearFormat,
    Case(Case),
}

#[derive(Clone, Copy)]
enum Case {
    Upper,
    Lower,
    /// First letter of each word in capitals.
    Title,
    /// First letter of each sentence in capitals.
    Sentence,
    /// Capitals become small letters and small letters capitals.
    Toggle,
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
    /// Join the caret's table cell with the one at its right, or give a joined cell its cells back.
    MergeCell,
    SplitCell,
    Landscape(bool),
    PageMargins(Margins),
    Paper(Paper),
    /// How many columns the text runs in on each page.
    Columns(u8),
    /// Where the selected picture stands: above its text, or beside it.
    Wrap(Wrap),
    /// Each margin apart: top, right, bottom, left, in twentieths of a point.
    Margins([u16; 4]),
    /// Show or hide the measures of the paragraph, or those of the page.
    ParagraphWindow,
    PageWindow,
    /// Show or hide what the header and the footer say.
    BannerWindow,
    /// Show or hide the address of a link, or what to say in a new comment.
    LinkWindow,
    CommentWindow,
    /// Show or hide the comments, beside the document.
    Comments,
    /// Track the changes to the text, or stop.
    Track,
    /// Check the spelling as the text is written, or stop.
    Spelling,
    Language(crate::spell::Language),
    /// Write the n-th suggestion in place of the misspelled word the caret is in.
    Suggest(usize),
    /// Leave that word alone from now on.
    IgnoreWord,
    /// Keep (true) or undo the tracked changes: those of the selection, or all of them.
    Changes(bool),
    /// An empty page after the caret.
    BlankPage,
    /// Show or hide the count of words.
    WordCount,
    /// Zoom so that the page fills the width of the window, or (true) shows whole.
    Fit(bool),
    /// Hide the tools to leave only the page, or bring them back.
    Focus,
    InsertText(&'static str),
    InsertDate,
    /// Pick up the look of the selection, to give it to what is selected next.
    FormatPainter,
    /// Show or hide the formatting marks.
    Marks,
    /// Insert a table of contents where the caret is.
    Contents,
    Export(Export),
    /// Send the document to a printer.
    Print,
    /// Put the selected paragraphs in the order of their words (true: from Z to A).
    Sort(bool),
    /// Select the next tracked change (false: the one before).
    NextChange(bool),
    /// Change the case of the selection to the next of: lower, Title, UPPER.
    CycleCase,
    /// Ask which page to go to; fill the screen, or stop.
    GotoWindow,
    FullScreen,
    /// Show or hide the ruler over the page, the options of the program, a dark page.
    Ruler,
    Options,
    DarkPage,
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
    /// Each page as its picture.
    Pdf,
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
    References,
    Review,
    View,
    Table,
}

impl Ribbon {
    const ALL: [(Self, &str); 8] = [
        (Self::File, "File"),
        (Self::Home, "Home"),
        (Self::Insert, "Insert"),
        (Self::Layout, "Layout"),
        (Self::References, "References"),
        (Self::Review, "Review"),
        (Self::View, "View"),
        (Self::Table, "Table"),
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
    /// Changes to the text are tracked: what is put in and taken out stays marked.
    tracking: bool,
    /// What the text was like (a number made from it) when its spelling was last checked.
    spelled: u64,
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
            tracking: false,
            spelled: 0,
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
        if let Some(pending) = self.pending {
            return pending;
        }
        let mut format = self.doc.format_at(self.selection.start);
        // A link, a comment and a tracked change cover their own words: what is typed after
        // them is plain text again.
        let next = self.doc.formats().get(self.selection.start);
        if format.link != 0 && next.is_none_or(|next| next.link != format.link) {
            (format.link, format.color, format.underline) = (0, [0, 0, 0], false);
        }
        if next.is_none_or(|next| next.note != format.note) {
            format.note = 0;
        }
        format.change = Change::None;
        format
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
            Edit::Spacing(spacing) => {
                return self.edit_paras(|para| (para.spacing, para.line) = (spacing, 0));
            }
            Edit::Left(left) => return self.edit_paras(|para| para.left = left),
            Edit::Right(right) => return self.edit_paras(|para| para.right = right),
            Edit::FirstLine(first) => return self.edit_paras(|para| para.first_line = first),
            Edit::Before(before) => return self.edit_paras(|para| para.before = before),
            Edit::After(after) => return self.edit_paras(|para| para.after = after),
            Edit::Line(line) => return self.edit_paras(|para| para.set_line(line)),
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
                let starts_sentence = std::cell::Cell::new(true);
                self.doc.map_chars(chars, |ch| match case {
                    Case::Upper => upper(ch),
                    Case::Lower => lower(ch),
                    Case::Title => {
                        let first = starts_word.replace(!ch.is_alphanumeric());
                        if first { upper(ch) } else { lower(ch) }
                    }
                    Case::Sentence => {
                        let first = starts_sentence.get();
                        if ch.is_alphanumeric() {
                            starts_sentence.set(false);
                        } else if matches!(ch, '.' | '!' | '?' | '\n') {
                            starts_sentence.set(true);
                        }
                        if first { upper(ch) } else { lower(ch) }
                    }
                    Case::Toggle if ch.is_uppercase() => lower(ch),
                    Case::Toggle => upper(ch),
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
    /// The count of words is shown.
    counting: bool,
    /// The measures of the paragraph, and those of the page, are shown.
    paragraph_open: bool,
    page_open: bool,
    /// The header and the footer are shown, to type them.
    banner_open: bool,
    /// What is typed in them now is one undo step.
    banner_typing: bool,
    /// The address of a link is shown, to type it.
    link_open: bool,
    link_url: String,
    /// A new comment is being written.
    comment_open: bool,
    comment_text: String,
    /// The comments are shown beside the document.
    show_comments: bool,
    /// Spelling is checked, in this language; its dictionary, once it is there.
    spelling: bool,
    language: crate::spell::Language,
    spell: Option<crate::spell::Speller>,
    spell_loading: Option<std::sync::mpsc::Receiver<Result<crate::spell::Speller, String>>>,
    /// The misspelled word the caret is in, and other spellings for it.
    suggested: Option<(Range<usize>, Vec<String>)>,
    /// The ruler shows over the page; the page shows dark.
    show_ruler: bool,
    dark_page: bool,
    /// The options are shown: who writes the comments, how large the program shows, and
    /// whether what is typed is put right.
    options_open: bool,
    /// Asking which page to go to; the window fills the screen.
    goto_open: bool,
    goto_page: usize,
    fullscreen: bool,
    author: String,
    scale: f32,
    autocorrect: bool,
    /// Only the page is shown, without the tools.
    focus: bool,
    /// Room the pages have in the window, for zooming to fit.
    view_size: egui::Vec2,
    /// The fonts of the app, to draw pages away from the window.
    fonts: egui::FontDefinitions,
    /// What a print or an export under way says when it is over.
    working: Option<std::sync::mpsc::Receiver<Result<Option<String>, String>>>,
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
        let scale = get("scale").and_then(|scale| scale.parse::<f32>().ok());
        let scale = scale.unwrap_or(1.0).clamp(0.8, 1.6);
        cc.egui_ctx.set_zoom_factor(scale);
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        layout::install_fonts(&mut fonts);
        crate::updates::add_bold_font(&mut fonts);
        cc.egui_ctx.set_fonts(fonts.clone());
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
            fonts,
            working: None,
            counting: false,
            paragraph_open: false,
            page_open: false,
            banner_open: false,
            banner_typing: false,
            link_open: false,
            link_url: String::new(),
            comment_open: false,
            comment_text: String::new(),
            show_comments: false,
            spelling: false,
            language: crate::spell::Language::default(),
            spell: None,
            spell_loading: None,
            suggested: None,
            show_ruler: get("ruler").is_none_or(|value| value == "1"),
            dark_page: get("dark_page").is_some_and(|value| value == "1"),
            options_open: false,
            goto_open: false,
            goto_page: 1,
            fullscreen: false,
            author: get("author").unwrap_or_default(),
            scale,
            autocorrect: get("autocorrect").is_none_or(|value| value == "1"),
            focus: false,
            view_size: egui::Vec2::ZERO,
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
            Cmd::MergeCell | Cmd::SplitCell => {
                if let Some(cell) = tab.para().cell {
                    let before = tab.doc.clone();
                    let done = if matches!(cmd, Cmd::MergeCell) {
                        tab.doc.merge_cell_right(cell)
                    } else {
                        tab.doc.split_cell(cell)
                    };
                    if done {
                        tab.checkpoint(before);
                        tab.touched();
                    } else if matches!(cmd, Cmd::MergeCell) {
                        self.toasts
                            .error("There is no cell at its right to join it with");
                    } else {
                        self.toasts.error("This cell is not joined");
                    }
                }
            }
            Cmd::TableCol(_) | Cmd::DeleteLine(false)
                if tab
                    .para()
                    .cell
                    .is_some_and(|cell| tab.doc.has_joined_cells(cell.table)) =>
            {
                self.toasts
                    .error("Split the joined cells of this table first");
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
                tab.doc.page.custom = None;
                tab.doc.page.margins = margins;
                tab.touched();
            }
            Cmd::Margins(sides) => {
                tab.checkpoint(tab.doc.clone());
                tab.doc.page.set_margins(sides);
                tab.touched();
            }
            Cmd::ParagraphWindow => self.paragraph_open = !self.paragraph_open,
            Cmd::PageWindow => self.page_open = !self.page_open,
            Cmd::BannerWindow => self.banner_open = !self.banner_open,
            Cmd::LinkWindow => {
                // The address of the link the caret is in, to change it.
                let link = tab.current().link;
                self.link_url = tab.doc.link(link).unwrap_or_default().to_owned();
                self.link_open = true;
                return;
            }
            Cmd::CommentWindow => {
                if tab.selection.is_empty() {
                    self.toasts
                        .error("Select the words the comment is about first");
                } else {
                    self.comment_open = true;
                    return;
                }
            }
            Cmd::Comments => self.show_comments = !self.show_comments,
            Cmd::Track => tab.tracking = !tab.tracking,
            Cmd::Sort(descending) => {
                let paras = tab.doc.paras_in(tab.selection.clone());
                let before = tab.doc.clone();
                if tab.doc.sort_paras(paras, descending) {
                    tab.checkpoint(before);
                    tab.touched();
                } else {
                    self.toasts
                        .error("Select several paragraphs, outside tables, to sort them");
                }
            }
            Cmd::NextChange(forward) => {
                let formats = tab.doc.formats();
                let changed = |index: &usize| formats[*index].change != Change::None;
                let from = tab.selection.clone();
                // The change after what is selected, or before it; then all of it.
                let at = if forward {
                    (from.end..formats.len())
                        .find(changed)
                        .or_else(|| (0..from.end).find(changed))
                } else {
                    let before = (0..from.start).rev().find(changed);
                    before.or_else(|| (from.start..formats.len()).rev().find(changed))
                };
                match at {
                    Some(at) => {
                        let same = |index: &usize| formats[*index].change == formats[at].change;
                        let start = at - (0..at).rev().take_while(same).count();
                        let end = at + (at..formats.len()).take_while(same).count();
                        tab.selection = start..end;
                        tab.view.scroll_to = Some(start);
                        set_cursor(ctx, editor_id, &tab.selection);
                    }
                    None => self.toasts.success("No tracked change in this document"),
                }
            }
            Cmd::CycleCase => {
                let letters = tab.doc.text().chars().skip(tab.selection.start);
                let chosen: String = letters.take(tab.selection.len()).collect();
                // From small letters to a capital on each word, then to capitals, and back.
                let next = if chosen == chosen.to_lowercase() {
                    Case::Title
                } else if chosen == chosen.to_uppercase() {
                    Case::Lower
                } else {
                    Case::Upper
                };
                tab.apply(Edit::Case(next));
            }
            Cmd::GotoWindow => {
                self.goto_open = true;
                return;
            }
            Cmd::FullScreen => {
                self.fullscreen = !self.fullscreen;
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.fullscreen));
            }
            Cmd::Ruler => self.show_ruler = !self.show_ruler,
            Cmd::Options => self.options_open = !self.options_open,
            Cmd::DarkPage => self.dark_page = !self.dark_page,
            Cmd::Spelling => {
                self.spelling = !self.spelling;
                if self.spelling {
                    self.load_spelling(ctx);
                }
            }
            Cmd::Language(language) => {
                (self.language, self.spell) = (language, None);
                if self.spelling {
                    self.load_spelling(ctx);
                }
            }
            Cmd::Suggest(choice) => {
                let picked = self.suggested.take();
                if let Some((word, words)) = picked
                    && let Some(right) = words.get(choice)
                    && let Some(format) = tab.doc.formats().get(word.start).copied()
                {
                    tab.checkpoint(tab.doc.clone());
                    tab.doc.replace(word.clone(), right, format);
                    let caret = word.start + right.chars().count();
                    tab.selection = caret..caret;
                    tab.touched();
                    set_cursor(ctx, editor_id, &tab.selection);
                }
            }
            Cmd::IgnoreWord => {
                if let (Some((word, _)), Some(speller)) =
                    (self.suggested.take(), self.spell.as_mut())
                {
                    let letters = tab.doc.text().chars().skip(word.start).take(word.len());
                    speller.ignore(&letters.collect::<String>());
                    tab.spelled = 0;
                }
            }
            Cmd::Changes(accept) => {
                let all = 0..tab.doc.len();
                let range = Some(tab.selection.clone()).filter(|range| !range.is_empty());
                tab.checkpoint(tab.doc.clone());
                tab.doc.settle_changes(range.unwrap_or(all), accept);
                tab.selection =
                    tab.selection.start.min(tab.doc.len())..tab.selection.start.min(tab.doc.len());
                tab.touched();
                set_cursor(ctx, editor_id, &tab.selection);
            }
            Cmd::Wrap(wrap) => match tab.view.selected_picture {
                Some(para) => {
                    tab.checkpoint(tab.doc.clone());
                    tab.doc.edit_paras(para..para + 1, |para| para.wrap = wrap);
                    tab.touched();
                }
                None => self.toasts.error("Click a picture first"),
            },
            Cmd::Columns(columns) => {
                tab.checkpoint(tab.doc.clone());
                tab.doc.page.columns = columns;
                tab.touched();
            }
            Cmd::Paper(paper) => {
                tab.checkpoint(tab.doc.clone());
                tab.doc.page.paper = paper;
                tab.touched();
            }
            Cmd::BlankPage => {
                // A page of its own: a break before it and one after.
                for _ in 0..2 {
                    let at = tab.selection.start;
                    if !tab.restructure(|doc| doc.insert_page_break(at)) {
                        self.toasts.error("A blank page cannot go inside a table");
                        break;
                    }
                }
                set_cursor(ctx, editor_id, &tab.selection);
            }
            Cmd::WordCount => self.counting = !self.counting,
            Cmd::Fit(whole) => {
                let (width, height) = tab.doc.page.size();
                let room = self.view_size - vec2(2.0 * editor::PAD + 16.0, 2.0 * editor::PAD);
                let by_width = room.x / width;
                let zoom = if whole {
                    by_width.min(room.y / height)
                } else {
                    by_width
                };
                self.zoom = zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
            }
            Cmd::Focus => {
                self.focus = !self.focus;
                if self.focus {
                    self.toasts
                        .success("Only the page: Esc brings the tools back");
                }
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
            Cmd::Print => self.output(ctx, None),
            Cmd::Export(Export::Pdf) => {
                if let Some(path) = platform::pick_export(&tab.name, "PDF document", "pdf") {
                    self.output(ctx, Some(path));
                }
            }
            Cmd::Export(export) => {
                let (label, extension) = match export {
                    Export::Markdown => ("Markdown", "md"),
                    Export::Html | Export::Pdf => ("Web page", "html"),
                };
                if let Some(path) = platform::pick_export(&tab.name, label, extension) {
                    let text = match export {
                        Export::Markdown => goddoc_core::export::markdown(&tab.doc),
                        Export::Html | Export::Pdf => {
                            goddoc_core::export::html(&tab.doc, &tab.name)
                        }
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
        let alt = Modifiers::COMMAND | Modifiers::ALT;
        let size = self.tabs[self.active].current().size;
        let resize = |by: f32| {
            let size = (size + by).clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end());
            Cmd::Edit(Edit::Size(size))
        };
        let kind = |kind: Kind| Cmd::Edit(Edit::Kind(kind));
        let spacing = |spacing: Spacing| Cmd::Edit(Edit::Spacing(spacing));
        // Esc leaves the page-only view.
        if self.focus && ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.focus = false;
        }
        // More specific (shifted) shortcuts first.
        let table = [
            (shift, Key::S, Cmd::SaveAs),
            (shift, Key::Z, Cmd::Redo),
            (Modifiers::SHIFT, Key::F3, Cmd::CycleCase),
            (Modifiers::NONE, Key::F11, Cmd::FullScreen),
            (ctrl, Key::G, Cmd::GotoWindow),
            (shift, Key::L, Cmd::Edit(Edit::List(List::Bullet))),
            (shift, Key::M, Cmd::Edit(Edit::Indent(-1))),
            (shift, Key::E, Cmd::Track),
            (shift, Key::C, Cmd::FormatPainter),
            (alt, Key::M, Cmd::CommentWindow),
            (ctrl, Key::M, Cmd::Edit(Edit::Indent(1))),
            (ctrl, Key::Num0, Cmd::Zoom(1.0)),
            (ctrl, Key::Equals, Cmd::Zoom(self.zoom + 0.1)),
            (ctrl, Key::Minus, Cmd::Zoom(self.zoom - 0.1)),
            (Modifiers::NONE, Key::F7, Cmd::Spelling),
            (Modifiers::NONE, Key::F12, Cmd::SaveAs),
            (shift, Key::N, kind(Kind::Normal)),
            (alt, Key::Num1, kind(Kind::Heading1)),
            (alt, Key::Num2, kind(Kind::Heading2)),
            (alt, Key::Num3, kind(Kind::Heading3)),
            (ctrl, Key::Num1, spacing(Spacing::Single)),
            (ctrl, Key::Num5, spacing(Spacing::OneAndHalf)),
            (ctrl, Key::Num2, spacing(Spacing::Double)),
            (ctrl, Key::CloseBracket, resize(1.0)),
            (ctrl, Key::OpenBracket, resize(-1.0)),
            (ctrl, Key::N, Cmd::New),
            (ctrl, Key::O, Cmd::Open),
            (ctrl, Key::K, Cmd::LinkWindow),
            (ctrl, Key::P, Cmd::Print),
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
        let p = &palette;
        let tab = &self.tabs[self.active];
        let format = tab.current();
        let para = tab.doc.paras()[tab.doc.para_at(tab.selection.start)];
        let edit = |edit: Edit| Cmd::Edit(edit);
        // In a bar that scrolls sideways the width on offer has no end: the tools then
        // take what they need.
        let wide = Some(ui.available_width()).filter(|wide| wide.is_finite());
        let row = vec2(wide.unwrap_or(0.0), BAR_HEIGHT);
        ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // A window too narrow scrolls the tools sideways: their names stay on one line.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.add_space(4.0);
            group(ui, p, "Undo", |ui| {
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
            });
            group(ui, p, "Clipboard", |ui| {
                ui.horizontal(|ui| {
                    tool(ui, p, clicked, icon::CLIPBOARD, "Paste", false, Cmd::Paste);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let painter =
                            "Format painter: click, then select the text that takes this look";
                        let painting = tab.painter.is_some();
                        let small = [
                            (icon::SCISSORS, "Cut", "Cut (Ctrl+X)", false, Cmd::Cut),
                            (icon::COPY, "Copy", "Copy (Ctrl+C)", false, Cmd::Copy),
                            (
                                icon::PAINT_BRUSH,
                                "Format",
                                painter,
                                painting,
                                Cmd::FormatPainter,
                            ),
                        ];
                        for (glyph, label, tip, on, cmd) in small {
                            if mini_tool(ui, glyph, label, tip, on).clicked() {
                                clicked.push(cmd);
                            }
                        }
                    });
                });
            });
            group(ui, p, "Font", |ui| {
                ui.horizontal(|ui| {
                    // Each font is named in its own letters.
                    let face = |font: Font| {
                        RichText::new(font.family())
                            .family(layout::family(font, false))
                            .size(14.5)
                    };
                    ComboBox::from_id_salt("font")
                        .width(150.0)
                        .height(380.0)
                        .selected_text(face(format.font))
                        .show_ui(ui, |ui| {
                            for font in layout::fonts(format.font) {
                                let chosen = ui.selectable_label(font == format.font, face(font));
                                if chosen.clicked() {
                                    clicked.push(edit(Edit::Font(font)));
                                }
                            }
                        })
                        .response
                        .on_hover_text("Font");
                    ComboBox::from_id_salt("size")
                        .width(52.0)
                        .height(380.0)
                        .selected_text(format!("{}", format.size))
                        .show_ui(ui, |ui| {
                            for size in SIZES {
                                let label = format!("{size}");
                                if ui.selectable_label(size == format.size, label).clicked() {
                                    clicked.push(edit(Edit::Size(size)));
                                }
                            }
                        })
                        .response
                        .on_hover_text("Font size (points)");
                    let resize = |by: f32| {
                        let size = (format.size + by).clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end());
                        edit(Edit::Size(size))
                    };
                    if widgets::small_icon_button(ui, icon::CARET_UP, "Larger text").clicked() {
                        clicked.push(resize(1.0));
                    }
                    if widgets::small_icon_button(ui, icon::CARET_DOWN, "Smaller text").clicked() {
                        clicked.push(resize(-1.0));
                    }
                    let case = widgets::icon_button(ui, icon::TEXT_AA, "Change case");
                    Popup::menu(&case).show(|ui| {
                        let cases = [
                            ("UPPERCASE", Case::Upper),
                            ("lowercase", Case::Lower),
                            ("Title Case", Case::Title),
                            ("Sentence case.", Case::Sentence),
                            ("tOGGLE cASE", Case::Toggle),
                        ];
                        for (label, case) in cases {
                            if ui.button(label).clicked() {
                                clicked.push(edit(Edit::Case(case)));
                            }
                        }
                    });
                    if widgets::icon_button(ui, icon::ERASER, "Clear formatting").clicked() {
                        clicked.push(edit(Edit::ClearFormat));
                    }
                });
                ui.horizontal(|ui| {
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
                            clicked.push(edit(Edit::Toggle(flag)));
                        }
                    }
                    let scripts = [
                        (icon::TEXT_SUBSCRIPT, "Subscript", Script::Sub),
                        (icon::TEXT_SUPERSCRIPT, "Superscript", Script::Super),
                    ];
                    for (glyph, tip, script) in scripts {
                        if widgets::toggle_button(ui, glyph, tip, format.script == script).clicked()
                        {
                            clicked.push(edit(Edit::Script(script)));
                        }
                    }
                    ui.add_space(4.0);
                    let marker = color_tool(
                        ui,
                        p,
                        icon::HIGHLIGHTER,
                        "Highlight colour",
                        format.highlight,
                    );
                    Popup::menu(&marker).show(|ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            highlight_swatches(ui, format.highlight, p, clicked);
                        });
                    });
                    let ink = color_tool(
                        ui,
                        p,
                        icon::TEXT_A_UNDERLINE,
                        "Text colour",
                        Some(format.color),
                    );
                    Popup::menu(&ink).show(|ui| {
                        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                        for line in TEXT_COLORS.chunks(10) {
                            ui.horizontal(|ui| {
                                for &(color, name) in line {
                                    let dot = swatch(ui, color, format.color == color, p);
                                    if dot.on_hover_text(name).clicked() {
                                        clicked.push(edit(Edit::Color(color)));
                                    }
                                }
                            });
                        }
                    });
                });
            });
            group(ui, p, "Paragraph", |ui| {
                ui.horizontal(|ui| {
                    let lists = [
                        (icon::LIST_BULLETS, "Bullet list", List::Bullet),
                        (icon::LIST_NUMBERS, "Numbered list", List::Number),
                        (icon::LIST_DASHES, "Dash list", List::Dash),
                    ];
                    for (glyph, tip, list) in lists {
                        if widgets::toggle_button(ui, glyph, tip, para.list == list).clicked() {
                            clicked.push(edit(Edit::List(list)));
                        }
                    }
                    if widgets::icon_button(ui, icon::TEXT_OUTDENT, "Decrease indent").clicked() {
                        clicked.push(edit(Edit::Indent(-1)));
                    }
                    if widgets::icon_button(ui, icon::TEXT_INDENT, "Increase indent").clicked() {
                        clicked.push(edit(Edit::Indent(1)));
                    }
                    let sort =
                        widgets::icon_button(ui, icon::SORT_ASCENDING, "Sort the paragraphs");
                    Popup::menu(&sort).show(|ui| {
                        if ui.button("A to Z").clicked() {
                            clicked.push(Cmd::Sort(false));
                        }
                        if ui.button("Z to A").clicked() {
                            clicked.push(Cmd::Sort(true));
                        }
                    });
                    let marks = "Show the spaces and the ends of paragraphs";
                    if widgets::toggle_button(ui, icon::PARAGRAPH, marks, self.show_marks).clicked()
                    {
                        clicked.push(Cmd::Marks);
                    }
                });
                ui.horizontal(|ui| {
                    let aligns = [
                        (icon::TEXT_ALIGN_LEFT, "Align left", ParaAlign::Left),
                        (icon::TEXT_ALIGN_CENTER, "Center", ParaAlign::Center),
                        (icon::TEXT_ALIGN_RIGHT, "Align right", ParaAlign::Right),
                        (icon::TEXT_ALIGN_JUSTIFY, "Justify", ParaAlign::Justify),
                    ];
                    for (glyph, tip, align) in aligns {
                        if widgets::toggle_button(ui, glyph, tip, para.align == align).clicked() {
                            clicked.push(edit(Edit::Align(align)));
                        }
                    }
                    let spacing = widgets::icon_button(ui, icon::ARROWS_VERTICAL, "Line spacing");
                    Popup::menu(&spacing).show(|ui| {
                        let now = (para.line_factor() * 100.0).round() as u16;
                        for (name, line) in LINE_SPACINGS {
                            if ui.selectable_label(line == now, name).clicked() {
                                clicked.push(edit(Edit::Line(line)));
                            }
                        }
                        ui.separator();
                        if ui.button("Line spacing options…").clicked() {
                            clicked.push(Cmd::ParagraphWindow);
                        }
                        // A space above or under the paragraph, put or taken away in one go.
                        let rooms = [
                            (
                                "before",
                                para.before,
                                Edit::Before(if para.before > 0 { 0 } else { 240 }),
                            ),
                            (
                                "after",
                                para.after,
                                Edit::After(if para.after > 0 { 0 } else { 160 }),
                            ),
                        ];
                        for (side, now, change) in rooms {
                            let verb = if now > 0 { "Remove" } else { "Add" };
                            if ui
                                .button(format!("{verb} space {side} paragraph"))
                                .clicked()
                            {
                                clicked.push(edit(change));
                            }
                        }
                    });
                    let rule = "A line under the paragraph";
                    if widgets::toggle_button(ui, icon::MINUS, rule, para.rule).clicked() {
                        clicked.push(edit(Edit::Rule));
                    }
                });
            });
            group(ui, p, "Styles", |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for kind in Kind::ALL {
                        if style_card(ui, p, kind, kind == para.kind).clicked() {
                            clicked.push(edit(Edit::Kind(kind)));
                        }
                    }
                });
            });
            group(ui, p, "Editing", |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let small = [
                    (icon::MAGNIFYING_GLASS, "Find", "Find (Ctrl+F)", Cmd::Find),
                    (
                        icon::SWAP,
                        "Replace",
                        "Find and replace (Ctrl+F)",
                        Cmd::Find,
                    ),
                    (
                        icon::SELECTION_ALL,
                        "Select all",
                        "Select all (Ctrl+A)",
                        Cmd::SelectAll,
                    ),
                ];
                for (glyph, label, tip, cmd) in small {
                    if mini_tool(ui, glyph, label, tip, false).clicked() {
                        clicked.push(cmd);
                    }
                }
            });
        });
    }

    fn status_bar(&self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let palette = Palette::of(ui);
        let tab = &self.tabs[self.active];
        ui.horizontal(|ui| {
            // The page the caret is on, among those of the document.
            let para = tab.doc.para_at(tab.selection.start);
            let page = tab.para_pages.get(para).map_or(1, |page| page + 1);
            let words = tab.doc.word_count();
            // With words selected: how many of them.
            let chosen: String = {
                let letters = tab.doc.text().chars().skip(tab.selection.start);
                letters.take(tab.selection.len()).collect()
            };
            let words = match chosen.split_whitespace().count() {
                0 => format!("{words} words"),
                chosen => format!("{chosen} of {words} words"),
            };
            let counts = format!(
                "Page {page} of {} · {words} · {} characters",
                tab.pages.max(page),
                tab.doc.len()
            );
            let counts = RichText::new(counts).size(13.0).color(palette.text_dim);
            let label = egui::Label::new(counts)
                .selectable(false)
                .sense(Sense::click());
            if ui.add(label).on_hover_text("Word count").clicked() {
                clicked.push(Cmd::WordCount);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let percent = RichText::new(format!("{:.0}%", self.zoom * 100.0)).size(13.0);
                let label = egui::Label::new(percent)
                    .selectable(false)
                    .sense(Sense::click());
                if ui
                    .add_sized(vec2(44.0, 22.0), label)
                    .on_hover_text("Back to 100%")
                    .clicked()
                {
                    clicked.push(Cmd::Zoom(1.0));
                }
                if widgets::small_icon_button(ui, icon::PLUS, "Zoom in").clicked() {
                    clicked.push(Cmd::Zoom(self.zoom + 0.1));
                }
                let mut zoom = self.zoom;
                ui.spacing_mut().slider_width = 110.0;
                let slider = egui::Slider::new(&mut zoom, ZOOM_RANGE).show_value(false);
                if ui.add(slider).changed() {
                    clicked.push(Cmd::Zoom(zoom));
                }
                if widgets::small_icon_button(ui, icon::MINUS, "Zoom out").clicked() {
                    clicked.push(Cmd::Zoom(self.zoom - 0.1));
                }
            });
        });
    }

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
        let p = &palette;
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
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.add_space(4.0);
            match self.ribbon {
                Ribbon::Home => {}
                Ribbon::File => {
                    tools(ui, p, "Document", |ui| {
                        tool(ui, p, clicked, icon::FILE_PLUS, "New", false, Cmd::New);
                        tool(ui, p, clicked, icon::FOLDER_OPEN, "Open", false, Cmd::Open);
                        tool(ui, p, clicked, icon::FLOPPY_DISK, "Save", false, Cmd::Save);
                        let save_as = icon::FLOPPY_DISK_BACK;
                        tool(ui, p, clicked, save_as, "Save as", false, Cmd::SaveAs);
                    });
                    tools(ui, p, "Print", |ui| {
                        tool(ui, p, clicked, icon::PRINTER, "Print", false, Cmd::Print);
                    });
                    tools(ui, p, "Export", |ui| {
                        let pdf = Cmd::Export(Export::Pdf);
                        tool(ui, p, clicked, icon::FILE_PDF, "PDF", false, pdf);
                        let markdown = Cmd::Export(Export::Markdown);
                        tool(ui, p, clicked, icon::FILE_MD, "Markdown", false, markdown);
                        let page = Cmd::Export(Export::Html);
                        tool(ui, p, clicked, icon::FILE_HTML, "Web page", false, page);
                    });
                    tools(ui, p, "Window", |ui| {
                        tool(ui, p, clicked, icon::X, "Close", false, Cmd::CloseTab);
                        tool(ui, p, clicked, icon::SIGN_OUT, "Quit", false, Cmd::Quit);
                    });
                    tools(ui, p, "Program", |ui| {
                        let open = self.options_open;
                        tool(ui, p, clicked, icon::GEAR, "Options", open, Cmd::Options);
                    });
                }
                Ribbon::Insert => {
                    tools(ui, p, "Pages", |ui| {
                        tool(
                            ui,
                            p,
                            clicked,
                            icon::FILE,
                            "Blank page",
                            false,
                            Cmd::BlankPage,
                        );
                        let page_break = icon::FILE_DASHED;
                        tool(
                            ui,
                            p,
                            clicked,
                            page_break,
                            "Page break",
                            false,
                            Cmd::PageBreak,
                        );
                    });
                    tools(ui, p, "Tables", |ui| {
                        let table = ribbon_button(ui, p, icon::TABLE, "Table", false);
                        Popup::menu(&table).show(|ui| {
                            if let Some((rows, cols)) = table_picker(ui, p) {
                                clicked.push(Cmd::Table(rows, cols));
                            }
                        });
                    });
                    tools(ui, p, "Illustrations", |ui| {
                        tool(
                            ui,
                            p,
                            clicked,
                            icon::IMAGE,
                            "Picture",
                            false,
                            Cmd::InsertImage,
                        );
                    });
                    tools(ui, p, "Links", |ui| {
                        let linked = tab.current().link != 0;
                        tool(ui, p, clicked, icon::LINK, "Link", linked, Cmd::LinkWindow);
                    });
                    tools(ui, p, "Header & Footer", |ui| {
                        let open = self.banner_open;
                        let header = icon::ALIGN_TOP;
                        tool(ui, p, clicked, header, "Header", open, Cmd::BannerWindow);
                        let footer = icon::ALIGN_BOTTOM;
                        tool(ui, p, clicked, footer, "Footer", open, Cmd::BannerWindow);
                        let numbered = tab.doc.page_numbers;
                        let numbers = icon::HASH;
                        tool(
                            ui,
                            p,
                            clicked,
                            numbers,
                            "Page number",
                            numbered,
                            Cmd::PageNumbers,
                        );
                    });
                    tools(ui, p, "Text", |ui| {
                        let date = icon::CALENDAR_BLANK;
                        tool(ui, p, clicked, date, "Date", false, Cmd::InsertDate);
                        tool(
                            ui,
                            p,
                            clicked,
                            icon::MINUS,
                            "Line",
                            para.rule,
                            edit(Edit::Rule),
                        );
                    });
                    tools(ui, p, "Symbols", |ui| {
                        let symbol = ribbon_button(ui, p, icon::ASTERISK, "Symbol", false);
                        Popup::menu(&symbol).show(|ui| {
                            for (symbol, name) in SYMBOLS {
                                menu_item(ui, clicked, symbol, name, "", Cmd::InsertText(symbol));
                            }
                        });
                    });
                }
                Ribbon::Layout => {
                    tools(ui, p, "Page Setup", |ui| {
                        let margins = ribbon_button(ui, p, icon::FRAME_CORNERS, "Margins", false);
                        Popup::menu(&margins).show(|ui| {
                            for kind in Margins::ALL {
                                let on = page.custom.is_none() && page.margins == kind;
                                if ui.selectable_label(on, kind.name()).clicked() {
                                    clicked.push(Cmd::PageMargins(kind));
                                }
                            }
                            ui.separator();
                            let custom = page.custom.is_some();
                            if ui.selectable_label(custom, "Custom margins…").clicked() {
                                clicked.push(Cmd::PageWindow);
                            }
                        });
                        let turned = ribbon_button(ui, p, icon::FILE_TEXT, "Orientation", false);
                        Popup::menu(&turned).show(|ui| {
                            for (label, landscape) in [("Portrait", false), ("Landscape", true)] {
                                let on = page.landscape == landscape;
                                if ui.selectable_label(on, label).clicked() {
                                    clicked.push(Cmd::Landscape(landscape));
                                }
                            }
                        });
                        let size = ribbon_button(ui, p, icon::RESIZE, "Size", false);
                        Popup::menu(&size).show(|ui| {
                            for paper in Paper::ALL {
                                if ui
                                    .selectable_label(page.paper == paper, paper.name())
                                    .clicked()
                                {
                                    clicked.push(Cmd::Paper(paper));
                                }
                            }
                        });
                        let columns = ribbon_button(ui, p, icon::COLUMNS, "Columns", false);
                        Popup::menu(&columns).show(|ui| {
                            for (name, count) in [("One", 1), ("Two", 2), ("Three", 3)] {
                                let on = page.columns.max(1) == count;
                                if ui.selectable_label(on, name).clicked() {
                                    clicked.push(Cmd::Columns(count));
                                }
                            }
                        });
                        let page_break = icon::FILE_DASHED;
                        tool(ui, p, clicked, page_break, "Breaks", false, Cmd::PageBreak);
                    });
                    tools(ui, p, "Arrange", |ui| {
                        let picture = tab.view.selected_picture;
                        let now = picture.map(|para| tab.doc.paras()[para].wrap);
                        ui.add_enabled_ui(picture.is_some(), |ui| {
                            let wrap = ribbon_button(ui, p, icon::TEXT_INDENT, "Wrap text", false);
                            Popup::menu(&wrap).show(|ui| {
                                let places = [
                                    ("Above its text", Wrap::Above),
                                    ("Left, text beside it", Wrap::Left),
                                    ("Right, text beside it", Wrap::Right),
                                ];
                                for (name, place) in places {
                                    if ui.selectable_label(now == Some(place), name).clicked() {
                                        clicked.push(Cmd::Wrap(place));
                                    }
                                }
                            });
                        });
                    });
                    tools(ui, p, "Paragraph", |ui| {
                        ui.vertical(|ui| {
                            // Two rows of fields under their names, in the height of a group.
                            ui.spacing_mut().interact_size.y = 18.0;
                            ui.spacing_mut().button_padding.y = 0.0;
                            let small = |name: &str| RichText::new(name).size(12.0);
                            let dim = |name: &str| small(name).color(p.text_dim);
                            let grid = egui::Grid::new("paragraph-measures").num_columns(4);
                            grid.spacing(vec2(6.0, 1.0))
                                .min_row_height(16.0)
                                .show(ui, |ui| {
                                    ui.label(dim("Indent"));
                                    ui.label("");
                                    ui.label(dim("Spacing"));
                                    ui.end_row();
                                    ui.label(small("Left"));
                                    if let Some(left) = length(ui, "left", para.left, 0.0..=20.0) {
                                        clicked.push(edit(Edit::Left(left)));
                                    }
                                    ui.label(small("Before"));
                                    if let Some(before) = points(ui, "before", para.before) {
                                        clicked.push(edit(Edit::Before(before)));
                                    }
                                    ui.end_row();
                                    ui.label(small("Right"));
                                    if let Some(right) = length(ui, "right", para.right, 0.0..=20.0)
                                    {
                                        clicked.push(edit(Edit::Right(right)));
                                    }
                                    ui.label(small("After"));
                                    if let Some(after) = points(ui, "after", para.after) {
                                        clicked.push(edit(Edit::After(after)));
                                    }
                                    ui.end_row();
                                });
                        });
                        let spacing =
                            ribbon_button(ui, p, icon::ARROWS_VERTICAL, "Line spacing", false);
                        Popup::menu(&spacing).show(|ui| {
                            let now = (para.line_factor() * 100.0).round() as u16;
                            for (name, line) in LINE_SPACINGS {
                                if ui.selectable_label(line == now, name).clicked() {
                                    clicked.push(edit(Edit::Line(line)));
                                }
                            }
                        });
                        let more = icon::SLIDERS_HORIZONTAL;
                        let open = self.paragraph_open;
                        tool(
                            ui,
                            p,
                            clicked,
                            more,
                            "Paragraph",
                            open,
                            Cmd::ParagraphWindow,
                        );
                    });
                }
                Ribbon::References => {
                    tools(ui, p, "Table of Contents", |ui| {
                        let contents = icon::LIST_MAGNIFYING_GLASS;
                        tool(ui, p, clicked, contents, "Contents", false, Cmd::Contents);
                    });
                    tools(ui, p, "Headings", |ui| {
                        let heading = para.kind.heading_level().is_some();
                        ui.add_enabled_ui(heading, |ui| {
                            let numbered = para.list == List::Number;
                            let numbers = icon::LIST_NUMBERS;
                            let number = edit(Edit::List(List::Number));
                            tool(ui, p, clicked, numbers, "Numbering", numbered, number);
                            let up = icon::ARROW_FAT_LEFT;
                            tool(
                                ui,
                                p,
                                clicked,
                                up,
                                "Level up",
                                false,
                                edit(Edit::Nest(false)),
                            );
                            let down = icon::ARROW_FAT_RIGHT;
                            tool(
                                ui,
                                p,
                                clicked,
                                down,
                                "Level down",
                                false,
                                edit(Edit::Nest(true)),
                            );
                        });
                    });
                    tools(ui, p, "Navigation", |ui| {
                        let pane = icon::SIDEBAR_SIMPLE;
                        tool(
                            ui,
                            p,
                            clicked,
                            pane,
                            "Headings pane",
                            self.show_outline,
                            Cmd::Outline,
                        );
                    });
                }
                Ribbon::Review => {
                    tools(ui, p, "Proofing", |ui| {
                        let check = icon::CHECK_SQUARE;
                        tool(
                            ui,
                            p,
                            clicked,
                            check,
                            "Spelling",
                            self.spelling,
                            Cmd::Spelling,
                        );
                        let language = ribbon_button(ui, p, icon::TRANSLATE, "Language", false);
                        Popup::menu(&language).show(|ui| {
                            for language in crate::spell::Language::ALL {
                                let on = self.language == language;
                                if ui.selectable_label(on, language.name()).clicked() {
                                    clicked.push(Cmd::Language(language));
                                }
                            }
                        });
                        let count = icon::LIST_NUMBERS;
                        tool(
                            ui,
                            p,
                            clicked,
                            count,
                            "Word count",
                            self.counting,
                            Cmd::WordCount,
                        );
                    });
                    tools(ui, p, "Comments", |ui| {
                        let new = icon::CHAT_TEXT;
                        tool(
                            ui,
                            p,
                            clicked,
                            new,
                            "New comment",
                            false,
                            Cmd::CommentWindow,
                        );
                        let shown = self.show_comments;
                        tool(
                            ui,
                            p,
                            clicked,
                            icon::CHATS,
                            "Show comments",
                            shown,
                            Cmd::Comments,
                        );
                    });
                    tools(ui, p, "Tracking", |ui| {
                        let track = icon::PENCIL_LINE;
                        tool(
                            ui,
                            p,
                            clicked,
                            track,
                            "Track changes",
                            tab.tracking,
                            Cmd::Track,
                        );
                        tool(
                            ui,
                            p,
                            clicked,
                            icon::CHECK,
                            "Accept",
                            false,
                            Cmd::Changes(true),
                        );
                        tool(
                            ui,
                            p,
                            clicked,
                            icon::X,
                            "Reject",
                            false,
                            Cmd::Changes(false),
                        );
                    });
                    tools(ui, p, "Changes", |ui| {
                        let back = icon::ARROW_UP;
                        tool(
                            ui,
                            p,
                            clicked,
                            back,
                            "Previous",
                            false,
                            Cmd::NextChange(false),
                        );
                        let on = icon::ARROW_DOWN;
                        tool(ui, p, clicked, on, "Next", false, Cmd::NextChange(true));
                    });
                    tools(ui, p, "Find", |ui| {
                        let find = icon::MAGNIFYING_GLASS;
                        tool(ui, p, clicked, find, "Find", self.find.open, Cmd::Find);
                        tool(ui, p, clicked, icon::SWAP, "Replace", false, Cmd::Find);
                        let go = icon::ARROW_BEND_DOWN_RIGHT;
                        tool(ui, p, clicked, go, "Go to", self.goto_open, Cmd::GotoWindow);
                    });
                    tools(ui, p, "Formatting", |ui| {
                        let marks = self.show_marks;
                        tool(ui, p, clicked, icon::PARAGRAPH, "Marks", marks, Cmd::Marks);
                        let clear = edit(Edit::ClearFormat);
                        tool(ui, p, clicked, icon::ERASER, "Clear", false, clear);
                    });
                }
                Ribbon::Table => {
                    tools(ui, p, "Table", |ui| {
                        let table = ribbon_button(ui, p, icon::TABLE, "New table", false);
                        Popup::menu(&table).show(|ui| {
                            if let Some((rows, cols)) = table_picker(ui, p) {
                                clicked.push(Cmd::Table(rows, cols));
                            }
                        });
                    });
                    // The rest acts on the table the caret is in.
                    let inside = para.cell.is_some();
                    tools(ui, p, "Rows & Columns", |ui| {
                        ui.add_enabled_ui(inside, |ui| {
                            let tools = [
                                (icon::ARROW_LINE_UP, "Row above", Cmd::TableRow(false)),
                                (icon::ARROW_LINE_DOWN, "Row below", Cmd::TableRow(true)),
                                (icon::ARROW_LINE_LEFT, "Column left", Cmd::TableCol(false)),
                                (icon::ARROW_LINE_RIGHT, "Column right", Cmd::TableCol(true)),
                            ];
                            for (glyph, label, cmd) in tools {
                                tool(ui, p, clicked, glyph, label, false, cmd);
                            }
                        });
                    });
                    tools(ui, p, "Merge", |ui| {
                        ui.add_enabled_ui(inside, |ui| {
                            let merge = icon::ARROWS_MERGE;
                            tool(ui, p, clicked, merge, "Merge right", false, Cmd::MergeCell);
                            let split = icon::ARROWS_SPLIT;
                            tool(ui, p, clicked, split, "Split cell", false, Cmd::SplitCell);
                        });
                    });
                    tools(ui, p, "Delete", |ui| {
                        ui.add_enabled_ui(inside, |ui| {
                            let tools = [
                                (icon::ROWS, "Row", Cmd::DeleteLine(true)),
                                (icon::COLUMNS, "Column", Cmd::DeleteLine(false)),
                                (icon::TRASH, "Table", Cmd::DeleteTable),
                            ];
                            for (glyph, label, cmd) in tools {
                                tool(ui, p, clicked, glyph, label, false, cmd);
                            }
                        });
                    });
                }
                Ribbon::View => {
                    tools(ui, p, "Views", |ui| {
                        let full = icon::ARROWS_OUT;
                        tool(
                            ui,
                            p,
                            clicked,
                            full,
                            "Full screen",
                            self.fullscreen,
                            Cmd::FullScreen,
                        );
                        tool(
                            ui,
                            p,
                            clicked,
                            icon::CORNERS_OUT,
                            "Focus",
                            false,
                            Cmd::Focus,
                        );
                    });
                    tools(ui, p, "Show", |ui| {
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
                        let ruler = self.show_ruler;
                        tool(ui, p, clicked, icon::RULER, "Ruler", ruler, Cmd::Ruler);
                    });
                    tools(ui, p, "Zoom", |ui| {
                        let out = icon::MAGNIFYING_GLASS_MINUS;
                        tool(
                            ui,
                            p,
                            clicked,
                            out,
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
                        let width = icon::ARROWS_HORIZONTAL;
                        tool(ui, p, clicked, width, "Page width", false, Cmd::Fit(false));
                        tool(
                            ui,
                            p,
                            clicked,
                            icon::FILE,
                            "One page",
                            false,
                            Cmd::Fit(true),
                        );
                    });
                    tools(ui, p, "Theme", |ui| {
                        let (glyph, label) = if self.dark {
                            (icon::SUN, "Light mode")
                        } else {
                            (icon::MOON, "Dark mode")
                        };
                        tool(ui, p, clicked, glyph, label, false, Cmd::ToggleTheme);
                        let dark = self.dark_page;
                        tool(ui, p, clicked, icon::FILE, "Dark page", dark, Cmd::DarkPage);
                    });
                }
            }
        });
    }

    /// What every page says in its top and bottom margins, to type.
    fn banner_window(&mut self, ctx: &Context) {
        if !self.banner_open || self.home {
            return;
        }
        let tab = &mut self.tabs[self.active];
        let (mut header, mut footer) = (tab.doc.header.clone(), tab.doc.footer.clone());
        let mut typing = false;
        self.banner_open = dialog(ctx, "Header & Footer", |ui| {
            let grid = egui::Grid::new("banners").num_columns(4);
            grid.spacing(vec2(8.0, 7.0)).show(ui, |ui| {
                ui.label("");
                for place in ["Left", "Middle", "Right"] {
                    ui.label(RichText::new(place).color(Palette::of(ui).text_dim));
                }
                ui.end_row();
                for (name, banner) in [("Header", &mut header), ("Footer", &mut footer)] {
                    ui.label(RichText::new(name).strong());
                    for zone in [&mut banner.left, &mut banner.center, &mut banner.right] {
                        let field = TextEdit::singleline(zone)
                            .desired_width(150.0)
                            .margin(widgets::FIELD_MARGIN);
                        typing |= ui.add_sized(vec2(160.0, 26.0), field).has_focus();
                    }
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            let hint = "{page} writes the number of the page, {pages} how many there are.";
            ui.label(RichText::new(hint).small().color(Palette::of(ui).text_dim));
        });
        if header != tab.doc.header || footer != tab.doc.footer {
            // One undo step for all that is typed before leaving the fields.
            if !std::mem::replace(&mut self.banner_typing, true) {
                tab.checkpoint(tab.doc.clone());
            }
            (tab.doc.header, tab.doc.footer) = (header, footer);
            tab.touched();
        } else if !typing {
            self.banner_typing = false;
        }
    }

    /// Where the selected words lead: an address to type, to open or to take away.
    fn link_window(&mut self, ctx: &Context) {
        if !self.link_open || self.home {
            return;
        }
        let tab = &mut self.tabs[self.active];
        // The words the link is on: those selected, or all of the link the caret is in.
        let caret = tab.selection.start.min(tab.doc.len().saturating_sub(1));
        let link = tab.doc.formats().get(caret).map_or(0, |format| format.link);
        let mut words = tab.selection.clone();
        if words.is_empty() && link != 0 {
            let linked = |index: &usize| tab.doc.formats()[*index].link == link;
            let before = (0..caret).rev().take_while(linked).count();
            let after = (caret..tab.doc.len()).take_while(linked).count();
            words = caret - before..caret + after;
        }
        let (mut apply, mut remove, mut open) = (false, false, false);
        let url = &mut self.link_url;
        self.link_open = dialog(ctx, "Link", |ui| {
            ui.label("Address");
            let field = TextEdit::singleline(url)
                .hint_text("https://")
                .desired_width(340.0)
                .margin(widgets::FIELD_MARGIN);
            let typed = ui.add(field);
            apply = typed.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                apply |= ui.button("Apply").clicked();
                ui.add_enabled_ui(link != 0, |ui| {
                    remove = ui.button("Remove link").clicked();
                    open = ui.button("Open").clicked();
                });
            });
        });
        if open
            && let Some(url) = tab.doc.link(link)
            && let Err(err) = platform::open_url(url)
        {
            self.toasts.error(err);
        }
        let typed = self.link_url.trim().to_owned();
        if apply && !typed.is_empty() {
            // An address without its start is one of the web.
            let url = if typed.contains(':') {
                typed
            } else {
                format!("https://{typed}")
            };
            tab.checkpoint(tab.doc.clone());
            let link = tab.doc.link_to(&url);
            let look = move |format: &mut Format| {
                (format.link, format.color, format.underline) = (link, LINK_COLOR, true);
            };
            if words.is_empty() {
                // Nothing to put it on: the address itself is written.
                let mut format = tab.typing();
                look(&mut format);
                let at = tab.selection.start;
                tab.doc.replace(at..at, &url, format);
            } else {
                tab.doc.restyle(words.clone(), look);
            }
            tab.touched();
            self.link_open = false;
        } else if remove && !words.is_empty() {
            tab.checkpoint(tab.doc.clone());
            tab.doc.restyle(words, |format| {
                (format.link, format.color, format.underline) = (0, [0, 0, 0], false);
            });
            tab.touched();
            self.link_open = false;
        }
    }

    /// What to say about the selected words.
    fn comment_window(&mut self, ctx: &Context) {
        if !self.comment_open || self.home {
            return;
        }
        let mut add = false;
        let text = &mut self.comment_text;
        self.comment_open = dialog(ctx, "New comment", |ui| {
            let field = TextEdit::multiline(text)
                .desired_width(340.0)
                .desired_rows(4)
                .margin(widgets::FIELD_MARGIN);
            ui.add(field).request_focus();
            ui.add_space(8.0);
            add = ui.button("Add").clicked();
        });
        let said = self.comment_text.trim().to_owned();
        if !add || said.is_empty() {
            return;
        }
        let tab = &mut self.tabs[self.active];
        tab.checkpoint(tab.doc.clone());
        // The name given in the options, or else the one of the session.
        let named = Some(self.author.trim().to_owned()).filter(|name| !name.is_empty());
        let author = named
            .or_else(|| std::env::var("USERNAME").ok())
            .or_else(|| std::env::var("USER").ok());
        let note = tab.doc.add_comment(Comment {
            author: author.unwrap_or_else(|| "Me".to_owned()),
            text: said,
        });
        tab.doc
            .restyle(tab.selection.clone(), |format| format.note = note);
        tab.touched();
        self.comment_text.clear();
        (self.comment_open, self.show_comments) = (false, true);
    }

    /// The comments of the document, beside it: who said what, to go to or to take away.
    fn comments_pane(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let palette = Palette::of(ui);
        let tab = &mut self.tabs[self.active];
        ui.label(RichText::new("Comments").strong());
        ui.add_space(6.0);
        let mut gone = None;
        let mut any = false;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for (index, comment) in tab.doc.comments.iter().enumerate() {
                let note = u16::try_from(index + 1).unwrap_or(0);
                // A comment whose words were all taken out is not shown.
                let Some(about) = tab.doc.noted(note) else {
                    continue;
                };
                any = true;
                Frame::new()
                    .fill(palette.surface)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(&comment.author).strong().size(12.5));
                        ui.label(&comment.text);
                        ui.horizontal(|ui| {
                            if ui.small_button("Show").clicked() {
                                clicked.push(Cmd::Goto(about.start));
                            }
                            if ui.small_button("Delete").clicked() {
                                gone = Some(note);
                            }
                        });
                    });
                ui.add_space(6.0);
            }
            if !any {
                let hint = "Select words, then Review > New comment.";
                ui.label(RichText::new(hint).color(palette.text_dim));
            }
        });
        if let Some(note) = gone {
            tab.checkpoint(tab.doc.clone());
            tab.doc.restyle(0..tab.doc.len(), |format| {
                if format.note == note {
                    format.note = 0;
                }
            });
            tab.touched();
        }
    }

    /// The measures of the paragraphs of the selection, to type or drag: indents, the first
    /// line, the room above and under, the height of the lines.
    fn paragraph_window(&mut self, ctx: &Context, clicked: &mut Vec<Cmd>) {
        if !self.paragraph_open || self.home {
            return;
        }
        let para = self.tabs[self.active].para();
        let edit = |edit: Edit| Cmd::Edit(edit);
        self.paragraph_open = dialog(ctx, "Paragraph", |ui| {
            let heading = |ui: &mut Ui, name: &str| {
                ui.add_space(6.0);
                ui.label(RichText::new(name).strong());
                ui.end_row();
            };
            let grid = egui::Grid::new("paragraph").num_columns(2);
            grid.spacing(vec2(28.0, 7.0)).show(ui, |ui| {
                heading(ui, "General");
                ui.label("Alignment");
                let aligns = [
                    ("Left", ParaAlign::Left),
                    ("Centered", ParaAlign::Center),
                    ("Right", ParaAlign::Right),
                    ("Justified", ParaAlign::Justify),
                ];
                let shown = aligns.iter().find(|(_, align)| *align == para.align);
                ComboBox::from_id_salt("para-align")
                    .selected_text(shown.map_or("Left", |(name, _)| name))
                    .show_ui(ui, |ui| {
                        for (name, align) in aligns {
                            if ui.selectable_label(align == para.align, name).clicked() {
                                clicked.push(edit(Edit::Align(align)));
                            }
                        }
                    });
                ui.end_row();

                heading(ui, "Indentation");
                ui.label("Left");
                if let Some(left) = length(ui, "left", para.left, 0.0..=20.0) {
                    clicked.push(edit(Edit::Left(left)));
                }
                ui.end_row();
                ui.label("Right");
                if let Some(right) = length(ui, "right", para.right, 0.0..=20.0) {
                    clicked.push(edit(Edit::Right(right)));
                }
                ui.end_row();
                ui.label("Special");
                let special = match para.first_line {
                    0 => "(none)",
                    first if first > 0 => "First line",
                    _ => "Hanging",
                };
                // The first line of a new special indent starts 1.25 cm away.
                let by = match para.first_line.unsigned_abs() {
                    0 => 709,
                    by => by,
                };
                let signed = |by: u16| i16::try_from(by).unwrap_or(i16::MAX);
                ComboBox::from_id_salt("para-special")
                    .selected_text(special)
                    .show_ui(ui, |ui| {
                        let choices = [
                            ("(none)", 0),
                            ("First line", signed(by)),
                            ("Hanging", -signed(by)),
                        ];
                        for (name, first) in choices {
                            if ui.selectable_label(name == special, name).clicked() {
                                clicked.push(edit(Edit::FirstLine(first)));
                            }
                        }
                    });
                ui.end_row();
                ui.label("By");
                ui.add_enabled_ui(para.first_line != 0, |ui| {
                    let shown = para.first_line.unsigned_abs();
                    if let Some(by) = length(ui, "by", shown, 0.0..=20.0) {
                        let by = signed(by);
                        let first = if para.first_line < 0 { -by } else { by };
                        clicked.push(edit(Edit::FirstLine(first)));
                    }
                });
                ui.end_row();

                heading(ui, "Spacing");
                ui.label("Before");
                if let Some(before) = points(ui, "before", para.before) {
                    clicked.push(edit(Edit::Before(before)));
                }
                ui.end_row();
                ui.label("After");
                if let Some(after) = points(ui, "after", para.after) {
                    clicked.push(edit(Edit::After(after)));
                }
                ui.end_row();
                ui.label("Line spacing");
                let lines = para.line_factor();
                let now = (lines * 100.0).round() as u16;
                let named = LINE_SPACINGS.iter().find(|(_, line)| *line == now);
                ComboBox::from_id_salt("para-lines")
                    .selected_text(named.map_or("Multiple", |(name, _)| name))
                    .show_ui(ui, |ui| {
                        for (name, line) in LINE_SPACINGS {
                            if ui.selectable_label(line == now, name).clicked() {
                                clicked.push(edit(Edit::Line(line)));
                            }
                        }
                    });
                ui.end_row();
                ui.label("At");
                if let Some(at) = number(ui, "lines", lines, 0.5..=5.0, 0.01, " lines") {
                    clicked.push(edit(Edit::Line((at * 100.0).round() as u16)));
                }
                ui.end_row();
            });
            ui.add_space(4.0);
        });
    }

    /// The page of the document in front: each margin, the paper and the way it is turned.
    fn page_window(&mut self, ctx: &Context, clicked: &mut Vec<Cmd>) {
        if !self.page_open || self.home {
            return;
        }
        let page = self.tabs[self.active].doc.page;
        let sides = page
            .margin_points()
            .map(|side| (side * goddoc_core::TWIPS_PER_POINT).round() as u16);
        self.page_open = dialog(ctx, "Page setup", |ui| {
            let grid = egui::Grid::new("page-setup").num_columns(2);
            grid.spacing(vec2(28.0, 7.0)).show(ui, |ui| {
                ui.label(RichText::new("Margins").strong());
                ui.end_row();
                let names = ["Top", "Right", "Bottom", "Left"];
                // In the order one reads a page: top, bottom, left, right.
                for index in [0, 2, 3, 1] {
                    ui.label(names[index]);
                    if let Some(side) = length(ui, names[index], sides[index], 0.0..=10.0) {
                        let mut sides = sides;
                        sides[index] = side;
                        clicked.push(Cmd::Margins(sides));
                    }
                    ui.end_row();
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Paper").strong());
                ui.end_row();
                ui.label("Size");
                ComboBox::from_id_salt("page-paper")
                    .selected_text(page.paper.name())
                    .show_ui(ui, |ui| {
                        for paper in Paper::ALL {
                            if ui
                                .selectable_label(page.paper == paper, paper.name())
                                .clicked()
                            {
                                clicked.push(Cmd::Paper(paper));
                            }
                        }
                    });
                ui.end_row();
                ui.label("Orientation");
                ui.horizontal(|ui| {
                    for (name, landscape) in [("Portrait", false), ("Landscape", true)] {
                        if ui
                            .selectable_label(page.landscape == landscape, name)
                            .clicked()
                        {
                            clicked.push(Cmd::Landscape(landscape));
                        }
                    }
                });
                ui.end_row();
            });
            ui.add_space(4.0);
        });
    }

    /// Print the document in front, or (with a path) write it as a PDF there. The pages are
    /// drawn away from the window, which stays free; [`Self::finish_output`] says how it went.
    fn output(&mut self, ctx: &Context, pdf: Option<PathBuf>) {
        if self.working.is_some() {
            self.toasts
                .error("Still busy with the last print or export");
            return;
        }
        let tab = &self.tabs[self.active];
        let (doc, name) = (tab.doc.clone(), tab.name.clone());
        let (fonts, ctx) = (self.fonts.clone(), ctx.clone());
        let (done, result) = std::sync::mpsc::channel();
        let work = move || {
            let said = match pdf {
                Some(path) => {
                    let pages = crate::render::render(&doc, fonts, PDF_DPI);
                    let written = crate::pdf::write(&pages, &path);
                    written.map(|()| Some(format!("Exported {}", file_name(&path))))
                }
                None => print_pages(&name, &doc, fonts),
            };
            // The window may be closed by now: then nobody listens.
            let _ = done.send(said);
            ctx.request_repaint();
        };
        match std::thread::Builder::new()
            .name("pages".to_owned())
            .spawn(work)
        {
            Ok(_) => {
                self.working = Some(result);
                self.toasts.success("Drawing the pages…");
            }
            Err(err) => self.toasts.error(format!("Cannot start: {err}")),
        }
    }

    /// Say how the print or the export under way went, once it is over.
    fn finish_output(&mut self) {
        let Some(result) = &self.working else {
            return;
        };
        let said = match result.try_recv() {
            Ok(said) => said,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("Drawing the pages failed".to_owned())
            }
        };
        self.working = None;
        match said {
            Ok(Some(done)) => self.toasts.success(done),
            Ok(None) => {}
            Err(err) => self.toasts.error(err),
        }
    }

    /// Get the dictionary of the language in use, away from the window.
    fn load_spelling(&mut self, ctx: &Context) {
        let language = self.language;
        let ready = self
            .spell
            .as_ref()
            .is_some_and(|speller| speller.language == language);
        if ready || self.spell_loading.is_some() {
            return;
        }
        let (done, result) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let work = move || {
            let _ = done.send(crate::spell::load(language));
            ctx.request_repaint();
        };
        match std::thread::Builder::new()
            .name("spelling".to_owned())
            .spawn(work)
        {
            Ok(_) => {
                self.spell_loading = Some(result);
                self.toasts
                    .success(format!("Getting the {} dictionary…", language.name()));
            }
            Err(err) => self.toasts.error(format!("Cannot start: {err}")),
        }
    }

    /// Take the dictionary once it is there.
    fn finish_spelling(&mut self) {
        let Some(result) = &self.spell_loading else {
            return;
        };
        let loaded = match result.try_recv() {
            Ok(loaded) => loaded,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("Reading the dictionary failed".to_owned())
            }
        };
        self.spell_loading = None;
        match loaded {
            // Another language was picked while this one was coming.
            Ok(speller) if speller.language != self.language => {}
            Ok(speller) => {
                self.toasts
                    .success(format!("Spelling: {}", speller.language.name()));
                self.spell = Some(speller);
            }
            Err(err) => {
                self.spelling = false;
                self.toasts.error(err);
            }
        }
    }

    /// The ruler over the page: its margins, in centimetres from the left one, and the
    /// indents of the paragraph the caret is in, to drag.
    fn ruler(&self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let palette = Palette::of(ui);
        let tab = &self.tabs[self.active];
        let (para, page, zoom) = (tab.para(), tab.doc.page, self.zoom);
        let size = vec2(ui.available_width(), RULER_HEIGHT);
        let (strip, _) = ui.allocate_exact_size(size, Sense::hover());
        let sheet = layout::page_size(page, zoom);
        let [_, right_margin, _, left_margin] = layout::margins(page, zoom);
        let painter = ui.painter_at(strip);
        painter.rect_filled(strip, 0.0, palette.bg);
        // The sheet, darker where its margins are.
        let left = strip.center().x - sheet.x / 2.0;
        let paper = Rect::from_min_max(
            pos2(left, strip.top() + 3.0),
            pos2(left + sheet.x, strip.bottom() - 3.0),
        );
        painter.rect_filled(paper, CornerRadius::same(2), palette.border);
        let text = Rect::from_min_max(
            pos2(paper.left() + left_margin, paper.top()),
            pos2(paper.right() - right_margin, paper.bottom()),
        );
        painter.rect_filled(text, 0.0, palette.surface);
        // A mark each quarter of a centimetre, a number each centimetre.
        let quarter = TWIPS_PER_CM / goddoc_core::TWIPS_PER_POINT * zoom / 4.0;
        let middle = strip.center().y;
        let mut mark = -((text.left() - paper.left()) / quarter).floor() as i32;
        loop {
            let x = text.left() + mark as f32 * quarter;
            if x > paper.right() {
                break;
            }
            if mark % 4 != 0 {
                let half = if mark % 2 == 0 { 3.0 } else { 1.5 };
                let stroke = Stroke::new(1.0, palette.text_dim);
                painter.vline(x, middle - half..=middle + half, stroke);
            } else if mark != 0 {
                let number = (mark / 4).abs().to_string();
                let font = FontId::proportional(9.5);
                painter.text(
                    pos2(x, middle),
                    Align2::CENTER_CENTER,
                    number,
                    font,
                    palette.text_dim,
                );
            }
            mark += 1;
        }
        // In a table, indents are counted from the cell: the ruler does not show them.
        if para.cell.is_some() {
            return;
        }
        // Screen points per twentieth of a point, and the step indents snap to.
        let twip = zoom / goddoc_core::TWIPS_PER_POINT;
        let step = TWIPS_PER_CM / 4.0;
        let snapped = |length: f32| ((length / twip / step).round() * step).clamp(0.0, 11000.0);
        let hanging = f32::from(para.first_line.min(0).unsigned_abs()) * twip;
        let start = text.left() + layout::level_room(&para, zoom) + f32::from(para.left) * twip;
        let first = start + f32::from(para.first_line.max(0)) * twip;
        let end = text.right() - f32::from(para.right) * twip;
        let level = start - f32::from(para.left) * twip;
        let markers: [Marker<'_>; 3] = [
            ("First line indent", first, true, &|x| {
                Edit::FirstLine(snapped(x - start) as i16)
            }),
            ("Left indent", start + hanging, false, &|x| {
                Edit::Left(snapped(x - hanging - level) as u16)
            }),
            ("Right indent", end, false, &|x| {
                Edit::Right(snapped(text.right() - x) as u16)
            }),
        ];
        for (name, at, top, moved) in markers {
            let y = if top {
                strip.top() + 6.0
            } else {
                strip.bottom() - 6.0
            };
            let hit = Rect::from_center_size(pos2(at, y), vec2(12.0, 11.0));
            let response = ui
                .interact(hit, ui.id().with(("ruler", name)), Sense::drag())
                .on_hover_text(name)
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            let id = response.id;
            // While it is dragged, the marker follows the pointer; the paragraph moves once it
            // is let go, in one step.
            let mut shown = at;
            if let Some(pointer) = response
                .interact_pointer_pos()
                .filter(|_| response.dragged())
            {
                shown = pointer.x.clamp(paper.left(), paper.right());
                ui.data_mut(|data| data.insert_temp(id, shown));
                let line = Stroke::new(1.0, palette.text_dim);
                ui.ctx().layer_painter(ui.layer_id()).vline(
                    shown,
                    strip.bottom()..=ui.clip_rect().bottom(),
                    line,
                );
            } else if let Some(dropped) = ui.data_mut(|data| data.remove_temp::<f32>(id)) {
                clicked.push(Cmd::Edit(moved(dropped)));
            }
            let tip = if top { 4.0 } else { -4.0 };
            let shape = vec![
                pos2(shown - 5.0, y - tip),
                pos2(shown + 5.0, y - tip),
                pos2(shown, y + tip),
            ];
            painter.add(egui::Shape::convex_polygon(
                shape,
                palette.text,
                Stroke::NONE,
            ));
        }
    }

    /// What the program is told once and for all: who writes, how large it shows, and
    /// whether it puts right what is typed.
    fn options_window(&mut self, ctx: &Context) {
        if !self.options_open {
            return;
        }
        let (author, autocorrect) = (&mut self.author, &mut self.autocorrect);
        let mut scale = None;
        let now = self.scale;
        self.options_open = dialog(ctx, "Options", |ui| {
            let grid = egui::Grid::new("options").num_columns(2);
            grid.spacing(vec2(28.0, 9.0)).show(ui, |ui| {
                ui.label("Your name");
                let field = TextEdit::singleline(author)
                    .hint_text("On your comments")
                    .desired_width(200.0)
                    .margin(widgets::FIELD_MARGIN);
                ui.add(field);
                ui.end_row();
                ui.label("Interface size");
                scale = number(ui, "scale", now * 100.0, 80.0..=160.0, 1.0, " %");
                ui.end_row();
                ui.label("AutoCorrect");
                ui.checkbox(autocorrect, "Quotes, dashes and signs as you type");
                ui.end_row();
            });
        });
        if let Some(scale) = scale {
            self.scale = (scale / 100.0).clamp(0.8, 1.6);
            ctx.set_zoom_factor(self.scale);
        }
    }

    /// Which page to go to.
    fn goto_window(&mut self, ctx: &Context, clicked: &mut Vec<Cmd>) {
        if !self.goto_open || self.home {
            return;
        }
        let tab = &self.tabs[self.active];
        let pages = tab.pages.max(1);
        let page = &mut self.goto_page;
        *page = (*page).clamp(1, pages);
        let mut go = false;
        self.goto_open = dialog(ctx, "Go to page", |ui| {
            ui.horizontal(|ui| {
                ui.label("Page");
                let field = egui::DragValue::new(page).range(1..=pages).speed(0.1);
                let typed = ui.add_sized(vec2(70.0, 24.0), field);
                ui.label(format!("of {pages}"));
                go = typed.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            });
            ui.add_space(8.0);
            go |= ui.button("Go").clicked();
        });
        if !go {
            return;
        }
        // The first paragraph that starts on the page, or after it.
        let wanted = self.goto_page - 1;
        let para = tab.para_pages.iter().position(|page| *page >= wanted);
        let lines = tab.doc.text().split('\n').take(para.unwrap_or(0));
        let at = lines.map(|line| line.chars().count() + 1).sum();
        clicked.push(Cmd::Goto(at));
        self.goto_open = false;
    }

    /// Words, characters and paragraphs of the document in front, in a small window.
    fn word_count(&mut self, ctx: &Context) {
        if !self.counting || self.home {
            return;
        }
        let tab = &self.tabs[self.active];
        let text = tab.doc.text();
        let letters = text.chars().filter(|letter| *letter != '\n');
        let spaces = letters
            .clone()
            .filter(|letter| letter.is_whitespace())
            .count();
        let characters = letters.count();
        let paragraphs = text.split('\n').filter(|line| !line.trim().is_empty());
        let counts = [
            ("Pages", tab.pages),
            ("Words", tab.doc.word_count()),
            ("Characters (no spaces)", characters - spaces),
            ("Characters (with spaces)", characters),
            ("Paragraphs", paragraphs.count()),
        ];
        self.counting = dialog(ctx, "Word count", |ui| {
            egui::Grid::new("word-count")
                .num_columns(2)
                .spacing(vec2(36.0, 8.0))
                .show(ui, |ui| {
                    for (what, count) in counts {
                        ui.label(what);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(count.to_string()).strong());
                        });
                        ui.end_row();
                    }
                });
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
                    Kind::Normal | Kind::Subtitle | Kind::Quote => continue,
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
        self.view_size = ui.available_size();
        layout::set_dark_page(self.dark_page);
        if self.show_ruler && !self.focus {
            self.ruler(ui, clicked);
        }
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
        // Spelling: the words to mark, looked up again only when the text changed; and other
        // spellings for the one the caret is in, for the right-click menu.
        let spelling = self.spelling;
        match self.spell.as_mut().filter(|_| spelling) {
            Some(speller) => {
                use std::hash::{Hash as _, Hasher as _};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                tab.doc.text().hash(&mut hasher);
                let stamp = hasher.finish() | 1;
                if stamp != tab.spelled {
                    tab.spelled = stamp;
                    tab.view.misspelled = speller.misspelled(tab.doc.text());
                    self.suggested = None;
                }
                let caret = tab.selection.start;
                let inside = |word: &&Range<usize>| word.start <= caret && caret <= word.end;
                let word = tab.view.misspelled.iter().find(inside).cloned();
                match word {
                    Some(word)
                        if self
                            .suggested
                            .as_ref()
                            .is_none_or(|(known, _)| *known != word) =>
                    {
                        let letters = tab.doc.text().chars().skip(word.start).take(word.len());
                        let others = speller.suggest(&letters.collect::<String>());
                        self.suggested = Some((word, others));
                    }
                    Some(_) => {}
                    None => self.suggested = None,
                }
            }
            None => {
                tab.spelled = 0;
                tab.view.misspelled.clear();
                self.suggested = None;
            }
        }
        let suggestions = self.suggested.as_ref().map(|(_, words)| words.clone());
        let out = editor::show(
            ui,
            tab.editor_id(),
            &mut tab.doc,
            typing,
            self.zoom,
            &mut tab.view,
            editor::Hooks {
                textures: &mut |ctx, image| texture_of(textures, ctx, image),
                menu: &mut |ui| {
                    quick_menu(
                        ui,
                        &palette,
                        format,
                        in_table,
                        picture,
                        suggestions.as_deref(),
                        clicked,
                    )
                },
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
        let mut selection = out.selection;
        // AutoCorrect: what one typed letter completes is put right.
        if self.autocorrect
            && !out.structural
            && let Some(before) = &out.before
            && tab.doc.len() == before.len() + 1
            && let Some(caret) = selection.as_ref().filter(|range| range.is_empty())
        {
            let french = self.language == crate::spell::Language::French;
            if let Some(caret) = autocorrect(&mut tab.doc, caret.start, french) {
                selection = Some(caret..caret);
                set_cursor(ui.ctx(), tab.editor_id(), &(caret..caret));
            }
        }
        if let Some(before) = &out.before
            && tab.tracking
            && !out.structural
        {
            let after = selection
                .as_ref()
                .map_or(tab.selection.start, |range| range.start);
            if let Some(caret) = track(before, &mut tab.doc, tab.selection.start, after) {
                selection = Some(caret..caret);
                set_cursor(ui.ctx(), tab.editor_id(), &(caret..caret));
            }
        }
        // Ctrl+click on a link opens it.
        let clicked_link =
            ui.input(|input| input.modifiers.command && input.pointer.primary_released());
        if clicked_link && let Some(caret) = selection.as_ref().filter(|range| range.is_empty()) {
            let link = tab
                .doc
                .formats()
                .get(caret.start)
                .map_or(0, |format| format.link);
            if let Some(url) = tab.doc.link(link)
                && let Err(err) = platform::open_url(url)
            {
                self.toasts.error(err);
            }
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
        if let Some(selection) = selection
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
        crate::window::track(ui.ctx());
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
            let tools = !self.focus;
            if tools {
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
            }
            if tools && self.find.open {
                Panel::top("find-bar")
                    .frame(
                        Frame::new()
                            .fill(palette.surface)
                            .inner_margin(Margin::symmetric(12, 5)),
                    )
                    .show(ui, |ui| self.find_bar(ui, &mut clicked));
            }
            if tools {
                Panel::bottom("status")
                    .frame(
                        Frame::new()
                            .fill(palette.bg)
                            .inner_margin(Margin::symmetric(12, 3)),
                    )
                    .show(ui, |ui| self.status_bar(ui, &mut clicked));
            }
            if tools && self.show_outline {
                Panel::left("outline")
                    .resizable(true)
                    .default_size(220.0)
                    .size_range(160.0..=380.0)
                    .frame(Frame::new().fill(palette.bg).inner_margin(Margin::same(10)))
                    .show(ui, |ui| self.outline(ui, &mut clicked));
            }
            if tools && self.show_comments {
                Panel::right("comments")
                    .resizable(true)
                    .default_size(240.0)
                    .size_range(180.0..=400.0)
                    .frame(Frame::new().fill(palette.bg).inner_margin(Margin::same(10)))
                    .show(ui, |ui| self.comments_pane(ui, &mut clicked));
            }
            CentralPanel::default()
                .frame(Frame::new().fill(palette.canvas))
                .show(ui, |ui| self.page(ui, &mut clicked));
        }

        self.finish_output();
        self.finish_spelling();
        self.options_window(&ctx);
        self.goto_window(&ctx, &mut clicked);
        self.banner_window(&ctx);
        self.link_window(&ctx);
        self.comment_window(&ctx);
        self.paragraph_window(&ctx, &mut clicked);
        self.page_window(&ctx, &mut clicked);
        for cmd in clicked {
            self.run(&ctx, cmd);
        }
        self.closing_dialog(&ctx);
        self.word_count(&ctx);
        self.updates.show(&ctx);
        self.toasts.show(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        crate::window::save();
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
        let flag = |on: bool| if on { "1" } else { "0" }.to_owned();
        storage.set_string("ruler", flag(self.show_ruler));
        storage.set_string("dark_page", flag(self.dark_page));
        storage.set_string("autocorrect", flag(self.autocorrect));
        storage.set_string("author", self.author.clone());
        storage.set_string("scale", self.scale.to_string());
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
    let size = vec2((galley.size().x + 14.0).max(50.0), TOOL_HEIGHT);
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

/// Twentieths of a point in a centimetre.
const TWIPS_PER_CM: f32 = 566.93;
/// Line spacings with a name, in hundredths of single spacing.
const LINE_SPACINGS: [(&str, u16); 4] = [
    ("Single", 100),
    ("1.15 lines", 115),
    ("1.5 lines", 150),
    ("Double", 200),
];

/// A number to type or to drag. Gives the new one once it is settled: at the end of a
/// drag, not at each of its steps, so that undoing takes it back in one go.
fn number(
    ui: &mut Ui,
    salt: &str,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
    unit: &str,
) -> Option<f32> {
    let id = ui.id().with(("measure", salt));
    // While it is dragged, the number shown is the one of the drag.
    let mut shown = ui.data(|data| data.get_temp::<f32>(id)).unwrap_or(value);
    let field = egui::DragValue::new(&mut shown)
        .range(range)
        .speed(speed)
        .max_decimals(2)
        .suffix(unit);
    // As tall as the other tools around it.
    let size = vec2(84.0, ui.spacing().interact_size.y.max(18.0));
    let response = ui.add_sized(size, field);
    if response.dragged() {
        ui.data_mut(|data| data.insert_temp(id, shown));
        return None;
    }
    let dragged = ui.data_mut(|data| data.remove_temp::<f32>(id));
    let settled = dragged.or(response.changed().then_some(shown));
    settled.filter(|settled| (settled - value).abs() > f32::EPSILON)
}

/// A length to type or drag, in centimetres; it is kept in twentieths of a point.
fn length(
    ui: &mut Ui,
    salt: &str,
    twips: u16,
    range: std::ops::RangeInclusive<f32>,
) -> Option<u16> {
    let shown = (f32::from(twips) / TWIPS_PER_CM * 100.0).round() / 100.0;
    number(ui, salt, shown, range, 0.02, " cm").map(|cm| (cm * TWIPS_PER_CM).round() as u16)
}

/// A room above or under a paragraph to type or drag, in points.
fn points(ui: &mut Ui, salt: &str, twips: u16) -> Option<u16> {
    let shown = f32::from(twips) / goddoc_core::TWIPS_PER_POINT;
    let typed = number(ui, salt, shown, 0.0..=200.0, 0.2, " pt");
    typed.map(|points| (points * goddoc_core::TWIPS_PER_POINT).round() as u16)
}

/// Dots per inch the pages of a PDF are drawn at, and those sent to a printer.
const PDF_DPI: f32 = 200.0;
#[cfg(windows)]
const PRINT_DPI: f32 = 300.0;

/// Print `doc` through the print dialog of the system. `None` when it was called off.
#[cfg(windows)]
fn print_pages(
    name: &str,
    doc: &Document,
    fonts: egui::FontDefinitions,
) -> Result<Option<String>, String> {
    let pages = crate::render::render(doc, fonts, PRINT_DPI);
    let printed = crate::print::print(name, pages.pages, |page| pages.page(page));
    printed.map(|said| said.map(str::to_owned))
}

#[cfg(not(windows))]
fn print_pages(
    _name: &str,
    _doc: &Document,
    _fonts: egui::FontDefinitions,
) -> Result<Option<String>, String> {
    Err("Printing is only there on Windows for now: export a PDF and print it".to_owned())
}

/// A small window over the document, which greys what is behind it. Gives false once it is
/// closed: by its button, by Esc or by a click beside it.
fn dialog(ctx: &Context, title: &str, content: impl FnOnce(&mut Ui)) -> bool {
    let mut open = true;
    let response = egui::Modal::new(Id::new(("dialog", title))).show(ctx, |ui| {
        ui.heading(title);
        ui.add_space(8.0);
        content(ui);
        ui.add_space(10.0);
        if ui.button("Close").clicked() {
            open = false;
        }
    });
    open && !response.should_close()
}

/// Colour of the words of a link.
const LINK_COLOR: [u8; 3] = [0x05, 0x63, 0xC1];

/// Mark as tracked what an edit made of `doc` (it was `before`, the caret at `caret`): what it
/// put in is marked as put in, and what it took out comes back, marked as taken out. Gives
/// where the caret goes when text came back.
fn track(before: &Document, doc: &mut Document, caret: usize, after: usize) -> Option<usize> {
    let old: Vec<char> = before.text().chars().collect();
    let new: Vec<char> = doc.text().chars().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let longest = old.len().min(new.len()) - prefix;
    let tails = old.iter().rev().zip(new.iter().rev());
    let suffix = tails.take_while(|(a, b)| a == b).count().min(longest);
    let removed = prefix..old.len() - suffix;
    let added = prefix..new.len() - suffix;
    doc.restyle(added.clone(), |format| format.change = Change::Inserted);
    // A paragraph taken out is gone for good: what it was like cannot come back with it.
    if removed.is_empty() || old[removed.clone()].contains(&'\n') {
        return None;
    }
    let mut at = prefix;
    let mut start = removed.start;
    for run in before.formats()[removed].chunk_by(|a, b| a == b) {
        let text: String = old[start..start + run.len()].iter().collect();
        start += run.len();
        // What had only just been put in goes for good.
        if run[0].change != Change::Inserted {
            let format = Format {
                change: Change::Deleted,
                ..run[0]
            };
            doc.replace(at..at, &text, format);
            at += run.len();
        }
    }
    let back = at - prefix;
    // Delete took what follows the caret: the caret goes past it. Backspace took what is
    // before: it stays in front. Typing over a selection: it stays after what was typed.
    let forward = caret == prefix && added.is_empty();
    let moved = if forward || after > prefix { back } else { 0 };
    (back > 0).then_some(if forward {
        prefix + back
    } else {
        after + moved
    })
}

/// Height of the ruler over the page.
const RULER_HEIGHT: f32 = 22.0;
/// A marker of the ruler: its name, where it stands, whether it hangs from the top, and the
/// change it makes when dropped at a place.
type Marker<'a> = (&'a str, f32, bool, &'a dyn Fn(f32) -> Edit);

/// Put right what the letter just typed before `caret` completes: quotes that open and
/// close, a dash for two hyphens, the sign for "(c)". Gives where the caret goes then.
fn autocorrect(doc: &mut Document, caret: usize, french: bool) -> Option<usize> {
    let from = caret.saturating_sub(5);
    let tail: Vec<char> = doc.text().chars().skip(from).take(caret - from).collect();
    let typed = *tail.last()?;
    let ends = |with: &str| tail.ends_with(&with.chars().collect::<Vec<char>>());
    // A quote opens at the start of a line, after a space or an opening sign.
    let before = tail.len().checked_sub(2).map(|at| tail[at]);
    let opens = before.is_none_or(|ch| ch.is_whitespace() || "([{\u{2014}\u{ab}".contains(ch));
    // A fraction stands alone: "11/2" is not eleven halves.
    let alone = |letters: usize| {
        let ahead = tail.len().checked_sub(letters + 1).map(|at| tail[at]);
        ahead.is_none_or(|ch| !ch.is_ascii_digit())
    };
    let (taken, put) = match typed {
        '"' if french && opens => (1, "\u{ab}\u{a0}"),
        '"' if french => (1, "\u{a0}\u{bb}"),
        '"' if opens => (1, "\u{201c}"),
        '"' => (1, "\u{201d}"),
        '\'' if opens => (1, "\u{2018}"),
        '\'' => (1, "\u{2019}"),
        '-' if ends("--") => (2, "\u{2014}"),
        '-' if ends("<-") => (2, "\u{2190}"),
        '>' if ends("->") => (2, "\u{2192}"),
        '.' if ends("...") => (3, "\u{2026}"),
        ')' if ends("(c)") => (3, "\u{a9}"),
        ')' if ends("(r)") => (3, "\u{ae}"),
        ')' if ends("(tm)") => (4, "\u{2122}"),
        '2' if ends("1/2") && alone(3) => (3, "\u{bd}"),
        '4' if ends("1/4") && alone(3) => (3, "\u{bc}"),
        '4' if ends("3/4") && alone(3) => (3, "\u{be}"),
        _ => return None,
    };
    let format = doc.formats().get(caret - 1).copied()?;
    doc.replace(caret - taken..caret, put, format);
    Some(caret - taken + put.chars().count())
}

/// A named group of large ribbon buttons, side by side.
fn tools(ui: &mut Ui, palette: &Palette, name: &str, buttons: impl FnOnce(&mut Ui)) {
    group(ui, palette, name, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            buttons(ui);
        });
    });
}

/// A named group of the ribbon: its tools, its name under them, a line after it.
fn group(ui: &mut Ui, palette: &Palette, name: &str, tools: impl FnOnce(&mut Ui)) {
    let height = GROUP_TOOLS + GROUP_NAME;
    let name = ui.painter().layout_no_wrap(
        name.to_owned(),
        FontId::proportional(11.0),
        palette.text_dim,
    );
    let inner = ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
        ui.set_height(height);
        // A group is at least as wide as its name.
        ui.set_min_width(name.size().x + 8.0);
        tools(ui);
    });
    let rect = inner.response.rect;
    let at = pos2(
        rect.center().x - name.size().x / 2.0,
        rect.max.y - name.size().y,
    );
    ui.painter().galley(at, name, palette.text_dim);
    let (line, _) = ui.allocate_exact_size(vec2(13.0, height - 10.0), Sense::hover());
    ui.painter().vline(
        line.center().x,
        line.y_range(),
        Stroke::new(1.0, palette.border),
    );
}

/// Small ribbon button: an icon beside its label; `on` shows it pressed.
fn mini_tool(ui: &mut Ui, glyph: &str, label: &str, tip: &str, on: bool) -> Response {
    let text = RichText::new(with_icon(glyph, label)).size(12.5);
    let button = Button::selectable(on, text)
        .frame_when_inactive(on)
        .stroke(Stroke::NONE)
        .min_size(vec2(0.0, 18.0));
    // Three of them stand in the height of a group.
    let added = ui.scope(|ui| {
        ui.spacing_mut().button_padding = vec2(4.0, 0.0);
        ui.spacing_mut().interact_size.y = 18.0;
        ui.add(button)
    });
    added.inner.on_hover_text(tip)
}

/// A tool that opens colours to pick from: its glyph over a bar of the colour in use
/// (`None`: no colour), a small arrow beside them.
fn color_tool(
    ui: &mut Ui,
    palette: &Palette,
    glyph: &str,
    tip: &str,
    shown: Option<[u8; 3]>,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(38.0, 28.0), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), palette.hover);
    }
    let middle = pos2(rect.min.x + 14.0, rect.center().y);
    painter.text(
        middle - vec2(0.0, 3.0),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(15.0),
        palette.text,
    );
    let bar = Rect::from_center_size(middle + vec2(0.0, 9.0), vec2(16.0, 3.5));
    match shown {
        Some([r, g, b]) => {
            painter.rect_filled(bar, CornerRadius::same(1), Color32::from_rgb(r, g, b))
        }
        None => painter.rect_stroke(
            bar,
            CornerRadius::same(1),
            Stroke::new(1.0, palette.border),
            egui::StrokeKind::Inside,
        ),
    };
    painter.text(
        pos2(rect.max.x - 8.0, rect.center().y),
        Align2::CENTER_CENTER,
        icon::CARET_DOWN,
        FontId::proportional(9.0),
        palette.text_dim,
    );
    response.on_hover_text(tip)
}

/// A paragraph style to pick: a few letters as it writes them, on paper, over its name.
fn style_card(ui: &mut Ui, palette: &Palette, kind: Kind, on: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(73.0, GROUP_TOOLS), Sense::click());
    let painter = ui.painter().with_clip_rect(rect);
    let edge = if on {
        Stroke::new(1.5, palette.text)
    } else if response.hovered() {
        Stroke::new(1.0, palette.text_dim)
    } else {
        Stroke::new(1.0, palette.border)
    };
    painter.rect(
        rect.shrink(1.0),
        CornerRadius::same(5),
        Color32::WHITE,
        edge,
        egui::StrokeKind::Inside,
    );
    let look = kind.format();
    let [r, g, b] = look.color;
    let lay = |size: f32| {
        let mut job = egui::text::LayoutJob::default();
        job.append(
            "AaBbCc",
            0.0,
            egui::TextFormat {
                font_id: FontId::new(size, layout::family(look.font, look.bold)),
                color: Color32::from_rgb(r, g, b),
                italics: look.italic,
                ..Default::default()
            },
        );
        ui.painter().layout_job(job)
    };
    // As large as the style writes, as long as the letters stay on the card.
    let (size, room) = (look.size.clamp(11.0, 19.0), rect.width() - 14.0);
    let mut letters = lay(size);
    if letters.size().x > room {
        letters = lay(size * room / letters.size().x);
    }
    let at = pos2(rect.min.x + 7.0, rect.min.y + 23.0 - letters.size().y / 2.0);
    painter.galley(at, letters, Color32::BLACK);
    painter.text(
        pos2(rect.center().x, rect.max.y - 6.0),
        Align2::CENTER_BOTTOM,
        kind.name(),
        FontId::proportional(10.5),
        Color32::from_gray(0x40),
    );
    response
        .on_hover_text(format!("Style: {}", kind.name()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
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
    suggestions: Option<&[String]>,
    clicked: &mut Vec<Cmd>,
) {
    ui.set_min_width(220.0);
    // The caret is in a word the dictionary does not know: other spellings come first.
    if let Some(words) = suggestions {
        for (choice, word) in words.iter().enumerate() {
            if ui.button(RichText::new(word).strong()).clicked() {
                clicked.push(Cmd::Suggest(choice));
            }
        }
        if words.is_empty() {
            ui.label(RichText::new("No other spelling").color(palette.text_dim));
        }
        if ui.button("Ignore this word").clicked() {
            clicked.push(Cmd::IgnoreWord);
        }
        ui.separator();
    }
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
    let clear = Cmd::Edit(Edit::ClearFormat);
    let edits = [
        (icon::SCISSORS, "Cut (Ctrl+X)", Cmd::Cut, false),
        (icon::COPY, "Copy (Ctrl+C)", Cmd::Copy, false),
        (icon::CLIPBOARD, "Paste (Ctrl+V)", Cmd::Paste, false),
        (
            icon::SELECTION_ALL,
            "Select all (Ctrl+A)",
            Cmd::SelectAll,
            false,
        ),
        (icon::ERASER, "Clear formatting", clear, false),
    ];
    menu_icons(ui, clicked, &edits);
    if in_table {
        ui.separator();
        let table = [
            (
                icon::ROWS_PLUS_TOP,
                "Insert row above",
                Cmd::TableRow(false),
                false,
            ),
            (
                icon::ROWS_PLUS_BOTTOM,
                "Insert row below",
                Cmd::TableRow(true),
                false,
            ),
            (
                icon::COLUMNS_PLUS_LEFT,
                "Insert column left",
                Cmd::TableCol(false),
                false,
            ),
            (
                icon::COLUMNS_PLUS_RIGHT,
                "Insert column right",
                Cmd::TableCol(true),
                false,
            ),
            (icon::ROWS, "Delete row", Cmd::DeleteLine(true), true),
            (icon::COLUMNS, "Delete column", Cmd::DeleteLine(false), true),
        ];
        menu_icons(ui, clicked, &table);
    }
    if picture {
        ui.separator();
        let image = [(icon::TRASH, "Delete image (Del)", Cmd::DeleteImage, true)];
        menu_icons(ui, clicked, &image);
    }
}

/// A row of icon buttons in a menu: (icon, what it does, its command, whether it deletes).
/// What each does shows when the pointer rests on it; a click queues its command.
fn menu_icons(ui: &mut Ui, clicked: &mut Vec<Cmd>, items: &[(&str, &str, Cmd, bool)]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (glyph, does, cmd, deletes) in items {
            let mut text = RichText::new(*glyph).size(16.0);
            if *deletes {
                text = text.color(ui.visuals().error_fg_color);
            }
            let button = Button::new(text)
                .frame_when_inactive(false)
                .stroke(egui::Stroke::NONE);
            let button = ui.add_sized(widgets::ICON_BUTTON, button);
            if button.on_hover_text(*does).clicked() {
                clicked.push(*cmd);
            }
        }
    });
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
pub fn texture_of(
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

#[cfg(test)]
mod tracking {
    use super::*;

    #[test]
    fn what_is_typed_is_put_right() {
        let typed = |text: &str, french: bool| {
            let mut doc = Document::default();
            doc.replace(0..0, text, Format::default());
            let caret = autocorrect(&mut doc, text.chars().count(), french);
            (doc.text().to_owned(), caret)
        };
        assert_eq!(typed("say \"", false), ("say \u{201c}".to_owned(), Some(5)));
        assert_eq!(typed("say \u{201c}hi\"", false).0, "say \u{201c}hi\u{201d}");
        assert_eq!(typed("dit \"", true).0, "dit \u{ab}\u{a0}");
        assert_eq!(typed("don'", false).0, "don\u{2019}");
        assert_eq!(typed("wait--", false), ("wait\u{2014}".to_owned(), Some(5)));
        assert_eq!(typed("and so...", false).0, "and so\u{2026}");
        assert_eq!(typed("(c)", false).0, "\u{a9}");
        assert_eq!(typed("a ->", false).0, "a \u{2192}");
        assert_eq!(typed("1/2", false).0, "\u{bd}");
        // Eleven halves stay as typed, and so does an ordinary letter.
        assert_eq!(typed("11/2", false), ("11/2".to_owned(), None));
        assert_eq!(typed("abc", false).1, None);
    }

    #[test]
    fn what_is_typed_and_taken_out_is_marked() {
        let mut before = Document::default();
        before.replace(0..0, "one two", Format::default());
        let change = |doc: &Document, at: usize| doc.formats()[at].change;

        // Backspace at the end: the letter comes back, marked, the caret in front of it.
        let mut doc = before.clone();
        doc.replace(6..7, "", Format::default());
        assert_eq!(track(&before, &mut doc, 7, 6), Some(6));
        assert_eq!((doc.text(), change(&doc, 6)), ("one two", Change::Deleted));

        // Delete at the start: the caret goes past what came back.
        let mut doc = before.clone();
        doc.replace(0..1, "", Format::default());
        assert_eq!(track(&before, &mut doc, 0, 0), Some(1));
        assert_eq!(change(&doc, 0), Change::Deleted);

        // Typing: marked as put in, nothing comes back.
        let mut doc = before.clone();
        doc.replace(3..3, "X", Format::default());
        assert_eq!(track(&before, &mut doc, 3, 4), None);
        assert_eq!(change(&doc, 3), Change::Inserted);

        // Typing over a selection: the old words stay, struck, before the new ones.
        let mut doc = before.clone();
        doc.replace(4..7, "X", Format::default());
        assert_eq!(track(&before, &mut doc, 4, 5), Some(8));
        assert_eq!(doc.text(), "one twoX");
        assert_eq!(
            (change(&doc, 4), change(&doc, 7)),
            (Change::Deleted, Change::Inserted)
        );

        // Taking out what was only just put in takes it out for good.
        let typed = doc.clone();
        doc.replace(7..8, "", Format::default());
        assert_eq!(track(&typed, &mut doc, 8, 7), None);
        assert_eq!(doc.text(), "one two");
    }
}
