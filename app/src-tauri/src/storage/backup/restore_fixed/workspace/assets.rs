use super::*;
use crate::saved_artifact::{RestoreAssets as Assets, Role};
use std::collections::BTreeMap;

pub(super) struct WorkspaceAssets<'a, 'p, 's, C> {
    pub(super) fixed: &'a FixedData<'p, 's>,
    pub(super) database: &'a Connection,
    pub(super) root: &'a Path,
    pub(super) meeting: &'a str,
    pub(super) entries: &'a BTreeMap<&'a str, &'a archive::Entry>,
    pub(super) cancelled: &'a C,
}
impl<C: Fn() -> bool> WorkspaceAssets<'_, '_, '_, C> {
    fn entry(&self, role: Role, id: &str, hash: &str) -> Result<&archive::Entry, String> {
        check_cancelled(self.cancelled)?;
        if matches!(role, Role::TemporaryPcm | Role::DiarizationLog) {
            return Err("除外されたPCM・ログ本体は復元しません".into());
        }
        self.fixed.ids.get("file_assets", self.meeting, id)?;
        let (media,size,old,state):(String,i64,String,String)=self.fixed.ids.source().database.query_row(
            "SELECT media_type,size_bytes,content_hash,state FROM file_assets WHERE id=?1 AND meeting_id=?2",
            params![id,self.meeting],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).map_err(|e|e.to_string())?;
        let entry = self
            .entries
            .get(id)
            .ok_or("途中成果物がアーカイブにありません")?;
        if media != role.media()
            || state != "ready"
            || old != hash
            || size < 0
            || entry.meeting_id.as_deref() != Some(self.meeting)
            || entry.source_sha256.as_deref() != Some(hash)
            || entry.source_size != Some(size as u64)
            || entry.size > role.max_bytes()
        {
            return Err("途中成果物の会議・種別・元SHA・容量が一致しません".into());
        }
        Ok(entry)
    }

    fn destination(&self, id: &str) -> Result<PathBuf, String> {
        let new = self.fixed.ids.get("file_assets", self.meeting, id)?;
        let meeting = self.fixed.ids.get("meetings", "", self.meeting)?;
        let relative = format!("meetings/{meeting}/restored/{new}");
        let stored: String = self
            .database
            .query_row(
                "SELECT relative_path FROM file_assets WHERE id=?1 AND meeting_id=?2",
                params![new, meeting],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if stored != relative {
            return Err("途中成果物の管理保存先が一致しません".into());
        }
        // Walk only minted components. Never create children through an existing
        // symlink, even before the final path validation or exclusive file open.
        let mut directory = self.root.to_path_buf();
        for part in ["meetings", meeting.as_str(), "restored"] {
            directory.push(part);
            scratch::directories(&directory)?;
            if !std::fs::symlink_metadata(&directory)
                .map_err(|e| e.to_string())?
                .is_dir()
                || directory.canonicalize().map_err(|e| e.to_string())? != directory
            {
                return Err("途中成果物の保存先フォルダーが変更されています".into());
            }
        }
        Ok(directory.join(new))
    }
}
impl<C: Fn() -> bool> Assets for WorkspaceAssets<'_, '_, '_, C> {
    fn read(&mut self, role: Role, id: &str, hash: &str) -> Result<Vec<u8>, String> {
        let entry = self.entry(role, id, hash)?;
        let path = self.fixed.ids.source().owned_asset_path(id)?;
        let (mut file, before) = open_source(&path)?;
        if before.len() != entry.size {
            return Err("途中成果物の物理容量が変更されています".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(entry.size + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        check_cancelled(self.cancelled)?;
        if bytes.len() as u64 != entry.size
            || format!("{:x}", Sha256::digest(&bytes)) != entry.sha256
            || !same_file(&before, &file.metadata().map_err(|e| e.to_string())?)
            || !same_file(
                &before,
                &std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?,
            )
            || path.canonicalize().map_err(|e| e.to_string())? != path
        {
            return Err("検査済み途中成果物の内容・パスが変更されています".into());
        }
        Ok(bytes)
    }
    fn write(
        &mut self,
        role: Role,
        id: &str,
        hash: &str,
        bytes: &[u8],
    ) -> Result<(String, String), String> {
        self.entry(role, id, hash)?;
        if bytes.len() as u64 > role.max_bytes() {
            return Err("再構成した途中成果物が容量上限を超えています".into());
        }
        let destination = self.destination(id)?;
        let expected = (bytes.len() as u64, format!("{:x}", Sha256::digest(bytes)));
        match private_file(&destination) {
            Ok(mut file) => {
                file.write_all(bytes).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
            }
            Err(e) => {
                // Historical checkpoints and retries can legitimately share the
                // same artifact. An existing different file is never replaced.
                if !destination.exists()
                    || digest_file(&destination, self.cancelled)? != expected
                    || destination.canonicalize().map_err(|e| e.to_string())? != destination
                {
                    return Err(format!("同じ途中成果物の保存内容が競合しています: {e}"));
                }
            }
        }
        check_cancelled(self.cancelled)?;
        let (size, new_hash) = self.fixed.asset_bytes(self.meeting, id, bytes)?;
        let new_id = self.fixed.ids.get("file_assets", self.meeting, id)?;
        if self.database.execute("UPDATE file_assets SET size_bytes=?1,content_hash=?2 WHERE id=?3 AND meeting_id=?4",params![size as i64,new_hash,new_id,self.fixed.ids.get("meetings","",self.meeting)?]).map_err(|e|e.to_string())? != 1 {
            return Err("再構成した途中成果物の登録先がありません".into());
        }
        Ok((new_id, new_hash))
    }
}
