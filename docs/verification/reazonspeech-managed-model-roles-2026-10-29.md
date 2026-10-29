# ReazonSpeechの管理モデル役割照合（2026-10-29）

## 問題と修正

ReazonSpeech-k2-v2（FP32）で文字起こしを再生成すると、解析計画を固定する段階で「固定した管理モデルの役割が不足・重複しています」となる。

ASRの実行設定が直接参照するのはワーカー、Encoder、VAD、Decoder、Joiner、Tokens、sherpa C APIライブラリ。管理パッケージには、それ以外のONNX Runtime等の依存ライブラリとライセンス文書も含まれる。従来の照合はASRパッケージ内の全ライブラリに実行設定の参照を要求し、正しい構成でも件数が一致しなかった。macOSの構成も同じ条件に該当する。

管理モデルの固定条件と再配置時の照合で、manifestに記載したバックエンドの直接参照ファイルを使用するよう修正した。依存ライブラリと文書は引き続き固定パッケージに保存し、実行前の全ファイル検証に含める。管理モデルの証拠を持たない旧ジョブの再配置条件は維持する。

## 検証

- Windows上で追加した3件のRust回帰テストが成功。FP32・軽量版それぞれのmacOS/Windows構成について、通常参照と可搬参照の照合、直接指定ライブラリの欠落・SHA不一致・依存DLLへの取り違え拒否、再配置先への参照解決を確認。
- 小さな合成パッケージの実ファイルを使い、依存ライブラリと文書を同じ容量の別内容に変更しても、実行前のSHA検証で拒否することを確認。
- 関連Rustテスト（`model_ asr analysis_flow`）は89成功、6 ignored、失敗なし。ログは `output/reazonspeech-related-tests.log`。Windowsの権限設定を使うテストは専用の `.cache/reazonspeech-managed-tests` 内でサンドボックス外実行した。
- `cargo fmt` と `git diff --check` が成功。

実会議「2026/10/24 17:35の会議」の再生成は実行していない。モデル重みや保存済みの会議データは変更していない。

修正版QAインストーラーは通常の `app/build-windows.ps1 -Qa` で生成する。ソースとバイナリの対応は `output/windows-qa-work/*-source.json` / `*-build.json`、ビルドログは `output/reazonspeech-managed-model-qa-build.log` に記録する。
