//! Peer-controlled text and file names, made safe to show and to save.

use std::path::PathBuf;

/// Drops control, zero-width and bidi-override characters from a peer-chosen string.
pub fn clean_name(s: &str) -> String {
    s.chars()
        .filter(|c| {
            let u = *c as u32;
            u >= 0x20
                && !(0x7f..=0x9f).contains(&u)
                && !matches!(u, 0x200b..=0x200f | 0x202a..=0x202e | 0x2066..=0x2069 | 0xfeff)
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(64)
        .collect()
}

/// A message from a peer: newlines and tabs kept, other control and bidi characters dropped.
pub fn clean_text(s: &str) -> String {
    s.chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .filter(|c| !matches!(*c as u32, 0x202a..=0x202e | 0x2066..=0x2069))
        .collect()
}

/// Keeps only a safe base name.
pub fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let clean: String = clean_name(base)
        .chars()
        .filter(|c| !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    // No leading dot (hidden files), no trailing dot or space (Windows drops them).
    let clean = clean.trim_start_matches('.').trim_end_matches(['.', ' ']).to_string();
    if clean.is_empty() {
        return "file".into();
    }
    // Windows device names open the device, not a file, whatever the extension.
    let stem = clean.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved { format!("_{clean}") } else { clean }
}

/// Creates a new file for `name` in `dir`, as `name (1)`, `name (2)`… when taken. Never opens an
/// existing path (nor follows a symlink planted there), so a download cannot overwrite anything.
pub fn create_unique(dir: &std::path::Path, name: &str) -> std::io::Result<(std::fs::File, PathBuf)> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 0..10_000 {
        let path = if n == 0 {
            dir.join(name)
        } else {
            dir.join(format!("{stem} ({n}){ext}"))
        };
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((f, path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free file name"))
}

/// A MIME type for a file we offer, from its extension.
pub fn guess_mime(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("txt" | "md") => "text/plain",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::new_id;

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  Al\u{202e}ice\u{0007} "), "Alice");
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name(".bashrc"), "bashrc");
        assert_eq!(sanitize_file_name("a:b?.txt"), "ab.txt");
        assert_eq!(sanitize_file_name(""), "file");
        assert_eq!(sanitize_file_name("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_file_name("COM1"), "_COM1");
        assert_eq!(sanitize_file_name("report. . "), "report");
    }

    #[test]
    fn unique_files() {
        let dir = std::env::temp_dir().join(format!("hush-test-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (_, a) = create_unique(&dir, "x.txt").unwrap();
        assert_eq!(a.file_name().unwrap(), "x.txt");
        assert_eq!(
            create_unique(&dir, "x.txt").unwrap().1.file_name().unwrap(),
            "x (1).txt"
        );
        // A symlink planted under the name is not followed.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("target"), dir.join("y.txt")).unwrap();
            assert_eq!(
                create_unique(&dir, "y.txt").unwrap().1.file_name().unwrap(),
                "y (1).txt"
            );
            assert!(!dir.join("target").exists());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
