//! Incremental, bounded parser for the CLI's stderr `progress 12.50%` records.
use std::fs::File;
use std::io::Read;

#[derive(Default)]
pub(super) struct LogProgress {
    pending: Vec<u8>,
    oversized: bool,
    latest: u32,
}

impl LogProgress {
    fn feed(&mut self, bytes: &[u8]) -> Option<u32> {
        let before = self.latest;
        for &byte in bytes {
            if byte == b'\n' || byte == b'\r' {
                if !self.oversized {
                    if let Some(value) = parse(&self.pending) {
                        self.latest = self.latest.max(value);
                    }
                }
                self.pending.clear();
                self.oversized = false;
            } else if self.pending.len() < 128 && !self.oversized {
                self.pending.push(byte);
            } else {
                self.pending.clear();
                self.oversized = true;
            }
        }
        (self.latest > before).then_some(self.latest)
    }

    pub(super) fn poll(&mut self, file: &mut File, publish: &impl Fn(u32)) {
        let before = self.latest;
        let mut buffer = [0; 8192];
        // The caller enforces the existing 4 MiB log limit. Bound each poll too.
        let mut remaining = super::MAX_LOG_BYTES as usize;
        while remaining > 0 {
            let limit = remaining.min(buffer.len());
            match file.read(&mut buffer[..limit]) {
                Ok(0) | Err(_) => break, // Missing telemetry never changes inference success.
                Ok(size) => {
                    self.feed(&buffer[..size]);
                    remaining -= size;
                }
            }
        }
        if self.latest > before {
            publish(self.latest);
        }
    }
}

fn parse(line: &[u8]) -> Option<u32> {
    let value = std::str::from_utf8(line)
        .ok()?
        .strip_prefix("progress ")?
        .strip_suffix('%')?;
    let (whole, fraction) = value.split_once('.')?;
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() != 2
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let value = whole
        .parse::<u32>()
        .ok()?
        .checked_mul(100)?
        .checked_add(fraction.parse::<u32>().ok()?)?;
    (value <= 10_000).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_crlf_monotonic_and_invalid_progress_records() {
        let mut parser = LogProgress::default();
        assert_eq!(parser.feed(b"config\nprogress 12."), None);
        assert_eq!(parser.feed(b"50%\r"), Some(1250));
        assert_eq!(parser.feed(b"\nprogress 11.00%\nprogress NaN%\nprogress 101.00%\nprogress -1.00%\nprogress 50.00% garbage\n"), None);
        assert_eq!(parser.feed(&[b'x'; 256]), None);
        assert_eq!(parser.feed(b"progress 90.00%\n"), None);
        assert_eq!(
            parser.feed(b"progress 99.99%\nprogress 100.00%\n"),
            Some(10000)
        );
    }
}
