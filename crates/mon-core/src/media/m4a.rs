//! Bounded ISO BMFF preflight shared by AudioToolbox and Media Foundation.
//! Payloads are skipped; the decoder receives only one audio track with valid
//! optional text chapter references. No filesystem path is opened here.
use std::collections::BTreeSet;

#[derive(Default)]
struct Track {
    id: Option<u64>,
    handler: Option<u64>,
    media: bool,
    references: bool,
    chapters: Vec<u64>,
}
const INVALID: &str = "invalid M4A audio/chapter track structure";
struct Reader<F> {
    read: F,
    boxes: usize,
    moov: usize,
    tracks: Vec<Track>,
}
impl<F: FnMut(u64, &mut [u8]) -> Result<(), &'static str>> Reader<F> {
    fn uint(&mut self, offset: u64, count: usize) -> Result<u64, &'static str> {
        let mut bytes = [0u8; 8];
        (self.read)(offset, &mut bytes[..count])?;
        Ok(bytes[..count]
            .iter()
            .fold(0, |n, b| (n << 8) | u64::from(*b)))
    }
    fn walk(&mut self, mut at: u64, end: u64, depth: u8, index: usize) -> Result<(), &'static str> {
        while at < end {
            self.boxes += 1;
            if self.boxes > 4096 || end - at < 8 {
                return Err(INVALID);
            }
            let mut length = self.uint(at, 4)?;
            let kind = self.uint(at + 4, 4)?;
            let mut header = 8;
            if length == 1 {
                if end - at < 16 {
                    return Err(INVALID);
                }
                length = self.uint(at + 8, 8)?;
                header = 16;
            } else if length == 0 {
                length = end - at;
            }
            if length < header || length > end - at {
                return Err(INVALID);
            }
            let body = at + header;
            let count = length - header;
            match (depth, kind) {
                (0, 0x6d6f6f76) => {
                    self.moov += 1;
                    self.walk(body, at + length, 1, 0)?;
                }
                (1, 0x7472616b) => {
                    if self.tracks.len() >= 128 {
                        return Err(INVALID);
                    }
                    let i = self.tracks.len();
                    self.tracks.push(Track::default());
                    self.walk(body, at + length, 2, i)?;
                }
                (2, 0x746b6864) => {
                    if self.tracks[index].id.is_some() || count < 4 {
                        return Err(INVALID);
                    }
                    let version = self.uint(body, 1)?;
                    let offset = if version == 0 { 12 } else { 20 };
                    if version > 1 || count < offset + 4 {
                        return Err(INVALID);
                    }
                    let id = self.uint(body + offset, 4)?;
                    if id == 0 {
                        return Err(INVALID);
                    }
                    self.tracks[index].id = Some(id);
                }
                (2, 0x6d646961) => {
                    if self.tracks[index].media {
                        return Err(INVALID);
                    }
                    self.tracks[index].media = true;
                    self.walk(body, at + length, 3, index)?;
                }
                (3, 0x68646c72) => {
                    if count < 12 || self.tracks[index].handler.is_some() {
                        return Err(INVALID);
                    }
                    self.tracks[index].handler = Some(self.uint(body + 8, 4)?);
                }
                (2, 0x74726566) => {
                    if self.tracks[index].references {
                        return Err(INVALID);
                    }
                    self.tracks[index].references = true;
                    self.walk(body, at + length, 4, index)?;
                }
                (4, _) => {
                    if kind != 0x63686170
                        || !self.tracks[index].chapters.is_empty()
                        || count == 0
                        || count % 4 != 0
                        || count > 512
                    {
                        return Err(INVALID);
                    }
                    for offset in (0..count).step_by(4) {
                        let id = self.uint(body + offset, 4)?;
                        self.tracks[index].chapters.push(id);
                    }
                }
                _ => {}
            }
            at += length;
        }
        Ok(())
    }
}
pub fn validate(
    length: u64,
    read: impl FnMut(u64, &mut [u8]) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut reader = Reader {
        read,
        boxes: 0,
        moov: 0,
        tracks: Vec::new(),
    };
    reader.walk(0, length, 0, 0)?;
    let mut ids = BTreeSet::new();
    let mut text = BTreeSet::new();
    let mut audio = None;
    for track in &reader.tracks {
        let id = track.id.ok_or(INVALID)?;
        if !ids.insert(id) {
            return Err(INVALID);
        }
        match track.handler {
            Some(0x736f756e) => {
                if audio.replace(track).is_some() {
                    return Err(INVALID);
                }
            }
            Some(0x74657874) => {
                if track.references {
                    return Err(INVALID);
                }
                text.insert(id);
            }
            _ => return Err(INVALID),
        }
    }
    let audio = audio.ok_or(INVALID)?;
    let references: BTreeSet<_> = audio.chapters.iter().copied().collect();
    if reader.moov != 1 || references.len() != audio.chapters.len() || references != text {
        return Err(INVALID);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut data = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        data.extend(kind);
        data.extend(body);
        data
    }
    fn track(id: u32, kind: &[u8; 4]) -> Vec<u8> {
        let mut tkhd = vec![0u8; 12];
        tkhd.extend(id.to_be_bytes());
        let mut handler = vec![0u8; 8];
        handler.extend(kind);
        let mut body = atom(b"tkhd", &tkhd);
        body.extend(atom(b"mdia", &atom(b"hdlr", &handler)));
        atom(b"trak", &body)
    }
    fn check(bytes: &[u8]) -> bool {
        validate(bytes.len() as u64, |offset, out| {
            let source = bytes
                .get(offset as usize..offset as usize + out.len())
                .ok_or(INVALID)?;
            out.copy_from_slice(source);
            Ok(())
        })
        .is_ok()
    }
    #[test]
    fn audio_chapter_structure_is_not_inferred_from_extension() {
        assert!(check(&atom(b"moov", &track(1, b"soun"))));
        assert!(!check(&atom(b"moov", &track(0, b"soun"))));
        assert!(!check(&atom(b"moov", &track(1, b"vide"))));
        let mut duplicate = track(1, b"soun");
        duplicate.extend(track(2, b"soun"));
        assert!(!check(&atom(b"moov", &duplicate)));
        assert!(!check(&[0, 0, 0, 64, b'm', b'o', b'o', b'v']));
    }
}
