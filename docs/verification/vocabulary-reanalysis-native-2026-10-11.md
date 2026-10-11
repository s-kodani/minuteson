# 共通・会議別用語と実解析の固定入力

2026-10-11、基準 `17ca93b2ae15be42b6bc93812649c983553f44cd`。M3-03（Q13・UX-29・DATA-14）とM2-03のユーザー一時停止／再開の限定Mac確認。正式品質・Windows・実録音／4時間・正式配布は保留。CI・追加取得・Rust／Python検査なし。アプリ変更／再ビルドなし。

## 通常GUIで確認した経路

既存 `/private/tmp/mon-settings-exit-final-qa.xUpA0Q/MON Onboarding QA.app` を再使用。main SHA `f808ad68936879d67a2fd0951b12d7b9fb7a5c16c7e1d5d257c02fb8a5e61f6d`、ad-hoc deep／strict署名照合成功。保存先は `jp.mon.desktop.qa.onboarding20261011` のみ。自作29.610375秒TTS会議 `18de4837-b6e1-466b-86a6-ba145b3caed4` と、このQA内の共通用語を操作。本番設定・録音権限・配布には触れない。

設定の共通用語に「点検日／てんけんび／共通QAヒント」「MON／モン／共通のみQA」を登録して保存。会議操作→会議別用語モーダルに「点検日／てんけんび／会議QA優先」を登録して保存。共通2件・会議1件から適用候補は重複排除した2件となり、Escapeで会議別用語入口へ戻ることを確認。今回の追加ヒント欄は空のまま、実Whisperを開始した。

最初のジョブの解析中表示を確認後、会議用語の補足を「会議QA変更後」へ編集して無効化し保存。保存時刻は最初のジョブ完了後だったため、この操作だけを実行中の入力保護の証拠とはしない。新しい候補#11は未採用のまま、現在#3の手動話者名／本文編集を含む比較を確認した。

通常終了／冷起動後に次の解析を開始し、解析中の同じジョブを一時停止。0/1区間・再開ボタンを確認してから設定へ移り、共通の補足を「共通QA変更後」「共通のみQA変更後」に変更して保存。通常終了後コピーでpaused／user・0区間のチェックポイントと、変更前の共通用語を固定した入力を確認。冷起動後も一時停止を保持し、同じジョブを明示再開→成功1/1・候補#12。さらに新規開始→成功1/1・候補#13を確認した。

| ジョブ／候補 | 固定した共通版／会議版 | 選択された用語・補足 | 開始〜完了 |
|---|---|---|---|
| `d0a01ead-a6d7-407e-817e-c9611d5a0333`／#11 | 2／1 | 点検日＝会議QA優先、MON＝共通のみQA | 13.977秒 |
| `49c1e5b7-70c2-47bb-8bee-216d6ee7df85`／#12 | 2／2 | 無効な会議用語は共通を抑制せず、点検日＝共通QAヒント、MON＝共通のみQA | 94.177秒。一時停止・設定編集・再起動・再開を含み、純粋推論時間ではない |
| `65c4a715-6ae9-4b01-acaa-881bd717c9e8`／#13 | 3／2 | 点検日＝共通QA変更後、MON＝共通のみQA変更後 | 13.646秒 |

いずれもsnapshot actualPromptとengine promptは一致し、選択順・読み・補足・entryVersion・scopeを保存。用語snapshot ID／hashは順に `b86bd5c1-58c4-4037-98a2-0ce25e5e72e1`／`a8e66f4031b4bd966c74e4d4ed8a541cd1137d607b5472d9d33f2cc67ffbc167`、`c71e788f-7f57-42b2-badc-a0dace2ca14b`／`e542d3986ca9fa1e573a5b4278fa74842b16e51cd93dd15f9c9131a723c453f3`、`f5a3b661-498e-40ce-b5e0-cf27f8fd63b5`／`5f436b8ec3da1d56daa66aa4421ef8441cc8b61fc9ee836a4eb2bf648f95b23a`。一時停止後の共通変更は再開ジョブへ混入せず、新規だけへ反映した。

候補は各1発言で「点検日」を含むが、元の8発言と同じ区切りではなく話者不明。今回話者分離・候補採用は実施していない。誤認識の自動置換や品質・分割時刻の合格、3／10話者・実会議への一般化はしない。現在#3の「点検費」等と保存済み編集を自動変更しないことを確認した。

最後に冷起動し、現在の手動名／保存済み表示、3ジョブ成功と候補#11〜13、会議用語の補足／無効状態と最新の共通2件を確認して通常終了。QA内に意図した用語3件と未採用候補3件を残した。既存の別QA会議の用語1件は保持し、削除操作はしない。

## 停止コピーの照合

live SQLiteへ接続せず、各通常終了と対象process不在確認後にDB＋WALをAPFS cloneしたコピーだけをread-onlyで照合。

- 開始前：前回 `/private/tmp/mon-manual-publication-cold.DzO85v/meetings.sqlite3`。
- 最初の解析後：`/private/tmp/mon-vocabulary-first.aAR5dm/meetings.sqlite3`、dump SHA `04565213c01b4c7fdbea6d653e0a3199221468f30da738b14017a64f1a89e17b`。
- 一時停止／共通変更後：`/private/tmp/mon-vocabulary-paused.011X5J/meetings.sqlite3`、dump SHA `5c5afa031cecbdec82d9c35260deb27a49d74bd9988115373ac613b7517ed1b4`。
- 3件完了後：`/private/tmp/mon-vocabulary-final.lCga3c/meetings.sqlite3` と最終冷起動後 `/private/tmp/mon-vocabulary-cold.wHM5ae/meetings.sqlite3`、両dump SHA `9334f4af476b8c70d28fc4a31c8da34ea622e32d58910c8779a174bb4f755cf0` 一致。

全コピーintegrity ok・FK違反0。開始前45表の既存全列はEXCEPT差0。全会議の解析／議事録current／previous headとlock、対象外4会議の全列も差0。本文・下書き・編集履歴・固定根拠／入力・旧用語・既存job・音声・モデル／出力等の既存行を保持した。追加job3・用語snapshot3・draft0。開始前は対象共通／会議用語scopeがなく、今回は新scope／新用語と新候補・音声処理結果を追加したため、開始前から全DB不変とはしない。

最初の解析後／一時停止コピーの用語snapshot既存行と、一時停止ジョブの全固定spec列も最終コピーとの差0。停止checkpoint時刻 `1791707450754` は共通変更保存 `1791707473289` より前で、再開後にsequence2／1区間を保存。照合ログ `/private/tmp/mon-vocabulary-native-preservation-20261011.log`。コピー検査のSQLで不存在列 `active_duration_ms`、`entry_version` を参照した2回の読取り失敗があり、現schemaを確認して再照合した。アプリjobの失敗／再試行ではなく、コピー以外のDBを書き換えない。

画像は用語優先 `/private/tmp/mon-vocabulary-precedence-20261011.png`（SHA `26ac2cf88d0b36ece27b52164bbf98632d47332e31f1fc998e37094a3694bbae`）、候補比較 `/private/tmp/mon-vocabulary-candidate-20261011.png`（SHA `899897c8ea552acd39f39e13abd903c0328d1bd163e9d163ce5ae10481ceb35b`）、冷起動用語 `/private/tmp/mon-vocabulary-cold-20261011.png`（SHA `9fd8faeed6a1207d3294f17c6fba820a90ef711732f2e909cab86c35b7bf82a3`）。system追従darkで確認し、テーマ設定変更・全テーマ網羅の証拠にはしない。

## 時間・残件

アプリ実装修正0、ビルド0。フロント853成功・0失敗／0skip、runner2.788698959秒・全体3.01秒（`/private/tmp/mon-vocabulary-native-tests-20261011.log`）。再開した推論とテストは並行で、単純加算しない。実機操作・SQL照合・記録作成は工程別未計測。アプリjob3件成功、意図した一時停止1回／同じjobの再開1回、前記読取り検査の失敗2回を区別する。

通常用語の保存／優先／無効化／変更後新規／固定入力再開を確認したが、全登録境界・保存失敗／競合・同条件再試行・実録音の優先停止は未網羅。[全23タスク監査](implementation-completion-audit-2026-10-11.md)を保持し、次は重複割当・統合／分割等と異常経路。暫定残工数12〜29時間以上：非録音UI／編集／異常4〜8時間、現在実施しない実録音／耐久5〜8時間、配布／更新／試用3〜13時間。外部署名準備待ち・正式品質評価・Windowsは別。
