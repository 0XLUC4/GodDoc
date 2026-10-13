//! A document as a PDF file: each page is its picture, as large as the sheet, with its text
//! written under it, unseen, so that it can be selected, copied and searched.

use std::io::{BufWriter, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::render::{Letter, Rendered};

/// Widths of the letters of Helvetica from the space to the tilde, in thousandths of the
/// font size: the unseen text is written in it, stretched to where the page shows each word.
const WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// A file being written, which knows how far it is: a PDF ends on where its parts start.
struct Counted<W: Write> {
    file: W,
    written: usize,
    /// Where each object starts; they are numbered from 1.
    starts: Vec<usize>,
}

impl<W: Write> Counted<W> {
    fn put(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.written += bytes.len();
        self.file.write_all(bytes)
    }

    /// Write the next object, whose number is its place in `starts`.
    fn object(&mut self, body: &[u8]) -> std::io::Result<()> {
        self.starts.push(self.written);
        self.put(format!("{} 0 obj\n", self.starts.len()).as_bytes())?;
        self.put(body)?;
        self.put(b"\nendobj\n")
    }

    /// Write the next object: `bytes`, packed.
    fn stream(&mut self, about: &str, bytes: &[u8]) -> std::io::Result<()> {
        let mut packer = ZlibEncoder::new(Vec::new(), Compression::fast());
        packer.write_all(bytes)?;
        let packed = packer.finish()?;
        let head = format!(
            "<< {about} /Filter /FlateDecode /Length {} >>\nstream\n",
            packed.len()
        );
        let mut body = head.into_bytes();
        body.extend(packed);
        body.extend(b"\nendstream");
        self.object(&body)
    }
}

/// The code of `letter` in the Western set of letters PDF fonts are written in.
fn code(letter: char) -> u8 {
    match letter {
        '\u{20ac}' => 0x80,
        '\u{2026}' => 0x85,
        '\u{152}' => 0x8C,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201c}' => 0x93,
        '\u{201d}' => 0x94,
        '\u{2013}' => 0x96,
        '\u{2014}' => 0x97,
        '\u{153}' => 0x9C,
        letter if (letter as u32) < 0x80 || (0xA0..=0xFF).contains(&(letter as u32)) => {
            letter as u8
        }
        _ => b'?',
    }
}

/// The text of a page, unseen: one word after the other, each where the page shows it and
/// stretched to its width there.
fn text_layer(letters: &[&Letter], height: f32) -> String {
    let mut layer = String::from("BT 3 Tr\n");
    let mut at = 0;
    while at < letters.len() {
        let first = letters[at];
        let visible = !first.letter.is_whitespace();
        // A word: letters that follow each other on a line, at one size.
        let same = |letter: &&&Letter| {
            letter.letter.is_whitespace() != visible
                && letter.baseline == first.baseline
                && letter.size == first.size
        };
        let end = at + letters[at..].iter().take_while(same).count().max(1);
        let word = &letters[at..end];
        at = end;
        if !visible {
            // What stands between words: a space, for who copies the text.
            layer.push_str("( ) Tj\n");
            continue;
        }
        let codes: Vec<u8> = word.iter().map(|letter| code(letter.letter)).collect();
        let natural: f32 = codes
            .iter()
            .map(|code| {
                f32::from(
                    code.checked_sub(32)
                        .and_then(|at| WIDTHS.get(usize::from(at)))
                        .copied()
                        .unwrap_or(556),
                )
            })
            .sum::<f32>()
            / 1000.0
            * first.size;
        let shown = word[word.len() - 1].end - first.x;
        let stretch = if natural > 0.0 {
            (shown / natural * 100.0).clamp(10.0, 1000.0)
        } else {
            100.0
        };
        let mut written = String::new();
        for code in codes {
            match code {
                b'(' | b')' | b'\\' => {
                    written.push('\\');
                    written.push(char::from(code));
                }
                0x20..=0x7E => written.push(char::from(code)),
                other => written.push_str(&format!("\\{other:03o}")),
            }
        }
        layer.push_str(&format!(
            "/F1 {} Tf {stretch:.1} Tz 1 0 0 1 {:.2} {:.2} Tm ({written}) Tj\n",
            first.size,
            first.x,
            height - first.baseline
        ));
    }
    layer + "ET"
}

/// Write the pages of `rendered` to `path`, one after the other: only one is held at a time.
pub fn write(rendered: &Rendered, path: &Path) -> Result<(), String> {
    let failed = |err: std::io::Error| format!("Cannot write {}: {err}", path.display());
    // Written beside the file, then put in its place: a failure leaves no half file.
    let temp = path.with_extension("pdf.goddoc.tmp");
    let file = std::fs::File::create(&temp).map_err(failed)?;
    let written = pages(rendered, BufWriter::new(file)).and_then(|()| std::fs::rename(&temp, path));
    written.map_err(|err| {
        let _ = std::fs::remove_file(&temp);
        failed(err)
    })
}

fn pages(rendered: &Rendered, file: impl Write) -> std::io::Result<()> {
    let mut out = Counted {
        file,
        written: 0,
        starts: Vec::new(),
    };
    let count = rendered.pages;
    // Object 1 is the catalog, 2 the list of pages, 3 the font of the unseen text; then, for
    // each page: the page, what it draws and its picture.
    let page_object = |page: usize| 4 + 3 * page;
    out.put(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")?;
    out.object(b"<< /Type /Catalog /Pages 2 0 R >>")?;
    let kids: Vec<String> = (0..count)
        .map(|page| format!("{} 0 R", page_object(page)))
        .collect();
    let list = format!(
        "<< /Type /Pages /Kids [{}] /Count {count} >>",
        kids.join(" ")
    );
    out.object(list.as_bytes())?;
    out.object(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    )?;
    let (width, height) = (rendered.sheet.x, rendered.sheet.y);
    for page in 0..count {
        let (contents, picture) = (page_object(page) + 1, page_object(page) + 2);
        let sheet = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Contents {contents} 0 R \
             /Resources << /XObject << /Page {picture} 0 R >> /Font << /F1 3 0 R >> >> >>"
        );
        out.object(sheet.as_bytes())?;
        // The picture, stretched over the sheet, then the text under it.
        let letters: Vec<&Letter> = rendered
            .letters
            .iter()
            .filter(|letter| letter.page == page)
            .collect();
        let draw = format!(
            "q {width} 0 0 {height} 0 0 cm /Page Do Q\n{}",
            text_layer(&letters, height)
        );
        out.stream("", draw.as_bytes())?;
        let bitmap = rendered.page(page);
        let about = format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB \
             /BitsPerComponent 8",
            bitmap.width, bitmap.height
        );
        out.stream(&about, &bitmap.rgb)?;
    }
    let table = out.written;
    let size = out.starts.len() + 1;
    out.put(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes())?;
    for start in out.starts.clone() {
        out.put(format!("{start:010} 00000 n \n").as_bytes())?;
    }
    let end = format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{table}\n%%EOF\n");
    out.put(end.as_bytes())?;
    out.file.flush()
}
