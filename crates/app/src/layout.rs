//! Page layout: every paragraph is laid out on its own (its width depends on lists and table
//! columns), then its rows are placed on A4 pages. The result is one galley the text field
//! edits, plus what is drawn around the text (table borders, list markers).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use egui::epaint::text::PlacedRow;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Galley, Painter, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use goddoc_core::{Align, Document, Font, Format, PageSetup, Para, Picture, Script};

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

/// (font, bold, egui family name, Windows file, other systems).
const FACES: [(Font, bool, &str, &str, &str); 16] = [
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
];

/// Register the document fonts from the system (Arial, Times New Roman, Courier New or their
/// Liberation twins). A missing file falls back to the UI font.
pub fn install_fonts(fonts: &mut egui::FontDefinitions) {
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (_, _, name, windows, other) in FACES {
        let mut family = fallback.clone();
        if let Some(bytes) = system_font(windows, other) {
            fonts
                .font_data
                .insert(name.to_owned(), egui::FontData::from_owned(bytes).into());
            family.insert(0, name.to_owned());
        }
        fonts.families.insert(FontFamily::Name(name.into()), family);
    }
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

fn family(font: Font, bold: bool) -> FontFamily {
    let name = FACES
        .iter()
        .find(|face| face.0 == font && face.1 == bold)
        .map_or("doc-sans", |face| face.2);
    FontFamily::Name(name.into())
}

/// How `format` is drawn at `zoom`; `spacing` is the paragraph's line spacing factor.
pub fn text_format(format: &Format, zoom: f32, spacing: f32) -> TextFormat {
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

/// Width of the margins on screen.
pub fn margin(page: PageSetup, zoom: f32) -> f32 {
    page.margins.points() * zoom
}

/// Size of the text area of a page on screen.
pub fn content_size(page: PageSetup, zoom: f32) -> Vec2 {
    page_size(page, zoom) - Vec2::splat(2.0 * margin(page, zoom))
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
    pub markers: Vec<Marker>,
    pub pictures: Vec<PictureBox>,
    pub blocks: Vec<Block>,
    /// Lines under paragraphs: (left end, length).
    pub rules: Vec<(Pos2, f32)>,
    /// Page (0 for the first) each paragraph starts on.
    pub para_pages: Vec<usize>,
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
        self.y = self.top();
    }

    /// Move to the next page when something `height` tall does not fit on this one.
    fn fit(&mut self, height: f32) {
        if self.y + height > self.top() + self.height + 0.5 && !self.at_top() {
            self.new_page();
        }
    }
}

/// Left inset of a paragraph from its indent level.
fn level_indent(para: &Para, zoom: f32) -> f32 {
    f32::from(para.indent) * INDENT_STEP * zoom
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
    let spacing = para.spacing.factor();
    if formats.is_empty() {
        // An empty section still gives the caret its height.
        job.append("", 0.0, text_format(&para.kind.format(), zoom, spacing));
    }
    let mut bytes = text.char_indices().map(|(byte, _)| byte).skip(1);
    let mut start = 0;
    for run in formats.chunk_by(|a, b| a == b) {
        let end = bytes.nth(run.len() - 1).unwrap_or(text.len());
        job.append(&text[start..end], 0.0, text_format(&run[0], zoom, spacing));
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
    let paras = doc.paras();
    let formats = doc.formats();
    let pad = CELL_PAD * zoom;
    // Space between a picture and the text under it.
    let picture_gap = 4.0 * zoom;
    let mut columns: HashMap<u32, f32> = HashMap::new();
    for cell in paras.iter().filter_map(|para| para.cell) {
        let count = columns.entry(cell.table).or_insert(1.0);
        *count = count.max(f32::from(cell.col) + 1.0);
    }
    // Width of the column a paragraph lives in, of what it can show, and of its text.
    let column_width = |para: &Para| match para.cell {
        Some(cell) => content.x / columns[&cell.table],
        None => content.x,
    };
    let inner_width = |para: &Para| {
        let padding = if para.cell.is_some() { 2.0 * pad } else { 0.0 };
        column_width(para) - padding
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
            let level = level_indent(para, zoom);
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
    let wrap_of = |index: usize| inner_width(&paras[index]) - insets[index];
    // Height a paragraph's picture takes, gap included.
    let picture_height = |para: &Para| {
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
                spacing: para.spacing.factor(),
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
        let size = picture_size(para, max_width, zoom);
        let corner = origin + vec2(shift(para.align, max_width, size.x), 0.0);
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
            let rows = &mut galley.rows[rows_of(index)];
            // A picture stays with the first line under it.
            placer.fit(picture_height(para) + rows[0].size.y);
            para_pages.push(placer.page);
            let top = placer.y;
            show_picture(index, pos2(0.0, placer.y));
            placer.y += picture_height(para);
            mark(index, pos2(level_indent(para, zoom), placer.y));
            for row in rows {
                placer.fit(row.size.y);
                let origin = pos2(inset, placer.y);
                placer.y += stack(
                    std::slice::from_mut(row),
                    origin,
                    wrap_of(index),
                    para.align,
                );
            }
            if para.rule {
                placer.y += 3.0 * zoom;
                rules.push((pos2(0.0, placer.y.round()), content.x));
                placer.y += 4.0 * zoom;
            }
            blocks.push(Block {
                rect: Rect::from_min_max(pos2(0.0, top), pos2(content.x, placer.y)),
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
            rows + picture_height(&paras[index])
        };
        let mut heights: HashMap<u16, f32> = HashMap::new();
        for (offset, para) in paras[index..end].iter().enumerate() {
            let col = para.cell.map_or(0, |cell| cell.col);
            *heights.entry(col).or_default() += height_of(index + offset);
        }
        let row_height = heights.values().copied().fold(0.0, f32::max) + 2.0 * pad;
        placer.fit(row_height);
        para_pages.resize(end, placer.page);
        let top = placer.y;
        let mut used: HashMap<u16, f32> = HashMap::new();
        for index in index..end {
            let para = &paras[index];
            let col = para.cell.map_or(0, |cell| cell.col);
            let left = f32::from(col) * width;
            let used = used.entry(col).or_default();
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
            let rect = Rect::from_min_size(pos2(left, top), vec2(width, row_height));
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
                rect: Rect::from_min_max(pos2(0.0, top), pos2(content.x, placer.y)),
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
    PageLayout {
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
