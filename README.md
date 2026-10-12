<p align="center"><img src="assets/logo.png" width="160" alt="GodDoc logo"></p>

<h1 align="center">GodDoc</h1>
<p align="center">A word processor in pure Rust. Local: no account, no cloud.</p>

<p align="center"><img src="docs/screenshot.png" alt="GodDoc with a document open" width="900"></p>

## Download

Windows: get `GodDoc-windows-x64.exe` from the [latest release](https://github.com/0XLUC4/GodDoc/releases/latest).
It is a single file, nothing to install. It is not signed, so Windows SmartScreen may ask before running it.
New versions come through the Updates button: it downloads one and restarts on it.

## Features

- Rich text: bold, italic, underline, strikethrough, superscript, subscript, highlight, eight fonts, sizes, colors, change case, clear formatting, format painter.
- Paragraph styles (Normal, Title, Heading 1-3), alignment (left, center, right, justified), line spacing, indent, horizontal lines.
- Lists with levels: bullets, numbers and dashes. `- `, `* ` or `1. ` starts one, Tab goes a level down, Shift+Tab a level up.
- Numbered headings that count through the document: 1. then 1.1. then 1.1.1. Tab makes a heading a sub-heading.
- Word files: opens and saves `.docx`. The styles of a file are read (fonts, sizes, colors, alignment), and headings numbered in Word keep their numbers.
- Real A4 pages that fill up and continue on the next one; page breaks (Ctrl+Enter), orientation, margins, page numbers.
- Table of contents written from the headings, with their numbers and pages.
- Tables: cells hold formatted text, Tab moves to the next cell; rows and columns can be added and removed.
- Pictures (PNG, JPEG): drag to move, corners to resize; any paragraph or table moves by its margin grip.
- Find and replace (Ctrl+F), date and special characters, formatting marks for spaces and ends of paragraphs.
- Export to Markdown and to a web page; also opens and saves `.txt`.
- Navigation pane of headings, home screen with recent documents, several documents in tabs, undo / redo, dark and light theme.

Not there yet: text wrapping around pictures, merged cells, headers and footers, comments, PDF export, spelling.

## Build

```
cargo run --release
cargo test --workspace
```

Layout: `crates/core` (document model, `.docx` reading and writing, export), `crates/app` (egui app).

## License

MIT OR Apache-2.0
