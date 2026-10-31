import type { ReactNode } from "react";
import { InfoPopover } from "./info-popover";

/** Shared explanation for the common, meeting and import vocabulary editors. */
export function VocabularyHelp({ label, context }: { label: string; context?: ReactNode }) {
  return <InfoPopover label={label} details={<>
    <p>例えば、取引先の会社名が「ミライ」なら、その名前を登録しておきます。「ミライに連絡してください」という発言が、「未来に連絡してください」ではなく、会社名の「ミライ」で文字起こしされやすくなります。</p>
    <p>使用できる項目や使用方法は、各モデルの詳細にある「用語辞書の対応」で確認できます。補足は保存・表示用です。</p>
    <p>必ず登録した表記になるとは限りません。また、似た別の言葉が登録語として文字起こしされる場合もあります。確定した本文の自動置換は行いません。</p>
    <p>変更は次回の新しい文字起こしから反映されます。実行中の処理・同じ条件での再試行・過去の結果は変わりません。</p>
    {context}
  </>}>社名・人名・専門用語など、文字起こしで使ってほしい表記を登録できます。登録した表記が選ばれやすくなるよう、文字起こし時のヒントとして使います。</InfoPopover>;
}
