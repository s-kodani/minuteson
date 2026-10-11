# 文字起こしの分割と話者統合を含む再解析の実機検証

2026-10-11、基準 `53b1dfa`。自作の約29.61秒の一話者音声を使い、複数話者の手動指定、発言の分割、話者統合と取り消しが実Sherpa再解析に引き継がれることを確認した。統合と一括話者変更の確認取消で操作位置が戻らない問題も修正した。正式な複数話者品質・長時間性能の評価とは区別する。実録音、追加モデル取得、CI、正式署名・公証は実施しない。

## 編集と再解析の保存結果

保存先は既存QA `jp.mon.desktop.qa.onboarding20261011` のみ。対象は自作会議 `18de4837-b6e1-466b-86a6-ba145b3caed4`、現在解析#3の下書き `c6b074b5-0f9b-4fcc-b6e7-4535591d107b`。本番データは対象外。複数話者指定は操作確認用で、実際に音声が重なっていることを意味しない。

先頭発言を「QA読み上げ」と「QA手動割当」の複数話者にし、第二発言を先頭から1.980000秒、本文10文字目で分割した。前半3.880〜5.860秒は「今日の会議では設備の」、後半5.860〜7.840秒は「点検費を決めます」で、後半を明示的な話者不明にした。元の認識原文は全文を残す。分割位置0秒はエラー表示と保存ボタン無効で拒否し、内側の時刻へ戻してから保存した。削除された分割元の設定ボタンへの復帰は、新しい前半の発言行を安全な代替先とした。

保存後は8発言から9発言になり、冷起動でも保持。下書きlock7、hash `f1e177f6eeb23bbc71ea954741c747eea756206e4f05304eb1f535c1dbb47866`。

| 実話者分離 | 固定入力 | 候補 | 手動指定を保持した発言 | 保存完了まで |
|---|---|---|---|---|
| 複数話者と分割後 | #14 `a9924fa5-1f33-4f5b-a3ef-76f96db62613` | #15 `c092e05a-7bff-45f3-9b5d-bdc934b8062c` | 先頭の2話者、分割前半、話者不明の後半の3件 | 6.560秒 |
| 統合を保存して冷起動後 | #16 `bb14daa7-b066-4c9c-8219-4a9be82c163c` | #17 `afcdf4d5-9f3c-404b-a3fe-1d889747abba` | 統合先を持つ8件と話者不明の1件、計9件 | 6.576秒 |

両ジョブとも一話者指定、通常GUIから開始し1/1単位で成功。ジョブはそれぞれ `b708aea5-934f-4d61-8f56-f19874576efc`、`35a6b954-0815-4978-a701-7f04396fc9b4`。時間はDBのstarted_at〜finished_atであり、60分性能やエンジンだけの推論時間ではない。候補#15と#17の9発言はlineage・本文・認識原文・時刻が全件一致した。

「QA読み上げ → QA手動割当」の8発言を明示的に統合すると、先頭の重複指定は同じ統合先1件になり、分割後半の話者不明は残った。下書きlock8、hash `07a237f72989be487d7e7eff8b52d4381a6c9de55d07a068d010ff14b5efe80b`、新しい編集操作は1件だけ。一括変更の確認取消は保存を追加しない。

候補#17を確認して採用せず閉じ、直前の統合を一度だけ取り消した。下書きlock9となり、payload全文とcontent hashは統合前lock7と完全一致。再起動後の現在版は複数話者・分割・不明を保持し、候補#17は統合時点の内容を別に保持した。複数段階の取り消し、今回の候補採用・復元、全統合チェーンの検証ではない。

## 確認取消後の操作位置の修正

修正前のMacで「話者を統合」をpointer操作して確認を開き、Escapeで取り消すと、設定モーダル全体へfocusが戻ることを再現。このMac WebKitの操作経路ではbuttonがactiveElementにならず、確認サービスの既定の記録対象が実ボタンと一致していなかった。

統合と一括変更で、確認を開くイベントのcurrentTargetを明示的なopenerとして渡した。選択欄を強制focusせず、保存・対象固定・取消・競合チェックは変更しない。新しいsource契約検査は修正前8成功・1失敗、修正後の全フロントは854成功。

最終QA `/private/tmp/mon-structure-focus-qa.HV5Omr/MON Onboarding QA.app`、main SHA `390aecec14f3c573c30d0bbf6b28e910237d2c01298bbdb92c5f2d1e6d6bb64f`。ad-hocのdeep・strict照合成功であり、Developer IDや公証ではない。

最終Macで両操作ともpointerで確認→Escape→実ボタンへfocus復帰→Enterで再確認→初期キャンセルにfocus→Enterで取消→同じ実ボタンへ復帰を確認した。元・先の選択、一括選択1件も保持。統合の明示保存後は選択を空にして実ボタンが無効になるため、親モーダルへの安全な代替復帰と外側入口への復帰を確認。全確認画面・実IME・VoiceOverの合格へ拡大しない。

## 停止後のデータ照合と実機画像

DBは通常終了と対象process不在の確認後にDB＋WALを複製し、そのコピーだけをread-onlyで照合。開始前45表中44表の既存行は全列不変、working_draftsは意図した現在版の1行だけ更新。旧版の名前編集や議事録の下書き・根拠・モデル・用語等は不変。他4会議も全列不変。現在／直前の解析headは#3／#10、議事録headは#1／#2、会議lock7を保持し、候補を追加した番号は進んだ。

開始コピー `/private/tmp/mon-structure-before.DjnAS0/meetings.sqlite3`、分割後 `/private/tmp/mon-structure-split.o6h6Zc/meetings.sqlite3`、候補#15後 `/private/tmp/mon-structure-candidate.y0KnNQ/meetings.sqlite3`、統合後 `/private/tmp/mon-structure-merged.wNX6o5/meetings.sqlite3`、取り消し後 `/private/tmp/mon-structure-undo.l4Fn8N/meetings.sqlite3`、冷起動後 `/private/tmp/mon-structure-cold.Qltb0P/meetings.sqlite3`。取り消し後と冷起動後の全dump SHAは `5405aeba0222250522c5e786dbb4f0b24b05b818767fd68df9acd8f269bfbde5` で一致。最終integrityはok、FK違反0。照合ログ `/private/tmp/mon-structure-native-preservation-20261011.log`。

- 統合ボタンの復帰画像 `/private/tmp/mon-structure-merge-focus-20261011.png`、SHA `88182c5618d9640bf9bf69f488f8bd572c6fd1bf14f1b66b24e8c17d1b686ac6`。
- 一括適用ボタンの復帰画像 `/private/tmp/mon-structure-bulk-focus-20261011.png`、SHA `6fdf5eb97ee9d7762ab53b7b3504efecaada9870e3022bb237760d772ce35a02`。
- 冷起動後の現在版と固定候補 `/private/tmp/mon-structure-cold-candidate-20261011.png`、SHA `6e2decd3f22a87b59ab726b4cdd839283db10d7da85e8fc38cda1f284a5d76f2`。

## 時間と残対応

ソース修正と回帰追加は約3分の概算。実機操作・保存データ照合・文書作成は個別未計測。修正前の回帰検査0.13秒は意図した失敗、最終フロント全体2.58秒、runner2.397257333秒、854成功・0失敗・0skip。ログ `/private/tmp/mon-structure-focus-tests-20261011.log`。

QA構築全体82.69秒、Cargo1分11秒、内包Vite1.50秒。型検査を含むフロント工程単独の時間は未分離。ログ `/private/tmp/mon-structure-focus-qa-build-20261011.log`。Swiftユーザーcache、bundleサイズ、未承認notice、公証未実施の既存警告は残る。Rustソース変更・通常Rust再検査なし。既存QA targetを再利用、incrementalなし、Rust test cache8247MiBで上限10240MiB以下。検査と構築は並行しており単純加算しない。追加のモデル・実行ファイル取得は0bytes。

全23タスクを保持。残工数は暫定12〜29時間以上、内訳は非録音の未網羅UI・編集・異常経路4〜8時間、ユーザー指定で今は実施しない実録音・耐久5〜8時間、配布・更新・試用3〜13時間。署名・公証の外部準備待ちは別。正式評価・Windowsは保留。次は未確認保存結果・競合・明示再試行と、残る確認操作のMac keyboard経路を進める。
