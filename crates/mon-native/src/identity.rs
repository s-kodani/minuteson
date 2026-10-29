//! Portable shape of local deletion proofs. These values are never standalone
//! authority: OS adapters must compare them to still-owned native handles.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Identity {
    pub device: u64,
    pub inode: u64,
    pub birth_seconds: u64,
    pub birth_nanos: u32,
    pub mode: u32,
}
impl Identity {
    pub fn registered(&self) -> Result<(u64, u64, u64), String> {
        Ok((
            self.device,
            self.inode,
            self.birth_seconds
                .checked_mul(1_000_000_000)
                .and_then(|n| n.checked_add(self.birth_nanos.into()))
                .ok_or("実体の時刻が範囲外です")?,
        ))
    }
    pub fn validate(&self, directory: bool) -> Result<(), String> {
        if self.birth_nanos >= 1_000_000_000
            || self.mode & 0o170000 != if directory { 0o040000 } else { 0o100000 }
        {
            return Err("管理対象の実体記録の形式が不正です".into());
        }
        Ok(())
    }
    pub fn same_persistent(&self, other: &Self) -> bool {
        self.inode == other.inode
            && self.birth_seconds == other.birth_seconds
            && self.birth_nanos == other.birth_nanos
            && self.mode == other.mode
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Proof {
    pub identity: Identity,
    pub size: u64,
    pub links: u64,
    pub modified_seconds: i64,
    pub modified_nanos: i64,
    pub change_seconds: i64,
    pub change_nanos: i64,
}
impl Proof {
    pub fn directory_scope(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            size: 0,
            links: 1,
            modified_seconds: 0,
            modified_nanos: 0,
            change_seconds: 0,
            change_nanos: 0,
        }
    }
    pub fn matches_after_rename(&self, original: &Self) -> bool {
        self.identity == original.identity
            && self.size == original.size
            && self.links == original.links
            && self.modified_seconds == original.modified_seconds
            && self.modified_nanos == original.modified_nanos
    }
    pub fn validate(&self, directory: bool) -> Result<(), String> {
        self.identity.validate(directory)?;
        if self.links == 0
            || (!directory && self.links != 1)
            || self.size > i64::MAX as u64
            || !(0..1_000_000_000).contains(&self.modified_nanos)
            || !(0..1_000_000_000).contains(&self.change_nanos)
        {
            return Err("管理対象の変更・容量・リンク記録が不正です".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn proof() -> Proof {
        Proof {
            identity: Identity {
                device: 1,
                inode: 2,
                birth_seconds: 3,
                birth_nanos: 4,
                mode: 0o100600,
            },
            size: 5,
            links: 1,
            modified_seconds: 6,
            modified_nanos: 7,
            change_seconds: 8,
            change_nanos: 9,
        }
    }
    #[test]
    fn proof_shape_and_rename_contract_remain_compatible() {
        let expected = r#"{"identity":{"device":1,"inode":2,"birthSeconds":3,"birthNanos":4,"mode":33152},"size":5,"links":1,"modifiedSeconds":6,"modifiedNanos":7,"changeSeconds":8,"changeNanos":9}"#;
        let original = proof();
        assert_eq!(serde_json::to_string(&original).unwrap(), expected);
        assert_eq!(serde_json::from_str::<Proof>(expected).unwrap(), original);
        let mut renamed = original.clone();
        renamed.change_nanos += 1;
        assert!(renamed.matches_after_rename(&original));
        renamed.identity.inode += 1;
        assert!(!renamed.matches_after_rename(&original));
    }
    #[test]
    fn malformed_or_shared_file_proofs_are_rejected() {
        let mut p = proof();
        p.validate(false).unwrap();
        p.links = 2;
        assert!(p.validate(false).is_err());
        let mut p = proof();
        p.modified_nanos = 1_000_000_000;
        assert!(p.validate(false).is_err());
        assert!(proof().validate(true).is_err());
        let directory = Proof {
            identity: Identity {
                mode: 0o040700,
                ..proof().identity
            },
            ..proof()
        };
        directory.directory_scope().validate(true).unwrap();
    }
}
