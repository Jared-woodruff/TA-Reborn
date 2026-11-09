//! An MP4's index moved before its pictures ("fast start", as qt-faststart does it): Media
//! Foundation writes the `moov` box, which says where every picture and sound is, after the
//! `mdat` box that holds them, so a player must have the whole file before it can begin. Here
//! `moov` is moved in front of `mdat`, and every chunk offset in it (`stco`, `co64`) moved on by
//! its length.

/// A top-level box: its kind, where it starts and how long it is (its header too).
struct Box4 {
    kind: [u8; 4],
    start: usize,
    len: usize,
}

/// The boxes laid one after another in `bytes` (`None` if they do not fill it exactly).
fn boxes(bytes: &[u8]) -> Option<Vec<Box4>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let size = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as u64;
        let kind: [u8; 4] = bytes.get(at + 4..at + 8)?.try_into().ok()?;
        let len = match size {
            0 => (bytes.len() - at) as u64,
            1 => u64::from_be_bytes(bytes.get(at + 8..at + 16)?.try_into().ok()?),
            n => n,
        };
        let len = usize::try_from(len).ok().filter(|&l| l >= 8 && at + l <= bytes.len())?;
        out.push(Box4 { kind, start: at, len });
        at += len;
    }
    Some(out)
}

/// Every chunk offset in the boxes of `bytes` (inside a `moov`, without its header) moved on by
/// `shift`; `None` if one would no longer fit its 32 bits.
fn shift_offsets(bytes: &mut [u8], shift: u64) -> Option<()> {
    for b in boxes(bytes)? {
        let body = b.start + if u32::from_be_bytes(bytes[b.start..b.start + 4].try_into().ok()?) == 1 { 16 } else { 8 };
        let end = b.start + b.len;
        match &b.kind {
            b"trak" | b"mdia" | b"minf" | b"stbl" => shift_offsets(&mut bytes[body..end], shift)?,
            kind @ (b"stco" | b"co64") => {
                let wide = kind == b"co64";
                let count = u32::from_be_bytes(bytes.get(body + 4..body + 8)?.try_into().ok()?) as usize;
                let size = if wide { 8 } else { 4 };
                let entries = bytes.get_mut(body + 8..body + 8 + count * size)?;
                for entry in entries.chunks_exact_mut(size) {
                    if wide {
                        let v = u64::from_be_bytes(entry.try_into().ok()?).checked_add(shift)?;
                        entry.copy_from_slice(&v.to_be_bytes());
                    } else {
                        let v = u32::try_from(u32::from_be_bytes(entry.try_into().ok()?) as u64 + shift).ok()?;
                        entry.copy_from_slice(&v.to_be_bytes());
                    }
                }
            }
            _ => {}
        }
    }
    Some(())
}

/// `bytes` (a whole MP4 file) with its `moov` before its `mdat`; `None` when it is so already, or
/// cannot be (not an MP4 laid out as expected, offsets too large).
pub fn fast_start(bytes: &[u8]) -> Option<Vec<u8>> {
    let boxes = boxes(bytes)?;
    let moov = boxes.iter().position(|b| &b.kind == b"moov")?;
    let mdat = boxes.iter().position(|b| &b.kind == b"mdat")?;
    if moov < mdat {
        return None;
    }
    let m = &boxes[moov];
    let mut index = bytes[m.start..m.start + m.len].to_vec();
    let header = if u32::from_be_bytes(index[0..4].try_into().ok()?) == 1 { 16 } else { 8 };
    shift_offsets(&mut index[header..], m.len as u64)?;
    let mut out = Vec::with_capacity(bytes.len());
    for (i, b) in boxes.iter().enumerate() {
        if i == mdat {
            out.extend_from_slice(&index);
        }
        if i != moov {
            out.extend_from_slice(&bytes[b.start..b.start + b.len]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    /// An `stco` box with these offsets.
    fn stco(offsets: &[u32]) -> Vec<u8> {
        let mut body = vec![0, 0, 0, 0];
        body.extend_from_slice(&(offsets.len() as u32).to_be_bytes());
        for o in offsets {
            body.extend_from_slice(&o.to_be_bytes());
        }
        boxed(b"stco", &body)
    }

    #[test]
    fn the_index_moves_first_and_its_offsets_follow_the_pictures() {
        let ftyp = boxed(b"ftyp", b"isom0000");
        let pictures: Vec<u8> = (0..64u8).collect();
        let mdat = boxed(b"mdat", &pictures);
        // Two chunks: at the start of mdat's data, and 32 bytes in.
        let first = (ftyp.len() + 8) as u32;
        let moov = boxed(b"moov", &boxed(b"trak", &boxed(b"mdia", &boxed(b"minf", &boxed(b"stbl", &stco(&[first, first + 32]))))));
        let file = [ftyp.clone(), mdat.clone(), moov.clone()].concat();
        let moved = fast_start(&file).expect("moov after mdat is moved");
        assert_eq!(moved.len(), file.len());
        assert_eq!(&moved[..ftyp.len()], &ftyp[..]);
        assert_eq!(&moved[ftyp.len() + 4..ftyp.len() + 8], b"moov");
        // The offsets now point at the same bytes as before.
        let entry = |at: usize| u32::from_be_bytes(moved[at..at + 4].try_into().unwrap()) as usize;
        let stco_at = moved.windows(4).position(|w| w == b"stco").unwrap() + 4;
        let (a, b) = (entry(stco_at + 8), entry(stco_at + 12));
        assert_eq!((moved[a], moved[b]), (0, 32));
        // A file laid out so already is left as it is.
        assert!(fast_start(&moved).is_none());
        assert!(fast_start(b"not an mp4 at all").is_none());
    }

    #[test]
    fn wide_offsets_are_moved_too() {
        let ftyp = boxed(b"ftyp", b"isom0000");
        let mdat = boxed(b"mdat", &[7u8; 40]);
        let first = (ftyp.len() + 8) as u64;
        let mut body = vec![0, 0, 0, 0];
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&first.to_be_bytes());
        let moov = boxed(b"moov", &boxed(b"trak", &boxed(b"mdia", &boxed(b"minf", &boxed(b"stbl", &boxed(b"co64", &body))))));
        let moved = fast_start(&[ftyp, mdat, moov.clone()].concat()).unwrap();
        let at = moved.windows(4).position(|w| w == b"co64").unwrap() + 4 + 8;
        let offset = u64::from_be_bytes(moved[at..at + 8].try_into().unwrap());
        assert_eq!(offset, first + moov.len() as u64);
        assert_eq!(moved[offset as usize], 7);
    }
}
