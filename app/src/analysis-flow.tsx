import { flowContinueLabel,flowStages,type AnalysisFlow } from "./analysis-flow-state";
import type { VersionSummary } from "./analysis-state";
export function AnalysisFlowStatus({flow,recorded,disabled,versions,start,resume,review}: {
  flow:AnalysisFlow|null;recorded:boolean;disabled:boolean;versions:VersionSummary[];
  start:()=>void;resume:(flow:AnalysisFlow)=>void;review:(summary:VersionSummary,opener:HTMLElement)=>void;
}) {
  const action=flow&&flowContinueLabel(flow);
  const outputs=flow?[flow.transcriptionJob?.outputAnalysisId,flow.diarizationJob?.outputAnalysisId].filter((id):id is string=>!!id):[];
  return <section className="analysis-flow" aria-label="音声保存後の解析工程">
    <h3>音声保存後の解析工程</h3>
    <p>音声の保存完了後、{flow?.profile==="high_accuracy"?"高精度":"標準"}設定で文字起こしから話者分離へ自動で進みます。人数はトラックごとに自動推定です。モデルと条件を固定し、結果は候補として保存します。採用・議事録作成は内容の確認後に行ってください。</p>
    {!flow ? <><p>{recorded?"この会議の一連の解析はまだ開始していません。個別の解析結果・編集は保持します。":"音声が保存されるまで解析を開始しません。"}</p><button type="button" disabled={disabled||!recorded} onClick={start}>文字起こし・話者分離を順に開始</button></> : <>
      <ol className="analysis-flow-stages">{flowStages(flow).map(s=><li key={s.name}><strong>{s.name}</strong><span>{s.state}</span></li>)}</ol>
      {flow.stage==="configuration"&&<p>音声は保存されています。下の文字起こし・話者分離の{flow.profile==="high_accuracy"?"高精度":"標準"}設定に実行ファイルとモデルを配置し、準備後に開始してください。ダウンロード・手動取り込みの専用画面は未接続です。</p>}
      {flow.stage==="review"&&<p role="status">解析結果を保存しました。本文・話者を確認して採用し、修正した内容から議事録を作成できます。</p>}
      {flow.errorMessage&&<p role="alert" className="action-error">工程を進められません: {flow.errorMessage}</p>}
      {action&&<button type="button" disabled={disabled||!recorded} onClick={()=>resume(flow)}>{action}</button>}
      <div className="analysis-actions">{outputs.map((id,index)=>{
        const summary=versions.find(v=>v.id===id);
        return summary&&summary.publicationState==="candidate"?<button key={id} type="button" disabled={disabled} onClick={e=>review(summary,e.currentTarget)}>{index===0?"成功した文字起こしを確認":"話者分離までの結果を確認"}</button>:null;
      })}</div>
      {(flow.transcriptionJob?.state==="failed"||flow.diarizationJob?.state==="failed")&&<p>失敗した工程だけを下のジョブ操作から同じ条件で再試行できます。成功済みの文字起こしはやり直しません。</p>}
    </>}
  </section>;
}
