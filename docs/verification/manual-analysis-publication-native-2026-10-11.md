# 編集済み解析の採用・直前復元と両版の下書き保持

2026-10-11、基準 `3cfdee533ad8853bdb8823fc651dfbbb3879d289`。M3-03／M3-04のR-17・DATA-03／05・UX-15通常経路の限定確認。正式評価・Windows・実録音／4時間・正式署名／公証は保留、CI・追加取得・再推論なし。既存QAを再利用しアプリコード変更／再ビルドなし。

## Mac実機の操作と結果

QA `/private/tmp/mon-settings-exit-final-qa.xUpA0Q/MON Onboarding QA.app`、main SHA `f808ad68936879d67a2fd0951b12d7b9fb7a5c16c7e1d5d257c02fb8a5e61f6d`、保存先 `jp.mon.desktop.qa.onboarding20261011`。自作29.610375秒TTS会議 `18de4837-b6e1-466b-86a6-ba145b3caed4` だけを操作。本番データ・録音権限・配布設定には触れない。

前回の[明示手動割当再解析](manual-assignment-reanalysis-native-2026-10-11.md)で保存した候補#10と、現在#3の保存済み編集を通常GUIで比較した。候補は先頭QA手動割当／次の話者不明、現在は先頭QA手動割当／次のQA読み上げ。比較後の採用確認は「#10を採用／#3と編集を新しい直前版へ移す」、初期focusは取消。対象を確認し「候補を採用する」をクリック。候補一覧から#10が外れ、直前#3となり、通常本文も候補の手動割当・話者不明・編集マーカーを保持した。

採用#10の手動話者名を通常名前モーダルで `QA採用後編集` に変更して保存。旧版#3の `QA手動割当` と区別するための意図したQA編集である。通常終了・再起動後も新しい名前と話者不明を確認した。

会議操作→「直前版を確認・復元」では、現在#10のQA採用後編集／話者不明と、復元対象#3のQA手動割当／QA読み上げを、双方の保存済み編集を含めて表示。「この直前版へ戻す」→対象#3／#10を新しい直前へ移す確認→Escapeで取消し、復元ボタンfocusへ戻ることを確認。Enterで同じ確認を再表示し、明示的に「直前版へ戻す」をクリック。親の直前版入口へfocusが戻り、表示は直前#10へ更新された。

親をEscapeで閉じると通常本文は旧#3のQA手動割当／QA読み上げ、編集マーカーと保存済み表示へ戻る。議事録は#1／編集あり・QA表を保持。根拠時刻クリックで生成時に固定したQA読み上げ／編集マーカー／前後文脈を確認し、採用#10・その名前編集・復元に合わせて引用先を切り替えない。

復元後に再度通常終了・冷起動し、現在#3の旧編集を確認。直前#10の比較を開くと `QA採用後編集`／話者不明と保存済み編集が残っていた。ここでは再復元しない。最終状態は現在#3・直前#10で、テスト用の#10の名前編集を保持した。全テーマ・全入力形態・VoiceOver／IME・競合／応答喪失まで検証したとはしない。

## 停止したDB＋WALのコピー照合

各コピー前に通常終了・対象process不在を確認。live DBへSQL接続せず、APFS cloneしたコピーだけをread-onlyで照合した。

| 状態 | コピー先 | 全dump SHA-256 |
|---|---|---|
| 開始前 | `/private/tmp/mon-manual-publication-before.xXbm2i/meetings.sqlite3` | `14fe03e9a1eb644d446132d3234f86e1b66947110b29074f35f3da021515fee7` |
| #10採用・名前編集保存後 | `/private/tmp/mon-manual-publication-adopted.RWPF3Z/meetings.sqlite3` | `57ae6adb73651360825b4fa6836a0384e9439fd720349a09998cc56edfa8f650` |
| #3復元後 | `/private/tmp/mon-manual-publication-restored.bxMBJY/meetings.sqlite3` | `4b5e4bd82f6b58742f206cd95715f41bd20e02579938684dd1758e3b14ca74c5` |
| 最終冷起動・比較確認後 | `/private/tmp/mon-manual-publication-cold.DzO85v/meetings.sqlite3` | `4b5e4bd82f6b58742f206cd95715f41bd20e02579938684dd1758e3b14ca74c5` |

後三つはintegrity ok、FK違反0。採用後はcurrent `2d53b397-7ba3-45c2-930d-4c58ba3e9931`（#10）／previous `1ffb20d3-3f8a-4bf2-8ce6-56e0e1a85b55`（#3）、meeting lock6。復元後はcurrent／previousを逆にしてlock7。議事録current `8054c0e5-cd4f-4869-93f5-a88b76c55602`／previous `689e7112-5087-4e78-ac6c-48260fc26cd6` は全工程で不変。

旧draft `c6b074b5-0f9b-4fcc-b6e7-4535591d107b` はlock5／hash `03d686c15d767df02d284568b5d19257d66971579eb6c987de0eff333f8aec89` を保持してactive 1→0→1。新draft `897b894e-fd9c-414b-b755-0a26dcda83e0` はbase#10、名前変更のみ、lock1／hash `ad22bba222eeab450ef97a95b5c6fd8f7a6362fc7cbc2bebf96ac1fae863b800`、active 1→0。旧下書きの本文／手動名／manual_speakersと旧履歴は消えず、新版の編集と混同しない。

開始前と復元後の45表の既存全列をEXCEPT照合し、44表は欠落／変更0（旧working_drafts全列・全edit_operationsを含む）。analysis_revisionsだけ#10のpublication_state candidate→committedで1行差。publication_stateを除く全列の差は0。対象外4会議の全列、全議事録head、根拠／固定入力／出力／音声／モデル／用語／既存jobは不変。新draft1・新編集操作1、job追加0。ログ `/private/tmp/mon-manual-publication-preservation-20261011.log`。採用／復元による対象会議のhead／lockと公開状態、新編集の意図した変更があるため、開始前から全DB不変とは言わない。最終冷起動確認で追加の保存変更がないことは全dump一致で確認した。

採用後画像 `/private/tmp/mon-manual-publication-adopted-20261011.png`（SHA `85e66c39b59fcd56ddf8c38c2ab0209d451680fbcabc93564279c7858efa0e4f`）、根拠 `/private/tmp/mon-manual-publication-evidence-20261011.png`（SHA `3a8e351856449fe3cd5907fdc4a2397952cd7ec13329886f588860387f41e6d9`）、冷起動比較 `/private/tmp/mon-manual-publication-cold-20261011.png`（SHA `623c6aa6152a9690e4de7b431c32f2ce39938fd6c78e6b49ab8cfdad5a8f35e6`）。最終比較画像を開き、両版固有の名前と次発言の差を確認した。最後の画像はシステム追従のdark表示であり、今回テーマ設定を変更／全テーマ網羅した証拠にしない。

## 時間と残件

アプリ実装変更0・ビルド0、記録作成／実機確認／照合は個別未計測。フロント853件成功・0失敗／0skip、runner1.74923775秒・全体1.93秒（`/private/tmp/mon-manual-publication-tests-20261011.log`）。失敗／再試行なし、復元確認の意図した取消1回。Rust／Pythonテスト・実AI・CIは今回実行しない。

次は共通／会議別用語変更、重複割当・統合・分割等の未網羅経路。今回の通常1往復から競合／保存失敗／既存直前版置換・全版整理の安全性を推測しない。[全23タスク監査](implementation-completion-audit-2026-10-11.md)を維持し、正式評価・Windows以外も未達を残す。暫定残工数12〜29時間以上：非録音UI／編集／異常経路4〜8時間、現在実施しない実録音／耐久5〜8時間、配布／更新／試用3〜13時間。外部署名準備待ちは別。
