//! The pages: one rich-text field laid out by [`crate::layout`], over the A4 sheets, table
//! borders, list markers and pictures drawn here.
//!
//! egui edits plain text; the document keeps a format per character and follows each edit.
//! Pictures and whole blocks (a paragraph, a table) are moved by dragging: a picture by
//! itself, anything else by the grip that shows in the left margin.

use std::ops::Range;
use std::sync::Arc;

use egui::text::{CCursor, CCursorRange};
use egui::{
    Color32, Context, CornerRadius, CursorIcon, FontId, Frame, Id, Margin, Rect, ScrollArea, Sense,
    Shadow, Stroke, StrokeKind, TextBuffer, TextEdit, TextureId, Ui, UiBuilder, Vec2, pos2, vec2,
};
use goddoc_core::{Align, Document, Format, Image, Kind, List, Picture};

use crate::layout::{self, PageLayout, PictureBox};
use crate::widgets::icon;

/// Space around the pages on the canvas.
pub const PAD: f32 = 28.0;
const TABLE_BORDER: Color32 = Color32::from_gray(0x33);
/// How far left of the text the block grip sits, and how wide the zone that shows it is.
const GRIP_OFFSET: f32 = 26.0;
const GRIP_ZONE: f32 = 44.0;
/// Narrowest a picture can be made, in points.
const MIN_PICTURE: f32 = 16.0;
const HANDLE: f32 = 9.0;
/// What is laid over selected text.
const SELECTION: Color32 = Color32::from_black_alpha(52);
/// Colour of the formatting marks: seen, and not taken for text.
const MARK_COLOR: Color32 = Color32::from_rgb(0x5B, 0x8D, 0xD6);
/// Seconds the caret shows, then hides: the blink of Windows.
const CARET_BLINK: f64 = 0.53;
/// Height of the caret, in font sizes.
const CARET_HEIGHT: f32 = 1.18;

/// Gives the texture of an image, decoding it on first use.
pub type Textures<'a> = &'a mut dyn FnMut(&Context, &Image) -> Option<TextureId>;

/// What the editor asks the app for.
pub struct Hooks<'a> {
    pub textures: Textures<'a>,
    /// Fills the right-click menu.
    pub menu: &'a mut dyn FnMut(&mut Ui),
}

enum Drag {
    /// Block (index in the layout) being moved; a picture also picks its alignment.
    Move { block: usize, picture: bool },
    /// Picture of paragraph `para` being resized to `width` on screen.
    Resize { para: usize, width: f32 },
}

/// A finished drag, to apply to the document.
enum Change {
    Resize(usize, Picture),
    Move {
        paras: Range<usize>,
        before: usize,
        align: Option<Align>,
    },
}

/// Editing state of one document besides the caret.
#[derive(Default)]
pub struct View {
    /// Paragraph whose picture is selected.
    pub selected_picture: Option<usize>,
    drag: Option<Drag>,
    /// Character to bring into view.
    pub scroll_to: Option<usize>,
    /// Match of the search in progress, shown marked.
    pub found: Option<Range<usize>>,
    /// Where the right-click menu is open.
    menu_at: Option<egui::Pos2>,
    /// Show what stands for spaces, tabs and the ends of paragraphs.
    pub marks: bool,
    /// Only the sheets and what is written on them: for printing, not for the screen.
    plain: bool,
    /// Words the dictionary does not know, in characters and in order: they are underlined.
    pub misspelled: Vec<Range<usize>>,
    /// Where the caret is, and since when: its blink starts over when it moves.
    caret: (usize, f64),
}

impl View {
    /// A view that shows only the sheets and what is written on them, to print them.
    pub fn plain() -> Self {
        Self {
            plain: true,
            ..Self::default()
        }
    }
}

pub struct Output {
    /// Selected characters (empty at a caret); `None` while the page has no focus.
    pub selection: Option<Range<usize>>,
    /// The document as it was before this frame's edit, if there was one.
    pub before: Option<Document>,
    /// The edit was a move or a resize, not typing: it is its own undo step.
    pub structural: bool,
    pub pages: usize,
    /// Page (0 for the first) each paragraph starts on.
    pub para_pages: Vec<usize>,
    /// Table cells on screen, for tests that click in them.
    #[cfg(test)]
    cells: Vec<Rect>,
}

/// Put the text field's caret at `head`, selecting from `anchor` (`None` keeps the current
/// anchor, for a drag in progress).
pub fn set_selection(ctx: &Context, id: Id, anchor: Option<usize>, head: usize) {
    let mut state = TextEdit::load_state(ctx, id).unwrap_or_default();
    let kept = state.cursor.char_range().map(|range| range.secondary);
    let secondary = anchor
        .map(CCursor::new)
        .or(kept)
        .unwrap_or_else(|| CCursor::new(head));
    state.cursor.set_char_range(Some(CCursorRange {
        primary: CCursor::new(head),
        secondary,
        h_pos: None,
    }));
    state.store(ctx, id);
}

/// Sheets, page numbers, table borders and list markers under the text. `origin` is the
/// top-left of the first page's text area.
fn paint_pages(
    ui: &Ui,
    pages: &PageLayout,
    first_page: Rect,
    origin: Vec2,
    zoom: f32,
    doc: &Document,
    plain: bool,
) {
    let painter = ui.painter();
    let shadow = Shadow {
        offset: [0, 4],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    let number_font = layout::text_format(&Format::default(), zoom, 1.0).font_id;
    let step = layout::page_step(doc.page, zoom);
    let [top, right, bottom, left] = layout::margins(doc.page, zoom);
    let (header, footer) = (&doc.header, doc.shown_footer());
    for page in 0..pages.pages {
        let sheet = first_page.translate(vec2(0.0, page as f32 * step));
        if !ui.is_rect_visible(sheet.expand(24.0)) {
            continue;
        }
        // To print, a page is only its sheet, square to its corners.
        let corners = if plain { 0 } else { 2 };
        if !plain {
            painter.add(shadow.as_shape(sheet, CornerRadius::same(corners)));
        }
        painter.rect_filled(sheet, CornerRadius::same(corners), layout::paper());
        // The header in the middle of the top margin, the footer in that of the bottom one;
        // each at the left, in the middle and at the right of the text area.
        let (from, to) = (sheet.left() + left, sheet.right() - right);
        let banners = [
            (header, sheet.top() + top / 2.0),
            (&footer, sheet.bottom() - bottom / 2.0),
        ];
        for (banner, y) in banners {
            let zones = [
                (&banner.left, from, egui::Align2::LEFT_CENTER),
                (
                    &banner.center,
                    (from + to) / 2.0,
                    egui::Align2::CENTER_CENTER,
                ),
                (&banner.right, to, egui::Align2::RIGHT_CENTER),
            ];
            for (zone, x, anchor) in zones {
                if !zone.trim().is_empty() {
                    let said = goddoc_core::Banner::on_page(zone, page + 1, pages.pages);
                    let ink = layout::ink(Color32::from_gray(0x44));
                    painter.text(pos2(x, y), anchor, said, number_font.clone(), ink);
                }
            }
        }
    }
    for cell in &pages.cells {
        let rect = cell.rect.translate(origin);
        if ui.is_rect_visible(rect) {
            let border = Stroke::new(1.0, layout::ink(TABLE_BORDER));
            painter.rect_stroke(rect, 0.0, border, StrokeKind::Middle);
        }
    }
    for (left, length) in &pages.rules {
        let left = *left + origin;
        let line = Stroke::new(1.0, layout::ink(TABLE_BORDER));
        painter.hline(left.x..=left.x + length, left.y, line);
    }
    for marker in &pages.markers {
        let look = layout::text_format(&marker.format, zoom, marker.spacing);
        let pos = marker.pos + origin;
        if ui.is_rect_visible(Rect::from_min_size(pos, Vec2::splat(look.font_id.size))) {
            let job = egui::text::LayoutJob::single_section(marker.text.clone(), look);
            painter.galley(pos, painter.layout_job(job), Color32::BLACK);
        }
    }
}

fn paint_pictures(
    ui: &Ui,
    pages: &PageLayout,
    origin: Vec2,
    doc: &Document,
    textures: Textures<'_>,
) {
    for picture in &pages.pictures {
        let rect = picture.rect.translate(origin);
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let texture = doc
            .images()
            .get(picture.picture.image as usize)
            .and_then(|image| textures(ui.ctx(), image));
        match texture {
            Some(texture) => {
                let whole = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                ui.painter().image(texture, rect, whole, Color32::WHITE);
            }
            // An image that cannot be decoded keeps its place.
            None => {
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_gray(0xE6));
            }
        }
    }
}

/// Right click on the pages: `menu` next to the pointer, until the next click.
fn context_menu(ui: &Ui, id: Id, sheets: Rect, view: &mut View, menu: &mut dyn FnMut(&mut Ui)) {
    let (right_click, any_click, escape, pointer) = ui.input(|input| {
        (
            input.pointer.secondary_clicked(),
            input.pointer.any_click(),
            input.key_pressed(egui::Key::Escape),
            input.pointer.interact_pos(),
        )
    });
    let opened =
        right_click && pointer.is_some_and(|at| sheets.contains(at) && ui.clip_rect().contains(at));
    if opened {
        view.menu_at = pointer;
        // The click landed on a cell or a picture, not the text field: keep the keyboard there.
        ui.memory_mut(|memory| memory.request_focus(id));
    }
    let Some(at) = view.menu_at else {
        return;
    };
    egui::Area::new(id.with("context-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .constrain(true)
        .show(ui.ctx(), |ui| {
            Frame::popup(ui.style()).show(ui, |ui| menu(ui));
        });
    // Any click after the one that opened it closes it, picking an option included.
    if (any_click && !opened) || escape {
        view.menu_at = None;
    }
}

/// Mark the characters `found` (a search match).
fn paint_found(ui: &Ui, pages: &PageLayout, origin: Vec2, found: &Range<usize>) {
    let start = pages.galley.pos_from_cursor(CCursor::new(found.start));
    let end = pages.galley.pos_from_cursor(CCursor::new(found.end));
    // A match never spans paragraphs; one wrapped over two lines is marked at its start.
    let same_line = (start.top() - end.top()).abs() < 1.0;
    let right = if same_line {
        end.right()
    } else {
        start.right() + 12.0
    };
    let mark = Rect::from_min_max(start.left_top(), pos2(right, start.bottom())).translate(origin);
    let painter = ui.painter();
    painter.rect_filled(mark, 2.0, Color32::from_black_alpha(50));
    let outline = Stroke::new(1.0, Color32::BLACK);
    painter.rect_stroke(mark, 2.0, outline, StrokeKind::Outside);
}

/// Clicks and drags in table cells: put the caret where the pointer is.
fn cell_pointer(ui: &Ui, id: Id, pages: &PageLayout, origin: Vec2) {
    // The columns first: a table cell in one of them is over it, and takes the click.
    for (index, cell) in pages.strips.iter().chain(&pages.cells).enumerate() {
        let rect = cell.rect.translate(origin);
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let response = ui
            .interact(rect, id.with(("cell", index)), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::Text);
        // Releasing the button counts as a click away from the text field, which drops its
        // focus: take it back then too, or nothing can be typed in the cell.
        if response.clicked() || response.drag_stopped() {
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        // Only the main button moves the caret: a right click keeps the selection.
        let primary = ui.input(|input| input.pointer.primary_down());
        if !response.is_pointer_button_down_on() || !primary {
            continue;
        }
        let Some(pointer) = response.interact_pointer_pos() else {
            continue;
        };
        let head = pages.cursor_in(cell, pointer.to_vec2() - origin);
        let (pressed, shift) =
            ui.input(|input| (input.pointer.any_pressed(), input.modifiers.shift));
        // A press starts a selection here; shift and dragging extend the current one.
        let anchor = (pressed && !shift).then_some(head);
        set_selection(ui.ctx(), id, anchor, head);
        ui.memory_mut(|memory| memory.request_focus(id));
        ui.ctx().request_repaint();
    }
}

/// Where a picture resized to `width` shows: it keeps its top and its paragraph's alignment.
fn resized_rect(picture: &PictureBox, rect: Rect, width: f32, align: Align) -> Rect {
    let column_left = rect.left() - layout::shift(align, picture.max_width, rect.width());
    let height = width * rect.height() / rect.width().max(1.0);
    let left = column_left + layout::shift(align, picture.max_width, width);
    Rect::from_min_size(pos2(left, rect.top()), vec2(width, height))
}

/// Select, move and resize pictures. Returns whether the pointer is pressed on one, and a
/// finished resize.
fn picture_pointer(
    ui: &Ui,
    id: Id,
    doc: &Document,
    pages: &PageLayout,
    origin: Vec2,
    zoom: f32,
    view: &mut View,
) -> (bool, Option<Change>) {
    let mut pressed = false;
    let mut change = None;
    for picture in &pages.pictures {
        let rect = picture.rect.translate(origin);
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let para = doc.paras()[picture.para];
        let response = ui
            .interact(
                rect,
                id.with(("picture", picture.para)),
                Sense::click_and_drag(),
            )
            .on_hover_cursor(CursorIcon::Grab);
        pressed |= response.is_pointer_button_down_on();
        if response.clicked() || response.drag_started() {
            view.selected_picture = Some(picture.para);
            let caret = doc.para_chars(picture.para).start;
            set_selection(ui.ctx(), id, Some(caret), caret);
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        // A picture in a table stays in its cell.
        if response.drag_started()
            && para.cell.is_none()
            && let Some(block) = pages
                .blocks
                .iter()
                .position(|block| block.paras.contains(&picture.para))
        {
            view.drag = Some(Drag::Move {
                block,
                picture: true,
            });
        }
        if view.selected_picture != Some(picture.para) {
            continue;
        }

        let resizing = match view.drag {
            Some(Drag::Resize { para, width }) if para == picture.para => Some(width),
            _ => None,
        };
        let shown = resizing.map_or(rect, |width| resized_rect(picture, rect, width, para.align));
        let painter = ui.painter();
        painter.rect_stroke(
            shown,
            0.0,
            Stroke::new(1.5, Color32::BLACK),
            StrokeKind::Outside,
        );
        let corners = [
            shown.left_top(),
            shown.right_top(),
            shown.right_bottom(),
            shown.left_bottom(),
        ];
        for (corner, at) in corners.into_iter().enumerate() {
            let handle = Rect::from_center_size(at, Vec2::splat(HANDLE));
            let cursor = if corner % 2 == 0 {
                CursorIcon::ResizeNwSe
            } else {
                CursorIcon::ResizeNeSw
            };
            let grab = ui
                .interact(
                    handle.expand(4.0),
                    id.with(("handle", corner)),
                    Sense::drag(),
                )
                .on_hover_cursor(cursor);
            pressed |= grab.is_pointer_button_down_on();
            ui.painter().rect(
                handle,
                0.0,
                Color32::WHITE,
                Stroke::new(1.5, Color32::BLACK),
                StrokeKind::Inside,
            );
            if grab.dragged()
                && let Some(pointer) = grab.interact_pointer_pos()
            {
                // The opposite side stays where it is.
                let width = if corner == 1 || corner == 2 {
                    pointer.x - shown.left()
                } else {
                    shown.right() - pointer.x
                };
                view.drag = Some(Drag::Resize {
                    para: picture.para,
                    width: width.clamp(MIN_PICTURE * zoom, picture.max_width),
                });
            }
            if grab.drag_stopped()
                && let Some(Drag::Resize { para, width }) = view.drag.take()
            {
                let aspect = f32::from(picture.picture.height) / f32::from(picture.picture.width);
                let points = (width / zoom).round().max(1.0);
                let resized = Picture {
                    width: points as u16,
                    height: (points * aspect).round().max(1.0) as u16,
                    ..picture.picture
                };
                change = Some(Change::Resize(para, resized));
            }
        }
    }
    (pressed, change)
}

/// Grip in the left margin of the block under the pointer: drag it to move the block.
fn block_grips(ui: &Ui, id: Id, pages: &PageLayout, origin: Vec2, view: &mut View) {
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let hovered = pointer.and_then(|pointer| {
        let local = pointer - origin;
        pages.blocks.iter().position(|block| {
            (block.rect.top()..=block.rect.bottom()).contains(&local.y)
                && (-GRIP_ZONE..=block.rect.right()).contains(&local.x)
        })
    });
    // The grip of a block being dragged stays, so the drag keeps its owner.
    let shown = match view.drag {
        Some(Drag::Move {
            block,
            picture: false,
        }) => Some(block),
        Some(_) => None,
        None => hovered,
    };
    let Some((index, block)) = shown.and_then(|index| Some((index, pages.blocks.get(index)?)))
    else {
        return;
    };
    let corner = block.rect.left_top() + origin + vec2(-GRIP_OFFSET, 0.0);
    let grip = Rect::from_min_size(corner, vec2(18.0, 22.0));
    let response = ui
        .interact(grip, id.with(("grip", index)), Sense::drag())
        .on_hover_cursor(CursorIcon::Grab)
        .on_hover_text("Drag to move");
    let color = if response.hovered() || response.dragged() {
        Color32::from_gray(0x33)
    } else {
        Color32::from_gray(0xAA)
    };
    ui.painter().text(
        grip.center(),
        egui::Align2::CENTER_CENTER,
        icon::DOTS_SIX_VERTICAL,
        FontId::proportional(16.0),
        color,
    );
    if response.drag_started() {
        view.drag = Some(Drag::Move {
            block: index,
            picture: false,
        });
    }
}

/// While a block is dragged: show where it would land; on release, say where it goes.
fn move_drag(ui: &Ui, pages: &PageLayout, origin: Vec2, view: &mut View) -> Option<Change> {
    let Some(Drag::Move { block, picture }) = view.drag else {
        return None;
    };
    let source = pages.blocks.get(block)?;
    let pointer = ui.input(|input| input.pointer.latest_pos())?;
    let local = pointer - origin;
    let (before, gap) = pages.drop_target(local.y);
    let width = pages.galley.rect.width();
    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    let line = Stroke::new(2.0, Color32::BLACK);
    let y = origin.y + gap;
    ui.painter().hline(origin.x..=origin.x + width, y, line);
    ui.painter()
        .circle_filled(pos2(origin.x, y), 3.5, Color32::BLACK);
    if ui.input(|input| input.pointer.primary_down()) {
        return None;
    }
    view.drag = None;
    // A picture also lands left, centred or right, wherever it is let go.
    let align = picture.then(|| match (local.x / width * 3.0).floor() as i32 {
        ..=0 => Align::Left,
        1 => Align::Center,
        _ => Align::Right,
    });
    Some(Change::Move {
        paras: source.paras.clone(),
        before,
        align,
    })
}

/// Apply a finished drag. Returns the document as it was before, when anything changed.
fn apply(ui: &Ui, id: Id, doc: &mut Document, change: Change, view: &mut View) -> Option<Document> {
    let before = doc.clone();
    match change {
        Change::Resize(para, picture) => {
            if doc.paras()[para].picture == Some(picture) {
                return None;
            }
            doc.set_picture(para, Some(picture));
        }
        Change::Move {
            paras,
            before: target,
            align,
        } => {
            let realign = align.filter(|align| doc.paras()[paras.start].align != *align);
            let moved = doc.move_paras(paras.clone(), target);
            if moved.is_none() && realign.is_none() {
                return None;
            }
            let start = moved.unwrap_or(paras.start);
            if let Some(align) = realign {
                doc.set_align(start..start + paras.len(), align);
            }
            if view.selected_picture.is_some() {
                view.selected_picture = Some(start);
            }
            let caret = doc.para_chars(start).start;
            set_selection(ui.ctx(), id, Some(caret), caret);
        }
    }
    Some(before)
}

/// The caret at character `index`: a thin line from `top` to `bottom` at `x`, blinking as
/// carets do in Windows.
fn paint_caret(ui: &Ui, x: f32, top: f32, bottom: f32, index: usize, view: &mut View) {
    let now = ui.input(|input| input.time);
    if view.caret.0 != index {
        view.caret = (index, now);
    }
    let phase = (now - view.caret.1) % (2.0 * CARET_BLINK);
    if phase < CARET_BLINK {
        let line = Stroke::new(1.0, Color32::BLACK);
        ui.painter().vline(x, top..=bottom, line);
    }
    let wait = CARET_BLINK - phase % CARET_BLINK;
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs_f64(wait));
}

/// Formatting marks of the text of `galley`, shown at `at`.
fn paint_marks(ui: &Ui, galley: &egui::Galley, at: egui::Pos2, doc: &Document, zoom: f32) {
    let visible = ui.clip_rect().translate(-at.to_vec2());
    for mark in layout::marks(galley, doc.formats(), zoom, visible) {
        let font = FontId::proportional(mark.size * 0.85);
        let centre = egui::Align2::CENTER_CENTER;
        ui.painter().text(
            mark.pos + at.to_vec2(),
            centre,
            mark.glyph,
            font,
            MARK_COLOR,
        );
    }
}

/// Underlines of the text of `galley`, shown at `at`.
fn paint_underlines(ui: &Ui, galley: &egui::Galley, at: egui::Pos2, doc: &Document, zoom: f32) {
    let visible = ui.clip_rect().translate(-at.to_vec2());
    for line in layout::underlines(galley, doc.formats(), zoom, visible) {
        let (from, to) = (line.from + at.to_vec2(), line.to + at.to_vec2());
        ui.painter().hline(from.x..=to.x, from.y, line.stroke);
    }
}

/// A red wave under the words of `galley` (shown at `at`) the dictionary does not know.
fn paint_misspelled(ui: &Ui, galley: &egui::Galley, at: egui::Pos2, words: &[Range<usize>]) {
    if words.is_empty() {
        return;
    }
    let visible = ui.clip_rect().translate(-at.to_vec2());
    let stroke = Stroke::new(1.0, Color32::from_rgb(0xE0, 0x2B, 0x2B));
    for (left, length) in layout::marked_words(galley, words, visible) {
        let steps = (length / 2.0).ceil() as usize;
        let wave = (0..=steps).map(|step| {
            let down = if step % 2 == 0 { 0.0 } else { 2.0 };
            left + at.to_vec2() + vec2((step as f32 * 2.0).min(length), down)
        });
        ui.painter().add(egui::Shape::line(wave.collect(), stroke));
    }
}

/// Byte at which character `index` of `text` starts.
fn byte_of(text: &str, index: usize) -> usize {
    let mut bytes = text.char_indices().map(|(byte, _)| byte);
    bytes.nth(index).unwrap_or(text.len())
}

/// What typing made of the document, or `None` when the edit is not allowed (it would break a
/// table apart). `caret` is where the caret was, when nothing was selected. The second value
/// is where the caret must go when the edit was reinterpreted.
fn edited(
    doc: &Document,
    buffer: &str,
    typing: Format,
    caret: Option<usize>,
) -> Option<(Document, Option<usize>)> {
    let (mut range, inserted) = doc.diff(buffer);
    // Typed among characters equal to it, text shows further than it was put: the caret
    // tells where it went (Enter between two line breaks, a letter doubled).
    if let Some(caret) = caret.filter(|caret| range.is_empty() && *caret < range.start) {
        let (before, after) = doc.text().split_at(byte_of(doc.text(), caret));
        if [before, inserted.as_str(), after].concat() == buffer {
            range = caret..caret;
        }
    }
    let para = doc.para_at(range.start);
    let chars = doc.para_chars(para);
    let here = doc.paras()[para];
    let mut edited = doc.clone();
    // Enter on an empty list item leaves its level, then the list, instead of adding an item.
    if range.is_empty() && inserted == "\n" && here.list != List::None && chars.is_empty() {
        if here.indent == 0 || !edited.nest(para..para + 1, false) {
            edited.set_list(para..para + 1, List::None);
        }
        return Some((edited, Some(range.start)));
    }
    // Backspace at the start of a list item takes it out of the list and keeps its text.
    if let Some(caret) = caret.filter(|caret| *caret > 0 && inserted.is_empty() && range.len() == 1)
    {
        let item = doc.para_at(caret);
        let listed = doc.paras()[item].list != List::None && doc.para_chars(item).start == caret;
        // Equal neighbours hide which character went: make sure it is the break before.
        let joined = doc.text().chars().enumerate();
        let joined: String = joined
            .filter(|(at, _)| *at != caret - 1)
            .map(|(_, ch)| ch)
            .collect();
        if listed && joined == buffer {
            edited.set_list(item..item + 1, List::None);
            return Some((edited, Some(caret)));
        }
    }
    // "-", "*" or "1." then a space at the start of a paragraph starts a list.
    if range.is_empty() && inserted == " " && here.list == List::None {
        let typed = doc.text().chars().skip(chars.start);
        let typed: String = typed.take(range.start - chars.start).collect();
        // A heading can be numbered, not bulleted.
        let list = List::of_shortcut(&typed)
            .filter(|list| here.kind == Kind::Normal || *list == List::Number);
        if let Some(list) = list {
            edited.replace(chars.start..range.start, "", typing);
            edited.set_list(para..para + 1, list);
            return Some((edited, Some(chars.start)));
        }
    }
    edited.replace(range, &inserted, typing);
    edited.tables_intact(doc).then_some((edited, None))
}

/// Show `doc` on its pages and let the user edit it. New text is typed in `typing`.
/// Points scrolled by one notch of the wheel.
const WHEEL_LINE: f32 = 100.0;

/// How far the wheel asks the pages to move this frame. Ctrl+wheel zooms: it is not counted.
fn wheel_delta(ui: &Ui) -> Vec2 {
    let page = ui.clip_rect().height();
    ui.input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } if !modifiers.command && !modifiers.ctrl => {
                    let delta = match unit {
                        egui::MouseWheelUnit::Point => *delta,
                        egui::MouseWheelUnit::Line => *delta * WHEEL_LINE,
                        egui::MouseWheelUnit::Page => *delta * page,
                    };
                    Some(if modifiers.shift {
                        vec2(delta.x + delta.y, 0.0)
                    } else {
                        delta
                    })
                }
                _ => None,
            })
            .fold(Vec2::ZERO, |total, delta| total + delta)
    })
}

pub fn show(
    ui: &mut Ui,
    id: Id,
    doc: &mut Document,
    typing: Format,
    zoom: f32,
    view: &mut View,
    hooks: Hooks<'_>,
) -> Output {
    let mut out = Output {
        selection: None,
        before: None,
        structural: false,
        pages: 1,
        para_pages: Vec::new(),
        #[cfg(test)]
        cells: Vec::new(),
    };
    // A picture that is gone (undo, deleted paragraph) is no longer selected.
    view.selected_picture = view.selected_picture.filter(|para| {
        doc.paras()
            .get(*para)
            .is_some_and(|para| para.picture.is_some())
    });
    let wheel = egui::scroll_area::ScrollSource {
        mouse_wheel: false,
        ..Default::default()
    };
    // The wheel is read here: each turn moves the pages at once, and none is lost while
    // they still move.
    let area = ScrollArea::both().scroll_source(wheel).animated(false);
    area.auto_shrink(false).show(ui, |ui| {
        if ui.rect_contains_pointer(ui.clip_rect()) {
            let delta = wheel_delta(ui);
            if delta != Vec2::ZERO {
                ui.scroll_with_delta(delta);
            }
        }
        let pages = layout::layout(ui.painter(), doc, zoom);
        out.pages = pages.pages;
        out.para_pages.clone_from(&pages.para_pages);
        let page_size = layout::page_size(doc.page, zoom);
        let step = layout::page_step(doc.page, zoom);
        let height = (pages.pages - 1) as f32 * step + page_size.y;
        let width = (page_size.x + 2.0 * PAD).max(ui.available_width());
        let (outer, _) = ui.allocate_exact_size(vec2(width, height + 2.0 * PAD), Sense::hover());
        let first_page = Rect::from_min_size(
            pos2(outer.center().x - page_size.x / 2.0, outer.min.y + PAD),
            page_size,
        );
        let text_area = Rect::from_min_size(
            first_page.min + {
                let [top, _, _, left] = layout::margins(doc.page, zoom);
                vec2(left, top)
            },
            pages.galley.size(),
        );
        let origin = text_area.min.to_vec2();
        #[cfg(test)]
        {
            out.cells = pages
                .cells
                .iter()
                .map(|cell| cell.rect.translate(origin))
                .collect();
        }
        paint_pages(ui, &pages, first_page, origin, zoom, doc, view.plain);
        paint_pictures(ui, &pages, origin, doc, hooks.textures);
        if let Some(index) = view.scroll_to.take() {
            let caret = pages.galley.pos_from_cursor(CCursor::new(index));
            let target = caret.translate(origin).expand2(vec2(0.0, 24.0));
            ui.scroll_to_rect(target, Some(egui::Align::Min));
        }
        // Under the text field: a click anywhere on the paper starts typing.
        let paper = ui
            .interact(
                first_page.with_max_y(first_page.min.y + height),
                id.with("paper"),
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::Text);

        let mut child = ui.new_child(UiBuilder::new().max_rect(text_area));
        // The paper is white in both themes.
        let visuals = child.visuals_mut();
        // The caret is drawn here, by `paint_caret`.
        visuals.text_cursor.stroke = Stroke::NONE;
        // The selection is drawn here, by `paint_selection`.
        visuals.selection.bg_fill = Color32::TRANSPARENT;
        visuals.selection.stroke.color = Color32::BLACK;

        let mut buffer = doc.text().to_owned();
        // Where the caret is before this frame's typing, when nothing is selected.
        let caret = TextEdit::load_state(ui.ctx(), id)
            .and_then(|state| state.cursor.char_range())
            .filter(|range| range.primary == range.secondary)
            .map(|range| range.primary.index.0);
        // A right click opens the menu on what is selected: it must not move the caret.
        let right_click = ui.input(|input| {
            let button = egui::PointerButton::Secondary;
            let pointer = &input.pointer;
            pointer.button_down(button)
                || pointer.button_pressed(button)
                || pointer.button_released(button)
        });
        let output = {
            let doc = &*doc;
            let mut layouter = |ui: &Ui, text: &dyn TextBuffer, _wrap: f32| {
                if text.as_str() == doc.text() {
                    return Arc::clone(&pages.galley);
                }
                // Mid-edit: lay out what the document is about to become.
                let mut edited = doc.clone();
                edited.sync(text.as_str(), typing);
                layout::layout(ui.painter(), &edited, zoom).galley
            };
            TextEdit::multiline(&mut buffer)
                .id(id)
                .frame(Frame::NONE)
                .margin(Margin::ZERO)
                .desired_width(text_area.width())
                .lock_focus(true)
                .interactive(!right_click)
                .layouter(&mut layouter)
                .show(&mut child)
        };
        cell_pointer(ui, id, &pages, origin);
        if let Some(found) = &view.found {
            paint_found(ui, &pages, origin, found);
        }
        let (on_picture, resized) = picture_pointer(ui, id, doc, &pages, origin, zoom, view);
        block_grips(ui, id, &pages, origin, view);
        let sheets = first_page.with_max_y(first_page.min.y + height);
        context_menu(ui, id, sheets, view, hooks.menu);
        let change = resized.or_else(|| move_drag(ui, &pages, origin, view));
        if ui.input(|input| input.pointer.any_pressed()) && !on_picture {
            view.selected_picture = None;
        }
        if paper.clicked() {
            output.response.response.request_focus();
        }
        if buffer != doc.text() {
            if let Some((edited, caret)) = edited(doc, &buffer, typing, caret) {
                out.before = Some(std::mem::replace(doc, edited));
                view.selected_picture = None;
                view.found = None;
                if let Some(caret) = caret {
                    set_selection(ui.ctx(), id, Some(caret), caret);
                }
            }
            ui.ctx().request_repaint();
        } else if let Some(change) = change
            && let Some(before) = apply(ui, id, doc, change, view)
        {
            out.before = Some(before);
            out.structural = true;
            ui.ctx().request_repaint();
        }
        // The galley is that of the text typed: the formats are only its own once the edit is in.
        if output.galley.text() == doc.text() {
            let focused = output.response.response.has_focus();
            if let Some(range) = output.cursor_range.filter(|_| focused) {
                let (a, b) = (range.primary.index.0, range.secondary.index.0);
                let height = zoom * CARET_HEIGHT;
                let formats = doc.formats();
                for rect in layout::selection(&output.galley, formats, a.min(b)..a.max(b), height) {
                    let rect = rect.translate(output.galley_pos.to_vec2());
                    ui.painter().rect_filled(rect, 0.0, SELECTION);
                }
            }
            paint_underlines(ui, &output.galley, output.galley_pos, doc, zoom);
            paint_misspelled(ui, &output.galley, output.galley_pos, &view.misspelled);
            if view.marks {
                paint_marks(ui, &output.galley, output.galley_pos, doc, zoom);
            }
        }
        if let Some(range) = output
            .cursor_range
            .filter(|_| output.response.response.has_focus())
        {
            let index = range.primary.index.0;
            let line = output.galley.pos_from_cursor(range.primary);
            let line = line.translate(output.galley_pos.to_vec2());
            // As tall as the text typed there, not as the line, which line spacing makes
            // taller. Text sits at the top of its share of the line, shares at its bottom.
            let size = typing.size * zoom;
            let spacing = doc.paras()[doc.para_at(index)].line_factor();
            let top = line
                .top()
                .max(line.bottom() - size * layout::LINE_HEIGHT * spacing);
            let bottom = (top + size * CARET_HEIGHT).min(line.bottom());
            paint_caret(ui, line.center().x, top, bottom, index, view);
        }
        out.selection = output.cursor_range.map(|range| {
            let (a, b) = (range.primary.index.0, range.secondary.index.0);
            a.min(b)..a.max(b)
        });
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, RawInput};

    /// The editor run frame by frame with scripted input.
    struct Harness {
        ctx: Context,
        doc: Document,
        view: View,
        time: f64,
        /// Frames in which the right-click menu was shown.
        menu_frames: usize,
    }

    impl Harness {
        fn new(doc: Document) -> Self {
            let ctx = Context::default();
            let mut fonts = egui::FontDefinitions::default();
            layout::install_fonts(&mut fonts);
            ctx.set_fonts(fonts);
            let mut harness = Self {
                ctx,
                doc,
                view: View::default(),
                time: 0.0,
                menu_frames: 0,
            };
            harness.frame(Vec::new());
            harness
        }

        fn frame(&mut self, events: Vec<Event>) -> Output {
            let Self {
                ctx,
                doc,
                view,
                time,
                menu_frames,
            } = self;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 1400.0))),
                events,
                time: Some(*time),
                focused: true,
                ..Default::default()
            };
            *time += 0.1;
            let mut out = None;
            let _ = ctx.run_ui(input, |ui| {
                let id = Id::new("editor");
                let typing = Format::default();
                let hooks = Hooks {
                    textures: &mut |_, _| None,
                    menu: &mut |_| *menu_frames += 1,
                };
                out = Some(show(ui, id, doc, typing, 1.0, view, hooks));
            });
            out.expect("the editor ran")
        }

        /// Click in the middle of table cell `cell`.
        fn click_cell(&mut self, cell: usize) {
            let pos = self.frame(Vec::new()).cells[cell].center();
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            self.frame(Vec::new());
        }

        fn type_text(&mut self, text: &str) {
            self.frame(vec![Event::Text(text.to_owned())]);
            self.frame(Vec::new());
        }

        /// Give the page the keyboard, the caret at character `caret`.
        fn focus(&mut self, caret: usize) {
            let id = Id::new("editor");
            self.ctx.memory_mut(|memory| memory.request_focus(id));
            set_selection(&self.ctx, id, Some(caret), caret);
            self.frame(Vec::new());
        }

        fn press(&mut self, key: egui::Key) {
            self.frame(vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]);
            self.frame(Vec::new());
        }

        fn lists(&self) -> Vec<List> {
            self.doc.paras().iter().map(|para| para.list).collect()
        }
    }

    #[test]
    fn right_click_opens_the_menu_and_keeps_the_caret() {
        let mut doc = Document::default();
        doc.replace(0..0, "intro", Format::default());
        doc.insert_table(0, 1, 2).expect("a table");
        let mut harness = Harness::new(doc);
        harness.click_cell(0);
        harness.type_text("ab");
        let caret = harness.frame(Vec::new()).selection;
        assert_eq!(caret, Some(8..8));

        // Right click in the other cell: the menu shows, the caret stays where it was.
        let pos = harness.frame(Vec::new()).cells[1].center();
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        harness.frame(vec![Event::PointerMoved(pos)]);
        harness.frame(vec![button(true)]);
        harness.frame(vec![button(false)]);
        assert_eq!(harness.menu_frames, 1);
        harness.frame(Vec::new());
        assert_eq!(harness.menu_frames, 2, "it stays open");
        harness.type_text("c");
        assert_eq!(harness.doc.text(), "intro\nabc\n\n");

        // A click elsewhere closes it.
        let before = harness.menu_frames;
        harness.click_cell(0);
        harness.frame(Vec::new());
        harness.frame(Vec::new());
        assert!(harness.menu_frames <= before + 4, "closed by the click");
        let closed = harness.menu_frames;
        harness.frame(Vec::new());
        assert_eq!(harness.menu_frames, closed);
    }

    #[test]
    fn typing_goes_into_the_cell_that_was_clicked_every_time() {
        let mut doc = Document::default();
        doc.replace(0..0, "intro", Format::default());
        doc.insert_table(0, 2, 2).expect("a table");
        let mut harness = Harness::new(doc);
        harness.click_cell(0);
        harness.type_text("a");
        assert_eq!(harness.doc.text(), "intro\na\n\n\n\n");
        harness.click_cell(1);
        harness.type_text("b");
        assert_eq!(harness.doc.text(), "intro\na\nb\n\n\n");
        harness.click_cell(3);
        harness.type_text("d");
        harness.click_cell(0);
        harness.type_text("x");
        harness.click_cell(2);
        harness.type_text("c");
        assert_eq!(harness.doc.text(), "intro\nax\nb\nc\nd\n");
    }

    #[test]
    fn a_dash_and_a_space_start_a_list_that_goes_on() {
        let mut harness = Harness::new(Document::default());
        harness.focus(0);
        harness.type_text("-");
        harness.type_text(" ");
        assert_eq!(
            (harness.doc.text(), harness.lists()),
            ("", vec![List::Dash])
        );
        harness.type_text("one");
        harness.press(egui::Key::Enter);
        harness.type_text("two");
        assert_eq!(harness.doc.text(), "one\ntwo");
        assert_eq!(harness.lists(), [List::Dash, List::Dash]);

        // Enter on an empty item ends the list there.
        harness.focus(3);
        harness.press(egui::Key::Enter);
        assert_eq!(harness.lists(), [List::Dash, List::Dash, List::Dash]);
        harness.press(egui::Key::Enter);
        assert_eq!(harness.doc.text(), "one\n\ntwo");
        assert_eq!(harness.lists(), [List::Dash, List::None, List::Dash]);

        // Backspace at the start of an item takes it out of the list and keeps its text.
        harness.focus(5);
        harness.press(egui::Key::Backspace);
        assert_eq!(harness.doc.text(), "one\n\ntwo");
        assert_eq!(harness.lists(), [List::Dash, List::None, List::None]);
        harness.press(egui::Key::Backspace);
        assert_eq!(harness.doc.text(), "one\ntwo");
    }

    #[test]
    fn enter_after_a_numbered_heading_starts_plain_text() {
        let mut doc = Document::default();
        doc.replace(0..0, "Part", Format::default());
        doc.set_kind(0..1, Kind::Heading1);
        doc.set_list(0..1, List::Number);
        let mut harness = Harness::new(doc);
        harness.focus(4);
        harness.press(egui::Key::Enter);
        harness.type_text("text");
        assert_eq!(harness.doc.text(), "Part\ntext");
        let second = harness.doc.paras()[1];
        assert_eq!((second.kind, second.list), (Kind::Normal, List::None));
        assert_eq!(harness.doc.markers(), [Some("1.".to_owned()), None]);
    }

    #[test]
    fn an_underline_runs_unbroken_under_bold_and_larger_text() {
        let mut doc = Document::default();
        doc.replace(0..0, "plain under bold big end", Format::default());
        doc.restyle(6..20, |format| format.underline = true);
        doc.restyle(12..16, |format| format.bold = true);
        doc.restyle(17..20, |format| format.size = 24.0);
        let harness = Harness::new(doc);
        let mut lines = Vec::new();
        let _ = harness.ctx.run_ui(RawInput::default(), |ui| {
            let pages = layout::layout(ui.painter(), &harness.doc, 1.0);
            let formats = harness.doc.formats();
            lines = layout::underlines(&pages.galley, formats, 1.0, Rect::EVERYTHING);
        });
        assert_eq!(lines.len(), 1, "one line under the whole stretch");
        assert!(lines[0].to.x - lines[0].from.x > 60.0);
        // Another colour draws its own stretch, at the same height.
        let mut doc = harness.doc.clone();
        doc.restyle(12..16, |format| format.color = [0xD9, 0x2D, 0x20]);
        let _ = harness.ctx.run_ui(RawInput::default(), |ui| {
            let pages = layout::layout(ui.painter(), &doc, 1.0);
            lines = layout::underlines(&pages.galley, doc.formats(), 1.0, Rect::EVERYTHING);
        });
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|line| line.from.y == lines[0].from.y));
        assert_eq!(lines[0].to.x, lines[1].from.x);
    }

    #[test]
    fn a_right_click_leaves_what_is_selected_selected() {
        let mut doc = Document::default();
        doc.replace(0..0, "some words to mark", Format::default());
        let mut harness = Harness::new(doc);
        let id = Id::new("editor");
        harness.ctx.memory_mut(|memory| memory.request_focus(id));
        set_selection(&harness.ctx, id, Some(5), 10);
        assert_eq!(harness.frame(Vec::new()).selection, Some(5..10));
        // On the first line of the page, right of what is selected.
        let pos = pos2(520.0, 108.0);
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        harness.frame(vec![Event::PointerMoved(pos)]);
        harness.frame(vec![button(true)]);
        harness.frame(vec![button(false)]);
        assert_eq!(harness.menu_frames, 1, "the menu opened");
        assert_eq!(harness.frame(Vec::new()).selection, Some(5..10));
        // A click of the main button, away from the menu, puts the caret as ever.
        let pos = pos2(470.0, 108.0);
        harness.frame(vec![Event::PointerMoved(pos)]);
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        harness.frame(vec![button(true)]);
        harness.frame(vec![button(false)]);
        let caret = harness
            .frame(Vec::new())
            .selection
            .expect("the page has the keyboard");
        assert!(caret.is_empty() && caret.start > 10);
    }
}
