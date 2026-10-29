use super::request::valid_uid;
use serde_json::Value;
use std::collections::BTreeSet;

/// Return indices into native metadata, preserving OS-only fields such as
/// AudioObjectID. Stable Unicode ordering is independent of the host locale.
pub fn order(
    bytes: &[u8],
    applications: bool,
    own: u32,
    parent: u32,
) -> Result<Vec<u32>, &'static str> {
    if bytes.len() > 512 * 1024 {
        return Err("capture catalog metadata exceeds capacity");
    }
    let rows: Vec<Value> = serde_json::from_slice(bytes).map_err(|_| "invalid capture catalog")?;
    if rows.len() > 128 {
        return Err("capture catalog exceeds capacity");
    }
    let mut seen = BTreeSet::new();
    let mut indices = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let name = row["name"]
            .as_str()
            .filter(|n| valid_uid(n))
            .ok_or("invalid capture name")?;
        let identity = if applications {
            let pid = row["pid"]
                .as_u64()
                .filter(|p| *p > 0 && *p <= i32::MAX as u64)
                .ok_or("invalid capture PID")?;
            if pid == u64::from(own) || pid == u64::from(parent) {
                continue;
            }
            row["bundleIdentifier"]
                .as_str()
                .filter(|n| valid_uid(n))
                .ok_or("invalid application identity")?;
            format!("{pid:010}")
        } else {
            row["uid"]
                .as_str()
                .filter(|n| valid_uid(n))
                .ok_or("invalid microphone UID")?
                .to_owned()
        };
        if !seen.insert(identity.clone()) {
            return Err("ambiguous capture identities");
        }
        indices.push((name.to_lowercase(), identity, index as u32));
    }
    indices.sort();
    Ok(indices.into_iter().map(|(_, _, index)| index).collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_bounded_sorted_and_excludes_owned_processes() {
        let devices = serde_json::json!([{"uid":"b","name":"ZOOM"},{"uid":"a","name":"alpha"}]);
        assert_eq!(
            order(&serde_json::to_vec(&devices).unwrap(), false, 0, 0).unwrap(),
            vec![1, 0]
        );
        let apps = serde_json::json!([{"pid":7,"name":"mon","bundleIdentifier":"self"},{"pid":8,"name":"parent","bundleIdentifier":"parent"},{"pid":9,"name":"app","bundleIdentifier":"app"}]);
        assert_eq!(
            order(&serde_json::to_vec(&apps).unwrap(), true, 7, 8).unwrap(),
            vec![2]
        );
        let overflow = vec![serde_json::json!({"uid":"x","name":"x"}); 129];
        assert!(order(&serde_json::to_vec(&overflow).unwrap(), false, 0, 0).is_err());
        let duplicate = vec![serde_json::json!({"uid":"x","name":"x"}); 2];
        assert!(order(&serde_json::to_vec(&duplicate).unwrap(), false, 0, 0).is_err());
    }
}
