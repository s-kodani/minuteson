//! Bounded streaming TAR/gzip/bzip2. Only the application opens destinations;
//! archive paths, permissions, links and ownership are never applied to disk.
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub struct Policy {
    pub max_entries: usize,
    pub max_expanded_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_entry_metadata_bytes: u64,
    pub deadline: Duration,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_expanded_bytes: 32 * 1024 * 1024 * 1024,
            max_metadata_bytes: 32 * 1024 * 1024,
            max_entry_metadata_bytes: 65 * 1024,
            deadline: Duration::from_secs(600),
        }
    }
}
pub struct Member<'a> {
    pub path: &'a str,
    pub size: u64,
    pub sha256: &'a str,
}
pub trait Target: Write {
    fn verify(&self) -> Result<(), String>;
    fn finish(self: Box<Self>) -> Result<(), String>;
}

fn normalized(raw: &str, directory: bool) -> Result<String, String> {
    let name = raw.strip_prefix("./").unwrap_or(raw);
    let name = if directory {
        name.trim_end_matches('/')
    } else {
        name
    };
    if directory && matches!(name, "" | ".") {
        return Ok(String::new());
    }
    if name.is_empty()
        || name.len() > 4096
        || name.chars().any(|c| "\\:<>\"|?*".contains(c))
        || name.chars().any(char::is_control)
        || name.split('/').any(|p| {
            p.is_empty() || matches!(p, "." | "..") || p.ends_with([' ', '.']) || reserved(p)
        })
    {
        return Err("アーカイブ内に不正なパスがあります".into());
    }
    Ok(name.to_owned())
}
fn reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}
fn check(cancelled: &dyn Fn() -> bool, started: Instant, deadline: Duration) -> Result<(), String> {
    if cancelled() {
        Err("モデルダウンロードを中断しました".into())
    } else if started.elapsed() >= deadline {
        Err("アーカイブ読取の時間上限を超えました".into())
    } else {
        Ok(())
    }
}
struct Bounded<'a> {
    inner: Box<dyn Read + 'a>,
    cancelled: &'a dyn Fn() -> bool,
    started: Instant,
    deadline: Duration,
    remaining: u64,
}
impl Read for Bounded<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        check(self.cancelled, self.started, self.deadline).map_err(io::Error::other)?;
        // Do not let a malformed extension/header allocate unbounded memory.
        let limit = out
            .len()
            .min(65_536)
            .min(self.remaining.saturating_add(1) as usize);
        let n = self.inner.read(&mut out[..limit])?;
        self.remaining = self
            .remaining
            .checked_sub(n as u64)
            .ok_or_else(|| io::Error::other("アーカイブ本文・メタデータの容量上限を超えました"))?;
        Ok(n)
    }
}
fn decoder<'a>(source: impl Read + 'a) -> Result<Box<dyn Read + 'a>, String> {
    let mut source = BufReader::new(source);
    let prefix = source
        .fill_buf()
        .map_err(|_| "アーカイブを読み取れません")?;
    if prefix.starts_with(&[0x1f, 0x8b]) {
        Ok(Box::new(flate2::read::MultiGzDecoder::new(source)))
    } else if prefix.starts_with(b"BZh") {
        Ok(Box::new(bzip2::read::MultiBzDecoder::new(source)))
    } else {
        Ok(Box::new(source))
    }
}
#[derive(Default)]
struct Extension {
    path: Option<String>,
    link: Option<String>,
    size: Option<u64>,
}
fn pax(bytes: &[u8], extension: &mut Extension, global: bool) -> Result<(), String> {
    let mut rest = bytes;
    let mut seen = BTreeSet::new();
    while !rest.is_empty() {
        let space = rest
            .iter()
            .position(|b| *b == b' ')
            .ok_or("PAX長が不正です")?;
        let length: usize = std::str::from_utf8(&rest[..space])
            .map_err(|_| "PAX長が不正です")?
            .parse()
            .map_err(|_| "PAX長が不正です")?;
        if length <= space + 2 || length > rest.len() || rest[length - 1] != b'\n' {
            return Err("PAXメタデータが不正です".into());
        }
        let pair =
            std::str::from_utf8(&rest[space + 1..length - 1]).map_err(|_| "PAX文字列が不正です")?;
        let (key, value) = pair.split_once('=').ok_or("PAX属性が不正です")?;
        if !seen.insert(key) || key.starts_with("GNU.sparse") {
            return Err("未対応または重複したPAX属性です".into());
        }
        if global && matches!(key, "path" | "linkpath" | "size") {
            return Err("大域PAXでファイルの参照を変更できません".into());
        }
        match key {
            "path" => extension.path = Some(value.into()),
            "linkpath" => extension.link = Some(value.into()),
            "size" => extension.size = Some(value.parse().map_err(|_| "PAX容量が不正です")?),
            _ => (),
        }
        rest = &rest[length..];
    }
    Ok(())
}

pub fn extract(
    source: impl Read,
    selected: &[Member<'_>],
    cancelled: &dyn Fn() -> bool,
    verify: &mut dyn FnMut() -> Result<(), String>,
    open: &mut dyn FnMut(usize) -> Result<Box<dyn Target>, String>,
    policy: Policy,
) -> Result<(), String> {
    let started = Instant::now();
    check(cancelled, started, policy.deadline)?;
    let members = selected
        .iter()
        .enumerate()
        .map(|(i, m)| Ok((normalized(m.path, false)?, i)))
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    if members.len() != selected.len() || selected.is_empty() {
        return Err("展開対象が重複または空です".into());
    }
    let remaining = policy
        .max_expanded_bytes
        .checked_add(policy.max_metadata_bytes)
        .and_then(|n| n.checked_add(policy.max_entries as u64 * 1024 + 1024))
        .ok_or("アーカイブ上限が不正です")?;
    let source = Bounded {
        inner: decoder(source)?,
        cancelled,
        started,
        deadline: policy.deadline,
        remaining,
    };
    let mut archive = tar::Archive::new(source);
    let mut seen = BTreeSet::new();
    let mut extracted = BTreeSet::new();
    let mut expanded = 0u64;
    let mut metadata = 0u64;
    let mut extension = Extension::default();
    let mut pending_extension = false;
    let mut count_entries = 0;
    // Raw mode makes GNU/PAX metadata visible before allocating its contents.
    for entry in archive
        .entries()
        .map_err(|_| "TARヘッダーを確認できません")?
        .raw(true)
    {
        check(cancelled, started, policy.deadline)?;
        verify()?;
        count_entries += 1;
        if count_entries > policy.max_entries {
            return Err("アーカイブ項目数の上限を超えました".into());
        }
        let mut entry = entry.map_err(|_| "アーカイブのヘッダーを確認できません")?;
        let kind = entry.header().entry_type();
        let length = entry.size();
        if kind.is_gnu_longname()
            || kind.is_gnu_longlink()
            || kind.is_pax_local_extensions()
            || kind.is_pax_global_extensions()
        {
            metadata = metadata
                .checked_add(length)
                .filter(|n| *n <= policy.max_metadata_bytes)
                .ok_or("アーカイブメタデータの容量上限を超えました")?;
            if length > policy.max_entry_metadata_bytes {
                return Err("アーカイブ項目のメタデータが大きすぎます".into());
            }
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|_| "アーカイブメタデータを読み取れません")?;
            if bytes.len() as u64 != length {
                return Err("アーカイブメタデータが欠損しています".into());
            }
            if kind.is_gnu_longname() || kind.is_gnu_longlink() {
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| "アーカイブ名が不正です")?
                    .trim_end_matches('\0')
                    .to_owned();
                let slot = if kind.is_gnu_longname() {
                    &mut extension.path
                } else {
                    &mut extension.link
                };
                if slot.replace(text).is_some() {
                    return Err("アーカイブ拡張属性が重複しています".into());
                }
            } else {
                pax(&bytes, &mut extension, kind.is_pax_global_extensions())?;
            }
            if !kind.is_pax_global_extensions() {
                pending_extension = true;
            }
            continue;
        }
        if kind.is_gnu_sparse() {
            return Err("疎なTARファイルは未対応です".into());
        }
        if extension.size.is_some_and(|n| n != length) {
            return Err("TARとPAXの容量が一致しません".into());
        }
        let raw = match extension.path.take() {
            Some(name) => name,
            None => std::str::from_utf8(&entry.path_bytes())
                .map_err(|_| "アーカイブ名が不正です")?
                .to_owned(),
        };
        let name = normalized(&raw, kind.is_dir())?;
        if !seen.insert(name.to_lowercase()) {
            return Err("アーカイブの重複パスを検出しました".into());
        }
        expanded = expanded
            .checked_add(length)
            .filter(|n| *n <= policy.max_expanded_bytes)
            .ok_or("展開容量の上限は32GiBです")?;
        let index = members.get(&name).copied();
        let mut destination = if let Some(index) = index {
            if !kind.is_file() || extension.link.is_some() || length != selected[index].size {
                return Err(
                    "選択したアーカイブ内ファイルが通常ファイル・確認容量と一致しません".into(),
                );
            }
            Some(open(index)?)
        } else {
            None
        };
        extension = Extension::default();
        pending_extension = false;
        let mut size = 0u64;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65_536];
        loop {
            check(cancelled, started, policy.deadline)?;
            verify()?;
            let n = entry
                .read(&mut buffer)
                .map_err(|_| "アーカイブ本文を確認できません")?;
            if n == 0 {
                break;
            }
            size = size
                .checked_add(n as u64)
                .filter(|n| *n <= length)
                .ok_or("アーカイブ本文容量が一致しません")?;
            if let Some(target) = &mut destination {
                target.verify()?;
                target
                    .write_all(&buffer[..n])
                    .map_err(|_| "選択ファイルを書き込めません")?;
                hash.update(&buffer[..n]);
            }
        }
        if size != length {
            return Err("アーカイブ本文が欠損しています".into());
        }
        if let Some(index) = index {
            if format!("{:x}", hash.finalize()) != selected[index].sha256 {
                return Err("展開したファイルの容量・SHAが一致しません".into());
            }
            destination.unwrap().finish()?;
            extracted.insert(index);
        }
    }
    if pending_extension {
        return Err("対象のないTAR拡張属性があります".into());
    }
    // Complete compression checksums and reject another non-padding archive.
    let mut source = archive.into_inner();
    let mut buffer = [0u8; 65_536];
    loop {
        let n = source
            .read(&mut buffer)
            .map_err(|_| "アーカイブ末尾を確認できません")?;
        if n == 0 {
            break;
        }
        if buffer[..n].iter().any(|b| *b != 0) {
            return Err("アーカイブ末尾に未確認のデータがあります".into());
        }
    }
    if extracted.len() != selected.len() {
        return Err("必要なファイルがアーカイブ内にありません".into());
    }
    verify()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Stored {
        bytes: Vec<u8>,
        finished: bool,
    }
    struct Memory(Arc<Mutex<Stored>>);
    impl Write for Memory {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Target for Memory {
        fn verify(&self) -> Result<(), String> {
            Ok(())
        }
        fn finish(self: Box<Self>) -> Result<(), String> {
            self.0.lock().unwrap().finished = true;
            Ok(())
        }
    }
    fn tar(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
        let mut output = tar::Builder::new(Vec::new());
        for (name, bytes, kind) in entries {
            let mut header = tar::Header::new_ustar();
            header.set_mode(0o777);
            header.set_size(bytes.len() as u64);
            header.set_entry_type(tar::EntryType::new(*kind));
            // Keep invalid paths available for malicious archive fixtures.
            header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            if *kind == b'2' {
                header.set_link_name("outside").unwrap();
            }
            header.set_cksum();
            output.append(&header, *bytes).unwrap();
        }
        output.into_inner().unwrap()
    }
    fn run(bytes: &[u8], policy: Policy) -> (Result<(), String>, Arc<Mutex<Stored>>) {
        let stored = Arc::new(Mutex::new(Stored::default()));
        let hash = format!("{:x}", Sha256::digest(b"model"));
        let member = Member {
            path: "pkg/model",
            size: 5,
            sha256: &hash,
        };
        let result = extract(
            bytes,
            &[member],
            &|| false,
            &mut || Ok(()),
            &mut |_| Ok(Box::new(Memory(stored.clone()))),
            policy,
        );
        (result, stored)
    }
    #[test]
    fn plain_gzip_and_bzip2_have_the_same_member_and_budget_contract() {
        let plain = tar(&[
            ("pkg/", b"", b'5'),
            ("pkg/model", b"model", b'0'),
            ("pkg/ignored", b"never run", b'0'),
        ]);
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&plain).unwrap();
        let gzip = gzip.finish().unwrap();
        let mut bzip = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        bzip.write_all(&plain).unwrap();
        let bzip = bzip.finish().unwrap();
        for bytes in [&plain, &gzip, &bzip] {
            let (result, stored) = run(bytes, Policy::default());
            result.unwrap();
            let stored = stored.lock().unwrap();
            assert_eq!(stored.bytes, b"model");
            assert!(stored.finished);
        }
        assert!(run(&gzip[..gzip.len() - 4], Policy::default()).0.is_err());
    }
    #[test]
    fn duplicate_paths_selected_links_and_unknown_bad_paths_are_rejected() {
        for entries in [
            vec![
                ("pkg/model", &b"model"[..], b'0'),
                ("pkg/MODEL", &b"other"[..], b'0'),
            ],
            vec![("pkg/model", &b""[..], b'2')],
            vec![
                ("pkg/model", &b"model"[..], b'0'),
                ("../unknown", &b"ignored"[..], b'0'),
            ],
        ] {
            assert!(run(&tar(&entries), Policy::default()).0.is_err());
        }
        let mut trailing = tar(&[("pkg/model", b"model", b'0')]);
        trailing.extend_from_slice(b"unchecked tail");
        assert!(run(&trailing, Policy::default()).0.is_err());
    }
    #[test]
    fn entire_archive_uses_one_entry_expansion_and_time_budget() {
        let plain = tar(&[
            ("pkg/model", b"model", b'0'),
            ("pkg/ignored", b"large unused", b'0'),
        ]);
        assert!(run(
            &plain,
            Policy {
                max_entries: 1,
                ..Policy::default()
            }
        )
        .0
        .is_err());
        assert!(run(
            &plain,
            Policy {
                max_expanded_bytes: 5,
                ..Policy::default()
            }
        )
        .0
        .is_err());
        assert!(run(
            &plain,
            Policy {
                deadline: Duration::ZERO,
                ..Policy::default()
            }
        )
        .0
        .is_err());
        let hash = format!("{:x}", Sha256::digest(b"model"));
        let member = Member {
            path: "pkg/model",
            size: 5,
            sha256: &hash,
        };
        let mut opened = false;
        assert!(extract(
            &plain[..],
            &[member],
            &|| true,
            &mut || Ok(()),
            &mut |_| {
                opened = true;
                Err("unexpected open".into())
            },
            Policy::default()
        )
        .is_err());
        assert!(!opened);
    }
    #[test]
    fn gnu_names_are_bounded_before_their_contents_are_allocated() {
        let plain = tar(&[
            ("././@LongLink", b"pkg/model\0", b'L'),
            ("placeholder", b"model", b'0'),
        ]);
        run(&plain, Policy::default()).0.unwrap();
        assert!(run(
            &plain,
            Policy {
                max_entry_metadata_bytes: 4,
                ..Policy::default()
            }
        )
        .0
        .is_err());
        let missing = tar(&[("pkg/other", b"model", b'0')]);
        assert!(run(&missing, Policy::default()).0.is_err());
    }
}
