<p align="center"><img src="assets/logo.png" width="160" alt="GodDoc logo"></p>

<h1 align="center">GodDoc</h1>
<p align="center">A word processor in pure Rust. Local: no account, no cloud.</p>

<p align="center"><img src="docs/screenshot.png" alt="GodDoc with a document open" width="900"></p>

## Download

Windows: get `GodDoc-windows-x64.exe` from the [latest release](https://github.com/0XLUC4/GodDoc/releases/latest).
It is a single file, nothing to install. It is not signed, so Windows SmartScreen may ask before running it.

## Features

- Rich text: bold, italic, underline, strikethrough, superscript, subscript, highlight, eight fonts, sizes, colors, change case, clear formatting.
- Paragraph styles (Normal, Title, Heading 1-3), alignment, line spacing, indent, horizontal lines, bullet and numbered lists.
- Real A4 pages that fill up and continue on the next one; page breaks (Ctrl+Enter).
- Tables: cells hold formatted text, Tab moves to the next cell; rows and columns can be added and removed.
- Find and replace (Ctrl+F), page orientation and margins, date and special characters.
- Pictures (PNG, JPEG): drag to move, corners to resize; any paragraph or table moves by its margin grip.
- Menu bar, navigation pane of headings, page numbers, home screen with recent documents.
- Opens and saves `.docx` and `.txt`; undo / redo; several documents in tabs.

Not there yet: text wrapping around pictures, merged cells, headers and footers, justified text, comments, PDF export.

```
cargo run --release
cargo test --workspace
```

Layout: `crates/core` (document model, `.docx` reading / writing), `crates/app` (egui app).
