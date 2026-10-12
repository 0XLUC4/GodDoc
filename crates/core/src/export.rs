//! Writing a document as Markdown or as a web page. Both keep the text, the headings, the
//! lists, the tables and what marks the characters; pictures are left out.

use crate::{Align, Document, Format, Kind, List, Para, Script};

/// One paragraph of a document: its text, the formats of its characters, its marker.
struct Line<'a> {
    text: &'a str,
    formats: &'a [Format],
    para: &'a Para,
    marker: Option<String>,
}

/// A paragraph outside tables, or a table as its rows of cells of paragraphs.
enum Block<'a> {
    Para(Line<'a>),
    Table(Vec<Vec<Vec<Line<'a>>>>),
}

fn blocks(doc: &Document) -> Vec<Block<'_>> {
    let mut blocks: Vec<Block<'_>> = Vec::new();
    // Table the last block is.
    let mut open = None;
    let mut start = 0;
    let paras = doc.paras().iter().zip(doc.markers());
    for (text, (para, marker)) in doc.text().split('\n').zip(paras) {
        let count = text.chars().count();
        let formats = &doc.formats()[start..start + count];
        start += count + 1;
        let line = Line {
            text,
            formats,
            para,
            marker,
        };
        let Some(cell) = para.cell else {
            open = None;
            blocks.push(Block::Para(line));
            continue;
        };
        if open != Some(cell.table) {
            open = Some(cell.table);
            blocks.push(Block::Table(Vec::new()));
        }
        if let Some(Block::Table(rows)) = blocks.last_mut() {
            let (row, col) = (usize::from(cell.row), usize::from(cell.col));
            rows.resize_with(rows.len().max(row + 1), Vec::new);
            let cells = &mut rows[row];
            cells.resize_with(cells.len().max(col + 1), Vec::new);
            cells[col].push(line);
        }
    }
    blocks
}

/// The text of `line` in stretches of one format, each written by `wrap`.
fn runs(line: &Line<'_>, wrap: impl Fn(&str, &Format) -> String) -> String {
    let mut chars = line.text.chars();
    let mut out = String::new();
    for run in line.formats.chunk_by(|a, b| a == b) {
        let text: String = chars.by_ref().take(run.len()).collect();
        out.push_str(&wrap(&text, &run[0]));
    }
    out
}

// ---------------------------------------------------------------- Markdown

/// The document as Markdown.
pub fn markdown(doc: &Document) -> String {
    let listed = |para: &Para| para.kind == Kind::Normal && para.list != List::None;
    let mut out = String::new();
    // The paragraph before, when the block before was one.
    let mut previous: Option<&Para> = None;
    for block in blocks(doc) {
        let line = match block {
            Block::Para(line) => line,
            Block::Table(rows) => {
                out.push('\n');
                for (index, row) in rows.iter().enumerate() {
                    let cells = row.iter().map(|cell| {
                        let lines = cell.iter().map(|line| markdown_text(line, false));
                        lines.collect::<Vec<_>>().join(" ")
                    });
                    out.push_str(&format!("| {} |\n", cells.collect::<Vec<_>>().join(" | ")));
                    // The rule under the first row is what makes these lines a table.
                    if index == 0 {
                        out.push_str(&format!("|{}\n", " --- |".repeat(row.len())));
                    }
                }
                previous = None;
                continue;
            }
        };
        let para = line.para;
        let heading = para.kind != Kind::Normal;
        let prefix = match (para.kind, &line.marker) {
            (Kind::Title | Kind::Heading1, _) => "# ".to_owned(),
            (Kind::Heading2, _) => "## ".to_owned(),
            (Kind::Heading3, _) => "### ".to_owned(),
            (Kind::Normal, Some(marker)) => {
                let bullet = if marker.ends_with('.') { marker } else { "-" };
                format!("{}{bullet} ", "    ".repeat(usize::from(para.indent)))
            }
            (Kind::Normal, None) => String::new(),
        };
        // Items of a list follow each other; anything else stands apart.
        let together = previous.is_some_and(|before| listed(before) && listed(para));
        if !out.is_empty() && !together {
            out.push('\n');
        }
        // The number of a numbered heading is part of its words.
        let number = line.marker.as_ref().filter(|_| heading);
        let number = number.map(|marker| format!("{marker} "));
        let text = markdown_text(&line, heading);
        out.push_str(&format!("{prefix}{}{text}\n", number.unwrap_or_default()));
        previous = Some(para);
    }
    out
}

/// The words of `line` with their marks; `heading` text is bold by itself.
fn markdown_text(line: &Line<'_>, heading: bool) -> String {
    runs(line, |text, format| {
        // Marks go around the words, not around the spaces beside them.
        let core = text.trim();
        if core.is_empty() {
            return text.to_owned();
        }
        let mut marked = escape_markdown(core);
        if format.strike {
            marked = format!("~~{marked}~~");
        }
        if format.italic {
            marked = format!("*{marked}*");
        }
        if format.bold && !heading {
            marked = format!("**{marked}**");
        }
        let lead = &text[..text.len() - text.trim_start().len()];
        let tail = &text[text.trim_end().len()..];
        format!("{lead}{marked}{tail}")
    })
}

fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(
            ch,
            '*' | '_' | '`' | '#' | '[' | ']' | '<' | '>' | '|' | '~'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

// ---------------------------------------------------------------- HTML

/// The document as a web page that stands on its own.
pub fn html(doc: &Document, title: &str) -> String {
    let mut body = String::new();
    for block in blocks(doc) {
        match block {
            Block::Para(line) => body.push_str(&html_para(&line)),
            Block::Table(rows) => {
                body.push_str("<table>\n");
                for row in rows {
                    body.push_str("<tr>");
                    for cell in row {
                        let paras: String = cell.iter().map(html_para).collect();
                        body.push_str(&format!("<td>{}</td>", paras.trim_end()));
                    }
                    body.push_str("</tr>\n");
                }
                body.push_str("</table>\n");
            }
        }
    }
    let style = "body{max-width:21cm;margin:2.5cm auto;padding:0 1cm;line-height:1.3}\
        p,h1,h2,h3{margin:0;min-height:1em}\
        table{border-collapse:collapse;width:100%}td{border:1px solid #333;padding:5px;vertical-align:top}";
    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n<style>{style}</style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        escape_html(title)
    )
}

fn html_para(line: &Line<'_>) -> String {
    let para = line.para;
    let tag = match para.kind {
        Kind::Normal => "p",
        Kind::Title | Kind::Heading1 => "h1",
        Kind::Heading2 => "h2",
        Kind::Heading3 => "h3",
    };
    let mut style = String::new();
    let align = match para.align {
        Align::Left => None,
        Align::Center => Some("center"),
        Align::Right => Some("right"),
        Align::Justify => Some("justify"),
    };
    if let Some(align) = align {
        style.push_str(&format!("text-align:{align};"));
    }
    if para.indent > 0 {
        style.push_str(&format!("margin-left:{}pt;", u32::from(para.indent) * 24));
    }
    if para.spacing != crate::Spacing::Single {
        let height = 1.3 * para.spacing.factor();
        style.push_str(&format!("line-height:{height};"));
    }
    if para.page_break {
        style.push_str("page-break-before:always;");
    }
    if para.rule {
        style.push_str("border-bottom:1px solid #333;");
    }
    let text = runs(line, html_run);
    let marker = line.marker.as_ref().map(|marker| {
        // In the look of the words it stands before.
        let look = line.formats.first().copied();
        html_run(
            &format!("{marker}\u{a0}"),
            &look.unwrap_or(para.kind.format()),
        )
    });
    format!(
        "<{tag} style=\"{style}\">{}{text}</{tag}>\n",
        marker.unwrap_or_default()
    )
}

fn html_run(text: &str, format: &Format) -> String {
    let [r, g, b] = format.color;
    let mut style = format!(
        "font-family:'{}';font-size:{}pt;color:#{r:02x}{g:02x}{b:02x};font-weight:{};",
        format.font.family(),
        format.size,
        if format.bold { "bold" } else { "normal" },
    );
    if format.italic {
        style.push_str("font-style:italic;");
    }
    let lines: Vec<&str> = [
        format.underline.then_some("underline"),
        format.strike.then_some("line-through"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !lines.is_empty() {
        style.push_str(&format!("text-decoration:{};", lines.join(" ")));
    }
    if let Some([r, g, b]) = format.highlight {
        style.push_str(&format!("background:#{r:02x}{g:02x}{b:02x};"));
    }
    let span = format!("<span style=\"{style}\">{}</span>", escape_html(text));
    match format.script {
        Script::Normal => span,
        Script::Super => format!("<sup>{span}</sup>"),
        Script::Sub => format!("<sub>{span}</sub>"),
    }
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&emsp;"),
            ch => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Document {
        let mut doc = Document::default();
        let text = "Report\nSome bold & <plain> text\none\ntwo\nsub\nEnd";
        doc.replace(0..0, text, Format::default());
        doc.set_kind(0..1, Kind::Heading1);
        doc.set_list(0..1, List::Number);
        doc.restyle(12..16, |format| format.bold = true);
        doc.set_list(2..5, List::Dash);
        doc.nest(4..5, true);
        doc.set_align(5..6, Align::Justify);
        let end = doc.len();
        let caret = doc.insert_table(end, 2, 2).unwrap();
        doc.replace(caret..caret, "a", Format::default());
        doc.replace(caret + 2..caret + 2, "b", Format::default());
        doc
    }

    #[test]
    fn markdown_keeps_headings_lists_marks_and_tables() {
        let expected = "# 1. Report\n\nSome **bold** & \\<plain\\> text\n\n- one\n- two\n    - sub\n\nEnd\n\n| a | b |\n| --- | --- |\n|  |  |\n\n\n";
        assert_eq!(markdown(&sample()), expected);
    }

    #[test]
    fn html_keeps_the_look_of_the_text() {
        let page = html(&sample(), "My <report>");
        assert!(page.contains("<title>My &lt;report&gt;</title>"));
        assert!(page.contains("<h1 style=\"\"><span"));
        assert!(page.contains("font-weight:bold;\">bold</span>"));
        assert!(page.contains("&amp; &lt;plain&gt; text"));
        assert!(page.contains("margin-left:24pt;"));
        assert!(page.contains("<p style=\"text-align:justify;\">"));
        assert_eq!(page.matches("<td>").count(), 4);
        assert_eq!(page.matches("<tr>").count(), 2);
    }
}
