# 現行モデル導入QAでの合成WAV取り込み・自動解析

2026-10-14。保存先を分離した `/private/tmp/mon-current-download-qa.Lz02Od/MON Download QA.app`（`jp.mon.desktop.qa.download20261014`）を使用。通常QA・本番会議・録音・OS権限・追加モデル取得には触れない。

直前の[ドロップ確認](audio-import-drop-regression-2026-10-14.md)では、利用者がFinderから1件を手動で渡し、確認画面が開いたと報告した。保存操作はしていない。本検証は別に、同じ合成WAVをアプリの「音声を取り込む」→macOSのファイル選択から選び、**選択から保存・解析まで**確認した。ドラッグしたまま保存まで通したという主張ではない。

- 元ファイル：`/private/tmp/mon-synthetic-flow-audio.iAJXQT/QA-generated-meeting-valid.wav`。RIFF WAV／PCM16／16kHz／mono、947,610bytes、SHA256 `ccc3316f158ff9691267f176c32039f51eb49d5faf79b21908f43e6f328bfa93`。取り込み後も元のSHAは一致。
- ネイティブ確認画面は同ファイル名、00:00:29、WAV／PCM／16,000Hz／1ch、原本0.9MiB、必要132.5MiB、空き371,909.8MiBを表示。会議名を `QA-download-model-import-20261014` として明示保存した。
- 保存直後に「文字起こし中」、続いて話者分離の保存成功を表示。発言一覧は8件、話者欄には「取り込み音声 話者1」を表示した。正常終了・冷起動後にも会議1件と発言8件、各発言の音声区間を開けた。
- QA終了後、DB本体とWALを `/private/tmp/mon-import-snapshot.Uq5x5C` へコピーして読取り。`integrity_check=ok`、外部キー違反0行。会議ID `de89bd53-bd5b-4ad9-9090-743ef3a288d6`、`source_type=audio_import`、`duration_us=29610375`、現在版の発言8件、最初0us・最後29,610,375us。`transcription` と `diarization` は各1/1で `succeeded`、失敗コードなし。
- 配置された `original.wav` は947,610bytesで元ファイルと同SHA。`audio.flac` は1,144,990bytes、SHA256 `76ffa261fbf0b534248872320caa84118dc0d3692d91676fe1b3c84c1884952b`。DBの両 `file_assets` はready・同容量・同ハッシュ。固定モデルSHAはWhisper small `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b` とint8 segmentation `d582f4b4c6b48205de7e0643c57df0df5615a3c176189be3fc461e9d18827b5d` に一致した。

これは短い合成発話の機能的な取り込み・解析であり、正解ラベルがないため文字起こし精度、話者分離の人数・品質、60分性能は判定しない。議事録モデルはこの専用QAに未導入で、Markdown生成は実施していない。PC音声・実マイク・4時間録音・異常終了・配布署名・公証・CIも未実施。今回の製品ソース変更・ビルド・自動テストの再実行はない。実機操作とDB照合は約5分の概算で、各解析ジョブ単独の実時間は測定していない。
