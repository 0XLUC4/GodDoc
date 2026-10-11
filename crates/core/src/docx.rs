//! Reading and writing Word documents (.docx): text, character formats, paragraph styles,
//! alignment, lists, page breaks and tables. Images and page layout are ignored; merged table
//! cells are read as the cells they are stored as.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{Cursor, Read as _, Write as _};

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
/// Numbering id of bullet lists; numbered lists take the ids after it, one per list.
const BULLET_NUM_ID: u32 = 1;

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

/// One bullet definition, one numbered definition, and a numbering instance per numbered
/// list so that each restarts at 1.
fn numbering_xml(numbered_lists: u32) -> String {
    let level = |format: &str, text: &str| {
        format!(
            r#"<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="{format}"/><w:lvlText w:val="{text}"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl>"#
        )
    };
    let mut xml = format!(
        r#"{XML_HEADER}<w:numbering xmlns:w="{WORD_NS}"><w:abstractNum w:abstractNumId="0">{}</w:abstractNum><w:abstractNum w:abstractNumId="1">{}</w:abstractNum><w:num w:numId="{BULLET_NUM_ID}"><w:abstractNumId w:val="0"/></w:num>"#,
        level("bullet", "•"),
        level("decimal", "%1.")
    );
    for list in 1..=numbered_lists {
        xml.push_str(&format!(
            r#"<w:num w:numId="{}"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="1"/></w:lvlOverride></w:num>"#,
            BULLET_NUM_ID + list
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
        let continues =
            previous.is_some_and(|before| before.list == List::Number && before.cell == para.cell);
        let num_id = match para.list {
            List::None => None,
            List::Bullet => Some(BULLET_NUM_ID),
            List::Number => {
                if !continues {
                    numbered_lists += 1;
                }
                Some(BULLET_NUM_ID + numbered_lists)
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
            num_id,
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
    num_id: Option<u32>,
    chars: &[char],
    formats: &[Format],
    drawing: Option<String>,
) {
    if let Some(drawing) = drawing {
        // The picture gets a paragraph of its own; its caption, if any, follows.
        let props = para_props(para, num_id);
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
    xml.push_str(&para_props(para, num_id));
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

/// `w:pPr` of a paragraph.
fn para_props(para: Para, num_id: Option<u32>) -> String {
    let mut xml = String::from("<w:pPr>");
    if para.kind != Kind::Normal {
        xml.push_str(&format!(r#"<w:pStyle w:val="{}"/>"#, para.kind.style_id()));
    }
    if para.page_break {
        xml.push_str("<w:pageBreakBefore/>");
    }
    if let Some(num_id) = num_id {
        xml.push_str(&format!(
            r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="{num_id}"/></w:numPr>"#
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
    if para.indent > 0 {
        let left = u32::from(para.indent) * INDENT_STEP;
        xml.push_str(&format!(r#"<w:ind w:left="{left}"/>"#));
    }
    let align = match para.align {
        Align::Left => None,
        Align::Center => Some("center"),
        Align::Right => Some("right"),
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
    let mut doc = parse(&document, lists, media)?;
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

/// Kind of list of each numbering id in `word/numbering.xml`.
fn numbering(xml: &str) -> HashMap<String, List> {
    let mut reader = Reader::from_str(xml);
    // Abstract definition id -> its first level is a bullet.
    let mut bullets: HashMap<String, bool> = HashMap::new();
    let mut nums: Vec<(String, String)> = Vec::new();
    let mut definition = String::new();
    let mut num = String::new();
    while let Ok(event) = reader.read_event() {
        let tag = match &event {
            Event::Start(tag) | Event::Empty(tag) => tag,
            Event::Eof => break,
            _ => continue,
        };
        let value = |name: &[u8]| attr(tag, name).map(Cow::into_owned).unwrap_or_default();
        match tag.name().as_ref() {
            b"w:abstractNum" => definition = value(b"w:abstractNumId"),
            b"w:numFmt" => {
                let bullet = value(b"w:val") == "bullet";
                bullets.entry(definition.clone()).or_insert(bullet);
            }
            b"w:num" => num = value(b"w:numId"),
            b"w:abstractNumId" => nums.push((num.clone(), value(b"w:val"))),
            _ => {}
        }
    }
    nums.into_iter()
        .map(|(num, definition)| {
            let list = if bullets.get(&definition).copied().unwrap_or(true) {
                List::Bullet
            } else {
                List::Number
            };
            (num, list)
        })
        .collect()
}

#[derive(Default)]
struct Parser {
    text: String,
    formats: Vec<Format>,
    paras: Vec<Para>,
    lists: HashMap<String, List>,
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
    lists: HashMap<String, List>,
    media: HashMap<String, Image>,
) -> Result<Document> {
    let mut reader = Reader::from_str(xml);
    let mut parser = Parser {
        lists,
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
            b"w:p" => self.break_paragraph(Para::default()),
            b"w:pStyle" => {
                if let Some(id) = attr(tag, b"w:val") {
                    self.para().kind = Kind::of_style(&id);
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
            b"w:ind" if !self.in_run => {
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
            b"w:numId" if !self.in_run => {
                let id = attr(tag, b"w:val").unwrap_or_default();
                // Id 0 switches numbering off; ids we know nothing about are shown as bullets.
                self.para().list = match self.lists.get(id.as_ref()) {
                    Some(list) => *list,
                    None if id == "0" => List::None,
                    None => List::Bullet,
                };
            }
            b"w:jc" => {
                self.para().align = match attr(tag, b"w:val").as_deref() {
                    Some("center") => Align::Center,
                    Some("right" | "end") => Align::Right,
                    _ => Align::Left,
                };
            }
            b"w:r" => {
                self.in_run = true;
                self.run = self.para().kind.format();
            }
            b"w:rPr" => self.in_run_props = self.in_run,
            b"w:t" => self.in_text = self.in_run,
            b"w:tab" if self.in_run && !self.in_run_props => self.push('\t'),
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
            name if self.in_run_props => self.run_prop(name, tag),
            _ => {}
        }
    }

    fn run_prop(&mut self, name: &[u8], tag: &BytesStart<'_>) {
        match name {
            b"w:b" => self.run.bold = is_on(tag),
            b"w:i" => self.run.italic = is_on(tag),
            b"w:strike" => self.run.strike = is_on(tag),
            b"w:u" => self.run.underline = is_on(tag),
            b"w:sz" => {
                if let Some(half_points) =
                    attr(tag, b"w:val").and_then(|val| val.parse::<f32>().ok())
                {
                    self.run.size = (half_points / 2.0).clamp(1.0, 400.0);
                }
            }
            b"w:color" => {
                if let Some(color) = attr(tag, b"w:val").and_then(|val| color_of(&val)) {
                    self.run.color = color;
                }
            }
            b"w:rFonts" => {
                if let Some(family) = attr(tag, b"w:ascii") {
                    self.run.font = font_of(&family);
                }
            }
            b"w:shd" => {
                if let Some(fill) = attr(tag, b"w:fill") {
                    self.run.highlight = color_of(&fill);
                }
            }
            b"w:highlight" => {
                self.run.highlight = attr(tag, b"w:val").and_then(|name| highlighter(&name));
            }
            b"w:vertAlign" => {
                self.run.script = match attr(tag, b"w:val").as_deref() {
                    Some("superscript") => Script::Super,
                    Some("subscript") => Script::Sub,
                    _ => Script::Normal,
                };
            }
            _ => {}
        }
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
        assert!(xml.contains(r#"<w:numId w:val="3"/>"#));
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
        let lists = HashMap::from([("7".to_owned(), List::Number)]);
        let doc = parse(xml, lists, HashMap::new()).unwrap();
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
        assert_eq!(lists.get("1"), Some(&List::Bullet));
        assert_eq!(lists.get("2"), Some(&List::Number));
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
}
