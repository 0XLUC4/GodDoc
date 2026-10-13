//! The pages of a document as pictures, for printing and for PDF: laid out and drawn by the
//! very code that shows them on screen, but off screen, then painted here dot by dot.

use std::collections::HashMap;

use egui::epaint::{ClippedPrimitive, Primitive};
use egui::{
    ColorImage, Context, FontDefinitions, Id, ImageData, Pos2, RawInput, Rect, TextureId, Vec2,
    pos2, vec2,
};
use goddoc_core::{Document, Format};

use crate::editor::{self, Hooks, View};
use crate::layout;

/// A page as a picture: `width * height * 3` bytes, the red, green and blue of each dot.
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// A document drawn off screen: what its pages are made of, ready to paint one at a time.
pub struct Rendered {
    shapes: Vec<ClippedPrimitive>,
    textures: HashMap<TextureId, ColorImage>,
    pub pages: usize,
    /// Size of a sheet, in points.
    pub sheet: Vec2,
    /// Distance between the tops of two pages, in points.
    step: f32,
    /// Dots per point.
    scale: f32,
    /// Every letter of the text and where it stands, in the order it is read: a PDF writes
    /// them under the picture of the page, so that its text can be selected and searched.
    pub letters: Vec<Letter>,
}

/// A letter of the text on its page. Lengths are in points, from the top-left of the sheet.
pub struct Letter {
    pub page: usize,
    /// Where it starts and where it ends.
    pub x: f32,
    pub end: f32,
    /// The line it stands on.
    pub baseline: f32,
    /// Size of its font.
    pub size: f32,
    /// The letter; a line break at the end of each row.
    pub letter: char,
}

/// The letters of `doc` and where its layout puts them.
fn letters(painter: &egui::Painter, doc: &Document) -> Vec<Letter> {
    let pages = layout::layout(painter, doc, 1.0);
    let formats = doc.formats();
    let step = layout::page_step(doc.page, 1.0);
    let [top, _, _, left] = layout::margins(doc.page, 1.0);
    let mut letters = Vec::new();
    let mut start = 0;
    for row in &pages.galley.rows {
        let first = start;
        start += row.char_count_including_newline().0;
        // Rows of later pages sit one step lower per page.
        let page = (row.pos.y / step).floor().max(0.0) as usize;
        let down = top + row.pos.y - page as f32 * step;
        let mut last = (left + row.pos.x, down, 11.0);
        for (index, glyph) in row.glyphs.iter().enumerate() {
            let size = formats
                .get(first + index)
                .map_or(11.0, |format| format.size);
            last = (left + row.pos.x + glyph.max_x(), down + glyph.pos.y, size);
            letters.push(Letter {
                page,
                x: left + row.pos.x + glyph.pos.x,
                end: last.0,
                baseline: last.1,
                size,
                letter: glyph.chr,
            });
        }
        letters.push(Letter {
            page,
            x: last.0,
            end: last.0,
            baseline: last.1,
            size: last.2,
            letter: '\n',
        });
    }
    letters
}

/// Draw `doc` off screen, to be painted at `dpi` dots per inch. `fonts` are those of the app.
pub fn render(doc: &Document, fonts: FontDefinitions, dpi: f32) -> Rendered {
    let ctx = Context::default();
    ctx.set_fonts(fonts);
    ctx.set_visuals(egui::Visuals::light());
    let scale = dpi / 72.0;
    ctx.set_pixels_per_point(scale);
    let mut doc = doc.clone();
    let mut view = View::plain();
    let sheet = layout::page_size(doc.page, 1.0);
    let step = layout::page_step(doc.page, 1.0);
    let mut pictures = HashMap::new();
    let mut textures: HashMap<TextureId, ColorImage> = HashMap::new();
    let mut pages = 1;
    let mut shapes = Vec::new();
    let mut text = Vec::new();
    // The first pass counts the pages; the next ones draw them all, and let the text settle.
    for _ in 0..3 {
        let height = (pages - 1) as f32 * step + sheet.y + 2.0 * editor::PAD;
        let size = vec2(sheet.x + 2.0 * editor::PAD, height);
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            max_texture_side: Some(8192),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| {
            let hooks = Hooks {
                textures: &mut |ctx, image| crate::app::texture_of(&mut pictures, ctx, image),
                menu: &mut |_| {},
            };
            let id = Id::new("pages");
            let shown = editor::show(ui, id, &mut doc, Format::default(), 1.0, &mut view, hooks);
            pages = shown.pages;
            text = letters(ui.painter(), &doc);
        });
        for (id, delta) in output.textures_delta.set {
            let ImageData::Color(image) = delta.image;
            match (delta.pos, textures.get_mut(&id)) {
                // A part of a texture that is there already: the font atlas, as it fills up.
                (Some([left, top]), Some(whole)) => {
                    for row in 0..image.size[1] {
                        let from = row * image.size[0];
                        let to = (top + row) * whole.size[0] + left;
                        let part = &image.pixels[from..from + image.size[0]];
                        whole.pixels[to..to + image.size[0]].copy_from_slice(part);
                    }
                }
                _ => {
                    textures.insert(id, (*image).clone());
                }
            }
        }
        shapes = ctx.tessellate(output.shapes, scale);
    }
    Rendered {
        shapes,
        textures,
        pages,
        sheet,
        step,
        scale,
        letters: text,
    }
}

/// Twice the area of the triangle a, b, c; its sign says which way it turns.
fn edge(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// The colour of `image` at `uv` (each from 0 to 1), smoothed between its four nearest dots.
/// Red, green, blue and opacity from 0 to 1, the colours already weighed by the opacity.
fn sample(image: &ColorImage, uv: Pos2) -> [f32; 4] {
    let [width, height] = image.size;
    let (x, y) = (uv.x * width as f32 - 0.5, uv.y * height as f32 - 0.5);
    let (left, top) = (x.floor(), y.floor());
    let (across, down) = (x - left, y - top);
    let at = |column: f32, row: f32| {
        let column = (column.max(0.0) as usize).min(width - 1);
        let row = (row.max(0.0) as usize).min(height - 1);
        image.pixels[row * width + column].to_array().map(f32::from)
    };
    let corners = [
        (at(left, top), (1.0 - across) * (1.0 - down)),
        (at(left + 1.0, top), across * (1.0 - down)),
        (at(left, top + 1.0), (1.0 - across) * down),
        (at(left + 1.0, top + 1.0), across * down),
    ];
    let mut color = [0.0; 4];
    for (corner, weight) in corners {
        for (sum, part) in color.iter_mut().zip(corner) {
            *sum += part * weight / 255.0;
        }
    }
    color
}

impl Rendered {
    /// Paint page `index` (the first is 0).
    pub fn page(&self, index: usize) -> Bitmap {
        let scale = self.scale;
        let origin = pos2(editor::PAD, editor::PAD + index as f32 * self.step);
        let width = (self.sheet.x * scale).round() as usize;
        let height = (self.sheet.y * scale).round() as usize;
        let mut rgb = vec![255_u8; width * height * 3];
        let sheet = Rect::from_min_size(origin, self.sheet);
        // A point of the drawing, in dots of this page.
        let dots = |point: Pos2| ((point.x - origin.x) * scale, (point.y - origin.y) * scale);
        let bound = |dot: f32, most: usize| (dot.max(0.0) as usize).min(most);
        for shape in &self.shapes {
            let Primitive::Mesh(mesh) = &shape.primitive else {
                continue;
            };
            let clip = shape.clip_rect.intersect(sheet);
            let Some(texture) = self.textures.get(&mesh.texture_id) else {
                continue;
            };
            if !clip.is_positive() || texture.size.contains(&0) {
                continue;
            }
            let (clip_min, clip_max) = (dots(clip.min), dots(clip.max));
            let clip_x = bound(clip_min.0, width)..bound(clip_max.0.ceil(), width);
            let clip_y = bound(clip_min.1, height)..bound(clip_max.1.ceil(), height);
            for triangle in mesh.indices.chunks_exact(3) {
                let corners = [0, 1, 2].map(|corner| &mesh.vertices[triangle[corner] as usize]);
                let at = corners.map(|corner| dots(corner.pos));
                let across = at.map(|dot| dot.0);
                let down = at.map(|dot| dot.1);
                let least = |of: [f32; 3]| of.into_iter().fold(f32::MAX, f32::min);
                let most = |of: [f32; 3]| of.into_iter().fold(f32::MIN, f32::max).ceil();
                let left = bound(least(across), width).max(clip_x.start);
                let right = bound(most(across), width).min(clip_x.end);
                let top = bound(least(down), height).max(clip_y.start);
                let bottom = bound(most(down), height).min(clip_y.end);
                let area = edge(at[0], at[1], at[2]);
                if left >= right || top >= bottom || area.abs() < 1e-6 {
                    continue;
                }
                let colors = corners.map(|corner| corner.color.to_array().map(f32::from));
                for row in top..bottom {
                    for column in left..right {
                        let dot = (column as f32 + 0.5, row as f32 + 0.5);
                        // How much of each corner the dot is made of.
                        let first = edge(at[1], at[2], dot) / area;
                        let second = edge(at[2], at[0], dot) / area;
                        let third = 1.0 - first - second;
                        if first < 0.0 || second < 0.0 || third < 0.0 {
                            continue;
                        }
                        let weights = [first, second, third];
                        let mut uv = Pos2::ZERO;
                        let mut color = [0.0_f32; 4];
                        for ((corner, shade), weight) in corners.iter().zip(&colors).zip(weights) {
                            uv += corner.uv.to_vec2() * weight;
                            for (sum, part) in color.iter_mut().zip(shade) {
                                *sum += part * weight / 255.0;
                            }
                        }
                        let texel = sample(texture, uv);
                        // The colours are already weighed by their opacity: they add to what
                        // shows through.
                        let through = 1.0 - color[3] * texel[3];
                        let dot = &mut rgb[(row * width + column) * 3..][..3];
                        for (channel, under) in dot.iter_mut().enumerate() {
                            let over = color[channel] * texel[channel] * 255.0;
                            *under = (over + f32::from(*under) * through).clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
        }
        Bitmap {
            width: width as u32,
            height: height as u32,
            rgb,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_painted_with_its_text() {
        let mut fonts = FontDefinitions::default();
        layout::install_fonts(&mut fonts);
        let mut doc = Document::default();
        doc.replace(0..0, "Hello page", Format::default());
        let rendered = render(&doc, fonts.clone(), 72.0);
        assert_eq!(rendered.pages, 1);
        let page = rendered.page(0);
        assert_eq!((page.width, page.height), (595, 842));
        // Paper to its corners, and ink where the text is.
        assert_eq!(page.rgb[..3], [255, 255, 255]);
        assert_eq!(page.rgb[page.rgb.len() - 3..], [255, 255, 255]);
        let ink = page.rgb.chunks(3).filter(|dot| dot[0] < 128).count();
        assert!(ink > 50, "{ink}");

        // With GODDOC_TEST_DOC and GODDOC_TEST_OUT set, a file is drawn for the eye to check.
        let (Ok(from), Ok(to)) = (
            std::env::var("GODDOC_TEST_DOC"),
            std::env::var("GODDOC_TEST_OUT"),
        ) else {
            return;
        };
        let doc = goddoc_core::docx::read(&std::fs::read(from).unwrap()).unwrap();
        let rendered = render(&doc, fonts, 110.0);
        let page = rendered.page(0);
        let (width, height) = (page.width, page.height);
        image::save_buffer(
            format!("{to}.png"),
            &page.rgb,
            width,
            height,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
        crate::pdf::write(&rendered, std::path::Path::new(&format!("{to}.pdf"))).unwrap();
        // With GODPDF_PRINT_TO set as well, it is printed to that file, without the dialog.
        #[cfg(windows)]
        if std::env::var("GODPDF_PRINT_TO").is_ok() {
            let printed = crate::print::print("test", rendered.pages, |page| rendered.page(page));
            assert_eq!(printed, Ok(Some("Sent to the printer")));
        }
    }
}
