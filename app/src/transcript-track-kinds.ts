// Match native track IDs, never infer a kind from a label, speaker or ID prefix.
export function transcriptTrackKinds(value: unknown): ReadonlyMap<string,string> {
  const tracks = value && typeof value === "object" && "tracks" in value ? value.tracks : null;
  if(!Array.isArray(tracks)||!tracks.length||tracks.length>1024)throw Error("音声トラックの対応を確認できません");
  const result = new Map<string,string>();
  for(const track of tracks){
    if(!track||typeof track.id!=="string"||!track.id.length||track.id.length>256||result.has(track.id)
      ||!["mic","system","imported","processed"].includes(track.kind))throw Error("音声トラックの対応が不正です");
    result.set(track.id,track.kind);
  }
  return result;
}
