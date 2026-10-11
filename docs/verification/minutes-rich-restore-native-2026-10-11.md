# 議事録の書式・候補復元・編集終了の限定Mac確認

2026-10-11。基準 `cf7364fc4143ed0b6c9d52df08d0ebfc6fa16c2b`。許可済みの独立QA `jp.mon.desktop.qa.onboarding20261011` の自作会議だけを操作した。正式評価／Windowsは保留、録音・音声再生・新しいAI推論・追加モデル取得・CIは実施していない。全23タスクや全書式・全異常経路の完了宣言ではない。

## 通常GUIで確認した範囲

対象会議 `18de4837-b6e1-466b-86a6-ba145b3caed4`、採用議事録#1 `8054c0e5-cd4f-4869-93f5-a88b76c55602`、既存候補#2 `689e7112-5087-4e78-ac6c-48260fc26cd6`。最初の確認は前回構築済み `/private/tmp/mon-import-focus-qa.5cGf8d/MON Onboarding QA.app` を使用し、その主実行ファイルSHA-256は `e5b36d63ecc06eba54c8e87bf0a8b1449ddda54b35316db85c0b74d6113e4990`。

- 日時段落内の自作「QA保存確認」に太字・斜体を適用。見出し2→箇条書き→番号付きリストへ順に変更し、本文と太字・斜体を保持した。最終状態は番号付き1項目。
- 日時の後ろへ2列1行の表を挿入し、「QA書式確認」「未確認」を入力・保存。元の8項目・8根拠と空のToDo表は保持した。日本語入力は通常貼り付けであり、実IMEの合格証拠ではない。
- 編集終了→通常OS保存画面から新しいMarkdownを書き出した。`/private/tmp/mon-minutes-format-qa.64wVWF/minutes-formats.md` は3057bytes、SHA-256 `c98d416764a7b1193e6d2fdd7b5e7e2fef853b79a1852a4ade1cba15f893b309`。番号リスト、太字・斜体、2列表、元の8箇条書きと8脚注／時刻参照を確認した。
- 候補#2の採用確認をEscapeで取消し、採用入口へfocus復帰。その後、明示的に#2を採用。直前版#1の復元確認もEscape取消→復元入口focus復帰→再表示して明示復元した。最終状態は編集済み#1が現在版、#2が直前版。候補#2はcommittedで、未採用候補を保持したという意味ではない。
- 通常終了→同じアプリを冷起動し、#1の書式・新しい表・保存済み表示を確認。最初の固定根拠を開き、生成時の引用と元発言文脈、手動編集表示、固定解析入力を確認した。根拠の音声は再生していない。

## 編集終了focusの発見・補修

最初のMac実機では編集終了後のAX焦点が「会議ワークスペース」に残り、編集ボタンへ戻らなかった。通常ブラウザーのクリックでは再現しなかったため、ブラウザーだけの成功をMac成功として扱わない。

終了入口の実buttonを操作開始前に明示focus・記録し、保存flush成功後だけ編集を終了する。終了処理が失った焦点だけを編集入口へ戻し、保存待ち中に別の要素を選んだ場合やmodalが開いている場合には奪わない。保存API・JSON・版管理は変更していない。新規2件はソース契約検査であり、DOM／WebKitの実行検査とは区別する。

最終QA `/private/tmp/mon-minutes-finish-focus-qa.McTnvO/MON Onboarding QA.app` はdeep／strict署名照合成功（QAアドホック、正式署名・公証ではない）。主実行ファイルSHA-256 `f91a27a9b58a1f407984d718d8b2441277c6aae3d583f093e4e7a120a1a32d62`。本文先頭と表の入力欄から終了する2反復で、Mac AXの焦点が実際の「議事録を編集」buttonに戻った。内容は変更していない。

合成ブラウザーでも通常終了の編集入口focusを確認。保存応答を3秒遅延させ、自作本文を変更→編集終了→外側の合成操作「議事録保存を復帰」へfocusを移した。保存中を観測し、完了後は編集が終了・保存済みになりながら、その合成操作のfocusを保持した。これはMacでの遅延／保存失敗／IME／全VoiceOver検証の代替ではない。エージェントの検証tabのみ終了した。

## 停止後の読取り照合

通常終了後にprocess不在を確認し、DBとWALをAPFS cloneしてreadonlyで照合した。SQL書込みやWALを無視するimmutable接続は使っていない。

| 照合対象 | 結果 |
|---|---|
| 復元後→冷起動終了→最終focus検査終了の全DB論理dump | 全て `dd191a4b5afc39cb7677f318b7890f56c1d1dc0e672aab7bb5ee15cc4f68ddf1` |
| 最終clone | `/private/tmp/mon-minutes-finish-focus-db.IfP2It/meetings.sqlite3` とWAL、integrity ok／FK違反0／5会議 |
| 書式編集前→冷起動後の他4会議・全文字起こしdraft・解析／jobs・モデル／設定／用語／音声asset metadataの選択した行群 | `c188296824aed14f50d957b59b094992f14bdf99ea81fc9543f6fb3e18fea946` 不変 |
| 議事録draftの8根拠JSONと元の生成版#1の行 | `d8f94f3811d30f8ad936db449e24ae5e39579efffb4f3443d50560120d942516` 不変 |
| 旧11会議QAの全draft／exportの選択した行群 | `acce9d590c669a10c3638fcd404a906ce250389e1e44156f1af037102db71e95` 不変、旧QAは起動せず |
| 旧QA bootstrap | `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` 不変 |

今回の書式・採用／復元・出力は対象テスト会議の意図した変更であり、開始前の全DB不変とはしない。議事録draft `5e4020e2-4543-490b-b559-5ae1acfeaee1` はlock10、content hash `a05243d014410c56cf5c14875a7b9830813082eed8e554497c64575fc1f00126`。新出力履歴 `06a378de-d677-4f46-8361-c906128035cd` はsucceeded・lock10、正規化Markdownのexport hash `ff3b4825b0a43a2136c1c2dc6e7a98e310dd47de196e5898e3c3b67a8ac26a23`（draft JSON hashとは別）。元の出力ファイルは2986bytes、SHA `ea4eff608194af1b983fcda8beceaf30659bc374e2f4dadaea2e36ca348c1c0a` を保持。

画像 `/private/tmp/mon-minutes-format-evidence-cold-native-20261011.png` は `cf5e6840102292ad803842f478b32fd364ccda4c3dd2392bf8941bb2e2ed938a`、最終focus画像 `/private/tmp/mon-minutes-finish-focus-native-20261011.png` は `0d6a332e94191f0e06e970bf59def1939ea82322a3366056e275a6f757af4209`。

## 時間・ログ・残件

- focus実装修正は約1分（概算、ドキュメント整理を含まない）。実機書式／復元と監査を含む初回確認区間はUTC06:46:42〜06:53:09、6分27秒。最終ビルド・ブラウザー／Mac確認と監査はUTC06:54:36〜06:59:45、5分09秒の区間内で重複し、純実行時間へ合算しない。
- フロント838成功／0失敗／0skip、runner1.873402084秒。型検査／Vite成功、Vite1.56秒。QA内Vite1.57秒／Cargo1分11秒、39.22MiB。型検査や署名を含むbuild全経過は分離計測なし。Sonner directive／bundleサイズ／Swiftユーザーcache／未承認notice等の警告は残り、正式配布成功とはしない。
- tests log `/private/tmp/mon-minutes-format-focus-tests-20261011.log` SHA `d88edd951895f8f5453da71eb4f00bfb078ec0ab5575e741715ae0712caf844a`。build log `/private/tmp/mon-minutes-format-focus-build-20261011.log` SHA `964223bd82880c68c6bc5095b79602fb796b23159e78e200dc3aed8d4667c92a`。QA log `/private/tmp/mon-minutes-format-focus-qa-build-20261011.log` SHA `09e79c8e2aca1a8584498f5da7f281f90ae5354a0ec9e672f39c0da83a3fd501`。
- Rust製品ソース変更／通常Rust／Python再実行なし。Rust test cache8357MiB（容量表示の丸め）、上限10240MiB。
- 全書式（H1／H3、削除・全表操作等）・長文全ページ・全保存失敗／競合・実IME／VoiceOverは残る。[全23タスク監査](implementation-completion-audit-2026-10-11.md)を維持する。暫定残工数12〜29時間以上：非録音の未網羅UI／編集／異常経路4〜8時間、現在実施しない実録音／耐久5〜8時間、配布／更新／試用3〜13時間。正式評価・10話者・Windows保留、署名準備／協力者等の待ち時間は別。
