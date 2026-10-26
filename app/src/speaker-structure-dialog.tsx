import { CommonModal } from "./common-modal";
import { splitPosition, textPieces, type Speaker, type Utterance } from "./transcript-state";

export type SpeakerStructureEdit =
  | {kind:"merge";analysisId:string;sourceId:string;targetId:string;choices:Speaker[];affected:number}
  | {kind:"split";analysisId:string;utterance:Utterance;seconds:string;offset:number;first:string;second:string};

export function SpeakerStructureDialog({edit,speakers,disabled,issue,saveFailed,onChange,onSave,onDismiss,restoreTarget,onPlay,playing,onPause,canPlay}:{
  edit:SpeakerStructureEdit|null;speakers:Speaker[];disabled:boolean;issue:string;saveFailed:boolean;
  onChange:(edit:SpeakerStructureEdit)=>void;onSave:(retry?:boolean)=>void;onDismiss:()=>void;
  restoreTarget:()=>HTMLElement|null;onPlay:()=>void;playing:boolean;onPause:()=>void;canPlay:boolean;
}) {
  const split=edit?.kind==="split"?edit:null,merge=edit?.kind==="merge"?edit:null;
  const pieces=split?textPieces(split.utterance.effectiveText):[];
  let validation="";
  if(split)try{splitPosition(split.utterance.startUs,split.utterance.endUs,split.seconds);if(!Number.isInteger(split.offset)||split.offset<1||split.offset>=pieces.length)throw Error("空の部分を作らない本文の切れ目を指定してください。");}catch(c){validation=String(c instanceof Error?c.message:c);}
  if(merge&&(!merge.sourceId||!merge.targetId||merge.sourceId===merge.targetId))validation="統合元と統合先には別の話者を選択してください。";
  const options=(merge?.choices??speakers).map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>);
  return <CommonModal portal nested open={!!edit} blocked={disabled||saveFailed} onDismiss={onDismiss} restoreTarget={restoreTarget} dismissLabel="閉じる" className={`speaker-structure-dialog${split?" split-dialog":""}`} aria-label={split?"発言の手動分割":"話者の統合"}>
    <h2>{split?"発言の手動分割":"話者の統合"}</h2>
    {merge&&<>
      <p>統合元の{merge.affected}発言を統合先の話者にまとめます。本文・時刻・登録した声は変更しません。</p>
      <fieldset disabled={disabled||saveFailed}>
        <label>統合元<select value={merge.sourceId} onChange={e=>onChange({...merge,sourceId:e.target.value})}><option value="">話者を選択</option>{options}</select></label>
        <label>統合先<select value={merge.targetId} onChange={e=>onChange({...merge,targetId:e.target.value})}><option value="">話者を選択</option>{options}</select></label>
      </fieldset>
    </>}
    {split&&<>
      <p>音声を聞き、時刻と本文の切れ目を指定してください。単語時刻の自動推定は行いません。保存後は前半・後半の話者をそれぞれ変更できます。</p>
      <button type="button" className="small-secondary" disabled={disabled||saveFailed||!canPlay} onClick={playing?onPause:onPlay}>{playing?"音声を一時停止":"分割対象の音声を聞く"}</button>
      <fieldset disabled={disabled||saveFailed}>
        <label>分割位置（発言先頭からの秒）<input type="text" inputMode="decimal" value={split.seconds} onChange={e=>onChange({...split,seconds:e.target.value})}/></label>
        <label>本文の切れ目（文字数）<input type="number" min={1} max={pieces.length-1} value={split.offset} onChange={e=>onChange({...split,offset:Number(e.target.value)})}/></label>
        <div className="split-preview"><section><h3>前半</h3><p>{pieces.slice(0,split.offset).join("")}</p></section><section><h3>後半</h3><p>{pieces.slice(split.offset).join("")}</p></section></div>
        <label>前半の話者<select value={split.first} onChange={e=>onChange({...split,first:e.target.value})}><option value="keep">現在の割り当てを維持</option><option value="">話者不明</option>{options}</select></label>
        <label>後半の話者<select value={split.second} onChange={e=>onChange({...split,second:e.target.value})}><option value="keep">現在の割り当てを維持</option><option value="">話者不明</option>{options}</select></label>
      </fieldset>
    </>}
    {validation&&!issue&&<p role="alert">{validation}</p>}
    <div className="transcript-actions"><button type="button" disabled={disabled||saveFailed||!!validation} onClick={()=>onSave()}>{split?"分割して保存":"統合して保存"}</button>{saveFailed&&<button type="button" disabled={disabled} onClick={()=>onSave(true)}>保存を再試行</button>}</div>
  </CommonModal>;
}
