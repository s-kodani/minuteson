use super::*;
use crate::storage::transcription::known::KnownInput;
use sha2::{Digest, Sha256};

pub(crate) fn split_fixture(root: &Path) -> (Storage, String, AsrConfig) {
    let (storage, meeting, config) = pending_fixture(root, 10);
    let path = storage
        .root
        .join("meetings")
        .join(&meeting)
        .join("audio/mic.caf");
    let pcm: Vec<u8> = (0..160000)
        .flat_map(|index| if index < 80000 { 0.25_f32 } else { -0.5_f32 }.to_le_bytes())
        .collect();
    let caf = caf_fixture(16000, 1, 3, 32, 4, &pcm);
    std::fs::write(path, &caf).unwrap();
    let db = storage.connection.lock().unwrap();
    db.execute("UPDATE file_assets SET size_bytes=?1,content_hash=?2 WHERE id=(SELECT file_asset_id FROM audio_chunks WHERE id='chunk')",params![caf.len() as i64,format!("{:x}",Sha256::digest(&caf))]).unwrap();
    db.execute_batch("UPDATE audio_chunks SET end_us=20000000; UPDATE meetings SET duration_us=20000000; UPDATE recording_sessions SET duration_us=20000000;").unwrap();
    for (id, first, end, start, last) in [
        ("left", 0, 80000, 0, 5000000),
        ("right", 80000, 160000, 15000000, 20000000),
    ] {
        db.execute("INSERT INTO track_clock_segments(id,track_id,chunk_id,source_frame_start,source_frame_end,timeline_start_us,timeline_end_us,mapping_json) VALUES (?1,'track','chunk',?2,?3,?4,?5,'{\"schema_version\":1,\"method\":\"linear\"}')",params![id,first,end,start,last]).unwrap();
    }
    db.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES ('gap','track',5000000,15000000,'capture_timestamp_gap',0)",[]).unwrap();
    drop(db);
    (storage, meeting, config)
}
