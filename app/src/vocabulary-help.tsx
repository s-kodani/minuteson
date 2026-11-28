import type { ReactNode } from "react";
import type { HotwordRejectionReason, IgnoredHotword } from "./generated/contracts";
import { InfoPopover } from "./info-popover";

/** Shared explanation for the common, meeting and import vocabulary editors. */
export function VocabularyHelp({ label, context }: { label: string; context?: ReactNode }) {
  return <InfoPopover label={label} details={<>
    <p>例えば、取引先の会社名が「ミライ」なら、その名前を登録しておきます。「ミライに連絡してください」という発言が、「未来に連絡してください」ではなく、会社名の「ミライ」で文字起こしされやすくなります。</p>
    <p>使用できる項目や使用方法は、各モデルの詳細にある「用語辞書の対応」で確認できます。補足は保存・表示用です。</p>
    <p>保存した表記をヒントに使えるかは、選択したモデルが判定します。ReazonSpeech が除外した表記と理由は、文字起こし結果の「用語辞書の適用情報」で確認できます。</p>
    <p>ヒントに採用された表記も、音声の内容やモデルの認識によって異なる表記になることがあります。似た別の言葉が登録語として文字起こしされる場合もあります。確定した本文の自動置換は行いません。</p>
    <p>変更は次回の新しい文字起こしから反映されます。実行中の処理・同じ条件での再試行・過去の結果は変わりません。</p>
    {context}
  </>}>社名・人名・専門用語など、文字起こしのヒント候補を辞書に保存できます。</InfoPopover>;
}

const rejectionLabels: Record<HotwordRejectionReason, string> = {
  reservedSyntax: "予約文字（: # /）を含んでいます",
  whitespace: "使用できない空白を含んでいます",
  outsideVocabulary: "モデルの語彙にない文字を含んでいます",
  controlCharacter: "制御文字を含んでいます",
  invalidUtf8: "文字の形式が無効です",
  empty: "表記が空です",
  unknown: "理由不明",
};

export function VocabularyApplication({ ignored }: { ignored?: IgnoredHotword[] }) {
  if (!ignored?.length) return null;
  return <details className="vocabulary-info">
    <summary>用語辞書の適用情報（未適用 {ignored.length} 件）</summary>
    <p>ReazonSpeech のヒントから除外された表記です。次回の生成に反映するには、辞書で表記を修正して保存してください。</p>
    <ul>{ignored.map(({ term, reason }) => <li key={term}><strong>「{term}」</strong> · {rejectionLabels[reason]}</li>)}</ul>
  </details>;
}
