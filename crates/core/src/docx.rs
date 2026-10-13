//! Reading and writing Word documents (.docx): text, character formats, paragraph styles,
//! alignment, lists, page breaks and tables. Images and page layout are ignored; merged table
//! cells are read as the cells they are stored as.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{Cursor, Read as _, Write as _};
use std::ops::Range;
use std::rc::Rc;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::{
    Align, Banner, Cell, Change, Comment, Document, Error, Font, Format, Image, ImageKind, Kind,
    List, MAX_INDENT, PageSetup, Paper, Para, Picture, Result, Script, Wrap,
};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/><Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#;

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
    let [top, right, bottom, left] = page.margin_points().map(|side| (side * TWIPS) as u32);
    let orient = if page.landscape {
        r#" w:orient="landscape""#
    } else {
        ""
    };
    // Text in columns: half an inch between them.
    let columns = if page.columns > 1 {
        format!(r#"<w:cols w:num="{}" w:space="720"/>"#, page.columns)
    } else {
        String::new()
    };
    format!(
        r#"<w:pgSz w:w="{width}" w:h="{height}"{orient}/><w:pgMar w:top="{top}" w:right="{right}" w:bottom="{bottom}" w:left="{left}" w:header="708" w:footer="708" w:gutter="0"/>{columns}"#
    )
}

/// Width between the margins, in twentieths of a point.
fn content_width(page: PageSetup) -> u32 {
    let [_, right, _, left] = page.margin_points();
    ((page.size().0 - left - right) * TWIPS) as u32
}

/// Relationships of the footer and of the header.
const FOOTER_REL: &str = "rIdFooter";
const HEADER_REL: &str = "rIdHeader";
/// English Metric Units (the unit of drawing sizes) per point.
const EMU_PER_POINT: f32 = 12700.0;

/// Push `text` as the value of an attribute.
fn push_attr(xml: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '"' => xml.push_str("&quot;"),
            ch => push_escaped(xml, ch),
        }
    }
}

/// `word/comments.xml`: the comments the text refers to, by their numbers less one.
fn comments_xml(doc: &Document) -> String {
    let mut xml = format!(r#"{XML_HEADER}<w:comments xmlns:w="{WORD_NS}">"#);
    for (index, comment) in doc.comments.iter().enumerate() {
        let note = u16::try_from(index + 1).unwrap_or(0);
        if doc.noted(note).is_none() {
            continue;
        }
        xml.push_str(&format!(r#"<w:comment w:id="{index}" w:author=""#));
        push_attr(&mut xml, &comment.author);
        xml.push_str(r#""><w:p><w:r><w:t xml:space="preserve">"#);
        comment
            .text
            .chars()
            .for_each(|ch| push_escaped(&mut xml, ch));
        xml.push_str("</w:t></w:r></w:p></w:comment>");
    }
    xml.push_str("</w:comments>");
    xml
}

/// The comments of a `word/comments.xml`, by the id the text refers to them with.
fn read_comments(xml: &str) -> Vec<(String, Comment)> {
    let mut reader = Reader::from_str(xml);
    let mut comments: Vec<(String, Comment)> = Vec::new();
    let mut in_text = false;
    while let Ok(event) = reader.read_event() {
        match &event {
            Event::Eof => break,
            Event::Start(tag) if tag.name().as_ref() == b"w:comment" => {
                let id = attr(tag, b"w:id").unwrap_or_default().into_owned();
                let author = attr(tag, b"w:author").unwrap_or_default().into_owned();
                let comment = Comment {
                    author,
                    text: String::new(),
                };
                comments.push((id, comment));
            }
            Event::Start(tag) if tag.name().as_ref() == b"w:t" => in_text = true,
            Event::End(tag) if tag.name().as_ref() == b"w:t" => in_text = false,
            // Paragraphs of a comment are its lines.
            Event::End(tag) if tag.name().as_ref() == b"w:p" => {
                if let Some((_, comment)) = comments.last_mut() {
                    comment.text.push('\n');
                }
            }
            Event::Text(text) if in_text => {
                if let Some((_, comment)) = comments.last_mut() {
                    comment.text.push_str(&text.unescape().unwrap_or_default());
                }
            }
            _ => {}
        }
    }
    for (_, comment) in &mut comments {
        comment.text = comment.text.trim().to_owned();
    }
    comments
}

/// Relationship of the link numbered `link`.
fn link_rel(link: u16) -> String {
    format!("rIdLink{link}")
}

/// `word/_rels/document.xml.rels`; `media` are the image files, in the order of their ids,
/// `links` where the links of the text lead.
fn document_rels(media: &[String], links: &[String]) -> String {
    let mut xml = format!(
        r#"{XML_HEADER}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RELS_NS}/styles" Target="styles.xml"/><Relationship Id="rId2" Type="{RELS_NS}/numbering" Target="numbering.xml"/><Relationship Id="{FOOTER_REL}" Type="{RELS_NS}/footer" Target="footer1.xml"/><Relationship Id="{HEADER_REL}" Type="{RELS_NS}/header" Target="header1.xml"/><Relationship Id="rIdComments" Type="{RELS_NS}/comments" Target="comments.xml"/>"#
    );
    for (index, link) in links.iter().enumerate() {
        let id = link_rel(u16::try_from(index + 1).unwrap_or(0));
        xml.push_str(&format!(
            r#"<Relationship Id="{id}" Type="{RELS_NS}/hyperlink" TargetMode="External" Target=""#
        ));
        push_attr(&mut xml, link);
        xml.push_str(r#""/>"#);
    }
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

/// A header (`w:hdr`) or footer (`w:ftr`) part: one paragraph, its middle and its right
/// reached by tabs set across the text area of `page`.
fn banner_xml(root: &str, banner: &Banner, page: PageSetup) -> String {
    let field = |kind: &str| format!(r#"<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>"#);
    let number = |name: &str| {
        format!(
            r#"{}<w:r><w:instrText xml:space="preserve"> {name} </w:instrText></w:r>{}<w:r><w:t>1</w:t></w:r>{}"#,
            field("begin"),
            field("separate"),
            field("end")
        )
    };
    let written = |text: &str| {
        let mut run = String::from(r#"<w:r><w:t xml:space="preserve">"#);
        text.chars().for_each(|ch| push_escaped(&mut run, ch));
        run + "</w:t></w:r>"
    };
    let zone = |text: &str| {
        let mut xml = String::new();
        let mut rest = text;
        // The longer name first: the other one starts the same.
        let fields = [(Banner::PAGES, "NUMPAGES"), (Banner::PAGE, "PAGE")];
        while !rest.is_empty() {
            let next = fields
                .iter()
                .filter_map(|(token, name)| Some((rest.find(token)?, *token, *name)))
                .min_by_key(|(at, token, _)| (*at, std::cmp::Reverse(token.len())));
            let Some((at, token, name)) = next else {
                xml.push_str(&written(rest));
                break;
            };
            if at > 0 {
                xml.push_str(&written(&rest[..at]));
            }
            xml.push_str(&number(name));
            rest = &rest[at + token.len()..];
        }
        xml
    };
    let width = content_width(page);
    let middle = width / 2;
    let tab = "<w:r><w:tab/></w:r>";
    format!(
        r#"{XML_HEADER}<{root} xmlns:w="{WORD_NS}"><w:p><w:pPr><w:tabs><w:tab w:val="center" w:pos="{middle}"/><w:tab w:val="right" w:pos="{width}"/></w:tabs></w:pPr>{}{tab}{}{tab}{}</w:p></{root}>"#,
        zone(&banner.left),
        zone(&banner.center),
        zone(&banner.right)
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
            Self::Subtitle => "Subtitle",
            Self::Heading1 => "Heading1",
            Self::Heading2 => "Heading2",
            Self::Heading3 => "Heading3",
            Self::Quote => "Quote",
        }
    }

    /// Kind of a style id, English or French Word ("Heading1", "Titre1", "Title", "Titre").
    fn of_style(id: &str) -> Self {
        let id = id.to_ascii_lowercase();
        match id.as_str() {
            "subtitle" | "sous-titre" | "soustitre" => return Self::Subtitle,
            "quote" | "citation" => return Self::Quote,
            _ => {}
        }
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
        (
            "word/_rels/document.xml.rels",
            document_rels(&names, &doc.links),
        ),
        ("word/comments.xml", comments_xml(doc)),
        ("word/styles.xml", styles_xml()),
        ("word/numbering.xml", numbering_xml(numbered_lists)),
        (
            "word/footer1.xml",
            banner_xml("w:ftr", &doc.shown_footer(), doc.page),
        ),
        (
            "word/header1.xml",
            banner_xml("w:hdr", &doc.header, doc.page),
        ),
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
    // Where the text each comment is about starts and ends.
    let noted: HashMap<u16, Range<usize>> = (1..=doc.comments.len())
        .filter_map(|note| {
            let note = u16::try_from(note).ok()?;
            Some((note, doc.noted(note)?))
        })
        .collect();
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
            (start, &chars[start..end], &formats[start..end]),
            &noted,
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
    let mut footer = String::new();
    if !doc.header.is_empty() {
        footer.push_str(&format!(
            r#"<w:headerReference w:type="default" r:id="{HEADER_REL}"/>"#
        ));
    }
    if !doc.shown_footer().is_empty() {
        footer.push_str(&format!(
            r#"<w:footerReference w:type="default" r:id="{FOOTER_REL}"/>"#
        ));
    }
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
    let columns = u32::from(doc.table_size(cell.table).1).max(1);
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
    // A joined cell takes several columns of the grid.
    let span = u32::from(doc.cell_span(cell));
    let joined = if span > 1 {
        format!(r#"<w:gridSpan w:val="{span}"/>"#)
    } else {
        String::new()
    };
    xml.push_str(&format!(
        r#"<w:tc><w:tcPr><w:tcW w:w="{}" w:type="dxa"/>{joined}</w:tcPr>"#,
        width * span
    ));
}

fn paragraph_xml(
    xml: &mut String,
    para: Para,
    num: Option<(u32, u32)>,
    (offset, chars, formats): (usize, &[char], &[Format]),
    noted: &HashMap<u16, Range<usize>>,
    drawing: Option<String>,
) {
    // A picture beside the text floats in the paragraph of that text.
    let mut beside = String::new();
    if let Some(drawing) = drawing.as_ref().filter(|_| para.wrap != Wrap::Above) {
        let side = if para.wrap == Wrap::Right {
            "right"
        } else {
            "left"
        };
        let anchor = format!(
            r#"<wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" relativeHeight="251658240" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:align>{side}</wp:align></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV>"#
        );
        beside = drawing
            .replace(
                r#"<wp:inline distT="0" distB="0" distL="0" distR="0">"#,
                &anchor,
            )
            .replace(
                "<wp:docPr",
                r#"<wp:wrapSquare wrapText="bothSides"/><wp:docPr"#,
            )
            .replace("</wp:inline>", "</wp:anchor>");
    } else if let Some(drawing) = &drawing {
        // The picture gets a paragraph of its own; its caption, if any, follows.
        let props = para_props(para, num);
        xml.push_str(&format!("<w:p>{props}{drawing}</w:p>"));
        if chars.is_empty() {
            return;
        }
    }
    let para = Para {
        page_break: para.page_break && (para.picture.is_none() || !beside.is_empty()),
        ..para
    };
    xml.push_str("<w:p>");
    xml.push_str(&para_props(para, num));
    xml.push_str(&beside);
    let mut start = 0;
    for run in formats.chunk_by(|a, b| a == b) {
        let text = &chars[start..start + run.len()];
        // Where the run is in the whole text.
        let place = offset + start..offset + start + run.len();
        start += run.len();
        let look = run[0];
        let about = noted.get(&look.note).filter(|_| look.note != 0);
        if about.is_some_and(|about| about.start == place.start) {
            let id = look.note - 1;
            xml.push_str(&format!(r#"<w:commentRangeStart w:id="{id}"/>"#));
        }
        if look.link != 0 {
            let id = link_rel(look.link);
            xml.push_str(&format!(r#"<w:hyperlink r:id="{id}" w:history="1">"#));
        }
        // A tracked change wraps its run; what was taken out is written as such.
        let (wrap, letters) = match look.change {
            Change::None => (None, "w:t"),
            Change::Inserted => (Some("w:ins"), "w:t"),
            Change::Deleted => (Some("w:del"), "w:delText"),
        };
        if let Some(wrap) = wrap {
            let id = place.start;
            xml.push_str(&format!(r#"<{wrap} w:id="{id}" w:author="GodDoc">"#));
        }
        // The style may be bold: say so when the run is not.
        let props = run_props(&look, para.kind.format().bold);
        xml.push_str(&format!("<w:r><w:rPr>{props}</w:rPr>"));
        for (index, piece) in text.split(|ch| *ch == '\t').enumerate() {
            if index > 0 {
                xml.push_str("<w:tab/>");
            }
            if !piece.is_empty() {
                xml.push_str(&format!(r#"<{letters} xml:space="preserve">"#));
                piece.iter().for_each(|ch| push_escaped(xml, *ch));
                xml.push_str(&format!("</{letters}>"));
            }
        }
        xml.push_str("</w:r>");
        if let Some(wrap) = wrap {
            xml.push_str(&format!("</{wrap}>"));
        }
        if look.link != 0 {
            xml.push_str("</w:hyperlink>");
        }
        if about.is_some_and(|about| about.end == place.end) {
            let id = look.note - 1;
            xml.push_str(&format!(
                r#"<w:commentRangeEnd w:id="{id}"/><w:r><w:commentReference w:id="{id}"/></w:r>"#
            ));
        }
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
    // 240 is single spacing.
    let line = (240.0 * para.line_factor()).round() as u32;
    if line != 240 || para.before > 0 || para.after > 0 {
        let (before, after) = (para.before, para.after);
        xml.push_str(&format!(
            r#"<w:spacing w:before="{before}" w:after="{after}" w:line="{line}" w:lineRule="auto"/>"#
        ));
    }
    // A list item is indented by its level in the list. Word counts the left indent up to
    // the lines after the first: a hanging first line starts that much before it.
    let hanging = u32::from(para.first_line.min(0).unsigned_abs());
    let left = u32::from(para.indent) * INDENT_STEP + u32::from(para.left) + hanging;
    if num.is_none() && (left > 0 || para.right > 0 || para.first_line != 0) {
        let first = match para.first_line {
            0 => String::new(),
            first if first > 0 => format!(r#" w:firstLine="{first}""#),
            _ => format!(r#" w:hanging="{hanging}""#),
        };
        let right = para.right;
        xml.push_str(&format!(
            r#"<w:ind w:left="{left}" w:right="{right}"{first}/>"#
        ));
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
    let comments = part("word/comments.xml")
        .map(|xml| read_comments(&xml))
        .unwrap_or_default();
    // The header and the footer of ordinary pages, from the parts the document names.
    let named = relationships(&rels);
    let mut banner = |tag: &str| {
        let id = default_reference(&document, tag)?;
        let (_, target) = named.iter().find(|(named, _)| *named == id)?;
        let xml = part(&format!("word/{}", target.trim_start_matches('/'))).ok()?;
        Some(read_banner(&xml))
    };
    let header = banner("<w:headerReference").unwrap_or_default();
    // Where links lead, and what the comments say, for the text that refers to them.
    let marks = Marks {
        targets: relationships(&rels).into_iter().collect(),
        comments,
    };
    let footer = banner("<w:footerReference").unwrap_or_default();
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
    let mut doc = parse(&document, lists, styles, media, marks)?;
    doc.header = header;
    let number_alone =
        footer.center == Banner::PAGE && footer.left.is_empty() && footer.right.is_empty();
    let placed = footer
        .zones()
        .iter()
        .any(|zone| zone.contains(Banner::PAGE));
    // A page number alone in the middle is what "page numbers" writes.
    (doc.footer, doc.page_numbers) = match (number_alone, placed) {
        (true, _) => (Banner::default(), true),
        (false, true) => (footer, false),
        (false, false) => (footer, page_numbers),
    };
    Ok(doc)
}

/// Id of the relationship `tag` (as `<w:headerReference`) gives for ordinary pages: not the
/// one of the first page alone, nor of even pages.
fn default_reference(document: &str, tag: &str) -> Option<String> {
    document.match_indices(tag).find_map(|(at, _)| {
        let end = document[at..].find('>')?;
        let found = &document[at..at + end];
        let id = found.split("r:id=\"").nth(1)?.split('"').next()?;
        let ordinary = !found.contains("w:type=\"first\"") && !found.contains("w:type=\"even\"");
        ordinary.then(|| id.to_owned())
    })
}

/// What a header or footer part says: its first written paragraph, cut at its tabs into
/// left, middle and right. Page fields become [`Banner::PAGE`] and [`Banner::PAGES`].
fn read_banner(xml: &str) -> Banner {
    let mut reader = Reader::from_str(xml);
    let mut zones = vec![String::new()];
    let mut align: Option<String> = None;
    // In paragraph properties, in a text, in the code of a field, in what a field shows.
    let (mut in_props, mut in_text, mut in_code, mut in_result) = (false, false, false, false);
    let token = |code: &str| {
        if code.contains("NUMPAGES") {
            Banner::PAGES
        } else if code.contains("PAGE") {
            Banner::PAGE
        } else {
            ""
        }
    };
    while let Ok(event) = reader.read_event() {
        let opens = matches!(event, Event::Start(_));
        match &event {
            Event::Eof => break,
            Event::Start(tag) | Event::Empty(tag) => match tag.name().as_ref() {
                b"w:pPr" => in_props = opens,
                b"w:jc" => align = attr(tag, b"w:val").map(Cow::into_owned),
                // A tab among the properties is a tab stop, not a tab.
                b"w:tab" | b"w:ptab" if !in_props => zones.push(String::new()),
                b"w:t" => in_text = opens,
                b"w:instrText" => in_code = opens,
                b"w:fldChar" => match attr(tag, b"w:fldCharType").as_deref() {
                    Some("separate") => in_result = true,
                    Some("end") => in_result = false,
                    _ => {}
                },
                b"w:fldSimple" => {
                    let code = attr(tag, b"w:instr").unwrap_or_default();
                    if let Some(zone) = zones.last_mut() {
                        zone.push_str(token(&code));
                    }
                    in_result = opens;
                }
                _ => {}
            },
            Event::End(tag) => match tag.name().as_ref() {
                b"w:pPr" => in_props = false,
                b"w:t" => in_text = false,
                b"w:instrText" => in_code = false,
                b"w:fldSimple" => in_result = false,
                b"w:p" => {
                    if zones.iter().any(|zone| !zone.trim().is_empty()) {
                        break;
                    }
                    (zones, align) = (vec![String::new()], None);
                }
                _ => {}
            },
            Event::Text(text) => {
                let text = text.unescape().unwrap_or_default();
                let Some(zone) = zones.last_mut() else {
                    continue;
                };
                if in_code {
                    zone.push_str(token(&text));
                } else if in_text && !in_result {
                    zone.push_str(&text);
                }
            }
            _ => {}
        }
    }
    let mut zones = zones.into_iter().map(|zone| zone.trim().to_owned());
    let (first, second) = (zones.next().unwrap_or_default(), zones.next());
    let rest: Vec<String> = zones.collect();
    match (second, align.as_deref()) {
        // One stretch of text: where its paragraph puts it.
        (None, Some("center")) => Banner {
            center: first,
            ..Banner::default()
        },
        (None, Some("right" | "end")) => Banner {
            right: first,
            ..Banner::default()
        },
        (None, _) => Banner {
            left: first,
            ..Banner::default()
        },
        (Some(center), _) => Banner {
            left: first,
            center,
            right: rest.join(" "),
        },
    }
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
    marks: Marks,
    /// Where the links met so far lead.
    links: Vec<String>,
    /// Columns the table cell being read takes.
    span: u8,
    /// The drawing being read floats beside the text, on this side; its side is being read.
    floating: Option<Wrap>,
    in_align: bool,
    /// The link, the tracked change and the comments the text being read is in.
    link: u16,
    change: Change,
    notes: Vec<String>,
}

/// What the text of a document refers to by an id: where its links lead, what its comments
/// say.
#[derive(Default)]
struct Marks {
    targets: HashMap<String, String>,
    comments: Vec<(String, Comment)>,
}

fn parse(
    xml: &str,
    lists: Lists,
    styles: Styles,
    media: HashMap<String, Image>,
    marks: Marks,
) -> Result<Document> {
    let mut reader = Reader::from_str(xml);
    let mut parser = Parser {
        lists,
        styles: Rc::new(styles),
        media,
        marks,
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
            Ok(Event::Text(text)) if parser.in_align => {
                if text.unescape().is_ok_and(|side| side.trim() == "right")
                    && parser.floating.is_some()
                {
                    parser.floating = Some(Wrap::Right);
                }
            }
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
        header: Banner::default(),
        footer: Banner::default(),
        links: parser.links,
        comments: parser
            .marks
            .comments
            .into_iter()
            .map(|(_, comment)| comment)
            .collect(),
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
        ("segoe", Font::SegoeUi),
        ("consolas", Font::Consolas),
        ("trebuchet", Font::Trebuchet),
        ("cambria", Font::Cambria),
        ("candara", Font::Candara),
        ("constantia", Font::Constantia),
        ("corbel", Font::Corbel),
        ("palatino", Font::Palatino),
        ("garamond", Font::Garamond),
        ("century gothic", Font::CenturyGothic),
        ("book antiqua", Font::BookAntiqua),
        ("impact", Font::Impact),
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
        // The comment it is about: the last one opened, by its place among the comments.
        let note = self.notes.last().and_then(|id| {
            let place = self
                .marks
                .comments
                .iter()
                .position(|(named, _)| named == id)?;
            u16::try_from(place + 1).ok()
        });
        self.formats.push(Format {
            link: self.link,
            change: self.change,
            note: note.unwrap_or(0),
            ..self.run
        });
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
        self.para().wrap = self.floating.unwrap_or_default();
    }

    /// Start a paragraph styled `para` after the current one.
    fn break_paragraph(&mut self, para: Para) {
        if !self.paras.is_empty() {
            self.text.push('\n');
            self.formats.push(Format::default());
        }
        let cell = self.cell();
        self.paras.push(Para {
            cell,
            span: if cell.is_some() && self.span > 1 {
                self.span
            } else {
                0
            },
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
                self.span = 1;
            }
            // A cell starts after the columns the one before it took.
            b"w:tc" if self.table_depth == 1 => {
                self.col += i32::from(self.span.max(1));
                self.span = 1;
            }
            b"w:gridSpan" if self.table_depth == 1 => {
                let span = attr(tag, b"w:val").and_then(|span| span.parse::<u8>().ok());
                self.span = span.unwrap_or(1).max(1);
            }
            b"wp:anchor" => self.floating = Some(Wrap::Left),
            b"wp:align" => self.in_align = true,
            b"w:hyperlink" => {
                let target =
                    attr(tag, b"r:id").and_then(|id| self.marks.targets.get(&*id).cloned());
                self.link = target.map_or(0, |target| {
                    let place = self.links.iter().position(|link| *link == target);
                    let place = place.unwrap_or_else(|| {
                        self.links.push(target);
                        self.links.len() - 1
                    });
                    u16::try_from(place + 1).unwrap_or(0)
                });
            }
            b"w:ins" => self.change = Change::Inserted,
            b"w:del" => self.change = Change::Deleted,
            b"w:delText" => self.in_text = true,
            b"w:commentRangeStart" => {
                if let Some(id) = attr(tag, b"w:id") {
                    self.notes.push(id.into_owned());
                }
            }
            b"w:commentRangeEnd" => {
                if let Some(id) = attr(tag, b"w:id") {
                    self.notes.retain(|open| *open != *id);
                }
            }
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
                let twips = |name: &[u8]| attr(tag, name).and_then(|side| side.parse::<f32>().ok());
                let measure = |twips: f32| twips.clamp(0.0, f32::from(u16::MAX)) as u16;
                if let Some(before) = twips(b"w:before") {
                    self.para().before = measure(before);
                }
                if let Some(after) = twips(b"w:after") {
                    self.para().after = measure(after);
                }
                // A height in points ("exact", "atLeast") is not a number of lines.
                let lines = attr(tag, b"w:lineRule").is_none_or(|rule| rule == "auto");
                if let Some(line) = twips(b"w:line").filter(|_| lines) {
                    self.para().set_line(measure((line / 2.4).round()));
                }
            }
            // A list item is indented by its level, not by what its indent measures.
            b"w:ind" if !self.in_run && self.para().list == List::None => {
                let twips = |name: &[u8]| attr(tag, name).and_then(|side| side.parse::<f32>().ok());
                let measure = |twips: f32| twips.clamp(0.0, f32::from(u16::MAX)) as u16;
                let hanging = twips(b"w:hanging").unwrap_or(0.0).max(0.0);
                if let Some(left) = twips(b"w:left").or_else(|| twips(b"w:start")) {
                    // Word counts the left indent up to the lines after the first.
                    let left = (left - hanging).max(0.0);
                    let step = INDENT_STEP as f32;
                    let level = (left / step).round();
                    // A whole number of steps is an indent level; anything else, a measure.
                    let stepped =
                        (left - level * step).abs() < 1.0 && level <= f32::from(MAX_INDENT);
                    let para = self.para();
                    (para.indent, para.left) = if stepped {
                        (level as u8, 0)
                    } else {
                        (0, measure(left))
                    };
                }
                if let Some(right) = twips(b"w:right").or_else(|| twips(b"w:end")) {
                    self.para().right = measure(right);
                }
                let first = twips(b"w:firstLine").unwrap_or(0.0);
                let limit = f32::from(i16::MAX);
                self.para().first_line = if hanging > 0.0 {
                    -(hanging.min(limit) as i16)
                } else {
                    first.clamp(0.0, limit) as i16
                };
            }
            b"w:cols" => {
                let count = attr(tag, b"w:num").and_then(|count| count.parse::<u8>().ok());
                self.page.columns = count.filter(|count| *count > 1).unwrap_or(0).min(4);
            }
            b"w:pgSz" => {
                let side = |name: &[u8]| attr(tag, name).and_then(|side| side.parse::<f32>().ok());
                if let (Some(width), Some(height)) = (side(b"w:w"), side(b"w:h")) {
                    self.page.landscape = width > height;
                    self.page.paper = Paper::closest(width / TWIPS, height / TWIPS);
                }
            }
            b"w:pgMar" => {
                let side = |name: &[u8]| {
                    let twips = attr(tag, name).and_then(|side| side.parse::<f32>().ok());
                    twips.map(|twips| twips.clamp(0.0, f32::from(u16::MAX)) as u16)
                };
                let sides = [
                    side(b"w:top"),
                    side(b"w:right"),
                    side(b"w:bottom"),
                    side(b"w:left"),
                ];
                if let [Some(top), Some(right), Some(bottom), Some(left)] = sides {
                    self.page.set_margins([top, right, bottom, left]);
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
            span: para.span,
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
            b"w:t" | b"w:delText" => self.in_text = false,
            b"w:hyperlink" => self.link = 0,
            b"wp:anchor" => self.floating = None,
            b"wp:align" => self.in_align = false,
            b"w:ins" | b"w:del" => self.change = Change::None,
            b"w:tbl" => self.table_depth = self.table_depth.saturating_sub(1),
            b"w:pBdr" => self.in_borders = false,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Margins, Spacing};

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
            paper: Paper::Letter,
            landscape: true,
            margins: Margins::Narrow,
            custom: None,
            columns: 2,
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
        let doc = parse(
            xml,
            lists,
            Styles::default(),
            HashMap::new(),
            Marks::default(),
        )
        .unwrap();
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
    fn measures_of_paragraphs_and_margins_come_back_from_a_file() {
        let mut doc = Document::default();
        doc.replace(
            0..0,
            "one
two
three",
            Format::default(),
        );
        doc.edit_paras(0..1, |para| {
            (para.left, para.right, para.first_line) = (567, 284, 400);
            (para.before, para.after) = (120, 240);
            para.set_line(115);
        });
        // A hanging first line, and an indent that is no whole number of levels.
        doc.edit_paras(1..2, |para| (para.left, para.first_line) = (300, -709));
        doc.page.set_margins([1000, 1500, 1200, 2000]);
        let back = read(&write(&doc).unwrap()).unwrap();
        assert_eq!(back.paras(), doc.paras());
        assert_eq!(back.page, doc.page);
        assert_eq!(back.page.margin_points(), [50.0, 75.0, 60.0, 100.0]);

        // Four sides that are a named margin are that margin, and line spacings with a name
        // keep it.
        let mut page = PageSetup::default();
        page.set_margins([720; 4]);
        assert_eq!((page.margins, page.custom), (Margins::Narrow, None));
        let mut para = Para::default();
        para.set_line(150);
        assert_eq!((para.spacing, para.line), (Spacing::OneAndHalf, 0));
        para.set_line(115);
        assert_eq!((para.line, para.line_factor()), (115, 1.15));
    }

    #[test]
    fn headers_and_footers_come_back_from_a_file() {
        let mut doc = Document::default();
        doc.replace(0..0, "text", Format::default());
        doc.header = Banner {
            left: "Report & notes".into(),
            center: String::new(),
            right: "Spring <edition>".into(),
        };
        doc.footer = Banner {
            left: "GodDoc".into(),
            center: String::new(),
            right: "Page {page} of {pages}".into(),
        };
        let back = read(&write(&doc).unwrap()).unwrap();
        assert_eq!((&back.header, &back.footer), (&doc.header, &doc.footer));
        assert!(!back.page_numbers);
        assert_eq!(Banner::on_page(&back.footer.right, 2, 5), "Page 2 of 5");

        // Page numbers alone are still only that, and join a footer that does not place them.
        let mut doc = Document {
            page_numbers: true,
            ..Document::default()
        };
        let back = read(&write(&doc).unwrap()).unwrap();
        assert!(back.page_numbers && back.footer.is_empty() && back.header.is_empty());
        doc.footer.center = "Draft".into();
        assert_eq!(doc.shown_footer().center, "Draft {page}");

        // A header written by Word: one centred paragraph, a field written the short way.
        let word = r#"<w:hdr xmlns:w="x"><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>Page </w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>7</w:t></w:r></w:fldSimple></w:p></w:hdr>"#;
        assert_eq!(read_banner(word).center, "Page {page}");
    }

    #[test]
    fn links_comments_and_tracked_changes_come_back_from_a_file() {
        let mut doc = Document::default();
        doc.replace(
            0..0,
            "See the site, a note here\nand old new text",
            Format::default(),
        );
        let link = doc.link_to("https://example.org/?a=1&b=\"2\"");
        doc.restyle(8..12, |format| format.link = link);
        // A comment about words of two paragraphs.
        let note = doc.add_comment(Comment {
            author: "Ada".into(),
            text: "Is this <right>?".into(),
        });
        doc.restyle(16..29, |format| format.note = note);
        doc.restyle(30..34, |format| format.change = Change::Deleted);
        doc.restyle(34..38, |format| format.change = Change::Inserted);
        let back = read(&write(&doc).unwrap()).unwrap();
        assert_eq!(back.text(), doc.text());
        assert_eq!(visible(&back), visible(&doc));
        assert_eq!((&back.links, &back.comments), (&doc.links, &doc.comments));
        assert_eq!(
            back.link(back.format_at(9).link),
            Some("https://example.org/?a=1&b=\"2\"")
        );
        assert_eq!(back.noted(note), Some(16..29));

        // Accepting keeps what was put in and drops what was taken out; rejecting, the reverse.
        let mut accepted = back.clone();
        accepted.settle_changes(0..accepted.len(), true);
        assert!(accepted.text().ends_with("and new text"));
        let mut rejected = back;
        rejected.settle_changes(0..rejected.len(), false);
        assert!(rejected.text().ends_with("and old text"));
        let settled = |doc: &Document| {
            doc.formats()
                .iter()
                .all(|format| format.change == Change::None)
        };
        assert!(settled(&accepted) && settled(&rejected));
    }

    #[test]
    fn joined_cells_come_back_from_a_file() {
        let mut doc = Document::default();
        doc.insert_table(0, 2, 3).unwrap();
        let first = doc.paras().iter().find_map(|para| para.cell).unwrap();
        assert!(doc.merge_cell_right(first));
        assert_eq!(
            (doc.cell_span(first), doc.table_size(first.table)),
            (2, (2, 3))
        );
        let cols = |doc: &Document| -> Vec<(u16, u16, u8)> {
            let cells = doc
                .paras()
                .iter()
                .filter_map(|para| Some((para.cell?, para.span)));
            cells
                .map(|(cell, span)| (cell.row, cell.col, span))
                .collect()
        };
        // The first row: one cell of two columns (its two paragraphs), then the third column.
        assert_eq!(cols(&doc)[..3], [(0, 0, 2), (0, 0, 2), (0, 2, 0)]);
        let back = read(&write(&doc).unwrap()).unwrap();
        assert_eq!(cols(&back), cols(&doc));
        assert!(back.has_joined_cells(first.table));

        // Split, the row has its three cells again; a third cell cannot be joined twice.
        assert!(doc.merge_cell_right(first) && !doc.merge_cell_right(first));
        assert!(doc.split_cell(first) && !doc.split_cell(first));
        let row: Vec<u16> = cols(&doc)
            .iter()
            .filter(|cell| cell.0 == 0)
            .map(|cell| cell.1)
            .collect();
        assert_eq!(row, [0, 0, 0, 1, 2]);
    }

    #[test]
    fn a_picture_beside_its_text_comes_back_from_a_file() {
        let image = Image {
            bytes: vec![0x89, b'P', b'N', b'G', 1, 2, 3].into(),
            kind: ImageKind::Png,
        };
        for wrap in [Wrap::Left, Wrap::Right] {
            let mut doc = Document::default();
            doc.replace(
                0..0,
                "intro
text beside the picture
outro",
                Format::default(),
            );
            doc.insert_image(8, image.clone(), 200, 120);
            let para = doc
                .paras()
                .iter()
                .position(|para| para.picture.is_some())
                .unwrap();
            doc.edit_paras(para..para + 1, |para| para.wrap = wrap);
            let back = read(&write(&doc).unwrap()).unwrap();
            assert_eq!(back.text(), doc.text());
            assert_eq!(back.paras(), doc.paras());
        }
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
        assert_eq!(Kind::of_style("Subtitle"), Kind::Subtitle);
        assert_eq!(Kind::of_style("Sous-titre"), Kind::Subtitle);
        assert_eq!(Kind::of_style("Citation"), Kind::Quote);
    }

    #[test]
    fn every_kind_and_every_font_comes_back_from_a_file() {
        let lines: Vec<&str> = Kind::ALL.iter().map(|kind| kind.name()).collect();
        let mut doc = Document::default();
        doc.replace(
            0..0,
            &lines.join(
                "
",
            ),
            Format::default(),
        );
        for (index, kind) in Kind::ALL.into_iter().enumerate() {
            doc.set_kind(index..index + 1, kind);
        }
        let back = read(&write(&doc).unwrap()).unwrap();
        let kinds: Vec<Kind> = back.paras().iter().map(|para| para.kind).collect();
        assert_eq!(kinds, Kind::ALL);
        // A quote is slanted and grey, as its style says.
        let quote = back.format_at(back.text().chars().count() - 1);
        assert!(quote.italic && quote.color == Kind::Quote.format().color);
        for font in Font::ALL {
            assert_eq!(font_of(font.family()), font, "{}", font.family());
        }
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
        let doc = parse(
            &xml,
            lists,
            read_styles(WORD_STYLES),
            HashMap::new(),
            Marks::default(),
        )
        .unwrap();
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
