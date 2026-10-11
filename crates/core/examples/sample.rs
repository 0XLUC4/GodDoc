//! Writes a formatted .docx showing every feature:
//! `cargo run -p goddoc-core --example sample -- out.docx [picture.png]`.

use goddoc_core::{Align, Document, Font, Format, Image, ImageKind, Kind, List, docx};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: sample <out.docx>")?;
    let lines = [
        "GodDoc",
        "A pure-Rust word processor",
        "Text can be bold, italic, underlined or struck through, in several fonts and colors.",
        "Centered paragraph in a serif font.",
        "Right-aligned monospace.",
        "Bullet item",
        "Another bullet whose text is long enough to wrap onto a second line inside the indent of the list.",
        "First step",
        "Second step",
        "Paragraph after the table.",
    ];
    let mut doc = Document::default();
    doc.replace(0..0, &lines.join("\n"), Format::default());
    doc.set_kind(0..1, Kind::Title);
    doc.set_kind(1..2, Kind::Heading2);
    doc.set_align(0..1, Align::Center);
    doc.set_align(3..4, Align::Center);
    doc.set_align(4..5, Align::Right);
    doc.set_list(5..7, List::Bullet);
    doc.set_list(7..9, List::Number);

    let body = doc.para_chars(2);
    let word = |text: &str| {
        let at = body.start + lines[2].find(text).unwrap_or(0);
        at..at + text.chars().count()
    };
    doc.restyle(word("bold"), |format| format.bold = true);
    doc.restyle(word("italic"), |format| format.italic = true);
    doc.restyle(word("underlined"), |format| format.underline = true);
    doc.restyle(word("struck through"), |format| format.strike = true);
    doc.restyle(word("colors"), |format| format.color = [0xD9, 0x2D, 0x20]);
    doc.restyle(doc.para_chars(3), |format| format.font = Font::Serif);
    doc.restyle(doc.para_chars(4), |format| format.font = Font::Mono);
    doc.restyle(word("several fonts"), |format| {
        format.highlight = Some([0xFF, 0xF1, 0x76]);
        format.font = Font::Georgia;
    });
    doc.restyle(word("or"), |format| {
        format.script = goddoc_core::Script::Super
    });
    // Indented, widely spaced, with a line under it.
    doc.edit_paras(6..7, |para| {
        para.indent = 2;
        para.spacing = goddoc_core::Spacing::OneAndHalf;
        para.rule = true;
    });

    // A 3 × 3 table after the numbered list, filled cell by cell.
    let mut caret = doc
        .insert_table(doc.para_chars(8).start, 3, 3)
        .ok_or("no table")?;
    let cells = [
        "Name",
        "Role",
        "Notes",
        "Ada",
        "Engineer",
        "Wrote the first program, long before computers.",
        "Linus",
        "Maintainer",
        "",
    ];
    for (index, text) in cells.into_iter().enumerate() {
        let format = Format {
            bold: index < 3,
            ..Format::default()
        };
        doc.replace(caret..caret, text, format);
        caret += text.chars().count() + 1;
    }

    // Enough text to run onto the following pages, then a forced page break.
    let filler = "The quick brown fox jumps over the lazy dog. ".repeat(6);
    for number in 1..=14 {
        let end = doc.len();
        doc.replace(
            end..end,
            &format!("\n{number}. {filler}"),
            Format::default(),
        );
    }
    let end = doc.len();
    let caret = doc.insert_page_break(end).ok_or("no page break")?;
    doc.replace(
        caret..caret,
        "This paragraph starts a new page.",
        Format::default(),
    );

    // A centred picture under the introduction, and numbered pages.
    if let Some(picture) = std::env::args().nth(2) {
        let image = Image {
            bytes: std::fs::read(picture)?.into(),
            kind: ImageKind::Png,
        };
        let caret = doc.insert_image(doc.para_chars(2).start, image, 140, 140);
        let para = doc.para_at(caret);
        doc.set_align(para..para + 1, Align::Center);
    }
    doc.page_numbers = true;

    std::fs::write(&path, docx::write(&doc)?)?;
    println!("wrote {path}");
    Ok(())
}
