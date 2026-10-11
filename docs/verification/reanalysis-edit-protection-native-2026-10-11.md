# 編集済み会議の実再解析と設定入口への復帰

2026-10-11、基準 `108ec4b03cdc7fdc82597bd1d186cb388d2956d6`。M3-03／M3-04／M3-08の限定確認。正式評価・Windowsは保留。実録音・マイク／PC音声取得・追加モデル取得・CI・通常Rust検査・署名／公証は行わない。

## 実モデルの再解析と採用取消

前回構築済み `/private/tmp/mon-panel-ring-qa.UPySLJ/MON Onboarding QA.app`（main SHA `b3d1d8b0082e45e6674d31e26c5fcc0e7e8643bbcdd210717c9d2fc8fbe8851e`）を再利用。namespace `jp.mon.desktop.qa.onboarding20261011` の自作会議 `18de4837-b6e1-466b-86a6-ba145b3caed4` だけを通常GUIで操作した。29.610375秒の単一話者TTS・準備済み標準モデルを使い、本番データは対象外。

会議操作の今回のヒントへ「点検日」を入力→文字起こし開始。job `94486c44-2915-42ff-855f-da39cf999776` はsucceeded 1/1、実DBのfinished_at−started_atは13,880ms。新候補#6 `f2699c64-0ecd-4664-8f2f-64bd6645f775` を確認すると、現在版には【QA編集確認】・QA読み上げが残り、候補は未編集本文／話者不明として並ぶ。新候補では「点検日」と認識したが、単一短音声・一回の結果であり、ヒントの一般的効果や正式品質を保証しない。engine prompt／vocabulary.actualPrompt／extraPromptは点検日、4threads／GPU無効をread-only SQLで確認。

「この候補を採用」→対象#6／現在#3を直前へ移す確認、初期取消focus→Escapeで採用操作へ戻る→Escapeで候補確認入口→Escapeで会議操作入口。採用は実行しない。

人数指定・話者分離の通常selectはAX項目クリックだけでは選択が確定せず、開いたメニューをDown／Returnで指定へ切替。単一話者素材に合わせて1を入力→開始。job `d181bdeb-de25-41f0-b837-29e792adca66` はsucceeded 1/1、6,567ms。固定input#7 `dc813218-49f8-4d25-9f0c-19ed6fb79aac` は現在#3と保存draft lock2から作り、固定tracksのnumSpeakersは1。新候補#8 `4b162bbc-1051-44ba-bb06-f0e5cb1eab70` は8発言・話者分離として表示し、【QA編集確認】を含む本文を保持。これも採用しない。

今回の入力では手動割当集合は空。名前QA読み上げは旧話者として候補内にも保持するが、再推定された話者との同一人物対応を自動確定せず、候補の発言表示は取り込み音声 話者1。この確認を、明示手動割当の継承・全話者編集順序・複数話者精度の証拠にしない。本文の手動編集と話者の手動割当は別の条件である。

再解析後の議事録は現在#1／編集あり・QA書式の表を保持。根拠時刻クリックで、元の固定本文【QA編集確認】・QA読み上げ・原認識本文と前後3発言を確認した。新候補に引用を付け替えない。

比較画像 `/private/tmp/mon-reanalysis-candidate-native-20261011.png`（SHA `3c161c2d810d3c0f57ce7fd558ef64dc884d0ea50475b6d2dc70ab6c82ba5b09`）、根拠画像 `/private/tmp/mon-reanalysis-evidence-native-20261011.png`（SHA `2d8629c509833ec7a35c51417538fa9907878dd00f8f3ea88ed9c969165d6996`）。比較画像を開いて現在版と候補の分離を確認した。画像は下記停止DB照合と併用し、見た目だけで保存整合性を判定しない。

## 停止後の保存データ照合

通常終了・対象process不在後、DB＋WALを `/private/tmp/mon-reanalysis-db.VxhGv1/meetings.sqlite3` へAPFS cloneしてread-only照合。開始前 `/private/tmp/mon-panel-ring-db.i2j2hR/meetings.sqlite3` と45表の既存全列をEXCEPTで比較し、欠落／変更はすべて0。既存の本文・話者・根拠・議事録・draft・出力・編集操作・版・旧job・用語・音声metadata・モデル設定／記録を保持。ログ `/private/tmp/mon-reanalysis-preservation-20261011.log`。会議の4head／titleは全5件で一致、対象以外4会議は全列一致、integrity ok・FK違反0。

新job2、候補2、固定内部入力1とそれらの関連行・処理成果物が追加され、対象会議の版番号counter等も進む。全DB不変とは言わない。現在解析 `1ffb20d3-3f8a-4bf2-8ce6-56e0e1a85b55`／現在議事録 `8054c0e5-cd4f-4869-93f5-a88b76c55602` は不変。

再解析終了後の全dump SHAは `f12ec17aee23553b59f5e4e7d4b67f9df54a63250087143c8e21141c8fc231a0`。開始前の全dump SHAとは異なり、新候補・job等の意図した追加がある。以後の冷起動／UI復帰確認はこの値を保護基準とする。

## 見つかった復帰先と修正

既存QAで人数指定をworkspaceから開く→Escapeを閉じると、人数指定ボタンではなく会議操作へfocusが戻ることを再現した。共通native境界のownerは会議操作で、閉じたdescriptorからはreplacementの実入口が失われる。Scopeは可視viewの実replacement入口を保持し、dialog内部の子入口で外側入口を上書きせず、全閉鎖後も保持するよう補修。最終接続・可視・disabled・modal性の検査は既存CommonModalが行う。新規native境界・保存処理・録音・DB commandは増やさない。

新規source契約は修正前9成功／1失敗、runner0.086091209秒／全体0.13秒。外側replacement優先・内側子／全閉鎖で外側保持・新しい兄弟入口とnull fallbackの純粋選択3件を追加。これらは選択ロジックとsource契約であり、DOM focusの実動作とは区別する。最終フロント853成功／0失敗／0skip、runner1.829728166秒・全体2.02秒。型検査＋Vite全体6.10秒・Vite1.49秒。ログ `/private/tmp/mon-settings-exit-before-20261011.log`、`/private/tmp/mon-settings-exit-tests-20261011.log`、`/private/tmp/mon-settings-exit-build-20261011.log`。

第一修正版 `/private/tmp/mon-settings-exit-qa.a9V2Zo/MON Onboarding QA.app`、main SHA `ff07ef008e42195a243c7fc8fbba624d9df1fcfc6f7bcd485c2434d17a715a42`、ad-hoc deep／strict成功、QA全体79.76秒・Cargo1分09秒・Vite1.52秒。通常冷起動の人数指定クリック→Escapeで正しい人数指定入口へ戻り、Enter再表示と子設定を挟んだ全閉鎖後にも同じ入口へ戻ることを確認。ただし子engine→親countsでは×focusとなり、元のengine入口へは戻らなかった。この中間観測を成功にしない。

Scopeの同じowner内のreplacement切替はsnapshot.returningが空のため、CommonModal側の親復帰より後に×へ移動していた。子のparentKeyが新viewのkeyと一致する復帰だけ、子のrestoreTargetを使う。既存の可視／接続／disabled・dialog内の条件と別dialog／別focusを奪わないguardは保持。契約追加の修正前は12成功／1失敗、runner0.087328041秒・全体0.13秒。追加修正後も853成功／0失敗／0skip、runner2.028988583秒・全体2.25秒。ログ `/private/tmp/mon-settings-inner-exit-before-20261011.log`、`/private/tmp/mon-settings-exit-tests-final-20261011.log`。第一QAは通常終了後に切り替えた。

## 最終Mac・冷起動後の保持

最終QA `/private/tmp/mon-settings-exit-final-qa.xUpA0Q/MON Onboarding QA.app`、main SHA `f808ad68936879d67a2fd0951b12d7b9fb7a5c16c7e1d5d257c02fb8a5e61f6d`、ad-hoc deep／strict成功。全体80.36秒・Cargo1分10秒・Vite1.51秒・39.22MiB（ログ `/private/tmp/mon-settings-exit-qa-final-20261011.log`）。既知のSwift user cache／chunkサイズ／通知未承認／公証未実施warningは残し、正式署名や配布条件の合格にはしない。

冷起動→対象会議→人数指定→子engine→Escapeで親engine入口focus→Escapeでworkspaceの人数指定入口focusを確認。その後、会議操作を開いてjob完了・未採用候補#8／#6／#1の保持を確認し、Escapeで会議操作入口へ復帰。人数指定を再度クリック→Escapeで人数指定へ復帰した。設定入力・再推論・採用は行わない。第一QAの「親では×focus」の再現は最終版で改善したが、全入れ子／異常順序・焦点raceの証拠にしない。全browser driver再実行も行っていない。

最終画像 `/private/tmp/mon-settings-exit-final-native-20261011.png`（SHA `0d9273818154ca3e2d79b4afc0fe29fb047293a30bcb37a6c99a5ea7365a1dee`）を開いて、人数指定入口の全周focus枠・編集済み本文とQA読み上げの保持を確認。通常終了・操作した3QA process不在後、DB＋WALを `/private/tmp/mon-settings-exit-final-db.oWiBvl/meetings.sqlite3` へclone。全dump SHA `f12ec17aee23553b59f5e4e7d4b67f9df54a63250087143c8e21141c8fc231a0` は再解析終了後cloneと一致、integrity ok・5会議・FK違反0。冷起動・focus確認に追加の保存変更なし。

## 工程時間と残件

実装・回帰追加は約2分（概算、調査・実機確認・照合・文書作成は別）。型検査／Vite6.10秒＋QA79.76／80.36秒の全実行時間は166.22秒（2分46.22秒）。最終フロント2.25秒、先行成功2.02秒と修正前失敗0.13秒×2を含む小計4.53秒。実モデルのjob記録時間は文字起こし13.880秒＋話者分離6.567秒＝20.447秒で、UIの確認やコピー時間を含まない。ビルドと確認・文書作成は一部並行し、工程時間を壁時計経過へ単純加算しない。

再生成可能なQA targetを再利用し、通常Rust／Pythonテストを重複実行せず、Rustソース変更なし。テストcache statusは8270.8MiB・終了後上限10240MiB。追加取得0bytes・録音なし。残工数は暫定12〜29時間以上：非録音UI／編集／異常経路4〜8時間、今は実施しない実録音／耐久5〜8時間、配布／更新／試用3〜13時間。外部署名準備待ちは別。[全23タスク監査](implementation-completion-audit-2026-10-11.md)を維持し、正式評価／Windows保留以外にも未達を残す。今回の単一話者再解析を正式精度・10話者・全手動割当の保護・計画全体の完了にしない。
