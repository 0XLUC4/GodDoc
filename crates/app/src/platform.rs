//! File dialogs and safe writes.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};

fn dialog() -> rfd::FileDialog {
    rfd::FileDialog::new()
        .add_filter("Word document", &["docx"])
        .add_filter("Plain text", &["txt"])
}

pub fn pick_documents() -> Vec<PathBuf> {
    dialog()
        .add_filter("All documents", &["docx", "txt"])
        .pick_files()
        .unwrap_or_default()
}

pub fn pick_images() -> Vec<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Images", &["png", "jpg", "jpeg"])
        .pick_files()
        .unwrap_or_default()
}

/// Ask where to save a document called `name`.
pub fn pick_save(name: &str) -> Option<PathBuf> {
    let known = Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("docx") || ext.eq_ignore_ascii_case("txt"));
    let name = if known {
        name.to_owned()
    } else {
        format!("{name}.docx")
    };
    dialog().set_file_name(name).save_file()
}

/// Ask where to write a copy of the document `name` in another format.
pub fn pick_export(name: &str, label: &str, extension: &str) -> Option<PathBuf> {
    let stem = Path::new(name).file_stem().unwrap_or(name.as_ref());
    let name = format!("{}.{extension}", stem.to_string_lossy());
    rfd::FileDialog::new()
        .add_filter(label, &[extension])
        .set_file_name(name)
        .save_file()
}

/// Write through a temporary file so a failure never leaves a half-written document.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut temp_name = target
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("document.docx"));
    temp_name.push(".goddoc.tmp");
    let temp = target.with_file_name(temp_name);
    let written = std::fs::File::create(&temp)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temp, target));
    written.map_err(|err| {
        // Best effort: the temp file may not exist.
        let _ = std::fs::remove_file(&temp);
        format!("Cannot write {}: {err}", target.display())
    })
}
