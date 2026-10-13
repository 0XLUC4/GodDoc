//! Printing on Windows: the print dialog of the system, then each page as GodDoc draws it.

use std::io::{BufRead as _, BufReader, BufWriter};
use std::os::windows::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use image::ImageEncoder as _;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};

use crate::render::Bitmap;

/// How many drawn pages may wait for the printer: more would only fill the disk.
const AHEAD: usize = 3;
/// Shows the dialog and prints the pages: see what it says of itself.
const SCRIPT: &str = include_str!("print.ps1");

/// Print the `pages` pages of the document `name` through the print dialog; `draw` paints
/// one (the first is 0). Returns what was done, to tell the user: `None` when the dialog was
/// closed without printing.
pub fn print(
    name: &str,
    pages: usize,
    draw: impl Fn(usize) -> Bitmap,
) -> Result<Option<&'static str>, String> {
    let dir = std::env::temp_dir().join(format!("goddoc-print-{}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("Cannot write {}: {err}", dir.display()))?;
    let done = run(name, pages, &draw, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    done
}

fn run(
    name: &str,
    pages: usize,
    draw: &impl Fn(usize) -> Bitmap,
    dir: &Path,
) -> Result<Option<&'static str>, String> {
    let mut child = Command::new("powershell")
        .args(["-NoProfile", "-STA", "-Command", SCRIPT])
        .env("GODDOC_PRINT_DIR", dir)
        .env("GODDOC_PRINT_NAME", name)
        .env("GODDOC_PRINT_PAGES", pages.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // No console window of its own.
        .creation_flags(0x0800_0000)
        .spawn()
        .map_err(|err| format!("Cannot open the print dialog: {err}"))?;
    let said = child.stdout.take().map(BufReader::new);
    let said = said.into_iter().flat_map(BufReader::lines);
    let mut done = Err("Printing stopped without saying why".to_owned());
    for line in said.map_while(Result::ok) {
        let line = line.trim();
        if line == "cancel" {
            done = Ok(None);
        } else if line == "done" {
            done = Ok(Some("Sent to the printer"));
        } else if let Some(why) = line.strip_prefix("error ") {
            done = Err(format!("Cannot print: {why}"));
        } else if let Some(range) = line.strip_prefix("pages ") {
            let mut numbers = range.split(' ').filter_map(|number| number.parse().ok());
            if let (Some(first), Some(last)) = (numbers.next(), numbers.next()) {
                draw_pages(draw, dir, first..=last, &mut child)?;
            }
        }
    }
    let _ = child.wait();
    done
}

/// Draw the pages `pages` (the first is 1) into `dir`, one file each, no more than [`AHEAD`]
/// before the printer takes them. Stops when the printing itself has.
fn draw_pages(
    draw: &impl Fn(usize) -> Bitmap,
    dir: &Path,
    pages: std::ops::RangeInclusive<usize>,
    printing: &mut Child,
) -> Result<(), String> {
    let stopped = |printing: &mut Child| !matches!(printing.try_wait(), Ok(None));
    for page in pages {
        let waiting = || std::fs::read_dir(dir).map_or(0, Iterator::count);
        while waiting() >= AHEAD {
            if stopped(printing) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        if stopped(printing) {
            return Ok(());
        }
        let failed = |err: &dyn std::fmt::Display| format!("Cannot print page {page}: {err}");
        let bitmap = draw(page.saturating_sub(1));
        // Written under another name: the printer must not take half a file.
        let part = dir.join(format!("{page}.part"));
        let file = std::fs::File::create(&part).map_err(|err| failed(&err))?;
        PngEncoder::new_with_quality(BufWriter::new(file), CompressionType::Fast, FilterType::Sub)
            .write_image(
                &bitmap.rgb,
                bitmap.width,
                bitmap.height,
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|err| failed(&err))?;
        std::fs::rename(&part, dir.join(format!("{page}.png"))).map_err(|err| failed(&err))?;
    }
    Ok(())
}
