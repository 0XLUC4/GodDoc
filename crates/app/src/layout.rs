//! Page layout: every paragraph is laid out on its own (its width depends on lists and table
//! columns), then its rows are placed on A4 pages. The result is one galley the text field
//! edits, plus what is drawn around the text (table borders, list markers).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use egui::epaint::text::PlacedRow;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Galley, Painter, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use goddoc_core::{Align, Document, Font, Format, PageSetup, Para, Picture, Script, Wrap};

/// Space between two pages on screen.
pub const PAGE_GAP: f32 = 18.0;
/// Line height, in font sizes.
pub const LINE_HEIGHT: f32 = 1.3;
/// Width of one indent level, in points.
const INDENT_STEP: f32 = 24.0;
/// Raised and lowered text is this much smaller.
const SCRIPT_SCALE: f32 = 0.62;
/// Indent of list paragraphs, in points; the marker sits in it.
const LIST_INDENT: f32 = 22.0;
/// Space left of a list marker and between a marker and its text, in points.
const MARKER_LEAD: f32 = 4.0;
const MARKER_GAP: f32 = 5.0;
/// Space between a table cell's border and its text, in points.
const CELL_PAD: f32 = 5.0;
/// Space between the columns of a page, in points.
const COLUMN_GAP: f32 = 36.0;

/// (font, bold, egui family name, Windows file, other systems).
const FACES: [(Font, bool, &str, &str, &str); 40] = [
    (
        Font::Sans,
        false,
        "doc-sans",
        "arial.ttf",
        "LiberationSans-Regular.ttf",
    ),
    (
        Font::Sans,
        true,
        "doc-sans-bold",
        "arialbd.ttf",
        "LiberationSans-Bold.ttf",
    ),
    (
        Font::Serif,
        false,
        "doc-serif",
        "times.ttf",
        "LiberationSerif-Regular.ttf",
    ),
    (
        Font::Serif,
        true,
        "doc-serif-bold",
        "timesbd.ttf",
        "LiberationSerif-Bold.ttf",
    ),
    (
        Font::Mono,
        false,
        "doc-mono",
        "cour.ttf",
        "LiberationMono-Regular.ttf",
    ),
    (
        Font::Mono,
        true,
        "doc-mono-bold",
        "courbd.ttf",
        "LiberationMono-Bold.ttf",
    ),
    (
        Font::Calibri,
        false,
        "doc-calibri",
        "calibri.ttf",
        "Carlito-Regular.ttf",
    ),
    (
        Font::Calibri,
        true,
        "doc-calibri-bold",
        "calibrib.ttf",
        "Carlito-Bold.ttf",
    ),
    (
        Font::Verdana,
        false,
        "doc-verdana",
        "verdana.ttf",
        "DejaVuSans.ttf",
    ),
    (
        Font::Verdana,
        true,
        "doc-verdana-bold",
        "verdanab.ttf",
        "DejaVuSans-Bold.ttf",
    ),
    (
        Font::Georgia,
        false,
        "doc-georgia",
        "georgia.ttf",
        "DejaVuSerif.ttf",
    ),
    (
        Font::Georgia,
        true,
        "doc-georgia-bold",
        "georgiab.ttf",
        "DejaVuSerif-Bold.ttf",
    ),
    (
        Font::Tahoma,
        false,
        "doc-tahoma",
        "tahoma.ttf",
        "DejaVuSans.ttf",
    ),
    (
        Font::Tahoma,
        true,
        "doc-tahoma-bold",
        "tahomabd.ttf",
        "DejaVuSans-Bold.ttf",
    ),
    (
        Font::Comic,
        false,
        "doc-comic",
        "comic.ttf",
        "ComicNeue-Regular.ttf",
    ),
    (
        Font::Comic,
        true,
        "doc-comic-bold",
        "comicbd.ttf",
        "ComicNeue-Bold.ttf",
    ),
    (
        Font::SegoeUi,
        false,
        "doc-segoe",
        "segoeui.ttf",
        "LiberationSans-Regular.ttf",
    ),
    (
        Font::SegoeUi,
        true,
        "doc-segoe-bold",
        "segoeuib.ttf",
        "LiberationSans-Bold.ttf",
    ),
    (
        Font::Consolas,
        false,
        "doc-consolas",
        "consola.ttf",
        "LiberationMono-Regular.ttf",
    ),
    (
        Font::Consolas,
        true,
        "doc-consolas-bold",
        "consolab.ttf",
        "LiberationMono-Bold.ttf",
    ),
    (
        Font::Trebuchet,
        false,
        "doc-trebuchet",
        "trebuc.ttf",
        "LiberationSans-Regular.ttf",
    ),
    (
        Font::Trebuchet,
        true,
        "doc-trebuchet-bold",
        "trebucbd.ttf",
        "LiberationSans-Bold.ttf",
    ),
    (
        Font::Cambria,
        false,
        "doc-cambria",
        "cambria.ttc",
        "LiberationSerif-Regular.ttf",
    ),
    (
        Font::Cambria,
        true,
        "doc-cambria-bold",
        "cambriab.ttf",
        "LiberationSerif-Bold.ttf",
    ),
    (
        Font::Candara,
        false,
        "doc-candara",
        "Candara.ttf",
        "LiberationSans-Regular.ttf",
    ),
    (
        Font::Candara,
        true,
        "doc-candara-bold",
        "Candarab.ttf",
        "LiberationSans-Bold.ttf",
    ),
    (
        Font::Constantia,
        false,
        "doc-constantia",
        "constan.ttf",
        "LiberationSerif-Regular.ttf",
    ),
    (
        Font::Constantia,
        true,
        "doc-constantia-bold",
        "constanb.ttf",
        "LiberationSerif-Bold.ttf",
    ),
    (
        Font::Corbel,
        false,
        "doc-corbel",
        "corbel.ttf",
        "LiberationSans-Regular.ttf",
    ),
    (
        Font::Corbel,
        true,
        "doc-corbel-bold",
        "corbelb.ttf",
        "LiberationSans-Bold.ttf",
    ),
    (
        Font::Palatino,
        false,
        "doc-palatino",
        "pala.ttf",
        "LiberationSerif-Regular.ttf",
    ),
    (
        Font::Palatino,
        true,
        "doc-palatino-bold",
        "palab.ttf",
        "LiberationSerif-Bold.ttf",
    ),
    (
        Font::Garamond,
        false,
        "doc-garamond",
        "GARA.TTF",
        "LiberationSerif-Regular.ttf",
    ),
    (
        Font::Garamond,
        true,
        "doc-garamond-bold",
        "GARABD.TTF",
        "LiberationSerif-Bold.ttf",
    ),
    (
        Font::CenturyGothic,
        false,
        "doc-gothic",
        "GOTHIC.TTF",
        "LiberationSans-Regular.ttf",
    ),
    (
        Font::CenturyGothic,
        true,
        "doc-gothic-bold",
        "GOTHICB.TTF",
        "LiberationSans-Bold.ttf",
    ),
    (
        Font::BookAntiqua,
        false,
        "doc-antiqua",
        "BKANT.TTF",
        "LiberationSerif-Regular.ttf",
    ),
    (
        Font::BookAntiqua,
        true,
        "doc-antiqua-bold",
        "ANTQUAB.TTF",
        "LiberationSerif-Bold.ttf",
    ),
    (
        Font::Impact,
        false,
        "doc-impact",
        "impact.ttf",
        "LiberationSans-Regular.ttf",
    ),
    (
        Font::Impact,
        true,
        "doc-impact-bold",
        "impact.ttf",
        "LiberationSans-Bold.ttf",
    ),
];

/// Register the document fonts from the system (Arial, Times New Roman, Courier New or their
/// Liberation twins). A missing file falls back to the UI font.
pub fn install_fonts(fonts: &mut egui::FontDefinitions) {
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut found = Vec::new();
    for (font, bold, name, windows, other) in FACES {
        let mut family = fallback.clone();
        if let Some(bytes) = system_font(windows, other) {
            if !bold {
                found.push(font);
            }
            fonts
                .font_data
                .insert(name.to_owned(), egui::FontData::from_owned(bytes).into());
            family.insert(0, name.to_owned());
        }
        fonts.families.insert(FontFamily::Name(name.into()), family);
    }
    let _ = FOUND.set(found);
}

/// The fonts this computer has, set by [`install_fonts`].
static FOUND: std::sync::OnceLock<Vec<Font>> = std::sync::OnceLock::new();

/// The fonts to offer, by name: those this computer has, and `current`, which a document
/// may ask for all the same.
pub fn fonts(current: Font) -> Vec<Font> {
    let found = FOUND.get().map_or(&[][..], Vec::as_slice);
    let mut fonts: Vec<Font> = Font::ALL
        .into_iter()
        .filter(|font| *font == current || found.contains(font))
        .collect();
    fonts.sort_by_key(|font| font.family());
    fonts
}

fn system_font(windows: &str, other: &str) -> Option<Vec<u8>> {
    let windows_dir = std::env::var_os("WINDIR")
        .map(|dir| std::path::PathBuf::from(dir).join("Fonts").join(windows));
    let unix_dirs = [
        "/usr/share/fonts/truetype/liberation",
        "/usr/share/fonts/liberation-sans",
        "/usr/share/fonts/liberation-serif",
        "/usr/share/fonts/liberation-mono",
        "/usr/share/fonts/truetype/dejavu",
        "/usr/share/fonts/truetype/crosextra",
        "/usr/share/fonts/TTF",
        "/Library/Fonts",
    ];
    windows_dir
        .into_iter()
        .chain(
            unix_dirs
                .iter()
                .map(|dir| std::path::Path::new(dir).join(other)),
        )
        .find_map(|path| std::fs::read(path).ok())
}

pub fn family(font: Font, bold: bool) -> FontFamily {
    let name = FACES
        .iter()
        .find(|face| face.0 == font && face.1 == bold)
        .map_or("doc-sans", |face| face.2);
    FontFamily::Name(name.into())
}

/// Colour of text put in or taken out while changes were tracked.
const TRACKED: [u8; 3] = [0xB4, 0x23, 0x8C];
/// Background of text a comment is about.
const NOTED: [u8; 3] = [0xFF, 0xE9, 0xA8];

/// How `format` shows: a tracked change and a comment each add their mark to the look the
/// text was given.
pub fn shown(format: &Format) -> Format {
    let mut shown = *format;
    match format.change {
        goddoc_core::Change::None => {}
        goddoc_core::Change::Inserted => (shown.color, shown.underline) = (TRACKED, true),
        goddoc_core::Change::Deleted => (shown.color, shown.strike) = (TRACKED, true),
    }
    if format.note != 0 && shown.highlight.is_none() {
        shown.highlight = Some(NOTED);
    }
    shown
}

/// How `format` is drawn at `zoom`; `spacing` is the paragraph's line spacing factor.
pub fn text_format(format: &Format, zoom: f32, spacing: f32) -> TextFormat {
    let format = &shown(format);
    let [r, g, b] = format.color;
    let color = Color32::from_rgb(r, g, b);
    let full = (format.size * zoom).max(1.0);
    let (size, valign) = match format.script {
        Script::Normal => (full, egui::Align::BOTTOM),
        Script::Super => (full * SCRIPT_SCALE, egui::Align::TOP),
        Script::Sub => (full * SCRIPT_SCALE, egui::Align::BOTTOM),
    };
    let line = |on: bool| {
        if on {
            Stroke::new((size / 14.0).max(1.0), color)
        } else {
            Stroke::NONE
        }
    };
    TextFormat {
        font_id: FontId::new(size, family(format.font, format.bold)),
        color,
        background: format
            .highlight
            .map_or(Color32::TRANSPARENT, |[r, g, b]| Color32::from_rgb(r, g, b)),
        line_height: Some(full * LINE_HEIGHT * spacing),
        italics: format.italic,
        // Underlines are drawn apart, by [`underlines`].
        underline: Stroke::NONE,
        strikethrough: line(format.strike),
        valign,
        ..Default::default()
    }
}

/// Size of a sheet on screen.
pub fn page_size(page: PageSetup, zoom: f32) -> Vec2 {
    let (width, height) = page.size();
    vec2(width, height) * zoom
}

/// The margins on screen: top, right, bottom, left.
pub fn margins(page: PageSetup, zoom: f32) -> [f32; 4] {
    page.margin_points().map(|side| side * zoom)
}

/// Size of the text area of a page on screen.
pub fn content_size(page: PageSetup, zoom: f32) -> Vec2 {
    let [top, right, bottom, left] = margins(page, zoom);
    page_size(page, zoom) - vec2(left + right, top + bottom)
}

/// A measure of a paragraph (twentieths of a point) on screen.
fn measure(twips: impl Into<f32>, zoom: f32) -> f32 {
    twips.into() / goddoc_core::TWIPS_PER_POINT * zoom
}

/// How far the first line of a paragraph starts before its other lines (a hanging indent).
fn hanging(para: &Para, zoom: f32) -> f32 {
    measure(para.first_line.min(0).unsigned_abs(), zoom)
}

/// Distance between the tops of two consecutive pages on screen.
pub fn page_step(page: PageSetup, zoom: f32) -> f32 {
    page_size(page, zoom).y + PAGE_GAP
}

/// A table cell. Positions are relative to the text area of the first page.
pub struct CellBox {
    pub rect: Rect,
    /// Its rows in the galley.
    rows: Range<usize>,
    /// Character index of its first row.
    chars: usize,
}

/// Bullet or number in front of a list paragraph, at the top-left of its indent.
pub struct Marker {
    pub pos: Pos2,
    pub text: String,
    pub format: Format,
    /// Line spacing factor of its paragraph.
    pub spacing: f32,
}

/// A stretch of underline, relative to the galley.
pub struct Underline {
    pub from: Pos2,
    pub to: Pos2,
    pub stroke: Stroke,
}

/// The underlines of the rows of `galley` that show in `visible`; `formats` has the format
/// of each of its characters. The text does not draw them itself: it would break them
/// wherever the font, its weight or its size changes.
pub fn underlines(galley: &Galley, formats: &[Format], zoom: f32, visible: Rect) -> Vec<Underline> {
    // Text put in while changes were tracked is underlined too.
    let marked: Vec<Format> = formats.iter().map(shown).collect();
    let formats = marked.as_slice();
    let mut lines = Vec::new();
    let mut start = 0;
    for row in &galley.rows {
        let first = start;
        start += row.char_count_including_newline().0;
        let glyphs = &row.glyphs;
        let hidden = row.max_y() < visible.top() || row.min_y() > visible.bottom();
        let Some(formats) = formats.get(first..first + glyphs.len()).filter(|_| !hidden) else {
            continue;
        };
        let mut at = 0;
        while at < glyphs.len() {
            if !formats[at].underline {
                at += 1;
                continue;
            }
            let end = (at..glyphs.len())
                .find(|index| !formats[*index].underline)
                .unwrap_or(glyphs.len());
            // One height and one thickness along the stretch, set by its largest text.
            let sizes = formats[at..end].iter().map(|format| format.size);
            let size = sizes.fold(0.0, f32::max) * zoom;
            let baselines = glyphs[at..end].iter().map(|glyph| glyph.pos.y);
            let y = row.pos.y + baselines.fold(f32::MIN, f32::max) + (size * 0.1).max(1.5);
            // A change of colour is the only thing that starts a new line.
            for colored in formats[at..end].chunk_by(|a, b| a.color == b.color) {
                let [r, g, b] = colored[0].color;
                let last = at + colored.len() - 1;
                // Up to where the next character of the stretch starts: no gap between two.
                let right = match glyphs.get(last + 1).filter(|_| last + 1 < end) {
                    Some(next) => next.pos.x,
                    None => glyphs[last].max_x(),
                };
                lines.push(Underline {
                    from: pos2(row.pos.x + glyphs[at].pos.x, y),
                    to: pos2(row.pos.x + right, y),
                    stroke: Stroke::new((size / 14.0).max(1.0), Color32::from_rgb(r, g, b)),
                });
                at = last + 1;
            }
        }
    }
    lines
}

/// Where to mark the words `ranges` (in characters, in order) of the rows of `galley` that
/// show in `visible`: for each stretch, its left end under the text, and its length.
pub fn marked_words(galley: &Galley, ranges: &[Range<usize>], visible: Rect) -> Vec<(Pos2, f32)> {
    let mut marks = Vec::new();
    let mut start = 0;
    // The first range that may still reach the row.
    let mut next = 0;
    for row in &galley.rows {
        let first = start;
        start += row.char_count_including_newline().0;
        while ranges.get(next).is_some_and(|range| range.end <= first) {
            next += 1;
        }
        if row.max_y() < visible.top() || row.min_y() > visible.bottom() {
            continue;
        }
        for range in ranges[next.min(ranges.len())..]
            .iter()
            .take_while(|range| range.start < start)
        {
            let from = range.start.max(first) - first;
            let to = (range.end.min(start) - first).min(row.glyphs.len());
            let Some(glyphs) = row.glyphs.get(from..to).filter(|glyphs| !glyphs.is_empty()) else {
                continue;
            };
            let left = glyphs[0].pos.x;
            let right = glyphs[glyphs.len() - 1].max_x();
            let baseline = glyphs
                .iter()
                .map(|glyph| glyph.pos.y)
                .fold(f32::MIN, f32::max);
            marks.push((row.pos + vec2(left, baseline + 2.5), right - left));
        }
    }
    marks
}

/// A picture as shown, above the text of paragraph `para`.
pub struct PictureBox {
    pub rect: Rect,
    pub para: usize,
    pub picture: Picture,
    /// Widest it can be here (the page or its table column).
    pub max_width: f32,
}

/// Something that moves as a whole: a paragraph (with its picture) or a table.
pub struct Block {
    /// From its top to its bottom, across the text area.
    pub rect: Rect,
    pub paras: Range<usize>,
}

pub struct PageLayout {
    /// The whole text; rows of later pages sit one [`page_step`] lower per page.
    pub galley: Arc<Galley>,
    pub pages: usize,
    pub cells: Vec<CellBox>,
    /// The columns of the pages, when there are several: each with the rows it holds.
    pub strips: Vec<CellBox>,
    pub markers: Vec<Marker>,
    pub pictures: Vec<PictureBox>,
    pub blocks: Vec<Block>,
    /// Lines under paragraphs: (left end, length).
    pub rules: Vec<(Pos2, f32)>,
    /// Page (0 for the first) each paragraph starts on.
    pub para_pages: Vec<usize>,
}

/// The boxes that mark the characters `range` of `galley` as selected, one per row: as tall as
/// the text of the row (`text_height` per point of font size), not as the line, which line
/// spacing makes taller.
pub fn selection(
    galley: &Galley,
    formats: &[Format],
    range: Range<usize>,
    text_height: f32,
) -> Vec<Rect> {
    let mut boxes = Vec::new();
    let mut start = 0;
    for row in &galley.rows {
        let first = start;
        start += row.char_count_including_newline().0;
        // The row holds the characters `first..start`, its line break included.
        let (from, to) = (range.start.max(first), range.end.min(start));
        if from >= to {
            continue;
        }
        let glyphs = &row.glyphs;
        let end = glyphs.last().map_or(0.0, |glyph| glyph.max_x());
        let (a, b) = (from - first, (to - first).min(glyphs.len()));
        let left = glyphs.get(a).map_or(end, |glyph| glyph.pos.x);
        let mut right = if b > a { glyphs[b - 1].max_x() } else { left };
        let sizes = formats.get(first..first + glyphs.len()).unwrap_or_default();
        let largest = sizes.iter().map(|format| format.size).fold(0.0, f32::max);
        let size = if largest > 0.0 {
            largest
        } else {
            formats.get(first).map_or(11.0, |format| format.size)
        };
        // A selected line break shows as a little room after the text.
        if to - first > glyphs.len() {
            right += size * text_height * 0.3;
        }
        let top = row.min_y();
        let bottom = top + row.size.y.min(size * text_height);
        boxes.push(Rect::from_min_max(
            pos2(row.pos.x + left, top),
            pos2(row.pos.x + right, bottom),
        ));
    }
    boxes
}

/// A formatting mark: what stands for a space, a tab or the end of a paragraph.
pub struct Mark {
    /// Its centre, relative to the galley.
    pub pos: Pos2,
    pub glyph: &'static str,
    /// Size on screen of the text it stands in.
    pub size: f32,
}

/// The formatting marks of the rows of `galley` that show in `visible`; `formats` has the
/// format of each of its characters.
pub fn marks(galley: &Galley, formats: &[Format], zoom: f32, visible: Rect) -> Vec<Mark> {
    let mut marks = Vec::new();
    let mut start = 0;
    let size_at = |index: usize| {
        let format = formats.get(index).or(formats.last());
        format.map_or(11.0, |format| format.size) * zoom
    };
    for row in &galley.rows {
        let first = start;
        start += row.char_count_including_newline().0;
        if row.max_y() < visible.top() || row.min_y() > visible.bottom() {
            continue;
        }
        for (index, glyph) in row.glyphs.iter().enumerate() {
            let sign = match glyph.chr {
                ' ' => "·",
                '\t' => "→",
                '\u{a0}' => "°",
                _ => continue,
            };
            let size = size_at(first + index);
            let centre = vec2(
                glyph.pos.x + glyph.advance_width / 2.0,
                glyph.pos.y - size * 0.3,
            );
            marks.push(Mark {
                pos: row.pos + centre,
                glyph: sign,
                size,
            });
        }
        // The last row of a paragraph ends on its break, or on the end of the text.
        if row.ends_with_newline || start >= formats.len() {
            let size = size_at(first + row.glyphs.len());
            let end = row.glyphs.last();
            let x = end.map_or(0.0, |glyph| glyph.max_x()) + size * 0.3;
            let baseline = end.map_or(size * 0.95, |glyph| glyph.pos.y);
            marks.push(Mark {
                pos: row.pos + vec2(x, baseline - size * 0.3),
                glyph: "¶",
                size,
            });
        }
    }
    marks
}

impl PageLayout {
    /// Character index under `pos` (relative to the text area) within `cell`. The text field
    /// cannot find it: it only looks at heights, and cells of a row share theirs.
    pub fn cursor_in(&self, cell: &CellBox, pos: Vec2) -> usize {
        let mut index = cell.chars;
        let mut best = (f32::INFINITY, index);
        for row in &self.galley.rows[cell.rows.clone()] {
            let distance = if (row.min_y()..=row.max_y()).contains(&pos.y) {
                0.0
            } else {
                (row.min_y() - pos.y).abs().min((row.max_y() - pos.y).abs())
            };
            if distance < best.0 {
                best = (distance, index + row.char_at(pos.x - row.pos.x).0);
            }
            index += row.char_count_including_newline().0;
        }
        best.1
    }

    /// Where something dropped at height `y` goes: (paragraph it lands in front of, height of
    /// the gap). Only gaps between blocks qualify, so nothing lands inside a table.
    pub fn drop_target(&self, y: f32) -> (usize, f32) {
        match self.blocks.iter().find(|block| y < block.rect.center().y) {
            Some(block) => (block.paras.start, block.rect.top()),
            None => self
                .blocks
                .last()
                .map_or((0, 0.0), |block| (block.paras.end, block.rect.bottom())),
        }
    }
}

/// Where the next row goes; pages are stacked one [`page_step`] apart.
struct Placer {
    y: f32,
    page: usize,
    step: f32,
    /// Height of the text area of a page.
    height: f32,
    /// Column of the page the rows go in, how many there are, and how far apart they start.
    strip: usize,
    strips: usize,
    stride: f32,
}

impl Placer {
    fn top(&self) -> f32 {
        self.page as f32 * self.step
    }

    fn at_top(&self) -> bool {
        self.y <= self.top() + 0.5
    }

    fn new_page(&mut self) {
        self.page += 1;
        self.strip = 0;
        self.y = self.top();
    }

    /// Left edge of the column the rows go in.
    fn x(&self) -> f32 {
        self.strip as f32 * self.stride
    }

    /// Move to the next column, or to the next page after the last one, when something
    /// `height` tall does not fit in this one.
    fn fit(&mut self, height: f32) {
        if self.y + height > self.top() + self.height + 0.5 && !self.at_top() {
            if self.strip + 1 < self.strips {
                self.strip += 1;
                self.y = self.top();
            } else {
                self.new_page();
            }
        }
    }
}

/// Left inset of a paragraph: its indent level and the room it asks for beyond it.
fn level_indent(para: &Para, zoom: f32) -> f32 {
    f32::from(para.indent) * INDENT_STEP * zoom + measure(para.left, zoom)
}

/// Size on screen of the paragraph's picture, at most `max_width` wide (zero without one).
fn picture_size(para: &Para, max_width: f32, zoom: f32) -> Vec2 {
    let Some(picture) = para.picture else {
        return Vec2::ZERO;
    };
    let size = vec2(f32::from(picture.width), f32::from(picture.height)) * zoom;
    size * (max_width / size.x.max(1.0)).min(1.0)
}

/// Lay one paragraph out `wrap` wide.
fn para_galley(
    painter: &Painter,
    text: &str,
    formats: &[Format],
    para: &Para,
    zoom: f32,
    wrap: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap.max(1.0);
    job.justify = para.align == Align::Justify;
    let spacing = para.line_factor();
    // The first line starts further right than the others.
    let mut lead = measure(para.first_line.max(0), zoom);
    if formats.is_empty() {
        // An empty section still gives the caret its height.
        job.append("", lead, text_format(&para.kind.format(), zoom, spacing));
    }
    let mut bytes = text.char_indices().map(|(byte, _)| byte).skip(1);
    let mut start = 0;
    for run in formats.chunk_by(|a, b| a == b) {
        let end = bytes.nth(run.len() - 1).unwrap_or(text.len());
        let look = text_format(&run[0], zoom, spacing);
        job.append(&text[start..end], std::mem::take(&mut lead), look);
        start = end;
    }
    painter.layout_job(job)
}

/// The job of the whole document: the text field reads its text from the galley.
fn document_job(doc: &Document, zoom: f32, wrap: f32) -> LayoutJob {
    let text = doc.text();
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let mut bytes = text.char_indices().map(|(byte, _)| byte).skip(1);
    let mut start = 0;
    for (range, format) in doc.runs() {
        let end = bytes.nth(range.len() - 1).unwrap_or(text.len());
        job.append(&text[start..end], 0.0, text_format(&format, zoom, 1.0));
        start = end;
    }
    job
}

/// Put `rows` (one paragraph) under each other from `origin`, aligned within `wrap`.
fn stack(rows: &mut [PlacedRow], origin: Pos2, wrap: f32, align: Align) -> f32 {
    let mut y = origin.y;
    for row in rows {
        row.pos = pos2(origin.x + shift(align, wrap, row.size.x), y).round();
        y += row.size.y;
    }
    y - origin.y
}

pub fn shift(align: Align, wrap: f32, width: f32) -> f32 {
    let free = (wrap - width).max(0.0);
    match align {
        Align::Left | Align::Justify => 0.0,
        Align::Center => free / 2.0,
        Align::Right => free,
    }
}

/// Lay the document out on pages at `zoom`.
pub fn layout(painter: &Painter, doc: &Document, zoom: f32) -> PageLayout {
    let content = content_size(doc.page, zoom);
    // The columns of a page: how many, how wide, how far apart they start.
    let strips = usize::from(doc.page.columns.max(1));
    let gap = COLUMN_GAP * zoom;
    let strip_width = ((content.x - gap * (strips - 1) as f32) / strips as f32).max(1.0);
    let paras = doc.paras();
    let formats = doc.formats();
    let pad = CELL_PAD * zoom;
    // Space between a picture and the text under it.
    let picture_gap = 4.0 * zoom;
    let mut columns: HashMap<u32, f32> = HashMap::new();
    // Columns a paragraph's cell takes: more than one when cells were joined.
    let joined = |para: &Para| f32::from(para.span.max(1));
    for para in paras {
        if let Some(cell) = para.cell {
            let count = columns.entry(cell.table).or_insert(1.0);
            *count = count.max(f32::from(cell.col) + joined(para));
        }
    }
    // Width of the column a paragraph lives in, of what it can show, and of its text.
    let column_width = |para: &Para| match para.cell {
        Some(cell) => strip_width / columns[&cell.table],
        None => strip_width,
    };
    let inner_width = |para: &Para| {
        let padding = if para.cell.is_some() { 2.0 * pad } else { 0.0 };
        column_width(para) * joined(para) - padding
    };
    // Size of a paragraph's picture when it stands beside the text (zero otherwise): no more
    // than a good half of the width, so that words still fit beside it. Not in table cells.
    let beside = |para: &Para| {
        if para.wrap == Wrap::Above || para.cell.is_some() {
            Vec2::ZERO
        } else {
            picture_size(para, inner_width(para) * 0.6, zoom)
        }
    };
    // Room the text leaves at its left or at its right for the picture beside it.
    let room_beside = |para: &Para, side: Wrap| {
        let size = beside(para);
        if para.wrap == side && size.x > 0.0 {
            size.x + 2.0 * picture_gap
        } else {
            0.0
        }
    };
    let mut starts = Vec::with_capacity(paras.len());
    let mut start = 0;
    for text in doc.text().split('\n') {
        starts.push(start);
        start += text.chars().count() + 1;
    }
    let labels = doc.markers();
    // Look of a paragraph's marker: that of the start of its text, plain.
    let marker_format = |index: usize| {
        let first = formats.get(starts[index]).copied();
        let empty = starts
            .get(index + 1)
            .is_none_or(|next| next - 1 == starts[index]);
        Format {
            italic: false,
            underline: false,
            strike: false,
            highlight: None,
            script: Script::Normal,
            ..first
                .filter(|_| !empty)
                .unwrap_or_else(|| paras[index].kind.format())
        }
    };
    // The number of a heading starts where text does; a list marker sits in from it.
    let lead = |para: &Para| match para.kind.heading_level() {
        Some(_) => 0.0,
        None => MARKER_LEAD * zoom,
    };
    // Left inset of each paragraph's text: its indent level, plus room for its marker.
    let insets: Vec<f32> = (0..paras.len())
        .map(|index| {
            let para = &paras[index];
            // The lines after the first of a hanging paragraph start further right.
            let level =
                level_indent(para, zoom) + hanging(para, zoom) + room_beside(para, Wrap::Left);
            let Some(label) = &labels[index] else {
                return level;
            };
            let look = text_format(&marker_format(index), zoom, 1.0);
            let job = LayoutJob::single_section(label.clone(), look);
            let room = lead(para) + painter.layout_job(job).size().x + MARKER_GAP * zoom;
            let least = match para.kind.heading_level() {
                Some(_) => 0.0,
                None => LIST_INDENT * zoom,
            };
            level + room.max(least)
        })
        .collect();
    let wrap_of = |index: usize| {
        let para = &paras[index];
        inner_width(para)
            - insets[index]
            - measure(para.right, zoom)
            - room_beside(para, Wrap::Right)
    };
    // Room above and under a paragraph.
    let room = |para: &Para| (measure(para.before, zoom), measure(para.after, zoom));
    // Height a paragraph's picture takes, gap included.
    let picture_height = |para: &Para| {
        if beside(para).x > 0.0 {
            return 0.0;
        }
        let size = picture_size(para, inner_width(para), zoom);
        if size.y > 0.0 {
            size.y + picture_gap
        } else {
            0.0
        }
    };

    let mut galleys = Vec::with_capacity(paras.len());
    for (index, (para, text)) in paras.iter().zip(doc.text().split('\n')).enumerate() {
        let slice = &formats[starts[index]..starts[index] + text.chars().count()];
        galleys.push(para_galley(
            painter,
            text,
            slice,
            para,
            zoom,
            wrap_of(index),
        ));
    }
    let job = Arc::new(document_job(doc, zoom, content.x));
    let mut galley = Galley::concat(job, &galleys, painter.pixels_per_point());

    let mut placer = Placer {
        y: 0.0,
        page: 0,
        step: page_step(doc.page, zoom),
        height: content.y,
        strip: 0,
        strips,
        stride: strip_width + gap,
    };
    // Where each column of each page starts in the rows: (page, column, first row).
    let mut starts_of: Vec<(usize, usize, usize)> = Vec::new();
    let mut enter = |placer: &Placer, row: usize| {
        let here = (placer.page, placer.strip);
        if starts_of.last().is_none_or(|last| (last.0, last.1) != here) {
            starts_of.push((here.0, here.1, row));
        }
    };
    let mut cells: Vec<CellBox> = Vec::new();
    let mut markers = Vec::new();
    let mut pictures = Vec::new();
    let mut blocks: Vec<Block> = Vec::new();
    let mut rules = Vec::new();
    let mut para_pages = Vec::with_capacity(paras.len());
    // Table the last block is, to grow it row by row.
    let mut open_table = None;
    // First galley row of each paragraph.
    let mut first_rows = Vec::with_capacity(paras.len());
    let mut row = 0;
    for para_galley in &galleys {
        first_rows.push(row);
        row += para_galley.rows.len();
    }
    let rows_of = |index: usize| first_rows[index]..first_rows[index] + galleys[index].rows.len();
    let mut mark = |index: usize, pos: Pos2| {
        let para = &paras[index];
        if let Some(text) = labels[index].clone() {
            markers.push(Marker {
                pos: pos + vec2(lead(para), 0.0),
                text,
                format: marker_format(index),
                spacing: para.line_factor(),
            });
        }
    };
    // Place paragraph `index`'s picture with its top-left column corner at `origin`.
    let mut show_picture = |index: usize, origin: Pos2| {
        let para = &paras[index];
        let Some(picture) = para.picture else {
            return;
        };
        let max_width = inner_width(para);
        let aside = beside(para);
        let (size, across) = if aside.x > 0.0 {
            let right = para.wrap == Wrap::Right;
            (aside, if right { max_width - aside.x } else { 0.0 })
        } else {
            let size = picture_size(para, max_width, zoom);
            (size, shift(para.align, max_width, size.x))
        };
        let corner = origin + vec2(across, 0.0);
        pictures.push(PictureBox {
            rect: Rect::from_min_size(corner.round(), size),
            para: index,
            picture,
            max_width,
        });
    };

    let mut index = 0;
    while index < paras.len() {
        let para = &paras[index];
        let Some(cell) = para.cell else {
            open_table = None;
            if para.page_break && !placer.at_top() {
                placer.new_page();
            }
            let inset = insets[index];
            let (before, after) = room(para);
            let rows = &mut galley.rows[rows_of(index)];
            // A picture stays with the first line under it.
            let first = (picture_height(para) + rows[0].size.y).max(beside(para).y);
            placer.fit(before + first);
            para_pages.push(placer.page);
            let started = (placer.page, placer.strip);
            let mut top = placer.y;
            placer.y += before;
            // The text goes at least as far down as the picture beside it.
            let under_picture = placer.y + beside(para).y;
            show_picture(index, pos2(placer.x(), placer.y));
            placer.y += picture_height(para);
            mark(index, pos2(placer.x() + level_indent(para, zoom), placer.y));
            let mut hang = hanging(para, zoom);
            for (offset, row) in rows.iter_mut().enumerate() {
                placer.fit(row.size.y);
                enter(&placer, first_rows[index] + offset);
                // Only the first line hangs out.
                let origin = pos2(placer.x() + inset - std::mem::take(&mut hang), placer.y);
                placer.y += stack(
                    std::slice::from_mut(row),
                    origin,
                    wrap_of(index),
                    para.align,
                );
            }
            if started == (placer.page, placer.strip) {
                placer.y = placer.y.max(under_picture);
            }
            placer.y += after;
            if para.rule {
                placer.y += 3.0 * zoom;
                rules.push((pos2(placer.x(), placer.y.round()), strip_width));
                placer.y += 4.0 * zoom;
            }
            // A paragraph that went on to another column or page is grabbed by its end.
            if started != (placer.page, placer.strip) {
                top = placer.top();
            }
            let corner = pos2(placer.x(), top);
            blocks.push(Block {
                rect: Rect::from_min_max(corner, pos2(placer.x() + strip_width, placer.y)),
                paras: index..index + 1,
            });
            index += 1;
            continue;
        };

        // A table row: its cells side by side, kept together on one page.
        let same_row = |para: &Para| {
            para.cell
                .is_some_and(|other| other.table == cell.table && other.row == cell.row)
        };
        let end = index
            + paras[index..]
                .iter()
                .take_while(|para| same_row(para))
                .count();
        let width = column_width(para);
        let height_of = |index: usize| -> f32 {
            let rows: f32 = galleys[index].rows.iter().map(|row| row.size.y).sum();
            let (before, after) = room(&paras[index]);
            before + rows + picture_height(&paras[index]) + after
        };
        let mut heights: HashMap<u16, f32> = HashMap::new();
        for (offset, para) in paras[index..end].iter().enumerate() {
            let col = para.cell.map_or(0, |cell| cell.col);
            *heights.entry(col).or_default() += height_of(index + offset);
        }
        let row_height = heights.values().copied().fold(0.0, f32::max) + 2.0 * pad;
        placer.fit(row_height);
        enter(&placer, first_rows[index]);
        para_pages.resize(end, placer.page);
        let top = placer.y;
        let across = placer.x();
        let mut used: HashMap<u16, f32> = HashMap::new();
        for index in index..end {
            let para = &paras[index];
            let col = para.cell.map_or(0, |cell| cell.col);
            let left = across + f32::from(col) * width;
            let used = used.entry(col).or_default();
            let (before, after) = room(para);
            *used += before;
            show_picture(index, pos2(left + pad, top + pad + *used));
            *used += picture_height(para);
            let origin = pos2(left + pad, top + pad + *used);
            mark(index, origin + vec2(level_indent(para, zoom), 0.0));
            let rows = rows_of(index);
            let text_origin = origin + vec2(insets[index], 0.0);
            *used += stack(
                &mut galley.rows[rows.clone()],
                text_origin,
                wrap_of(index),
                para.align,
            );
            *used += after;
            if let Some(first) = galley.rows[rows.clone()].first_mut() {
                first.pos.x -= hanging(para, zoom);
            }
            let rect = Rect::from_min_size(pos2(left, top), vec2(width * joined(para), row_height));
            match cells.last_mut() {
                // Another paragraph of the same cell.
                Some(last) if last.rect == rect => last.rows.end = rows.end,
                _ => cells.push(CellBox {
                    rect,
                    rows,
                    chars: starts[index],
                }),
            }
        }
        placer.y = top + row_height;
        match blocks.last_mut() {
            Some(block) if open_table == Some(cell.table) => {
                block.rect.max.y = placer.y;
                block.paras.end = end;
            }
            _ => blocks.push(Block {
                rect: Rect::from_min_max(pos2(across, top), pos2(across + strip_width, placer.y)),
                paras: index..end,
            }),
        }
        open_table = Some(cell.table);
        index = end;
    }

    galley.rect = Rect::from_min_size(Pos2::ZERO, vec2(content.x, placer.top() + content.y));
    galley.mesh_bounds = galley.rows.iter().fold(Rect::NOTHING, |bounds, row| {
        bounds.union(row.visuals.mesh_bounds.translate(row.pos.to_vec2()))
    });
    // With several columns, a click finds its row among those of the column it is in.
    let mut strip_boxes = Vec::new();
    if strips > 1 {
        let counts: Vec<usize> = galley
            .rows
            .iter()
            .map(|row| row.char_count_including_newline().0)
            .collect();
        for (at, (page, strip, first)) in starts_of.iter().enumerate() {
            let end = starts_of
                .get(at + 1)
                .map_or(galley.rows.len(), |next| next.2);
            let corner = pos2(*strip as f32 * placer.stride, *page as f32 * placer.step);
            strip_boxes.push(CellBox {
                rect: Rect::from_min_size(corner, vec2(strip_width, content.y)),
                rows: *first..end,
                chars: counts[..*first].iter().sum(),
            });
        }
    }
    PageLayout {
        strips: strip_boxes,
        galley: Arc::new(galley),
        pages: placer.page + 1,
        cells,
        markers,
        pictures,
        blocks,
        rules,
        para_pages,
    }
}
