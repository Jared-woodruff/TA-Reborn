//! A music file's title, for the jukebox: its tags where it has them (ID3v2 in MP3s, Vorbis
//! comments in Ogg and FLAC, the INFO list in WAV), else its file name. Only the head of the
//! file is read.

use std::io::Read;
use std::path::Path;

/// How much of a file is looked through for its tags.
const HEAD: usize = 256 * 1024;

/// "Artist – Title", the title alone, or the file's name as it reads ("my_track-01" →
/// "my track 01").
pub fn title_of(path: &Path) -> String {
    let mut head = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(HEAD as u64).read_to_end(&mut head);
    }
    let (title, artist) = crate::guarded(|| tags(&head)).unwrap_or((None, None));
    match (title, artist) {
        (Some(t), Some(a)) => format!("{a} – {t}"),
        (Some(t), None) => t,
        _ => path.file_stem().map(|s| s.to_string_lossy().replace(['_', '-'], " ").split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default(),
    }
}

/// (title, artist) from a file's head.
pub fn tags(head: &[u8]) -> (Option<String>, Option<String>) {
    if head.starts_with(b"ID3") {
        return id3(head);
    }
    if head.starts_with(b"RIFF") {
        return riff_info(head);
    }
    // Ogg Vorbis and FLAC: Vorbis comments, each a little-endian length and "KEY=value".
    (comment(head, b"title="), comment(head, b"artist="))
}

fn clean(s: String) -> Option<String> {
    let s = s.trim_matches(char::from(0)).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// ID3v2.3/2.4: the TIT2 and TPE1 frames.
fn id3(b: &[u8]) -> (Option<String>, Option<String>) {
    if b.len() < 10 {
        return (None, None);
    }
    let version = b[3];
    let syncsafe = |x: &[u8]| ((x[0] as usize & 0x7f) << 21) | ((x[1] as usize & 0x7f) << 14) | ((x[2] as usize & 0x7f) << 7) | (x[3] as usize & 0x7f);
    let size = syncsafe(&b[6..10]).min(b.len() - 10);
    let tag = &b[10..10 + size];
    let (mut title, mut artist) = (None, None);
    let mut at = 0;
    while at + 10 <= tag.len() {
        let id = &tag[at..at + 4];
        if id[0] == 0 {
            break;
        }
        let len = if version >= 4 { syncsafe(&tag[at + 4..at + 8]) } else { u32::from_be_bytes([tag[at + 4], tag[at + 5], tag[at + 6], tag[at + 7]]) as usize };
        let body = &tag[(at + 10).min(tag.len())..(at + 10 + len).min(tag.len())];
        match id {
            b"TIT2" => title = text_frame(body),
            b"TPE1" => artist = text_frame(body),
            _ => {}
        }
        at += 10 + len;
    }
    (title, artist)
}

/// An ID3 text frame: its encoding byte, then the text.
fn text_frame(body: &[u8]) -> Option<String> {
    let (&encoding, text) = body.split_first()?;
    let s = match encoding {
        0 => text.iter().map(|&c| c as char).collect(),
        1 | 2 => {
            // UTF-16 with a byte order mark (1) or big-endian (2).
            let big = encoding == 2 || text.starts_with(&[0xFE, 0xFF]);
            let text = if text.starts_with(&[0xFE, 0xFF]) || text.starts_with(&[0xFF, 0xFE]) { &text[2..] } else { text };
            let units: Vec<u16> = text.as_chunks::<2>().0.iter().map(|&c| if big { u16::from_be_bytes(c) } else { u16::from_le_bytes(c) }).take_while(|&u| u != 0).collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(text).into_owned(),
    };
    clean(s)
}

/// A Vorbis comment `key` ("title="), found by its name; its length precedes it.
fn comment(b: &[u8], key: &[u8]) -> Option<String> {
    let lower: Vec<u8> = b.iter().map(|c| c.to_ascii_lowercase()).collect();
    let mut from = 0;
    while let Some(i) = find(&lower[from..], key).map(|i| i + from) {
        if i >= 4 {
            let len = u32::from_le_bytes([b[i - 4], b[i - 3], b[i - 2], b[i - 1]]) as usize;
            if len > key.len() && i + len <= b.len() {
                return clean(String::from_utf8_lossy(&b[i + key.len()..i + len]).into_owned());
            }
        }
        from = i + 1;
    }
    None
}

/// WAV: the INFO list's INAM (title) and IART (artist).
fn riff_info(b: &[u8]) -> (Option<String>, Option<String>) {
    let field = |id: &[u8]| {
        let i = find(b, id)?;
        let len = u32::from_le_bytes(b.get(i + 4..i + 8)?.try_into().ok()?) as usize;
        clean(String::from_utf8_lossy(b.get(i + 8..i + 8 + len)?).into_owned())
    };
    (field(b"INAM"), field(b"IART"))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_come_from_their_tags() {
        // ID3v2.3: TIT2 "Battle" (ISO-8859-1), TPE1 "Band".
        let mut id3 = b"ID3\x03\x00\x00".to_vec();
        let frames: Vec<u8> = [(&b"TIT2"[..], &b"\x00Battle"[..]), (&b"TPE1"[..], &b"\x00Band"[..])]
            .iter()
            .flat_map(|(id, body)| [id.to_vec(), (body.len() as u32).to_be_bytes().to_vec(), vec![0, 0], body.to_vec()].concat())
            .collect();
        id3.extend([0, 0, 0, frames.len() as u8]);
        id3.extend(frames);
        assert_eq!(tags(&id3), (Some("Battle".into()), Some("Band".into())));
        // Vorbis comments.
        let mut ogg = b"OggS....\x03vorbis".to_vec();
        for c in ["TITLE=Peace", "ARTIST=Someone"] {
            ogg.extend((c.len() as u32).to_le_bytes());
            ogg.extend(c.as_bytes());
        }
        assert_eq!(tags(&ogg), (Some("Peace".into()), Some("Someone".into())));
        assert_eq!(tags(b"\xff\xfb\x90\x00 no tags"), (None, None));
    }
}
