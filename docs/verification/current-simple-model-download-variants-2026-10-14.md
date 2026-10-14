# 現行設定画面からの追加モデル初回導入

2026-10-14。録音と実会議を使わず、専用 `MON Download QA` の現行「選択→ダウンロードして使用」から、Whisper base・smallと話者分離の標準・int8を公式URLで初回取得した。対象アプリは前回と同じ `/private/tmp/mon-current-download-qa.Lz02Od/MON Download QA.app`、識別子 `jp.mon.desktop.qa.download20261014`。通常QA、本番の会議・モデル・設定には触れていない。Developer ID署名・公証済み配布、配布元署名による真正性や再配布条件の確認ではない。

| モデル | 取得ID | 固定recipe SHA256 | HTTPS取得対象 | 画面での結果 |
| --- | --- | --- | ---: | --- |
| Whisper base（多言語） | `1325434d-2510-4f13-bfab-bcdfdf96d475` | `f96419de8187ef53545739cf40dac6776e854fb95a47ac5ad8fd1c61d9ab22e5` | 147,955,724 bytes | 使用中まで成功 |
| Whisper small（多言語） | `f19e0603-ef25-4215-8461-0c9bf1fcc242` | `25fb94331716ab82ae54ecbd1b06d08356482138d5c04225842f7b0aa9d8ab3b` | 487,606,226 bytes | 使用中まで成功 |
| pyannote segmentation 3.0 + ERes2Net base | `ccd386a3-0bac-404e-8680-6244bb159983` | `0996bb77a0fab2f604c97af2bde017cebe813e83529228be92778c4befd4a7c8` | 65,153,215 bytes | 使用中まで成功 |
| pyannote segmentation 3.0 int8 + ERes2Net base | `1d6b6850-e8d3-4c66-a329-17cdb5bafbf0` | `7295993aa83e948a5f3fd36ca26992e7a9833a7a90208148b0d6bf3306064350` | 65,153,215 bytes | 使用中まで成功 |

取得量は、各保存済みrecipeのHTTPS URLを重複除去し、archiveは固定archive容量、単体ファイルはmanifest容量から算出した。4件で765,868,380 bytes。前回の既知累計7,448,334,876 bytesへ重複取得も安全側に加えると8,214,203,256 bytesで、承認上限8GiB（8,589,934,592 bytes）まで375,731,336 bytes。ネットワーク上の転送量の実測ではない。Qwen 1.5B Q4/Q8とWhisper mediumの新規HTTPS取得はこの残枠では行わない。

各操作では選択後に「モデルを準備しています」、完了後に「使用中」と導入済み一覧を確認した。smallは途中で「導入済み」を表示し、CPU確認・適用完了後に「使用中」へ移った。アプリを正常終了して全5導入済みモデルのmanifest記載ファイルをSHA256で再照合し、相違0件。冷起動後もWhisper smallと話者分離int8の「使用中」、それ以前のtiny/base/標準の「導入済み」を確認し、再び正常終了した。会議は0件のまま、録音・音声取り込み・OS権限変更・CIは実施していない。

最初の起動を制限環境の `open` で試みるとLaunchServicesが `kLSNoExecutableErr` を返し、同じ環境から実行ファイルを直接起動するとAppKitのアプリ登録中にSIGABRTとなった。実行ファイルは存在し、アドホック署名のdeep/strict検証に成功した。許可された通常のMac起動経路で同じアプリを開くと画面操作・4件の取得・再起動が成功したため、制限環境の起動失敗を製品の再現済み不具合とも、原因解明済みとも扱わない。

今回の製品ソース変更・ビルドは0件。実機操作、取得、SHA照合、冷起動確認は約7分の概算で、ネットワークとCPUの各工程を独立計測していない。文書作成・検査は別計測。既取得素材を使った全8候補のCPU・短い合成音声の統合証拠は[前回](remaining-model-integration-2026-10-14.md)にあり、今回の4件の初回HTTPS導入を正式な精度・60分性能、長時間録音や配布条件の合格へ広げない。残るMac実録音・権限/機器異常・4時間、依存通知文と配布条件、Developer ID署名・公証・Gatekeeper・実アプリ置換と先行試用は未完了。正式評価・10話者・Windows固有部分は保留、VoiceOverは対象外。
