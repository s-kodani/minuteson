# 明示手動話者・話者不明の実再解析保持

2026-10-11、基準 `6fbd57c0156fcabe144d61920601399ab5d41377`。M3-03／M3-04の限定実機確認。正式評価・Windows・実録音／4時間・正式署名／公証は保留を維持し、CI・追加モデル取得は実施しない。アプリコード変更なし。

## 対象と実操作

既存QA `/private/tmp/mon-settings-exit-final-qa.xUpA0Q/MON Onboarding QA.app`（main SHA `f808ad68936879d67a2fd0951b12d7b9fb7a5c16c7e1d5d257c02fb8a5e61f6d`）を再利用。保存先 `jp.mon.desktop.qa.onboarding20261011` の自作会議 `18de4837-b6e1-466b-86a6-ba145b3caed4` のみを操作した。素材は既存29.610375秒の単一話者TTSであり、人工的な話者ラベルを実際の複数話者品質の証拠にしない。本番会議は対象外。

先頭発言の設定→「新しい話者を作成してこの発言に割り当て」→表示名 `QA手動割当`→作成して保存。手動話者ID `manual:0beb13b5-af4e-48fc-a13b-9f28efbb36b6` が先頭発言へ割り当てられ、編集済み本文 `【QA編集確認】` は保持された。次の00:00:03発言で元の `QA読み上げ` のcheckboxを外し、明示的な話者不明として保存。再起動後も両指定と保存済み表示を確認した。

保存draft `c6b074b5-0f9b-4fcc-b6e7-4535591d107b` はlock4、hash `129717242aa4e65afc3b8c3c1b5d46812cdc54267001b06c2b1226d106d623cc`。前回の再解析では明示割当集合が空だったが、今回の固定入力には手動指定の2発言がある。

人数指定の通常selectはDown／Returnで指定へ切り替え、number fieldのsetValueで1を入力してAX値1を確認してから開始。Sherpa job `b5428691-2044-48ac-8c00-05ceb03d294f` はsucceeded 1/1、job記録6,573ms。固定input#9 `69f26792-fb9c-42f3-ba07-54a5e1b5c2ef` は現在#3とdraft lock4／上記hashから作られ、numSpeakers1、manualAssignmentHash `214a1296c61f092f5dde8a297ae232e8821c8b4869d87e9dbb9184f613c74143`。

候補#10 `2d53b397-7ba3-45c2-930d-4c58ba3e9931` の比較画面で、先頭 `QA手動割当`、次の `話者不明` と両方の「手動割り当てを保持」を確認。残る6発言は再推定した `取り込み音声 話者1`。read-onlyのコピーDBで、固定手動ledgerの2行と候補のlineage対応、先頭assignment_source manual、次のattribution_state unknown／割当行なし、本文保持を確認した。候補hashは `1fb3e6d0a89946bb18a6a8ef677a6a3a6cc2fd1545e0da9b0d15bbe91fabe3f4`。

「この候補を採用」→#10／現在#3を直前へ移す確認→初期取消focus→Escapeで採用ボタンへ復帰→比較／会議操作をEscapeで閉鎖。採用は実行していない。議事録#1／編集あり・QA表を保持し、根拠時刻をクリックすると旧固定inputの `QA読み上げ`・`【QA編集確認】`・前後発言を表示。新しい手動話者や候補へ根拠を付け替えない。

## 検証手順の失敗と安全な変更

最初のlive DB読取りはsandbox内でopen失敗。続く `sqlite3 -readonly` の読取りが、それまで存在しなかったSQLiteの `-shm` を作成した。アプリのDB制御ファイル有無の保護チェックが拒否し、開始操作は保存barrierで止まった。この試行で新jobは作られていない。number fieldへのtypeTextも値3のままであり、1人指定の成功として数えない。

通常終了・対象process不在後にDB＋WALをコピーしてintegrity ok／draft lock4保存／新jobなしを確認。通常再起動で保護状態が解消し、上述のsetValue→AX値1→成功jobを確認した。保護チェックは弱めず、DB／補助ファイルは削除しない。以降のSQLは停止後コピーのみへ限定する。これは検証処理が補助ファイルを作った失敗であり、データ消失やモデル推論失敗として記録しない。

## 保存・取り消し・固定候補の照合

通常終了・process不在を各コピー前に確認し、APFS cloneしたDB＋WALをread-onlyで検査した。

- 手動指定保存後・再解析前 `/private/tmp/mon-manual-reanalysis-before.GkhQiG/meetings.sqlite3`、全dump SHA `06ee3a3fec9d3bc93babbf149b91280ecef0cef2e4c8885d267e186631aba620`。
- 再解析・採用取消・固定根拠確認後 `/private/tmp/mon-manual-reanalysis-after.ZFAsBF/meetings.sqlite3`、全dump SHA `41b1a621043fb9e47883cb5c2484f1642ab2030ed4cca675e684bcdf8c8e6d6a`。前コピーの45表の既存全列をEXCEPT照合し欠落／変更は全て0。全5会議のtitle／4head一致、対象外4会議は全列一致。ログ `/private/tmp/mon-manual-reanalysis-preservation-20261011.log`。新job・固定input・候補・関連行と対象の版counterが追加されるため、全DB不変とはしない。
- 再起動後、編集オプション→「直前の編集を取り消す」で00:00:03が `QA読み上げ` へ戻り、先頭の `QA手動割当` は保持。再度#10比較を開き、候補の2発言は再解析時の手動名／話者不明のままで、取り消し後の現在入力へ追従しないことを確認した。
- 取り消し後 `/private/tmp/mon-manual-reanalysis-undo.LGxsP0/meetings.sqlite3`、全dump SHA `14fe03e9a1eb644d446132d3234f86e1b66947110b29074f35f3da021515fee7`、draft lock5、hash `03d686c15d767df02d284568b5d19257d66971579eb6c987de0eff333f8aec89`（操作3の保存hashと一致）。操作5のundoが操作4を戻した。45表照合では対象working_draftsの1行だけが既存行との差となり、他の既存行は保持。undo履歴1行追加、他draft／全head／対象外会議不変。ログ `/private/tmp/mon-manual-reanalysis-undo-preservation-20261011.log`。

後二つのコピーはintegrity ok、FK違反0。現在解析 `1ffb20d3-3f8a-4bf2-8ce6-56e0e1a85b55`／現在議事録 `8054c0e5-cd4f-4869-93f5-a88b76c55602` を維持。QAの手動話者追加と先頭割当は次の採用経路検証用に残す。開始前lock2の全payloadへ戻したとは言わず、不可逆削除・直接SQL更新・二段Undoによる復元は行っていない。

比較画像 `/private/tmp/mon-manual-reanalysis-candidate-20261011.png`（SHA `f2f49f495bd560cf09d4590986bc4d34323fd461c02f3dcffd0dddf88dec4762`）、固定根拠画像 `/private/tmp/mon-manual-reanalysis-evidence-20261011.png`（SHA `68be9eb6f999dbd5a4c5fffa51fc02274bbfa04f3c1f12d1ba1873862ddcba16`）、取り消し後比較 `/private/tmp/mon-manual-reanalysis-after-undo-20261011.png`（SHA `636e5fbcc05ae48a707eb682662033186f87da1970049f6b0a5a28e6cbcc7b53`）。最終比較画像を開いて、現在と固定候補の違いを確認した。保存整合性は画像だけで判定しない。

## 時間と残件

アプリ実装修正0、既存QA再利用につきビルド0。検証記録更新は個別未計測。フロント853件成功／0失敗／0skip、runner1.760232166秒・全体1.94秒（`/private/tmp/mon-manual-reanalysis-tests-20261011.log`）。実モデルjob6.573秒、UI・SQL照合・最初の開始拒否／再起動は個別未計測として分ける。Rustソース変更／通常Rustテスト・Python再実行なし、追加モデル取得0。

次はこの明示割当候補の採用／直前版復元、共通／会議別用語変更等の未網羅Mac経路。重複割当・統合・分割・全人数、競合／保存失敗、実複数話者品質まで合格とはしない。[全23タスク監査](implementation-completion-audit-2026-10-11.md)を維持。暫定残工数12〜29時間以上：非録音UI／編集／異常経路4〜8時間、現在実施しない実録音／耐久5〜8時間、配布／更新／試用3〜13時間。正式評価・Windows保留、外部署名準備待ちは別。
