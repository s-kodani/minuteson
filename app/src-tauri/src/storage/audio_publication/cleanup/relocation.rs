//! Local receipt aliases travel with the existing, admitted native handoff.
//! Hash equality alone never authorizes adopting a replacement file.
use super::*;
use crate::bootstrap::placement::{NativeHistoryAdmission, Placement};
use crate::storage::backup::relocation::NativeSnapshot;

const MAX_FILES: usize = 32_768;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::storage) struct Claim {
    meeting: String,
    path: String,
    bytes: u64,
    hash: String,
    recorded: Identity,
    observed: Identity,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::storage) struct Bound {
    identity: Identity,
}

pub(in crate::storage) type Aliases = BTreeMap<(String, String), (Claim, Identity)>;

pub(in crate::storage) fn validate(claims: &[Claim]) -> Result<(), String> {
    if claims.len() > MAX_FILES {
        return Err("音声整理の移動対応が安全上限を超えています".into());
    }
    let mut paths = BTreeSet::new();
    for c in claims {
        if !crate::input_levels::valid_owner(&c.meeting)
            || !Path::new(&c.path).starts_with(Path::new("meetings").join(&c.meeting))
            || Path::new(&c.path)
                .components()
                .any(|p| !matches!(p, std::path::Component::Normal(_)))
            || c.path.contains(['\\', '\0'])
            || c.path.len() > 1024
            || c.hash.len() != 64
            || !c
                .hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !paths.insert(&c.path)
        {
            return Err("音声整理の移動元記録が不正です".into());
        }
        c.recorded.validate(false)?;
        c.observed.validate(false)?;
    }
    Ok(())
}

pub(in crate::storage) fn capture(
    storage: &Storage,
    snapshot: &NativeSnapshot<'_>,
    db: &Connection,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<Claim>, String> {
    // Handoff verification repeats after the source write fence. This census
    // only reads the held snapshot and must not require source write access.
    storage.managed_anchor.verify()?;
    let mut q = db.prepare("SELECT meeting_id,cleanup_json FROM meeting_audio_commits WHERE phase='committed' ORDER BY meeting_id").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let native: BTreeMap<_, _> = snapshot
        .manifest()
        .entries
        .iter()
        .map(|n| (n.relative_path.as_str(), n))
        .collect();
    let mut result = Vec::new();
    for row in rows {
        let (meeting, json) = row.map_err(|e| e.to_string())?;
        let files: Vec<RetiredAudio> = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        for file in files {
            if cancelled() {
                return Err("音声整理の移動対応の確認を中断しました".into());
            }
            let Some(recorded) = file.identity.as_ref() else {
                continue;
            };
            if !Path::new(&file.relative).starts_with(Path::new("meetings").join(&meeting)) {
                continue;
            }
            let Some(node) = native.get(file.relative.as_str()) else {
                continue;
            };
            if node.directory
                || node.size != file.bytes
                || node.hash.as_deref() != Some(file.hash.as_str())
            {
                return Err("音声整理の台帳と固定した移動像が一致しません".into());
            }
            let observed = Identity::read(
                &snapshot
                    .source_metadata(&file.relative)?
                    .ok_or("音声整理の移動元実体がありません")?,
            )?;
            let mut effective = Some(recorded.clone());
            crate::storage::recordings::flac::relocation::effective_cleanup_identity(
                storage,
                &meeting,
                &file,
                &mut effective,
            )?;
            if effective.as_ref() != Some(&observed) {
                return Err("音声整理の移動元実体が差し替えられています".into());
            }
            result.push(Claim {
                meeting: meeting.clone(),
                path: file.relative,
                bytes: file.bytes,
                hash: file.hash,
                recorded: recorded.clone(),
                observed,
            });
        }
    }
    validate(&result)?;
    Ok(result)
}

pub(in crate::storage) fn bind(
    claims: &[Claim],
    placement: &Placement<'_>,
) -> Result<Vec<Bound>, String> {
    validate(claims)?;
    claims
        .iter()
        .map(|c| {
            let identity = Identity::read(
                &placement
                    .owned_path(&c.path)?
                    .ok_or("音声整理のコピー先所有記録がありません")?,
            )?;
            identity.validate(false)?;
            Ok(Bound { identity })
        })
        .collect()
}

pub(in crate::storage) fn admit(
    admission: &NativeHistoryAdmission<'_, '_>,
    claims: Vec<Claim>,
    bound: &[Bound],
) -> Result<Aliases, String> {
    validate(&claims)?;
    if claims.len() != bound.len() {
        return Err("音声整理のコピー先対応件数が不正です".into());
    }
    let mut aliases = BTreeMap::new();
    for (claim, target) in claims.into_iter().zip(bound) {
        target.identity.validate(false)?;
        if target.identity.registered()? != admission.owned_path(&claim.path, false)?
            || admission.copied_file_hash(&claim.path)? != claim.hash
        {
            return Err("音声整理の移動先は承認したコピーではありません".into());
        }
        aliases.insert(
            (claim.meeting.clone(), claim.path.clone()),
            (claim, target.identity.clone()),
        );
    }
    Ok(aliases)
}

pub(in crate::storage) fn effective(
    aliases: &Aliases,
    meeting: &str,
    file: &RetiredAudio,
    identity: &mut Option<Identity>,
) -> Result<(), String> {
    let Some((claim, target)) = aliases.get(&(meeting.into(), file.relative.clone())) else {
        return Ok(());
    };
    if file.bytes != claim.bytes
        || file.hash != claim.hash
        || file.identity.as_ref() != Some(&claim.recorded)
    {
        return Err("音声整理の台帳が承認した移動元と一致しません".into());
    }
    *identity = Some(target.clone());
    Ok(())
}
