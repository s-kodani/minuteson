---
type: "Guide"
title: "テストと任意の品質評価"
description: "共通・OS別の検査コマンド、Rustキャッシュと品質評価の使い分け。"
tags: ["testing", "evaluation", "maintenance"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/Makefile"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/tools/mon_rust_tests.py"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/tools/mon_ci_native_tests.py"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/.github/workflows/desktop-platform-build.yml"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/verification/rust-test-efficiency-2026-10-09.md"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/evaluation/README.md"
---

# テストと任意の品質評価

## 通常の検査

リポジトリルートで実行します。画面の検査には`npm ci --prefix app`が必要です。

```sh
python3 -m unittest discover -s tests -v
python3 tools/mon_contracts.py --check
python3 tools/mon_model_catalog.py --check
npm --prefix app run check:contracts
npm --prefix app run build
npm --prefix app test
cargo test --workspace --exclude mon-desktop --locked --lib
cargo fmt --all --check
```

契約やモデルカタログを変更した場合は対応する生成コマンドを実行し、生成済みファイルの差分もレビューします。`--check`は生成物を更新しません。OS別の構築条件と検査範囲は[CI定義](../../.github/workflows/desktop-platform-build.yml)にあります。

## macOSのRust本体とキャッシュ

```sh
python3 tools/mon_rust_tests.py
python3 tools/mon_rust_tests.py recording_segments::tests
make build-rust-tests
make rust-test-cache
```

このランナーはmacOS／Unix専用です。全件は一度ビルドして通常のテストとdoc testを実行し、重点検査は指定フィルターのライブラリ検査に限定します。全件成功と重点検査成功は区別します。

`.cache/rust-tests`を所有付き共有キャッシュとして再利用します。標準ビルド並列数は最大4、終了後のソフト予算は10,240 MiB、開始前は2,048 MiB以上の空きを要求します。実行中の厳密な上限ではありません。`--jobs`で1〜16を指定できます。状態確認では回収せず、明示的な整理は`python3 tools/mon_rust_tests.py --clean`です。子プロセスが終了していない、所有不明、リンクがある領域を回収しません。

`make check`はPython・Rust・画面・ブラウザー・Swift録音プローブ・専用連携をまとめて実行します。実マイク録音の品質評価とは別です。

## Windowsのネイティブ検査

[QAビルド](build-windows.md)で録音ヘルパーと推論ランタイムを準備してから実行します。

```powershell
ctest --test-dir probes/windows-capture/build -C Release --output-on-failure --no-tests=error
python tools/mon_ci_native_tests.py
```

Windows専用のリンク・実行結果はWindows上で取得します。Macでのクロスターゲット型検査をWASAPI・実画面・インストーラーの合格にしません。

## ブラウザーとASR

`make check-browser`または`npm --prefix app run test:browser`は合成IPCを使う画面検査です。実機WebView、OS権限、実音声推論とは区別します。

ASR変更時は`python3 tools/mon_asr_assets.py --check`と、`python3 tools/mon_rust_tests.py -- asr model_ transcription analysis_flow local_engine::resident ipc_acl`を使います。実モデル資材と許可された合成音声がある場合のみ、`MON_ASR_ASSETS=/absolute/assets python3 tools/mon_rust_tests.py native_asr_models -- --ignored`で追加確認します。必要な資材と条件は当該テストの定義を確認してください。

## 任意の品質と性能評価

音声と正解データに同意・匿名化・内容ハッシュを付け、OS・端末・電源・モデル・実行ファイル・実行時間・最大RSSと人手確認を記録します。手順とCLIは[評価キット](../../evaluation/README.md)を参照してください。準備確認は次のとおりです。

```sh
python3 tools/mon_eval.py doctor
python3 tools/mon_eval.py validate
python3 tools/mon_eval.py list-scenarios
```

既存キットは16GB端末・10話者・会議アプリ／録音範囲／機器の評価条件を保持しています。32GBの開発機や短い合成入力の結果を保証値へ転用しません。議事録の構造化項目・根拠IDの採点は旧評価器の入力契約であり、現行Markdown UIの必須機能ではありません。機械採点に必要な形式と、人間による内容確認を区別してください。

今後の評価結果を記録する場合は、対象版・試行ID・入力／モデル／実行ファイルのSHA、端末条件、実測値、内容レビュー、未測定事項、採否の理由を残します。採用方針が変わる場合だけADRへ反映し、工程ごとの検証日誌を増やしません。評価実施を今回の完成扱いの前提へ戻しません。
