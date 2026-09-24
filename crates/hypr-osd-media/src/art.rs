//! Album art: `mpris:artUrl` turned into something the card can draw.
//!
//! Players disagree about what belongs in that field. VLC and most local players
//! hand out a *percent-encoded* `file://` URL (VLC's extracted cover lives under
//! `~/.cache/vlc/art`), while players that stream - and some browsers - use
//! `http(s)://`. Both are handled: the local file is decoded, the remote one is
//! fetched with `curl`, which keeps this crate at zero HTTP dependencies and in
//! line with the collection's "shell out to the tool that already does this"
//! rule (wpctl, playerctl, curl).
//!
//! Anything else - a `blob:` URL, a `data:` URI - has no file behind it that
//! `gdk::Texture` could read, so the card keeps the glyph it shows for "no
//! artwork". Some browser players are known to expose artwork that way.
//!
//! Nothing here may block: `on_ready` always runs later, on the main loop.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use gtk::gdk;
use gtk::glib;

/// How long a remote cover may take before the card gives up on it.
const DOWNLOAD_TIMEOUT: &str = "6";

/// Names the download files of one daemon apart.
static DOWNLOADS: AtomicU32 = AtomicU32::new(0);

/// Load the artwork behind `url` and hand the texture to `on_ready` once.
/// `None` means "no artwork", and the card falls back to its glyph.
pub fn load(url: &str, on_ready: impl FnOnce(Option<gdk::Texture>) + 'static) {
    if let Some(path) = file_path(url) {
        // A local decode is fast, but it still happens on the main loop's next
        // turn: one uniform contract ("your callback runs later, never during
        // the call") is worth more than the frame it saves. A zero timeout
        // rather than an idle source, because the callback belongs to a GTK
        // widget and idle sources may be called from any thread.
        glib::timeout_add_local_once(Duration::ZERO, move || on_ready(decode(&path)));
        return;
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        fetch(url, on_ready);
        return;
    }
    if !url.is_empty() {
        eprintln!("hypr-osd-media: cannot read artwork from `{url}`");
    }
    glib::timeout_add_local_once(Duration::ZERO, move || on_ready(None));
}

/// Fetch `url` with curl, then load what landed on disk.
fn fetch(url: &str, on_ready: impl FnOnce(Option<gdk::Texture>) + 'static) {
    let path = std::env::temp_dir().join(format!(
        "hypr-osd-media-art-{}-{}",
        std::process::id(),
        DOWNLOADS.fetch_add(1, Ordering::Relaxed)
    ));

    let spawned = Command::new("curl")
        .args(["-fsSL", "--max-time", DOWNLOAD_TIMEOUT, "-o"])
        .arg(&path)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    let child = match spawned {
        Ok(child) => child,
        Err(error) => {
            eprintln!("hypr-osd-media: cannot run curl ({error}) - cover art stays a glyph");
            glib::timeout_add_local_once(Duration::ZERO, move || on_ready(None));
            return;
        }
    };

    // GLib's child watch reaps the process (so dropping the handle here does not
    // leave a zombie) and calls back with its exit status.
    let pid = glib::Pid(child.id() as i32);
    let pending = std::cell::RefCell::new(Some(on_ready));
    glib::child_watch_add_local(pid, move |_, status| {
        let texture = if status == 0 { decode(&path) } else { None };
        let _ = std::fs::remove_file(&path);
        if let Some(on_ready) = pending.borrow_mut().take() {
            on_ready(texture);
        }
    });
}

/// The path behind a `file://` URL, percent-decoded.
fn file_path(url: &str) -> Option<PathBuf> {
    Some(PathBuf::from(percent_decode(url.strip_prefix("file://")?)))
}

fn decode(path: &Path) -> Option<gdk::Texture> {
    match gdk::Texture::from_filename(path) {
        Ok(texture) => Some(texture),
        Err(error) => {
            eprintln!(
                "hypr-osd-media: cannot read cover art {}: {error}",
                path.display()
            );
            None
        }
    }
}

/// `%20` becomes a space, `%C3%A9` an é - VLC escapes the artist and album in
/// its cache path, so a cover for "Beyoncé" arrives as `Beyonc%C3%A9`.
///
/// A malformed escape is passed through instead of being dropped: a path with a
/// stray `%` in it is still more likely to be the file the player meant than a
/// truncated one.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                decoded.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_paths_players_actually_send() {
        // The shape VLC uses for its extracted covers.
        assert_eq!(
            file_path("file:///home/lars/.cache/vlc/art/artistalbum/Test%20Artist/art.png"),
            Some(PathBuf::from(
                "/home/lars/.cache/vlc/art/artistalbum/Test Artist/art.png"
            ))
        );
        assert_eq!(
            percent_decode("Beyonc%C3%A9%20-%20Album"),
            "Beyoncé - Album"
        );
        // A valid escape may produce a "%" of its own, and a malformed one is
        // passed through rather than dropped: a path with a stray "%" in it is
        // still more likely to be the file the player meant.
        assert_eq!(percent_decode("100%25"), "100%");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("100%zz"), "100%zz");
        assert_eq!(percent_decode("plain.png"), "plain.png");
    }

    #[test]
    fn only_file_urls_are_local_paths() {
        assert_eq!(file_path("https://example.com/a.png"), None);
        assert_eq!(file_path("blob:https://example.com/1234"), None);
        assert_eq!(file_path(""), None);
    }
}
