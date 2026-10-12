//! Reading and writing Word documents (.docx): text, character formats, paragraph styles,
//! alignment, lists, page breaks and tables. Images and page layout are ignored; merged table
//! cells are read as the cells they are stored as.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{Cursor, Read as _, Write as _};
use std::rc::Rc;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::{
    Align, Cell, Document, Error, Font, Format, Image, ImageKind, Kind, List, MAX_INDENT, Margins,
    PageSetup, Para, Picture, Result, Script, Spacing,
};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

const XML_HEADER: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const WORD_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const RELS_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const DRAWING_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const GRAPHIC_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const PICTURE_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
/// Twentieths of a point (the unit of page and table sizes) per point.
const TWIPS: f32 = 20.0;
/// Left indent of one indent level, in twentieths of a point.
const INDENT_STEP: u32 = 720;

/// `w:pgSz` and `w:pgMar` of the section.
fn page_setup_xml(page: PageSetup) -> String {
    let (width, height) = page.size();
    let (width, height) = ((width * TWIPS) as u32, (height * TWIPS) as u32);
    let margin = (page.margins.points() * TWIPS) as u32;
    let orient = if page.landscape {
        r#" w:orient="landscape""#
    } else {
        ""
    };
    format!(
        r#"<w:pgSz w:w="{width}" w:h="{height}"{orient}/><w:pgMar w:top="{margin}" w:right="{margin}" w:bottom="{margin}" w:left="{margin}" w:header="708" w:footer="708" w:gutter="0"/>"#
    )
}

/// Width between the margins, in twentieths of a point.
fn content_width(page: PageSetup) -> u32 {
    ((page.size().0 - 2.0 * page.margins.points()) * TWIPS) as u32
}

/// Relationship of the footer that shows the page number.
const FOOTER_REL: &str = "rIdFooter";
/// English Metric Units (the unit of drawing sizes) per point.
const EMU_PER_POINT: f32 = 12700.0;

/// `word/_rels/document.xml.rels`; `media` are the image files, in the order of their ids.
fn document_rels(media: &[String]) -> String {
    let mut xml = format!(
        r#"{XML_HEADER}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RELS_NS}/styles" Target="styles.xml"/><Relationship Id="rId2" Type="{RELS_NS}/numbering" Target="numbering.xml"/><Relationship Id="{FOOTER_REL}" Type="{RELS_NS}/footer" Target="footer1.xml"/>"#
    );
    for (index, file) in media.iter().enumerate() {
        xml.push_str(&format!(
            r#"<Relationship Id="{}" Type="{RELS_NS}/image" Target="media/{file}"/>"#,
            image_rel(index)
        ));
    }
    xml.push_str("</Relationships>");
    xml
}

fn image_rel(index: usize) -> String {
    format!("rIdImage{}", index + 1)
}

/// Footer with the page number in the middle.
fn footer_xml() -> String {
    let field = |kind: &str| format!(r#"<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>"#);
    format!(
        r#"{XML_HEADER}<w:ftr xmlns:w="{WORD_NS}"><w:p><w:pPr><w:jc w:val="center"/></w:pPr>{}<w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r>{}<w:r><w:t>1</w:t></w:r>{}</w:p></w:ftr>"#,
        field("begin"),
        field("separate"),
        field("end")
    )
}

/// Inline drawing showing the image behind relationship `rel`.
fn drawing_xml(picture: Picture, rel: &str, number: usize) -> String {
    let width = (f32::from(picture.width) * EMU_PER_POINT) as u64;
    let height = (f32::from(picture.height) * EMU_PER_POINT) as u64;
    format!(
        r#"<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{width}" cy="{height}"/><wp:docPr id="{number}" name="Picture {number}"/><a:graphic xmlns:a="{GRAPHIC_NS}"><a:graphicData uri="{PICTURE_NS}"><pic:pic xmlns:pic="{PICTURE_NS}"><pic:nvPicPr><pic:cNvPr id="{number}" name="Picture {number}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rel}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{width}" cy="{height}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
    )
}
/// Numbering ids of bullet lists, dash lists and numbered headings; numbered lists take the
/// ids after them, one per list.
const BULLET_NUM_ID: u32 = 1;
const DASH_NUM_ID: u32 = 2;
const HEADING_NUM_ID: u32 = 3;
/// Levels a list is written with.
const LIST_LEVELS: usize = MAX_INDENT as usize + 1;

impl Kind {
    /// Style id in .docx files.
    fn style_id(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Title => "Title",
            Self::Heading1 => "Heading1",
            Self::Heading2 => "Heading2",
            Self::Heading3 => "Heading3",
        }
    }

    /// Kind of a style id, English or French Word ("Heading1", "Titre1", "Title", "Titre").
    fn of_style(id: &str) -> Self {
        let id = id.to_ascii_lowercase();
        let Some(level) = id
            .strip_prefix("heading")
            .or_else(|| id.strip_prefix("titre"))
            .or_else(|| id.strip_prefix("title"))
        else {
            return Self::Normal;
        };
        match level.trim().parse::<u8>() {
            Ok(1) => Self::Heading1,
            Ok(2) => Self::Heading2,
            Ok(_) => Self::Heading3,
            Err(_) if level.is_empty() => Self::Title,
            Err(_) => Self::Normal,
        }
    }
}

// ---------------------------------------------------------------- writing

/// The document as a .docx file.
pub fn write(doc: &Document) -> Result<Vec<u8>> {
    let failed = |err: &dyn std::fmt::Display| Error::Write(err.to_string());
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let (document, numbered_lists, used_images) = document_xml(doc);
    // Each image used becomes `word/media/imageN.ext`.
    let media: Vec<(String, &Image)> = used_images
        .iter()
        .filter_map(|id| doc.images().get(*id as usize))
        .enumerate()
        .map(|(index, image)| {
            let name = format!("image{}.{}", index + 1, image.kind.extension());
            (name, image)
        })
        .collect();
    let names: Vec<String> = media.iter().map(|(name, _)| name.clone()).collect();
    let mut parts: Vec<(String, Vec<u8>)> = [
        ("[Content_Types].xml", CONTENT_TYPES.to_owned()),
        ("_rels/.rels", ROOT_RELS.to_owned()),
        ("word/_rels/document.xml.rels", document_rels(&names)),
        ("word/styles.xml", styles_xml()),
        ("word/numbering.xml", numbering_xml(numbered_lists)),
        ("word/footer1.xml", footer_xml()),
        ("word/document.xml", document),
    ]
    .into_iter()
    .map(|(name, content)| (name.to_owned(), content.into_bytes()))
    .collect();
    for (name, image) in media {
        parts.push((format!("word/media/{name}"), image.bytes.to_vec()));
    }
    for (name, content) in parts {
        zip.start_file(name, options).map_err(|err| failed(&err))?;
        zip.write_all(&content).map_err(|err| failed(&err))?;
    }
    let cursor = zip.finish().map_err(|err| failed(&err))?;
    Ok(cursor.into_inner())
}

fn styles_xml() -> String {
    let mut xml = format!(
        r#"{XML_HEADER}<w:styles xmlns:w="{WORD_NS}"><w:docDefaults><w:rPrDefault><w:rPr>{}</w:rPr></w:rPrDefault></w:docDefaults>"#,
        run_props(&Format::default(), false)
    );
    for kind in Kind::ALL {
        let id = kind.style_id();
        let default = if kind == Kind::Normal {
            r#" w:default="1""#
        } else {
            ""
        };
        xml.push_str(&format!(
            r#"<w:style w:type="paragraph"{default} w:styleId="{id}"><w:name w:val="{}"/><w:rPr>{}</w:rPr></w:style>"#,
            kind.name().to_lowercase(),
            run_props(&kind.format(), false)
        ));
    }
    xml.push_str("</w:styles>");
    xml
}

/// The levels of a list definition: `level` gives the number format and the text of each.
fn list_levels(
    count: usize,
    heading: bool,
    level: impl Fn(usize) -> (&'static str, String),
) -> String {
    (0..count)
        .map(|index| {
            let (format, text) = level(index);
            // The number of a heading stands where its text starts, a space after it.
            let (suffix, left, hanging) = if heading {
                (r#"<w:suff w:val="space"/>"#, 0, 0)
            } else {
                ("", (index as u32 + 1) * INDENT_STEP, INDENT_STEP / 2)
            };
            format!(
                r#"<w:lvl w:ilvl="{index}"><w:start w:val="1"/><w:numFmt w:val="{format}"/>{suffix}<w:lvlText w:val="{text}"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="{left}" w:hanging="{hanging}"/></w:pPr></w:lvl>"#
            )
        })
        .collect()
}

/// Bullets, numbers, dashes and heading numbers, each with its levels, and a numbering
/// instance per numbered list so that each restarts at 1.
fn numbering_xml(numbered_lists: u32) -> String {
    let bullets = list_levels(LIST_LEVELS, false, |level| {
        ("bullet", ["•", "◦", "▪"][level % 3].to_owned())
    });
    let numbers = list_levels(LIST_LEVELS, false, |level| {
        let format = ["decimal", "lowerLetter", "lowerRoman"][level % 3];
        (format, format!("%{}.", level + 1))
    });
    let dashes = list_levels(LIST_LEVELS, false, |_| ("bullet", "–".to_owned()));
    let headings = list_levels(Kind::HEADINGS.len(), true, |level| {
        let text = (1..=level + 1).map(|number| format!("%{number}."));
        ("decimal", text.collect())
    });
    let mut xml = format!(r#"{XML_HEADER}<w:numbering xmlns:w="{WORD_NS}">"#);
    for (id, levels) in [bullets, numbers, dashes, headings].iter().enumerate() {
        xml.push_str(&format!(
            r#"<w:abstractNum w:abstractNumId="{id}"><w:multiLevelType w:val="multilevel"/>{levels}</w:abstractNum>"#
        ));
    }
    for (num, definition) in [(BULLET_NUM_ID, 0), (DASH_NUM_ID, 2), (HEADING_NUM_ID, 3)] {
        xml.push_str(&format!(
            r#"<w:num w:numId="{num}"><w:abstractNumId w:val="{definition}"/></w:num>"#
        ));
    }
    for list in 1..=numbered_lists {
        xml.push_str(&format!(
            r#"<w:num w:numId="{}"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="1"/></w:lvlOverride></w:num>"#,
            HEADING_NUM_ID + list
        ));
    }
    xml.push_str("</w:numbering>");
    xml
}

/// `word/document.xml` and the number of numbered lists it uses.
fn document_xml(doc: &Document) -> (String, u32, Vec<u32>) {
    let chars: Vec<char> = doc.text().chars().collect();
    let formats = doc.formats();
    let paras = doc.paras();
    let mut xml = format!(
        r#"{XML_HEADER}<w:document xmlns:w="{WORD_NS}" xmlns:r="{RELS_NS}" xmlns:wp="{DRAWING_NS}"><w:body>"#
    );
    // Ids of the images shown, in order of first use: the n-th is `image_rel(n)`.
    let mut used_images: Vec<u32> = Vec::new();
    let mut numbered_lists = 0;
    // The numbered list being written goes on: nothing but list items since it started.
    let mut list_open = false;
    let mut start = 0;
    let mut previous: Option<Para> = None;
    for para in paras {
        let end = chars[start..]
            .iter()
            .position(|ch| *ch == '\n')
            .map_or(chars.len(), |offset| start + offset);
        table_tags(
            &mut xml,
            previous.and_then(|para| para.cell),
            para.cell,
            doc,
        );
        let heading = para
            .kind
            .heading_level()
            .filter(|_| para.list == List::Number);
        let same_cell = previous.is_none_or(|before| before.cell == para.cell);
        list_open &= same_cell && para.list != List::None && heading.is_none();
        let level = u32::from(para.indent);
        // (numbering id, level) of the paragraph.
        let num = match (para.list, heading) {
            (List::None, _) => None,
            (_, Some(level)) => Some((HEADING_NUM_ID, level as u32)),
            (List::Bullet, _) => Some((BULLET_NUM_ID, level)),
            (List::Dash, _) => Some((DASH_NUM_ID, level)),
            (List::Number, _) => {
                if !list_open {
                    numbered_lists += 1;
                    list_open = true;
                }
                Some((HEADING_NUM_ID + numbered_lists, level))
            }
        };
        let drawing = para.picture.map(|picture| {
            let index = used_images
                .iter()
                .position(|id| *id == picture.image)
                .unwrap_or_else(|| {
                    used_images.push(picture.image);
                    used_images.len() - 1
                });
            drawing_xml(picture, &image_rel(index), index + 1)
        });
        paragraph_xml(
            &mut xml,
            *para,
            num,
            &chars[start..end],
            &formats[start..end],
            drawing,
        );
        start = (end + 1).min(chars.len());
        previous = Some(*para);
    }
    let last_cell = previous.and_then(|para| para.cell);
    table_tags(&mut xml, last_cell, None, doc);
    if last_cell.is_some() {
        // Word wants a paragraph after a table.
        xml.push_str("<w:p/>");
    }
    let footer = if doc.page_numbers {
        format!(r#"<w:footerReference w:type="default" r:id="{FOOTER_REL}"/>"#)
    } else {
        String::new()
    };
    xml.push_str(&format!(
        "<w:sectPr>{footer}{}</w:sectPr></w:body></w:document>",
        page_setup_xml(doc.page)
    ));
    (xml, numbered_lists, used_images)
}

/// Table markup between a paragraph in cell `from` and the next one in cell `to`.
fn table_tags(xml: &mut String, from: Option<Cell>, to: Option<Cell>, doc: &Document) {
    if from == to {
        return;
    }
    let same_table = matches!((from, to), (Some(a), Some(b)) if a.table == b.table);
    let same_row = same_table && from.map(|cell| cell.row) == to.map(|cell| cell.row);
    if from.is_some() {
        xml.push_str("</w:tc>");
        if !same_row {
            xml.push_str("</w:tr>");
        }
        if !same_table {
            xml.push_str("</w:tbl>");
        }
    }
    let Some(cell) = to else {
        return;
    };
    let columns = doc.paras()[doc.table_paras(cell.table)]
        .iter()
        .filter_map(|para| para.cell)
        .map(|cell| u32::from(cell.col) + 1)
        .max()
        .unwrap_or(1);
    let table_width = content_width(doc.page);
    let width = table_width / columns;
    if !same_table {
        let border = |side: &str| {
            format!(r#"<w:{side} w:val="single" w:sz="4" w:space="0" w:color="000000"/>"#)
        };
        let borders: String = ["top", "left", "bottom", "right", "insideH", "insideV"]
            .into_iter()
            .map(border)
            .collect();
        let grid = format!(r#"<w:gridCol w:w="{width}"/>"#).repeat(columns as usize);
        xml.push_str(&format!(
            r#"<w:tbl><w:tblPr><w:tblW w:w="{table_width}" w:type="dxa"/><w:tblBorders>{borders}</w:tblBorders></w:tblPr><w:tblGrid>{grid}</w:tblGrid>"#
        ));
    }
    if !same_row {
        xml.push_str("<w:tr>");
    }
    xml.push_str(&format!(
        r#"<w:tc><w:tcPr><w:tcW w:w="{width}" w:type="dxa"/></w:tcPr>"#
    ));
}

fn paragraph_xml(
    xml: &mut String,
    para: Para,
    num: Option<(u32, u32)>,
    chars: &[char],
    formats: &[Format],
    drawing: Option<String>,
) {
    if let Some(drawing) = drawing {
        // The picture gets a paragraph of its own; its caption, if any, follows.
        let props = para_props(para, num);
        xml.push_str(&format!("<w:p>{props}{drawing}</w:p>"));
        if chars.is_empty() {
            return;
        }
    }
    let para = Para {
        page_break: para.page_break && para.picture.is_none(),
        ..para
    };
    xml.push_str("<w:p>");
    xml.push_str(&para_props(para, num));
    let mut start = 0;
    for run in formats.chunk_by(|a, b| a == b) {
        let text = &chars[start..start + run.len()];
        start += run.len();
        // The style may be bold: say so when the run is not.
        let props = run_props(&run[0], para.kind.format().bold);
        xml.push_str(&format!("<w:r><w:rPr>{props}</w:rPr>"));
        for (index, piece) in text.split(|ch| *ch == '\t').enumerate() {
            if index > 0 {
                xml.push_str("<w:tab/>");
            }
            if !piece.is_empty() {
                xml.push_str(r#"<w:t xml:space="preserve">"#);
                piece.iter().for_each(|ch| push_escaped(xml, *ch));
                xml.push_str("</w:t>");
            }
        }
        xml.push_str("</w:r>");
    }
    xml.push_str("</w:p>");
}

/// `w:pPr` of a paragraph; `num` is its (numbering id, level) in a list.
fn para_props(para: Para, num: Option<(u32, u32)>) -> String {
    let mut xml = String::from("<w:pPr>");
    if para.kind != Kind::Normal {
        xml.push_str(&format!(r#"<w:pStyle w:val="{}"/>"#, para.kind.style_id()));
    }
    if para.page_break {
        xml.push_str("<w:pageBreakBefore/>");
    }
    if let Some((id, level)) = num {
        xml.push_str(&format!(
            r#"<w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="{id}"/></w:numPr>"#
        ));
    }
    if para.rule {
        xml.push_str(
            r#"<w:pBdr><w:bottom w:val="single" w:sz="6" w:space="1" w:color="000000"/></w:pBdr>"#,
        );
    }
    if para.spacing != Spacing::Single {
        // 240 is single spacing.
        let line = (240.0 * para.spacing.factor()) as u32;
        xml.push_str(&format!(
            r#"<w:spacing w:line="{line}" w:lineRule="auto"/>"#
        ));
    }
    // A list item is indented by its level in the list.
    if para.indent > 0 && num.is_none() {
        let left = u32::from(para.indent) * INDENT_STEP;
        xml.push_str(&format!(r#"<w:ind w:left="{left}"/>"#));
    }
    let align = match para.align {
        Align::Left => None,
        Align::Center => Some("center"),
        Align::Right => Some("right"),
        Align::Justify => Some("both"),
    };
    if let Some(align) = align {
        xml.push_str(&format!(r#"<w:jc w:val="{align}"/>"#));
    }
    xml.push_str("</w:pPr>");
    xml
}

/// Children of `w:rPr` for `format`, in schema order.
fn run_props(format: &Format, style_is_bold: bool) -> String {
    let family = format.font.family();
    let [r, g, b] = format.color;
    let half_points = (format.size * 2.0).round() as u32;
    let mut props = format!(r#"<w:rFonts w:ascii="{family}" w:hAnsi="{family}" w:cs="{family}"/>"#);
    if format.bold {
        props.push_str("<w:b/>");
    } else if style_is_bold {
        props.push_str(r#"<w:b w:val="0"/>"#);
    }
    if format.italic {
        props.push_str("<w:i/>");
    }
    if format.strike {
        props.push_str("<w:strike/>");
    }
    props.push_str(&format!(
        r#"<w:color w:val="{r:02X}{g:02X}{b:02X}"/><w:sz w:val="{half_points}"/><w:szCs w:val="{half_points}"/>"#
    ));
    if format.underline {
        props.push_str(r#"<w:u w:val="single"/>"#);
    }
    if let Some([r, g, b]) = format.highlight {
        props.push_str(&format!(
            r#"<w:shd w:val="clear" w:color="auto" w:fill="{r:02X}{g:02X}{b:02X}"/>"#
        ));
    }
    let script = match format.script {
        Script::Normal => None,
        Script::Super => Some("superscript"),
        Script::Sub => Some("subscript"),
    };
    if let Some(script) = script {
        props.push_str(&format!(r#"<w:vertAlign w:val="{script}"/>"#));
    }
    props
}

fn push_escaped(xml: &mut String, ch: char) {
    match ch {
        '&' => xml.push_str("&amp;"),
        '<' => xml.push_str("&lt;"),
        '>' => xml.push_str("&gt;"),
        // Control characters are not allowed in XML.
        ch if ch.is_control() => {}
        ch => xml.push(ch),
    }
}

// ---------------------------------------------------------------- reading

/// Parse a .docx file.
pub fn read(bytes: &[u8]) -> Result<Document> {
    let bad = |err: &dyn std::fmt::Display| Error::NotDocx(err.to_string());
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|err| bad(&err))?;
    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    let mut bytes_of = |name: &str| -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        archive
            .by_name(name)
            .map_err(|err| bad(&err))?
            .read_to_end(&mut bytes)
            .map_err(|err| bad(&err))?;
        Ok(bytes)
    };
    let mut part =
        |name: &str| bytes_of(name).map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let document = part("word/document.xml")?;
    // Documents without lists have no numbering part.
    let lists = part("word/numbering.xml")
        .map(|xml| numbering(&xml))
        .unwrap_or_default();
    // A file without styles: its text looks as its kind does here.
    let styles = part("word/styles.xml")
        .map(|xml| read_styles(&xml))
        .unwrap_or_default();
    let page_numbers = document.contains("<w:footerReference")
        && names
            .iter()
            .filter(|name| name.starts_with("word/footer"))
            .any(|name| part(name).is_ok_and(|xml| xml.contains("PAGE")));
    // Relationship id -> image file.
    let mut media = HashMap::new();
    let rels = part("word/_rels/document.xml.rels").unwrap_or_default();
    for (id, target) in relationships(&rels) {
        let kind = target
            .rsplit_once('.')
            .and_then(|(_, extension)| ImageKind::of_extension(extension));
        let path = format!("word/{}", target.trim_start_matches('/'));
        if let (Some(kind), Ok(bytes)) = (kind, bytes_of(&path)) {
            media.insert(
                id,
                Image {
                    bytes: bytes.into(),
                    kind,
                },
            );
        }
    }
    let mut doc = parse(&document, lists, styles, media)?;
    doc.page_numbers = page_numbers;
    Ok(doc)
}

/// (id, target) of every relationship in a `.rels` part.
fn relationships(xml: &str) -> Vec<(String, String)> {
    let mut reader = Reader::from_str(xml);
    let mut found = Vec::new();
    while let Ok(event) = reader.read_event() {
        let tag = match &event {
            Event::Start(tag) | Event::Empty(tag) => tag,
            Event::Eof => break,
            _ => continue,
        };
        if let (Some(id), Some(target)) = (attr(tag, b"Id"), attr(tag, b"Target")) {
            found.push((id.into_owned(), target.into_owned()));
        }
    }
    found
}

/// Kind of list of each level of the numbering ids in `word/numbering.xml`.
type Lists = HashMap<String, Vec<List>>;

fn numbering(xml: &str) -> Lists {
    let mut reader = Reader::from_str(xml);
    // Abstract definition id -> the list each of its levels makes.
    let mut levels: Lists = HashMap::new();
    let mut nums: Vec<(String, String)> = Vec::new();
    let mut definition = String::new();
    let mut num = String::new();
    // Inside a `w:num`, whose levels only adjust those of its definition.
    let mut in_num = false;
    while let Ok(event) = reader.read_event() {
        let tag = match &event {
            Event::Start(tag) | Event::Empty(tag) => tag,
            Event::Eof => break,
            _ => continue,
        };
        let value = |name: &[u8]| attr(tag, name).map(Cow::into_owned).unwrap_or_default();
        let level = levels
            .get_mut(&definition)
            .and_then(|levels| levels.last_mut())
            .filter(|_| !in_num);
        match (tag.name().as_ref(), level) {
            (b"w:abstractNum", _) => {
                definition = value(b"w:abstractNumId");
                in_num = false;
            }
            (b"w:num", _) => {
                num = value(b"w:numId");
                in_num = true;
            }
            (b"w:abstractNumId", _) => nums.push((num.clone(), value(b"w:val"))),
            (b"w:lvl", _) if !in_num => levels
                .entry(definition.clone())
                .or_default()
                .push(List::Number),
            (b"w:numFmt", Some(level)) => {
                *level = match value(b"w:val").as_str() {
                    "bullet" => List::Bullet,
                    "none" => List::None,
                    _ => List::Number,
                };
            }
            // A bullet drawn as a dash is a dash list.
            (b"w:lvlText", Some(level)) if *level == List::Bullet => {
                if matches!(value(b"w:val").trim(), "-" | "–" | "—" | "‒" | "−") {
                    *level = List::Dash;
                }
            }
            _ => {}
        }
    }
    nums.into_iter()
        .map(|(num, definition)| {
            let levels = levels.get(&definition).cloned().unwrap_or_default();
            (num, levels)
        })
        .collect()
}

/// A style of `word/styles.xml`, as far as it is followed.
#[derive(Default)]
struct Style {
    name: String,
    based_on: Option<String>,
    /// What its `w:rPr` holds.
    run: Vec<BytesStart<'static>>,
    /// What it says of its paragraphs: numbering, alignment, indent, line spacing.
    para: Vec<BytesStart<'static>>,
}

#[derive(Default)]
struct Styles {
    by_id: HashMap<String, Style>,
    /// What the `w:rPr` of the document defaults holds.
    defaults: Vec<BytesStart<'static>>,
    /// Style of the paragraphs that name none.
    default_para: Option<String>,
}

impl Styles {
    /// The style `id` and those it is based on, the most basic first.
    fn chain(&self, id: &str) -> Vec<&Style> {
        let mut chain = Vec::new();
        let mut next = self.by_id.get(id);
        // Styles based on each other in a circle would never end.
        while let Some(style) = next.filter(|_| chain.len() < 16) {
            chain.push(style);
            next = style.based_on.as_deref().and_then(|id| self.by_id.get(id));
        }
        chain.reverse();
        chain
    }

    /// Paragraph kind of the style `id`: its own, or that of a style it is based on.
    fn kind(&self, id: &str) -> Kind {
        let chain = self.chain(id);
        let named = chain.iter().rev().map(|style| Kind::of_style(&style.name));
        std::iter::once(Kind::of_style(id))
            .chain(named)
            .find(|kind| *kind != Kind::Normal)
            .unwrap_or_default()
    }

    /// Look of the text of a paragraph of style `id` (`None`: the default style) and `kind`.
    fn format(&self, id: Option<&str>, kind: Kind) -> Format {
        let chain = id
            .or(self.default_para.as_deref())
            .map(|id| self.chain(id))
            .unwrap_or_default();
        // A heading style the file does not describe looks as headings do here.
        if chain.is_empty() && kind != Kind::Normal {
            return kind.format();
        }
        let mut format = Kind::Normal.format();
        let own = chain.iter().flat_map(|style| &style.run);
        self.defaults
            .iter()
            .chain(own)
            .for_each(|tag| run_prop(&mut format, tag));
        format
    }

    /// Give `format` the look of the character style `id`.
    fn restyle(&self, id: &str, format: &mut Format) {
        let chain = self.chain(id);
        let tags = chain.iter().flat_map(|style| &style.run);
        tags.for_each(|tag| run_prop(format, tag));
    }
}

/// The styles of `word/styles.xml`.
fn read_styles(xml: &str) -> Styles {
    let mut reader = Reader::from_str(xml);
    let mut styles = Styles::default();
    // Id of the style being read.
    let mut current: Option<String> = None;
    let mut in_defaults = false;
    let mut in_run = false;
    let mut in_para = false;
    loop {
        let event = match reader.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(event) => event,
        };
        let (tag, empty) = match &event {
            Event::Start(tag) => (tag, false),
            Event::Empty(tag) => (tag, true),
            Event::End(tag) => {
                match tag.name().as_ref() {
                    b"w:style" => current = None,
                    b"w:docDefaults" => in_defaults = false,
                    b"w:rPr" => in_run = false,
                    b"w:pPr" => in_para = false,
                    _ => {}
                }
                continue;
            }
            _ => continue,
        };
        let value = |name: &[u8]| attr(tag, name).map(Cow::into_owned).unwrap_or_default();
        let style = current.as_ref().and_then(|id| styles.by_id.get_mut(id));
        match (tag.name().as_ref(), style) {
            (b"w:docDefaults", _) => in_defaults = true,
            (b"w:style", _) => {
                let id = value(b"w:styleId");
                if value(b"w:type") == "paragraph" && is_set(tag, b"w:default") {
                    styles.default_para = Some(id.clone());
                }
                styles.by_id.insert(id.clone(), Style::default());
                current = (!empty).then_some(id);
            }
            (b"w:rPr", _) => in_run = !empty,
            (b"w:pPr", _) => in_para = !empty,
            (_, Some(style)) if in_run => style.run.push(tag.to_owned()),
            (_, None) if in_run && in_defaults => styles.defaults.push(tag.to_owned()),
            (b"w:name", Some(style)) => style.name = value(b"w:val"),
            (b"w:basedOn", Some(style)) => style.based_on = Some(value(b"w:val")),
            (b"w:ilvl" | b"w:numId" | b"w:jc" | b"w:ind" | b"w:spacing", Some(style))
                if in_para =>
            {
                style.para.push(tag.to_owned());
            }
            _ => {}
        }
    }
    styles
}

#[derive(Default)]
struct Parser {
    text: String,
    formats: Vec<Format>,
    paras: Vec<Para>,
    lists: Lists,
    styles: Rc<Styles>,
    /// Look of the text of the current paragraph, from its style.
    base: Format,
    /// Level in its list of the paragraph being read.
    level: usize,
    /// Format of the run being read.
    run: Format,
    in_run: bool,
    in_run_props: bool,
    in_text: bool,
    /// Nesting of `w:tbl`; only the outermost table makes cells.
    table_depth: u32,
    tables: u32,
    /// Row and column being read in the outermost table (-1 before the first).
    row: i32,
    col: i32,
    /// Image files by relationship id.
    media: HashMap<String, Image>,
    images: Vec<Image>,
    /// Relationship id -> index in `images`, for images shown more than once.
    image_ids: HashMap<String, u32>,
    /// Size in points of the drawing being read.
    extent: Option<(u16, u16)>,
    /// The current paragraph already has text.
    has_text: bool,
    /// Inside the borders of a paragraph (`w:pBdr`).
    in_borders: bool,
    page: PageSetup,
}

fn parse(
    xml: &str,
    lists: Lists,
    styles: Styles,
    media: HashMap<String, Image>,
) -> Result<Document> {
    let mut reader = Reader::from_str(xml);
    let mut parser = Parser {
        lists,
        styles: Rc::new(styles),
        media,
        ..Parser::default()
    };
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) => parser.open(&tag),
            Ok(Event::Empty(tag)) => {
                parser.open(&tag);
                parser.close(tag.name().as_ref());
            }
            Ok(Event::End(tag)) => parser.close(tag.name().as_ref()),
            Ok(Event::Text(text)) if parser.in_text => {
                let text = text
                    .unescape()
                    .map_err(|err| Error::NotDocx(err.to_string()))?;
                text.chars().for_each(|ch| parser.push(ch));
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(err) => return Err(Error::NotDocx(err.to_string())),
        }
    }
    if parser.paras.is_empty() {
        parser.paras.push(Para::default());
    }
    Ok(Document {
        text: parser.text,
        formats: parser.formats,
        paras: parser.paras,
        images: parser.images,
        page_numbers: false,
        page: parser.page,
    })
}

fn attr<'a>(tag: &'a BytesStart<'_>, name: &[u8]) -> Option<Cow<'a, str>> {
    tag.attributes()
        .flatten()
        .find(|attr| attr.key.as_ref() == name)
        .and_then(|attr| attr.unescape_value().ok())
}

/// Value of an on/off property such as `<w:b/>` or `<w:b w:val="0"/>`.
fn is_on(tag: &BytesStart<'_>) -> bool {
    !matches!(
        attr(tag, b"w:val").as_deref(),
        Some("0" | "false" | "off" | "none")
    )
}

/// Whether the on/off attribute `name` is there and on, as in `w:default="1"`.
fn is_set(tag: &BytesStart<'_>, name: &[u8]) -> bool {
    matches!(attr(tag, name).as_deref(), Some("1" | "true" | "on"))
}

fn font_of(family: &str) -> Font {
    let family = family.to_ascii_lowercase();
    let has = |names: &[&str]| names.iter().any(|name| family.contains(name));
    let named = [
        ("calibri", Font::Calibri),
        ("verdana", Font::Verdana),
        ("georgia", Font::Georgia),
        ("tahoma", Font::Tahoma),
        ("comic", Font::Comic),
    ];
    if let Some((_, font)) = named.iter().find(|(name, _)| family.contains(name)) {
        *font
    } else if has(&["courier", "consolas", "mono", "menlo"]) {
        Font::Mono
    } else if has(&["times", "cambria", "garamond", "serif"]) && !family.contains("sans") {
        Font::Serif
    } else {
        Font::Sans
    }
}

/// Colour of one of Word's named highlighters.
fn highlighter(name: &str) -> Option<[u8; 3]> {
    Some(match name {
        "yellow" => [0xFF, 0xFF, 0x00],
        "green" => [0x00, 0xFF, 0x00],
        "cyan" => [0x00, 0xFF, 0xFF],
        "magenta" => [0xFF, 0x00, 0xFF],
        "red" => [0xFF, 0x00, 0x00],
        "blue" => [0x00, 0x00, 0xFF],
        "lightGray" => [0xD3, 0xD3, 0xD3],
        "darkGray" => [0x80, 0x80, 0x80],
        _ => return None,
    })
}

/// Apply one child of a `w:rPr` to `run`.
fn run_prop(run: &mut Format, tag: &BytesStart<'_>) {
    match tag.name().as_ref() {
        b"w:b" => run.bold = is_on(tag),
        b"w:i" => run.italic = is_on(tag),
        b"w:strike" | b"w:dstrike" => run.strike = is_on(tag),
        b"w:u" => run.underline = is_on(tag),
        b"w:sz" => {
            if let Some(half_points) = attr(tag, b"w:val").and_then(|val| val.parse::<f32>().ok()) {
                run.size = (half_points / 2.0).clamp(1.0, 400.0);
            }
        }
        b"w:color" => {
            if let Some(color) = attr(tag, b"w:val").and_then(|val| color_of(&val)) {
                run.color = color;
            }
        }
        b"w:rFonts" => {
            if let Some(family) = attr(tag, b"w:ascii") {
                run.font = font_of(&family);
            } else if attr(tag, b"w:asciiTheme").is_some() {
                // The fonts of Word's themes: Calibri, unless the theme was changed.
                run.font = Font::Calibri;
            }
        }
        b"w:shd" => {
            if let Some(fill) = attr(tag, b"w:fill") {
                run.highlight = color_of(&fill);
            }
        }
        b"w:highlight" => {
            run.highlight = attr(tag, b"w:val").and_then(|name| highlighter(&name));
        }
        b"w:vertAlign" => {
            run.script = match attr(tag, b"w:val").as_deref() {
                Some("superscript") => Script::Super,
                Some("subscript") => Script::Sub,
                _ => Script::Normal,
            };
        }
        _ => {}
    }
}

fn color_of(hex: &str) -> Option<[u8; 3]> {
    let value = u32::from_str_radix(hex, 16)
        .ok()
        .filter(|_| hex.len() == 6)?;
    Some([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

impl Parser {
    fn para(&mut self) -> &mut Para {
        if self.paras.is_empty() {
            self.paras.push(Para::default());
        }
        let last = self.paras.len() - 1;
        &mut self.paras[last]
    }

    /// Cell being read, if any.
    fn cell(&self) -> Option<Cell> {
        let inside = self.table_depth > 0 && self.row >= 0 && self.col >= 0;
        inside.then(|| Cell {
            table: self.tables - 1,
            row: self.row.min(i32::from(u16::MAX)) as u16,
            col: self.col.min(i32::from(u16::MAX)) as u16,
        })
    }

    fn push(&mut self, ch: char) {
        if ch.is_control() && ch != '\t' {
            return;
        }
        self.para();
        self.text.push(ch);
        self.formats.push(self.run);
        self.has_text = true;
    }

    /// Show the image behind relationship `rel` in the current paragraph, or in a new one
    /// when that already has text or a picture.
    fn place_picture(&mut self, rel: &str) {
        let image = match self.image_ids.get(rel) {
            Some(image) => *image,
            None => {
                let Some(image) = self.media.get(rel).cloned() else {
                    return;
                };
                self.images.push(image);
                let id = self.images.len() as u32 - 1;
                self.image_ids.insert(rel.to_owned(), id);
                id
            }
        };
        if self.has_text || self.para().picture.is_some() {
            let para = Para {
                page_break: false,
                picture: None,
                ..*self.para()
            };
            self.break_paragraph(para);
        }
        let (width, height) = self.extent.take().unwrap_or((200, 150));
        self.para().picture = Some(Picture {
            image,
            width,
            height,
        });
    }

    /// Start a paragraph styled `para` after the current one.
    fn break_paragraph(&mut self, para: Para) {
        if !self.paras.is_empty() {
            self.text.push('\n');
            self.formats.push(Format::default());
        }
        self.paras.push(Para {
            cell: self.cell(),
            ..para
        });
        self.has_text = false;
    }

    fn open(&mut self, tag: &BytesStart<'_>) {
        match tag.name().as_ref() {
            b"w:tbl" => {
                self.table_depth += 1;
                if self.table_depth == 1 {
                    self.tables += 1;
                    self.row = -1;
                    self.col = -1;
                }
            }
            b"w:tr" if self.table_depth == 1 => {
                self.row += 1;
                self.col = -1;
            }
            b"w:tc" if self.table_depth == 1 => self.col += 1,
            b"w:p" => {
                self.break_paragraph(Para::default());
                self.use_style(None);
            }
            b"w:pStyle" => {
                if let Some(id) = attr(tag, b"w:val") {
                    self.use_style(Some(&id));
                }
            }
            b"w:pageBreakBefore" if self.table_depth == 0 => self.para().page_break = is_on(tag),
            b"w:pBdr" => self.in_borders = true,
            b"w:bottom" if self.in_borders => self.para().rule = is_on(tag),
            // Line spacing of the paragraph (runs have a `w:spacing` of their own).
            b"w:spacing" if !self.in_run => {
                if let Some(line) = attr(tag, b"w:line").and_then(|line| line.parse::<f32>().ok()) {
                    self.para().spacing = match line {
                        line if line >= 440.0 => Spacing::Double,
                        line if line >= 330.0 => Spacing::OneAndHalf,
                        _ => Spacing::Single,
                    };
                }
            }
            // A list item is indented by its level, not by what its indent measures.
            b"w:ind" if !self.in_run && self.para().list == List::None => {
                let left = attr(tag, b"w:left").or_else(|| attr(tag, b"w:start"));
                if let Some(left) = left.and_then(|left| left.parse::<f32>().ok()) {
                    let level = (left / INDENT_STEP as f32)
                        .round()
                        .clamp(0.0, f32::from(MAX_INDENT));
                    self.para().indent = level as u8;
                }
            }
            b"w:pgSz" => {
                let side = |name: &[u8]| attr(tag, name).and_then(|side| side.parse::<f32>().ok());
                if let (Some(width), Some(height)) = (side(b"w:w"), side(b"w:h")) {
                    self.page.landscape = width > height;
                }
            }
            b"w:pgMar" => {
                if let Some(left) = attr(tag, b"w:left").and_then(|left| left.parse::<f32>().ok()) {
                    self.page.margins = match left / TWIPS {
                        points if points <= 54.0 => Margins::Narrow,
                        points if points >= 90.0 => Margins::Wide,
                        _ => Margins::Normal,
                    };
                }
            }
            b"w:ilvl" if !self.in_run => {
                let level = attr(tag, b"w:val").and_then(|level| level.parse().ok());
                self.level = level.unwrap_or(0);
            }
            b"w:numId" if !self.in_run => {
                let id = attr(tag, b"w:val").unwrap_or_default();
                // Id 0 switches numbering off; ids we know nothing about are shown as bullets.
                let list = match self.lists.get(id.as_ref()) {
                    Some(levels) => {
                        let level = levels.get(self.level).or(levels.first());
                        level.copied().unwrap_or(List::Bullet)
                    }
                    None if id == "0" => List::None,
                    None => List::Bullet,
                };
                let level = self.level.min(usize::from(MAX_INDENT)) as u8;
                let para = self.para();
                para.list = list;
                if list != List::None {
                    // The number of a heading tells its level by itself.
                    let heading = para.kind.heading_level().is_some();
                    para.indent = if heading { 0 } else { level };
                }
            }
            b"w:jc" => {
                self.para().align = match attr(tag, b"w:val").as_deref() {
                    Some("center") => Align::Center,
                    Some("right" | "end") => Align::Right,
                    Some("both" | "distribute") => Align::Justify,
                    _ => Align::Left,
                };
            }
            b"w:r" => {
                self.in_run = true;
                self.run = self.base;
            }
            b"w:rPr" => self.in_run_props = self.in_run,
            b"w:t" => self.in_text = self.in_run,
            b"w:tab" if self.in_run && !self.in_run_props => self.push('\t'),
            b"w:noBreakHyphen" if self.in_run && !self.in_run_props => self.push('-'),
            // A break inside a paragraph becomes a paragraph of the same style.
            b"w:br" | b"w:cr" if self.in_run => {
                let page = attr(tag, b"w:type").as_deref() == Some("page");
                let para = Para {
                    page_break: page && self.table_depth == 0,
                    picture: None,
                    ..*self.para()
                };
                self.break_paragraph(para);
            }
            b"wp:extent" => {
                let points = |name: &[u8]| {
                    let emu = attr(tag, name).and_then(|value| value.parse::<f32>().ok())?;
                    Some((emu / EMU_PER_POINT).round().clamp(1.0, 2000.0) as u16)
                };
                self.extent = points(b"cx").zip(points(b"cy"));
            }
            b"a:blip" => {
                if let Some(rel) = attr(tag, b"r:embed") {
                    self.place_picture(&rel);
                }
            }
            // A character style (a link, an emphasis) gives the run its look.
            b"w:rStyle" if self.in_run_props => {
                if let Some(id) = attr(tag, b"w:val") {
                    self.styles.restyle(&id, &mut self.run);
                }
            }
            _ if self.in_run_props => run_prop(&mut self.run, tag),
            _ => {}
        }
    }

    /// Give the current paragraph the style `id` (`None`: the default one): its kind, what
    /// the style says of its paragraphs, and the look of its text.
    fn use_style(&mut self, id: Option<&str>) {
        let styles = Rc::clone(&self.styles);
        let id = id.or(styles.default_para.as_deref());
        let kind = id.map_or(Kind::Normal, |id| styles.kind(id));
        self.level = 0;
        let para = self.para();
        *para = Para {
            kind,
            cell: para.cell,
            page_break: para.page_break,
            picture: para.picture,
            ..Para::default()
        };
        let chain = id.map(|id| styles.chain(id)).unwrap_or_default();
        for tag in chain.iter().flat_map(|style| &style.para) {
            self.open(tag);
        }
        self.base = styles.format(id, kind);
    }

    fn close(&mut self, name: &[u8]) {
        match name {
            b"w:r" => self.in_run = false,
            b"w:rPr" => self.in_run_props = false,
            b"w:t" => self.in_text = false,
            b"w:tbl" => self.table_depth = self.table_depth.saturating_sub(1),
            b"w:pBdr" => self.in_borders = false,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Formats of everything but line breaks, which carry none in a .docx.
    fn visible(doc: &Document) -> Vec<Format> {
        let breaks = doc.text().chars().map(|ch| ch == '\n');
        doc.formats()
            .iter()
            .zip(breaks)
            .filter(|(_, is_break)| !is_break)
            .map(|(format, _)| *format)
            .collect()
    }

    #[test]
    fn round_trip_keeps_text_formats_and_paragraphs() {
        let mut doc = Document::default();
        doc.replace(0..0, "Report\nÉté <&> done\tok\nEnd", Format::default());
        doc.set_kind(0..1, Kind::Heading1);
        doc.set_align(0..1, Align::Center);
        doc.set_align(2..3, Align::Right);
        doc.restyle(7..10, |format| {
            format.bold = true;
            format.italic = true;
            format.underline = true;
            format.strike = true;
            format.color = [0xD9, 0x2D, 0x20];
            format.size = 14.5;
            format.font = Font::Serif;
        });
        doc.restyle(0..2, |format| format.bold = false);

        let back = read(&write(&doc).unwrap()).unwrap();
        assert_eq!(back.text(), doc.text());
        assert_eq!(back.paras(), doc.paras());
        assert_eq!(visible(&back), visible(&doc));
    }

    #[test]
    fn round_trip_keeps_lists_page_breaks_and_tables() {
        let mut doc = Document::default();
        doc.replace(0..0, "one\ntwo\nplain\nthree\ndot\nlast", Format::default());
        doc.set_list(0..2, List::Number);
        doc.set_list(3..4, List::Number);
        doc.set_list(4..5, List::Bullet);
        let end = doc.len();
        doc.insert_page_break(end).unwrap();
        let caret = doc.insert_table(doc.para_chars(2).start, 2, 2).unwrap();
        doc.replace(caret..caret, "cell\nsecond line", Format::default());

        let bytes = write(&doc).unwrap();
        let back = read(&bytes).unwrap();
        assert_eq!(back.text(), doc.text());
        assert_eq!(back.paras(), doc.paras());
        // Two numbered lists: each gets its own numbering instance.
        let (xml, lists, _) = document_xml(&doc);
        assert_eq!(lists, 2);
        assert!(xml.contains(r#"<w:numId w:val="5"/>"#));
        assert_eq!(xml.matches("<w:tc>").count(), 4);
        assert_eq!(xml.matches("<w:tr>").count(), 2);
    }

    #[test]
    fn round_trip_keeps_pictures_and_page_numbers() {
        let image = Image {
            bytes: vec![0x89, b'P', b'N', b'G', 1, 2, 3].into(),
            kind: ImageKind::Png,
        };
        let mut doc = Document::default();
        doc.replace(0..0, "intro\noutro", Format::default());
        let caret = doc.insert_image(2, image.clone(), 300, 150);
        doc.set_align(1..2, Align::Center);
        doc.page_numbers = true;

        let bytes = write(&doc).unwrap();
        let back = read(&bytes).unwrap();
        assert_eq!(back.text(), doc.text());
        assert_eq!(back.paras(), doc.paras());
        assert_eq!(back.images(), [image]);
        assert!(back.page_numbers);
        // A caption is written as the paragraph after the picture.
        doc.replace(caret..caret, "caption", Format::default());
        doc.page_numbers = false;
        let back = read(&write(&doc).unwrap()).unwrap();
        assert_eq!(back.text(), "intro\n\ncaption\noutro");
        assert!(back.paras()[1].picture.is_some() && back.paras()[2].picture.is_none());
        assert!(!back.page_numbers);
    }

    #[test]
    fn round_trip_keeps_looks_spacing_and_page_setup() {
        let mut doc = Document::default();
        doc.replace(0..0, "H2O is wet\nsecond\nthird", Format::default());
        doc.restyle(1..2, |format| format.script = Script::Sub);
        doc.restyle(4..6, |format| {
            format.highlight = Some([0xFF, 0xE0, 0x66]);
            format.font = Font::Georgia;
        });
        doc.restyle(11..17, |format| format.script = Script::Super);
        doc.edit_paras(1..2, |para| {
            para.spacing = Spacing::OneAndHalf;
            para.indent = 2;
        });
        doc.edit_paras(2..3, |para| {
            para.spacing = Spacing::Double;
            para.rule = true;
        });
        doc.insert_table(0, 1, 2).unwrap();
        doc.page = PageSetup {
            landscape: true,
            margins: Margins::Narrow,
        };

        let back = read(&write(&doc).unwrap()).unwrap();
        assert_eq!(back.text(), doc.text());
        assert_eq!(back.paras(), doc.paras());
        assert_eq!(visible(&back), visible(&doc));
        assert_eq!(back.page, doc.page);
        assert_eq!(font_of("Comic Sans MS"), Font::Comic);
        assert_eq!(font_of("Liberation Serif"), Font::Serif);
    }

    #[test]
    fn reads_word_markup() {
        let xml = r#"<w:document xmlns:w="x"><w:body>
            <w:p><w:pPr><w:pStyle w:val="Titre2"/><w:jc w:val="center"/><w:rPr><w:b/></w:rPr></w:pPr>
              <w:r><w:rPr><w:rFonts w:ascii="Courier New"/><w:b w:val="0"/><w:sz w:val="30"/></w:rPr><w:t>A &amp; B</w:t></w:r></w:p>
            <w:p/>
            <w:p><w:r><w:t>one</w:t><w:br/><w:t>two</w:t><w:br w:type="page"/><w:t>three</w:t></w:r></w:p>
            <w:tbl><w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc>
              <w:tc><w:tbl><w:tr><w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/></w:tc></w:tr></w:tbl>
            <w:p><w:pPr><w:numPr><w:numId w:val="7"/></w:numPr></w:pPr><w:r><w:t>item</w:t></w:r></w:p>
            </w:body></w:document>"#;
        let lists = HashMap::from([("7".to_owned(), vec![List::Number])]);
        let doc = parse(xml, lists, Styles::default(), HashMap::new()).unwrap();
        assert_eq!(doc.text(), "A & B\n\none\ntwo\nthree\na\nb\n\nitem");
        assert_eq!(doc.paras()[0].kind, Kind::Heading2);
        assert_eq!(doc.paras()[0].align, Align::Center);
        let first = doc.formats()[0];
        assert_eq!(
            (first.font, first.bold, first.size),
            (Font::Mono, false, 15.0)
        );
        assert!(doc.paras()[4].page_break && !doc.paras()[3].page_break);
        let cell = |para: usize| doc.paras()[para].cell.map(|cell| (cell.row, cell.col));
        // The nested table's text lands in the cell that holds it.
        assert_eq!(
            [cell(5), cell(6), cell(7), cell(8)],
            [Some((0, 0)), Some((0, 1)), Some((0, 1)), None]
        );
        assert_eq!(doc.paras()[8].list, List::Number);
        assert_eq!(doc.formats().len(), doc.text().chars().count());
    }

    #[test]
    fn numbering_part_tells_bullets_from_numbers() {
        let xml = r#"<w:numbering xmlns:w="x">
            <w:abstractNum w:abstractNumId="4"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum>
            <w:abstractNum w:abstractNumId="5"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum>
            <w:num w:numId="1"><w:abstractNumId w:val="5"/></w:num>
            <w:num w:numId="2"><w:abstractNumId w:val="4"/></w:num></w:numbering>"#;
        let lists = numbering(xml);
        assert_eq!(lists["1"], [List::Bullet]);
        assert_eq!(lists["2"], [List::Number, List::Bullet]);
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(matches!(read(b"not a zip"), Err(Error::NotDocx(_))));
    }

    #[test]
    fn style_ids() {
        assert_eq!(Kind::of_style("Heading1"), Kind::Heading1);
        assert_eq!(Kind::of_style("Titre3"), Kind::Heading3);
        assert_eq!(Kind::of_style("Title"), Kind::Title);
        assert_eq!(Kind::of_style("ListParagraph"), Kind::Normal);
    }

    #[test]
    fn round_trip_keeps_list_levels_dashes_and_numbered_headings() {
        let mut doc = Document::default();
        doc.replace(0..0, "Part\none\ntwo\nthree\ndash\nSub", Format::default());
        doc.set_kind(0..1, Kind::Heading1);
        doc.set_kind(5..6, Kind::Heading2);
        doc.set_list(0..4, List::Number);
        doc.set_list(4..5, List::Dash);
        doc.set_list(5..6, List::Number);
        doc.nest(2..4, true);
        doc.nest(3..4, true);
        let back = read(&write(&doc).unwrap()).unwrap();
        assert_eq!(back.paras(), doc.paras());
        assert_eq!(back.markers(), doc.markers());
        let levels: Vec<u8> = back.paras().iter().map(|para| para.indent).collect();
        assert_eq!(levels, [0, 0, 1, 2, 0, 0]);
        let (xml, lists, _) = document_xml(&doc);
        // The headings share the heading numbering, by level; the list has its own.
        assert_eq!(lists, 1);
        assert!(xml.contains(r#"<w:ilvl w:val="1"/><w:numId w:val="3"/>"#));
        assert!(xml.contains(r#"<w:ilvl w:val="2"/><w:numId w:val="4"/>"#));
    }

    const WORD_STYLES: &str = r#"<w:styles xmlns:w="x">
        <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi"/><w:sz w:val="24"/></w:rPr></w:rPrDefault></w:docDefaults>
        <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
        <w:style w:type="paragraph" w:styleId="Titre1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/>
          <w:pPr><w:numPr><w:numId w:val="9"/></w:numPr><w:jc w:val="center"/></w:pPr>
          <w:rPr><w:b/><w:color w:val="2F5496"/><w:sz w:val="32"/></w:rPr></w:style>
        <w:style w:type="paragraph" w:styleId="Titre2"><w:name w:val="heading 2"/><w:basedOn w:val="Titre1"/>
          <w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="9"/></w:numPr><w:jc w:val="left"/></w:pPr>
          <w:rPr><w:sz w:val="26"/></w:rPr></w:style>
        <w:style w:type="paragraph" w:styleId="Chapitre"><w:name w:val="Chapitre"/><w:basedOn w:val="Titre1"/></w:style>
        <w:style w:type="character" w:styleId="Lien"><w:name w:val="Hyperlink"/><w:rPr><w:u w:val="single"/></w:rPr></w:style>
        </w:styles>"#;
    const WORD_NUMBERING: &str = r#"<w:numbering xmlns:w="x"><w:abstractNum w:abstractNumId="2">
        <w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl>
        <w:lvl w:ilvl="1"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1.%2."/></w:lvl>
        <w:lvl w:ilvl="2"><w:numFmt w:val="bullet"/><w:lvlText w:val="-"/></w:lvl></w:abstractNum>
        <w:num w:numId="9"><w:abstractNumId w:val="2"/></w:num></w:numbering>"#;

    #[test]
    fn word_headings_numbered_by_their_style_keep_their_numbers() {
        let lists = numbering(WORD_NUMBERING);
        assert_eq!(lists["9"], [List::Number, List::Number, List::Dash]);
        let para = |style: &str, text: &str| {
            format!(
                r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
            )
        };
        let body = r#"<w:p><w:r><w:t>body </w:t></w:r><w:r><w:rPr><w:rStyle w:val="Lien"/><w:b/></w:rPr><w:t>link</w:t></w:r></w:p>"#;
        let item = r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="2"/><w:numId w:val="9"/></w:numPr><w:ind w:left="2160" w:hanging="360"/></w:pPr><w:r><w:t>item</w:t></w:r></w:p>"#;
        let xml = format!(
            r#"<w:document xmlns:w="x"><w:body>{}{body}{}{}{}{item}</w:body></w:document>"#,
            para("Titre1", "One"),
            para("Titre2", "Sub"),
            para("Chapitre", "Two"),
            para("Titre2", "Sub"),
        );
        let doc = parse(&xml, lists, read_styles(WORD_STYLES), HashMap::new()).unwrap();
        let markers: Vec<String> = doc.markers().into_iter().flatten().collect();
        assert_eq!(markers, ["1.", "1.1.", "2.", "2.1.", "–"]);
        let kinds: Vec<Kind> = doc.paras().iter().map(|para| para.kind).collect();
        assert_eq!(
            kinds[..4],
            [Kind::Heading1, Kind::Normal, Kind::Heading2, Kind::Heading1]
        );
        // A style gives its paragraphs their alignment and their text its look.
        assert_eq!(doc.paras()[0].align, Align::Center);
        assert_eq!(doc.paras()[2].align, Align::Left);
        let heading = doc.formats()[0];
        assert_eq!(
            (heading.size, heading.bold, heading.color),
            (16.0, true, [0x2F, 0x54, 0x96])
        );
        let (body, link) = (doc.formats()[4], doc.formats()[9]);
        assert_eq!(
            (body.size, body.font, body.underline),
            (12.0, Font::Calibri, false)
        );
        assert!(link.underline && link.bold);
        // The item is at the third level of its list, whatever its indent measures.
        assert_eq!(doc.paras()[5].indent, 2);
    }
}
