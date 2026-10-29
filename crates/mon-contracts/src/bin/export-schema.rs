//! Export contracts explicitly; normal builds and tests never rewrite sources.
use mon_contracts::*;
use schemars::{schema_for, JsonSchema};
use serde_json::{json, Value};
fn entry<T: JsonSchema>(name: &str, entries: &mut serde_json::Map<String, Value>) {
    entries.insert(name.into(), serde_json::to_value(schema_for!(T)).unwrap());
}
fn main() {
    let mut entries = serde_json::Map::new();
    entry::<WindowEnvironment>("WindowEnvironment", &mut entries);
    entry::<AppError>("AppError", &mut entries);
    entry::<JobKind>("JobKind", &mut entries);
    entry::<JobState>("JobState", &mut entries);
    entry::<PauseReason>("PauseReason", &mut entries);
    entry::<Recipe>("Recipe", &mut entries);
    entry::<DownloadSource>("DownloadSource", &mut entries);
    entry::<ArchiveMember>("ArchiveMember", &mut entries);
    entry::<PackageManifest>("PackageManifest", &mut entries);
    entry::<PackageFile>("PackageFile", &mut entries);
    entry::<Engine>("Engine", &mut entries);
    entry::<Target>("Target", &mut entries);
    entry::<Role>("Role", &mut entries);
    entry::<BackendManifest>("BackendManifest", &mut entries);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"schemaVersion":1,"contracts":entries})).unwrap()
    );
}
