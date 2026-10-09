import {
  canRecoverPartialRecording, partialLaneLabel, partialLaneStateLabel,
  partialTimelineLabel, type PartialRecordingSummary as Summary,
} from "./partial-recording-state";

// Presentation only. The confirmation modal/controller must retain its native
// capability and resolve an uncertain command result before permitting retry.
export function PartialRecordingSummary({ summary, saved = false }: { summary: Summary; saved?: boolean }) {
  return <section aria-label="録音の部分救出範囲">
    <h3>{saved ? "部分救出として登録された確定範囲" : "確定済み音声の部分救出"}</h3>
    <p>{saved ? "保存時に検証・登録した範囲の記録です。元録音を正常終了とは扱いません。録音・解析を自動再開しません。" : "元の録音を正常終了として扱わず、検証できた確定範囲だけを救出します。録音は再開しません。"}</p>
    <p>実際の取得終了・UTC原点・末尾の損失時間は不明です。末尾損失5秒以内は保証しません。</p>
    <p>{saved ? "現在の音声本体の存在・再生可能性は別確認です。復元済み・音声削除済みの会議にも保存時の記録を表示します。" : "未確定・破損・検証できない音声と、元の記録は保持します。"}</p>
    {canRecoverPartialRecording(summary)
      ? <p>{saved ? "保存時に登録した時間軸の範囲" : "救出する時間軸の範囲"}: 0〜{partialTimelineLabel(summary.durationUs)}。この範囲にも欠損が含まれる場合があります。</p>
      : <p role="status">現在、救出できる確定済み音声はありません。</p>}
    <ul>{summary.lanes.map(lane => <li key={lane.lane}>
      <h4>{partialLaneLabel(lane.lane)}</h4>
      <p>{saved ? ({ available: "保存時に検証・登録済み", empty: "救出時に確定済み音声なし", missing: "救出時に確定範囲の記録なし", unreadable: "救出時に検証できず対象外" })[lane.state] : partialLaneStateLabel(lane.state)}</p>
      {lane.state === "available" && <p>
        {lane.sealedSegments}セグメント・{lane.frames}フレーム、
        {partialTimelineLabel(lane.startUs!)}〜{partialTimelineLabel(lane.endUs!)}
        （{lane.sampleRate}Hz・{lane.channels}ch）。区間内の欠損はそのまま保持します。
      </p>}
    </li>)}</ul>
  </section>;
}
