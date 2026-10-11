//! GodDoc document model: plain text with one [`Format`] per character and one [`Para`] per
//! paragraph (paragraphs are separated by `\n`). All positions are character indices.
//!
//! A table is a run of consecutive paragraphs carrying a [`Cell`]: its cells in reading order
//! (row by row), each cell holding one or more paragraphs.

pub mod docx;

use std::ops::Range;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a Word document: {0}")]
    NotDocx(String),
    #[error("cannot write the document: {0}")]
    Write(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Font {
    #[default]
    Sans,
    Serif,
    Mono,
    Calibri,
    Verdana,
    Georgia,
    Tahoma,
    Comic,
}

impl Font {
    pub const ALL: [Self; 8] = [
        Self::Sans,
        Self::Serif,
        Self::Mono,
        Self::Calibri,
        Self::Verdana,
        Self::Georgia,
        Self::Tahoma,
        Self::Comic,
    ];

    /// Font family written into .docx files.
    pub fn family(self) -> &'static str {
        match self {
            Self::Sans => "Arial",
            Self::Serif => "Times New Roman",
            Self::Mono => "Courier New",
            Self::Calibri => "Calibri",
            Self::Verdana => "Verdana",
            Self::Georgia => "Georgia",
            Self::Tahoma => "Tahoma",
            Self::Comic => "Comic Sans MS",
        }
    }
}

/// Raised or lowered small text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Script {
    #[default]
    Normal,
    Super,
    Sub,
}

/// Look of one character.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Format {
    pub font: Font,
    /// Size in points.
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// RGB.
    pub color: [u8; 3],
    /// Background colour behind the text (highlighter), RGB.
    pub highlight: Option<[u8; 3]>,
    pub script: Script,
}

impl Default for Format {
    fn default() -> Self {
        Kind::Normal.format()
    }
}

/// Paragraph style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Normal,
    Title,
    Heading1,
    Heading2,
    Heading3,
}

impl Kind {
    pub const ALL: [Self; 5] = [
        Self::Normal,
        Self::Title,
        Self::Heading1,
        Self::Heading2,
        Self::Heading3,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Title => "Title",
            Self::Heading1 => "Heading 1",
            Self::Heading2 => "Heading 2",
            Self::Heading3 => "Heading 3",
        }
    }

    /// Size and weight of text in a paragraph of this kind.
    pub fn format(self) -> Format {
        let (size, bold) = match self {
            Self::Normal => (11.0, false),
            Self::Title => (26.0, true),
            Self::Heading1 => (20.0, true),
            Self::Heading2 => (16.0, true),
            Self::Heading3 => (13.0, true),
        };
        Format {
            font: Font::Sans,
            size,
            bold,
            italic: false,
            underline: false,
            strike: false,
            color: [0, 0, 0],
            highlight: None,
            script: Script::Normal,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum List {
    #[default]
    None,
    Bullet,
    Number,
}

/// Place of a paragraph in a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Identifies the table within the document.
    pub table: u32,
    pub row: u16,
    pub col: u16,
}

/// Distance between the lines of a paragraph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Spacing {
    #[default]
    Single,
    OneAndHalf,
    Double,
}

impl Spacing {
    pub const ALL: [Self; 3] = [Self::Single, Self::OneAndHalf, Self::Double];

    pub fn name(self) -> &'static str {
        match self {
            Self::Single => "Single",
            Self::OneAndHalf => "1.5 lines",
            Self::Double => "Double",
        }
    }

    /// Line height relative to single spacing.
    pub fn factor(self) -> f32 {
        match self {
            Self::Single => 1.0,
            Self::OneAndHalf => 1.5,
            Self::Double => 2.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Margins {
    #[default]
    Normal,
    Narrow,
    Wide,
}

impl Margins {
    pub const ALL: [Self; 3] = [Self::Normal, Self::Narrow, Self::Wide];

    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "Normal (2.5 cm)",
            Self::Narrow => "Narrow (1.3 cm)",
            Self::Wide => "Wide (3.8 cm)",
        }
    }

    /// Width of each margin, in points.
    pub fn points(self) -> f32 {
        match self {
            Self::Normal => 72.0,
            Self::Narrow => 36.0,
            Self::Wide => 108.0,
        }
    }
}

/// Paper (always A4) orientation and margins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageSetup {
    pub landscape: bool,
    pub margins: Margins,
}

impl PageSetup {
    /// (width, height) of a page, in points.
    pub fn size(self) -> (f32, f32) {
        if self.landscape {
            (842.0, 595.0)
        } else {
            (595.0, 842.0)
        }
    }
}

/// Deepest indent level of a paragraph.
pub const MAX_INDENT: u8 = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Para {
    pub kind: Kind,
    pub align: Align,
    pub list: List,
    /// The paragraph starts a new page.
    pub page_break: bool,
    pub cell: Option<Cell>,
    /// Picture shown above the paragraph's text (which then reads as its caption).
    pub picture: Option<Picture>,
    pub spacing: Spacing,
    /// Indent level, up to [`MAX_INDENT`].
    pub indent: u8,
    /// A line is drawn across the page under the paragraph.
    pub rule: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
}

impl ImageKind {
    /// Kind of a file from its extension.
    pub fn of_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
        }
    }
}

/// An image file embedded in the document.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub bytes: std::sync::Arc<[u8]>,
    pub kind: ImageKind,
}

/// An [`Image`] placed in a paragraph, at a size in points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picture {
    /// Index in [`Document::images`].
    pub image: u32,
    pub width: u16,
    pub height: u16,
}

/// Invariants: `formats` has one entry per character of `text` (line breaks included) and
/// `paras` one entry per paragraph (number of `\n` + 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    text: String,
    formats: Vec<Format>,
    paras: Vec<Para>,
    /// Images pictures refer to; only ever appended, so indices stay valid across undo.
    images: Vec<Image>,
    /// Show the page number at the bottom of every page.
    pub page_numbers: bool,
    pub page: PageSetup,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            text: String::new(),
            formats: Vec::new(),
            paras: vec![Para::default()],
            images: Vec::new(),
            page_numbers: false,
            page: PageSetup::default(),
        }
    }
}

/// One paragraph taken out of a document: (text, formats, paragraph).
type Block = (String, Vec<Format>, Para);

impl Document {
    pub fn images(&self) -> &[Image] {
        &self.images
    }

    /// Show `image` at `width` × `height` points where the caret `index` is: in its paragraph
    /// when that is empty, else in a new paragraph after it. Returns the caret position in
    /// the picture's paragraph.
    pub fn insert_image(&mut self, index: usize, image: Image, width: u16, height: u16) -> usize {
        let picture = Picture {
            image: self.images.len() as u32,
            width,
            height,
        };
        self.images.push(image);
        let mut para = self.para_at(index.min(self.len()));
        let chars = self.para_chars(para);
        if !chars.is_empty() || self.paras[para].picture.is_some() {
            self.replace(chars.end..chars.end, "\n", Format::default());
            para += 1;
            self.paras[para] = Para {
                cell: self.paras[para].cell,
                ..Para::default()
            };
        }
        self.paras[para].picture = Some(picture);
        self.para_chars(para).start
    }

    /// Resize (`Some`) or remove (`None`) the picture of paragraph `para`.
    pub fn set_picture(&mut self, para: usize, picture: Option<Picture>) {
        self.paras[para].picture = picture;
    }

    fn blocks(&self) -> Vec<Block> {
        let mut start = 0;
        self.text
            .split('\n')
            .zip(&self.paras)
            .map(|(text, para)| {
                let count = text.chars().count();
                let formats = self.formats[start..start + count].to_vec();
                start += count + 1;
                (text.to_owned(), formats, *para)
            })
            .collect()
    }

    /// Move the paragraphs `paras` in front of paragraph `before` (`paras().len()` moves them
    /// to the end). Returns where they start afterwards, or `None` when nothing would move.
    /// Callers move whole tables and never drop inside one.
    pub fn move_paras(&mut self, paras: Range<usize>, before: usize) -> Option<usize> {
        if paras.is_empty() || paras.end > self.paras.len() || paras.contains(&before) {
            return None;
        }
        if before == paras.end {
            return None;
        }
        let mut blocks = self.blocks();
        let moved: Vec<Block> = blocks.drain(paras.clone()).collect();
        // Where `before` is once the moved paragraphs are taken out.
        let target = if before > paras.start {
            before - moved.len()
        } else {
            before
        };
        blocks.splice(target..target, moved);
        self.rebuild(blocks);
        Some(target)
    }

    /// Make the document these paragraphs.
    fn rebuild(&mut self, mut blocks: Vec<Block>) {
        if blocks.is_empty() {
            blocks.push((String::new(), Vec::new(), Para::default()));
        }
        self.text.clear();
        self.formats.clear();
        self.paras.clear();
        for (index, (text, formats, para)) in blocks.into_iter().enumerate() {
            if index > 0 {
                self.text.push('\n');
                self.formats.push(Format::default());
            }
            self.text.push_str(&text);
            self.formats.extend(formats);
            self.paras.push(para);
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn formats(&self) -> &[Format] {
        &self.formats
    }

    pub fn paras(&self) -> &[Para] {
        &self.paras
    }

    /// Number of characters.
    pub fn len(&self) -> usize {
        self.formats.len()
    }

    pub fn is_empty(&self) -> bool {
        self.formats.is_empty()
    }

    pub fn word_count(&self) -> usize {
        self.text.split_whitespace().count()
    }

    fn byte_of(&self, index: usize) -> usize {
        self.text
            .char_indices()
            .nth(index)
            .map_or(self.text.len(), |(byte, _)| byte)
    }

    /// Index of the paragraph containing the character position `index`.
    pub fn para_at(&self, index: usize) -> usize {
        self.text
            .chars()
            .take(index)
            .filter(|ch| *ch == '\n')
            .count()
    }

    /// Characters of paragraph `para`, without its line break.
    pub fn para_chars(&self, para: usize) -> Range<usize> {
        let mut start = 0;
        let mut current = 0;
        for (index, ch) in self.text.chars().enumerate() {
            if ch != '\n' {
                continue;
            }
            if current == para {
                return start..index;
            }
            current += 1;
            start = index + 1;
        }
        start..self.len()
    }

    /// Format new text typed at `index` takes: the character before it in the same paragraph,
    /// else the one after, else the paragraph kind's.
    pub fn format_at(&self, index: usize) -> Format {
        let para = self.para_at(index);
        let chars = self.para_chars(para);
        if index > chars.start {
            self.formats[index - 1]
        } else if index < chars.end {
            self.formats[index]
        } else {
            self.paras[para].kind.format()
        }
    }

    /// Replace the characters `range` by `inserted`, typed in `format`.
    pub fn replace(&mut self, range: Range<usize>, inserted: &str, format: Format) {
        let range = range.start.min(self.len())..range.end.min(self.len());
        let para = self.para_at(range.start);
        let bytes = self.byte_of(range.start)..self.byte_of(range.end);
        let removed_breaks = self.text[bytes.clone()].matches('\n').count();
        // A line break typed at the end of a heading starts a normal paragraph, as in Word.
        let at_end = self.text[bytes.end..]
            .chars()
            .next()
            .is_none_or(|ch| ch == '\n');
        self.paras.drain(para + 1..=para + removed_breaks);
        let mut next = self.paras[para];
        next.page_break = false;
        next.picture = None;
        next.rule = false;
        if at_end {
            next.kind = Kind::Normal;
        }
        let added_breaks = inserted.matches('\n').count();
        self.paras
            .splice(para + 1..para + 1, std::iter::repeat_n(next, added_breaks));
        self.text.replace_range(bytes, inserted);
        self.formats
            .splice(range, std::iter::repeat_n(format, inserted.chars().count()));
    }

    /// The single stretch that turns the text into `new_text`: (replaced characters, what
    /// replaces them). This is what an edit in a text field amounts to.
    pub fn diff(&self, new_text: &str) -> (Range<usize>, String) {
        let old: Vec<char> = self.text.chars().collect();
        let new: Vec<char> = new_text.chars().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let room = old.len().min(new.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        let inserted = new[prefix..new.len() - suffix].iter().collect();
        (prefix..old.len() - suffix, inserted)
    }

    /// Make the text `new_text`, keeping the formats of what did not change.
    pub fn sync(&mut self, new_text: &str, format: Format) {
        if new_text != self.text {
            let (range, inserted) = self.diff(new_text);
            self.replace(range, &inserted, format);
        }
    }

    /// Change the format of the characters `range`.
    pub fn restyle(&mut self, range: Range<usize>, change: impl Fn(&mut Format)) {
        let end = range.end.min(self.len());
        self.formats[range.start.min(end)..end]
            .iter_mut()
            .for_each(change);
    }

    /// Paragraphs touched by the characters `range` (a caret counts as its paragraph).
    pub fn paras_in(&self, range: Range<usize>) -> Range<usize> {
        self.para_at(range.start)..self.para_at(range.end) + 1
    }

    /// Give the paragraphs `paras` the style `kind`: its size and weight apply to their text.
    pub fn set_kind(&mut self, paras: Range<usize>, kind: Kind) {
        let look = kind.format();
        for para in paras {
            self.paras[para].kind = kind;
            let chars = self.para_chars(para);
            self.restyle(chars, |format| {
                format.size = look.size;
                format.bold = look.bold;
            });
        }
    }

    pub fn set_align(&mut self, paras: Range<usize>, align: Align) {
        for para in &mut self.paras[paras] {
            para.align = align;
        }
    }

    pub fn set_list(&mut self, paras: Range<usize>, list: List) {
        for para in &mut self.paras[paras] {
            para.list = list;
        }
    }

    /// Stretches of equally formatted characters, as character ranges covering the document.
    pub fn runs(&self) -> Vec<(Range<usize>, Format)> {
        let mut runs: Vec<(Range<usize>, Format)> = Vec::new();
        for (index, format) in self.formats.iter().enumerate() {
            match runs.last_mut() {
                Some((range, last)) if last == format => range.end = index + 1,
                _ => runs.push((index..index + 1, *format)),
            }
        }
        runs
    }

    /// Split the paragraph at `index`; what follows starts a new page. Returns the caret
    /// position after the break, or `None` inside a table (cells cannot span pages).
    pub fn insert_page_break(&mut self, index: usize) -> Option<usize> {
        let index = index.min(self.len());
        let para = self.para_at(index);
        if self.paras[para].cell.is_some() {
            return None;
        }
        self.replace(index..index, "\n", self.format_at(index));
        self.paras[para + 1].page_break = true;
        Some(index + 1)
    }

    // ------------------------------------------------------------ tables

    /// Insert an empty `rows` × `cols` table after the paragraph containing `index`. An empty
    /// paragraph follows when the table would end the document. Returns the caret position in
    /// the first cell, or `None` when `index` is already in a table or the size is empty.
    pub fn insert_table(&mut self, index: usize, rows: u16, cols: u16) -> Option<usize> {
        let para = self.para_at(index.min(self.len()));
        if self.paras[para].cell.is_some() || rows == 0 || cols == 0 {
            return None;
        }
        let table = self
            .paras
            .iter()
            .filter_map(|para| para.cell)
            .map(|cell| cell.table + 1)
            .max()
            .unwrap_or(0);
        let cells = (0..rows).flat_map(|row| {
            (0..cols).map(move |col| Para {
                cell: Some(Cell { table, row, col }),
                ..Para::default()
            })
        });
        let end = self.para_chars(para).end;
        let format = Format::default();
        // One line break before each cell; a paragraph after the table when none follows it.
        let tail = self
            .paras
            .get(para + 1)
            .is_none_or(|next| next.cell.is_some());
        let breaks = usize::from(rows) * usize::from(cols) + usize::from(tail);
        let byte = self.byte_of(end);
        self.text.insert_str(byte, &"\n".repeat(breaks));
        self.formats
            .splice(end..end, std::iter::repeat_n(format, breaks));
        self.paras
            .splice(para + 1..para + 1, cells.chain(tail.then(Para::default)));
        Some(end + 1)
    }

    /// Paragraphs of the table `table`.
    pub fn table_paras(&self, table: u32) -> Range<usize> {
        let inside = |para: &Para| para.cell.is_some_and(|cell| cell.table == table);
        let start = self.paras.iter().position(inside).unwrap_or(0);
        let len = self.paras[start..]
            .iter()
            .take_while(|para| inside(para))
            .count();
        start..start + len
    }

    /// Remove the table `table` with its content. Returns the caret position where it was.
    pub fn delete_table(&mut self, table: u32) -> usize {
        let paras = self.table_paras(table);
        if paras.is_empty() {
            return 0;
        }
        let start = self.para_chars(paras.start).start;
        let chars = if paras.end < self.paras.len() {
            start..self.para_chars(paras.end).start
        } else if paras.start > 0 {
            // Last thing in the document: its leading line break goes with it.
            start - 1..self.len()
        } else {
            self.text.clear();
            self.formats.clear();
            self.paras = vec![Para::default()];
            return 0;
        };
        let bytes = self.byte_of(chars.start)..self.byte_of(chars.end);
        self.text.replace_range(bytes, "");
        self.formats.drain(chars.clone());
        self.paras.drain(paras);
        chars.start
    }

    /// (rows, columns) of the table `table`.
    pub fn table_size(&self, table: u32) -> (u16, u16) {
        let cells = self.paras.iter().filter_map(|para| para.cell);
        cells
            .filter(|cell| cell.table == table)
            .fold((0, 0), |(rows, cols), cell| {
                (rows.max(cell.row + 1), cols.max(cell.col + 1))
            })
    }

    /// Add an empty row above or below the row of `cell`. Returns the caret position in it.
    pub fn insert_table_row(&mut self, cell: Cell, below: bool) -> usize {
        let (_, cols) = self.table_size(cell.table);
        let new_row = cell.row + u16::from(below);
        let table = self.table_paras(cell.table);
        let mut blocks = self.blocks();
        // The new row goes in front of the first paragraph of the rows it pushes down.
        let at = table
            .clone()
            .find(|index| {
                blocks[*index]
                    .2
                    .cell
                    .is_some_and(|cell| cell.row >= new_row)
            })
            .unwrap_or(table.end);
        for block in &mut blocks[at..table.end] {
            if let Some(cell) = &mut block.2.cell {
                cell.row += 1;
            }
        }
        let row = (0..cols).map(|col| {
            let place = Cell {
                table: cell.table,
                row: new_row,
                col,
            };
            empty_cell(place)
        });
        blocks.splice(at..at, row);
        self.rebuild(blocks);
        self.para_chars(at).start
    }

    /// Add an empty column left or right of the column of `cell`. Returns the caret position
    /// in it, on the row of `cell`.
    pub fn insert_table_col(&mut self, cell: Cell, right: bool) -> usize {
        let new_col = cell.col + u16::from(right);
        let table = self.table_paras(cell.table);
        let mut blocks = self.blocks();
        let old: Vec<Block> = blocks.drain(table.clone()).collect();
        let mut rebuilt: Vec<Block> = Vec::with_capacity(old.len());
        // Row being copied, and whether it got its new cell yet.
        let mut row: Option<(u16, bool)> = None;
        let new_cell = |row: u16| {
            empty_cell(Cell {
                table: cell.table,
                row,
                col: new_col,
            })
        };
        for mut block in old {
            let Some(place) = block.2.cell else {
                rebuilt.push(block);
                continue;
            };
            if row.map(|(row, _)| row) != Some(place.row) {
                // The previous row ended left of the new column: it goes at its end.
                if let Some((previous, false)) = row {
                    rebuilt.push(new_cell(previous));
                }
                row = Some((place.row, false));
            }
            if place.col >= new_col {
                if row == Some((place.row, false)) {
                    rebuilt.push(new_cell(place.row));
                    row = Some((place.row, true));
                }
                block.2.cell = Some(Cell {
                    col: place.col + 1,
                    ..place
                });
            }
            rebuilt.push(block);
        }
        if let Some((previous, false)) = row {
            rebuilt.push(new_cell(previous));
        }
        let wanted = Cell {
            col: new_col,
            ..cell
        };
        let offset = rebuilt
            .iter()
            .position(|block| block.2.cell == Some(wanted))
            .unwrap_or(0);
        blocks.splice(table.start..table.start, rebuilt);
        self.rebuild(blocks);
        self.para_chars(table.start + offset).start
    }

    /// Remove the row (`row` true) or the column of `cell` with its content; the last one
    /// takes the table with it. Returns the caret position where it was.
    pub fn delete_table_line(&mut self, cell: Cell, row: bool) -> usize {
        let table = self.table_paras(cell.table);
        let mut blocks = self.blocks();
        let mut kept: Vec<Block> = Vec::new();
        for mut block in blocks.drain(table.clone()) {
            let Some(place) = &mut block.2.cell else {
                continue;
            };
            let (line, removed) = if row {
                (&mut place.row, cell.row)
            } else {
                (&mut place.col, cell.col)
            };
            if *line == removed {
                continue;
            }
            if *line > removed {
                *line -= 1;
            }
            kept.push(block);
        }
        blocks.splice(table.start..table.start, kept);
        self.rebuild(blocks);
        let para = table.start.min(self.paras.len() - 1);
        self.para_chars(para).start
    }

    // ------------------------------------------------------------ text tools

    /// Character ranges where `query` occurs (never across paragraphs), in order.
    pub fn find(&self, query: &str, match_case: bool) -> Vec<Range<usize>> {
        let fold = |ch: char| {
            if match_case {
                ch
            } else {
                ch.to_lowercase().next().unwrap_or(ch)
            }
        };
        let needle: Vec<char> = query.chars().map(fold).collect();
        if needle.is_empty() || needle.contains(&'\n') {
            return Vec::new();
        }
        let text: Vec<char> = self.text.chars().map(fold).collect();
        let mut found = Vec::new();
        let mut at = 0;
        while at + needle.len() <= text.len() {
            if text[at..at + needle.len()] == needle[..] {
                found.push(at..at + needle.len());
                at += needle.len();
            } else {
                at += 1;
            }
        }
        found
    }

    /// Replace every occurrence of `query` by `with`, in the format of what it replaces.
    /// Returns how many were replaced.
    pub fn replace_all(&mut self, query: &str, with: &str, match_case: bool) -> usize {
        let found = self.find(query, match_case);
        for range in found.iter().rev() {
            let format = self.formats[range.start];
            self.replace(range.clone(), with, format);
        }
        found.len()
    }

    /// Change each character of `range` (line breaks excepted) to what `map` makes of it.
    pub fn map_chars(&mut self, range: Range<usize>, map: impl Fn(char) -> char) {
        self.text = self
            .text
            .chars()
            .enumerate()
            .map(|(index, ch)| {
                if range.contains(&index) && ch != '\n' {
                    map(ch)
                } else {
                    ch
                }
            })
            .collect();
    }

    /// Give the characters of `range` back the plain look of their paragraph's style.
    pub fn clear_format(&mut self, range: Range<usize>) {
        let mut para = 0;
        for (index, ch) in self.text.chars().enumerate() {
            if range.contains(&index) {
                self.formats[index] = self.paras[para].kind.format();
            }
            if ch == '\n' {
                para += 1;
            }
        }
    }

    /// Change the paragraphs `paras` (spacing, indent, rule...).
    pub fn edit_paras(&mut self, paras: Range<usize>, change: impl Fn(&mut Para)) {
        self.paras[paras].iter_mut().for_each(change);
    }

    /// Cells of each table, in order: (table, [(row, col)]).
    fn table_cells(&self) -> Vec<(u32, Vec<(u16, u16)>)> {
        let mut tables: Vec<(u32, Vec<(u16, u16)>)> = Vec::new();
        for cell in self.paras.iter().filter_map(|para| para.cell) {
            let place = (cell.row, cell.col);
            match tables.last_mut() {
                Some((table, cells)) if *table == cell.table => {
                    if cells.last() != Some(&place) {
                        cells.push(place);
                    }
                }
                _ => tables.push((cell.table, vec![place])),
            }
        }
        tables
    }

    /// True when every table of `before` is still whole here, or gone entirely: an edit that
    /// would merge or drop single cells is not a valid edit.
    pub fn tables_intact(&self, before: &Self) -> bool {
        let now = self.table_cells();
        before.table_cells().iter().all(|(table, cells)| {
            now.iter()
                .find(|(id, _)| id == table)
                .is_none_or(|(_, kept)| kept == cells)
        })
    }
}

/// An empty paragraph in the table cell `cell`.
fn empty_cell(cell: Cell) -> Block {
    let para = Para {
        cell: Some(cell),
        ..Para::default()
    };
    (String::new(), Vec::new(), para)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cells of the document's paragraphs as (row, col), `None` outside tables.
    fn cells(doc: &Document) -> Vec<Option<(u16, u16)>> {
        doc.paras()
            .iter()
            .map(|para| para.cell.map(|cell| (cell.row, cell.col)))
            .collect()
    }

    #[test]
    fn table_rows_and_columns_come_and_go() {
        let mut doc = doc("x");
        let caret = doc.insert_table(0, 2, 2).unwrap();
        // Label the cells a b / c d.
        for (offset, label) in ["a", "b", "c", "d"].into_iter().enumerate() {
            let at = caret + 2 * offset;
            doc.replace(at..at, label, Format::default());
        }
        assert_eq!(doc.text(), "x\na\nb\nc\nd\n");
        let cell = |row, col| Cell { table: 0, row, col };

        let caret = doc.insert_table_row(cell(0, 1), true);
        consistent(&doc);
        assert_eq!((doc.text(), caret), ("x\na\nb\n\n\nc\nd\n", 6));
        assert_eq!(doc.table_size(0), (3, 2));
        assert_eq!(cells(&doc)[5], Some((2, 0)));

        let caret = doc.insert_table_col(cell(2, 0), true);
        consistent(&doc);
        assert_eq!(doc.text(), "x\na\n\nb\n\n\n\nc\n\nd\n");
        assert_eq!(doc.table_size(0), (3, 3));
        assert_eq!(cells(&doc)[2], Some((0, 1)));
        assert_eq!(
            doc.para_at(caret),
            8,
            "caret in the new cell of the row it was asked from"
        );
        // A column after the last one lands at the end of every row.
        doc.insert_table_col(cell(0, 2), true);
        assert_eq!(doc.table_size(0), (3, 4));
        assert_eq!(cells(&doc)[4], Some((0, 3)));

        doc.delete_table_line(cell(0, 3), false);
        doc.delete_table_line(cell(0, 1), false);
        doc.delete_table_line(cell(1, 0), true);
        consistent(&doc);
        assert_eq!(doc.text(), "x\na\nb\nc\nd\n");
        assert_eq!(doc.table_size(0), (2, 2));
        // Removing the last rows removes the table.
        doc.delete_table_line(cell(0, 0), true);
        doc.delete_table_line(cell(0, 0), true);
        assert_eq!(doc.text(), "x\n");
        assert!(doc.paras().iter().all(|para| para.cell.is_none()));
    }

    #[test]
    fn find_replace_and_case() {
        let mut doc = doc("Cat cat\nconCATenate");
        doc.restyle(4..7, |format| format.bold = true);
        assert_eq!(doc.find("cat", false), [0..3, 4..7, 11..14]);
        assert_eq!(doc.find("cat", true).first(), Some(&(4..7)));
        assert!(doc.find("t\nc", false).is_empty());
        assert_eq!(doc.replace_all("cat", "lion", true), 1);
        assert_eq!(doc.text(), "Cat lion\nconCATenate");
        assert!(doc.formats()[7].bold, "the replacement keeps the look");
        consistent(&doc);
        doc.map_chars(0..5, |ch| ch.to_ascii_uppercase());
        assert_eq!(doc.text(), "CAT Lion\nconCATenate");
        doc.clear_format(0..doc.len());
        assert!(doc.formats().iter().all(|format| !format.bold));
    }

    fn bold() -> Format {
        Format {
            bold: true,
            ..Format::default()
        }
    }

    fn doc(text: &str) -> Document {
        let mut doc = Document::default();
        doc.replace(0..0, text, Format::default());
        doc
    }

    fn consistent(doc: &Document) {
        assert_eq!(doc.formats().len(), doc.text().chars().count());
        assert_eq!(doc.paras().len(), doc.text().matches('\n').count() + 1);
    }

    #[test]
    fn replace_keeps_formats_and_paragraphs_in_step() {
        let mut doc = doc("Hello\nworld");
        assert_eq!((doc.len(), doc.paras().len()), (11, 2));
        doc.replace(5..6, " ", bold());
        assert_eq!(doc.text(), "Hello world");
        assert_eq!(doc.paras().len(), 1);
        assert!(doc.formats()[5].bold && !doc.formats()[4].bold);
        doc.replace(0..0, "é\n", Format::default());
        assert_eq!((doc.text(), doc.paras().len()), ("é\nHello world", 2));
        assert_eq!(doc.para_chars(1), 2..13);
        assert_eq!(doc.para_at(2), 1);
    }

    #[test]
    fn sync_finds_the_edited_stretch() {
        let mut doc = doc("abcdef");
        doc.restyle(0..6, |format| format.italic = true);
        doc.sync("abXYef", bold());
        assert_eq!(doc.text(), "abXYef");
        let bolds: Vec<bool> = doc.formats().iter().map(|format| format.bold).collect();
        assert_eq!(bolds, [false, false, true, true, false, false]);
        doc.sync("aef", bold());
        assert_eq!((doc.text(), doc.len()), ("aef", 3));
        // Repeated characters: the insertion is still one stretch of the right length.
        doc.sync("aeeef", bold());
        assert_eq!((doc.text(), doc.len()), ("aeeef", 5));
    }

    #[test]
    fn enter_after_a_heading_starts_a_normal_paragraph() {
        let mut doc = doc("Title");
        doc.set_kind(0..1, Kind::Heading1);
        assert_eq!(doc.formats()[0].size, 20.0);
        doc.replace(5..5, "\n", doc.format_at(5));
        assert_eq!(doc.paras()[1].kind, Kind::Normal);
        assert_eq!(doc.format_at(6), Kind::Normal.format());
        // In the middle, both halves stay headings.
        doc.replace(2..2, "\n", doc.format_at(2));
        assert_eq!(doc.paras()[1].kind, Kind::Heading1);
    }

    #[test]
    fn runs_group_equal_formats() {
        let mut doc = doc("abcd");
        doc.restyle(1..3, |format| format.bold = true);
        let runs = doc.runs();
        let ranges: Vec<Range<usize>> = runs.iter().map(|(range, _)| range.clone()).collect();
        assert_eq!(ranges, [0..1, 1..3, 3..4]);
        assert_eq!(doc.paras_in(1..1), 0..1);
    }

    #[test]
    fn page_break_splits_the_paragraph_once() {
        let mut doc = doc("ab");
        assert_eq!(doc.insert_page_break(1), Some(2));
        assert_eq!(doc.text(), "a\nb");
        assert!(doc.paras()[1].page_break && !doc.paras()[0].page_break);
        // Enter in the broken paragraph does not repeat the break.
        doc.replace(3..3, "\n", Format::default());
        assert!(!doc.paras()[2].page_break);
        consistent(&doc);
    }

    #[test]
    fn tables_insert_guard_and_delete() {
        let mut doc = doc("before\nafter");
        let caret = doc.insert_table(2, 2, 3).unwrap();
        consistent(&doc);
        assert_eq!(caret, 7);
        assert_eq!(doc.text(), "before\n\n\n\n\n\n\nafter");
        assert_eq!(doc.table_paras(0), 1..7);
        assert_eq!(
            doc.paras()[6].cell,
            Some(Cell {
                table: 0,
                row: 1,
                col: 2
            })
        );
        assert!(doc.paras()[7].cell.is_none());
        // No table inside a table, no page break either.
        assert_eq!(doc.insert_table(caret, 1, 1), None);
        assert_eq!(doc.insert_page_break(caret), None);

        // Typing and line breaks inside a cell keep the table whole.
        let mut typed = doc.clone();
        typed.replace(caret..caret, "x\ny", Format::default());
        assert!(typed.tables_intact(&doc));
        // Backspace at the start of a cell would swallow it.
        let mut merged = doc.clone();
        merged.replace(caret..caret + 1, "", Format::default());
        assert!(!merged.tables_intact(&doc));
        // Selecting across the whole table and deleting removes it.
        let mut gone = doc.clone();
        gone.replace(3..doc.len() - 2, "", Format::default());
        assert!(gone.tables_intact(&doc));
        assert!(gone.paras().iter().all(|para| para.cell.is_none()));

        assert_eq!(doc.delete_table(0), 7);
        assert_eq!(doc.text(), "before\nafter");
        consistent(&doc);
    }

    #[test]
    fn pictures_get_their_own_paragraph_and_move_with_it() {
        let image = Image {
            bytes: vec![1, 2, 3].into(),
            kind: ImageKind::Png,
        };
        let mut doc = doc("one\ntwo\nthree");
        doc.restyle(4..7, |format| format.bold = true);
        let caret = doc.insert_image(1, image.clone(), 120, 80);
        consistent(&doc);
        assert_eq!((doc.text(), caret), ("one\n\ntwo\nthree", 4));
        assert_eq!(
            doc.paras()[1].picture.map(|picture| picture.width),
            Some(120)
        );
        // Enter in the caption does not copy the picture.
        doc.replace(4..4, "caption\n", Format::default());
        assert!(doc.paras()[1].picture.is_some() && doc.paras()[2].picture.is_none());
        // An empty paragraph takes the picture itself.
        let caret = doc.insert_image(12, image, 10, 10);
        assert_eq!(caret, 12);
        assert_eq!(doc.paras().len(), 5);

        // Move the first picture (paragraph 1) to the very end, then "three" to the front.
        assert_eq!(doc.move_paras(1..2, 5), Some(4));
        consistent(&doc);
        assert_eq!(doc.text(), "one\n\ntwo\nthree\ncaption");
        assert!(doc.paras()[4].picture.is_some());
        assert_eq!(doc.move_paras(3..4, 0), Some(0));
        assert_eq!(doc.text(), "three\none\n\ntwo\ncaption");
        assert!(doc.formats()[11].bold, "formats travel with their text");
        // Dropping a paragraph where it already is does nothing.
        assert_eq!(doc.move_paras(1..2, 1), None);
        assert_eq!(doc.move_paras(1..2, 2), None);
    }

    #[test]
    fn deleting_a_table_that_ends_the_document() {
        let mut doc = doc("x");
        doc.insert_table(0, 1, 2).unwrap();
        // Merge the paragraph after the table into the last cell.
        doc.replace(doc.len() - 1..doc.len(), "", Format::default());
        assert_eq!(doc.table_paras(0), 1..3);
        doc.delete_table(0);
        assert_eq!(doc.text(), "x");
        consistent(&doc);
    }
}
