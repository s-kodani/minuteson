---
type: "Guide"
title: "テストと任意の品質評価"
description: "共通・OS別の検査コマンド、Rustキャッシュと品質評価の使い分け。"
tags: ["testing", "evaluation", "maintenance"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/0648448fa12d6997d9611c34d4a65a1d6a53727f/Makefile"
  - resource: "https://github.com/s-kodani/minuteson/blob/0648448fa12d6997d9611c34d4a65a1d6a53727f/tools/mon_rust_tests.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/0648448fa12d6997d9611c34d4a65a1d6a53727f/tools/mon_ci_native_tests.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/0648448fa12d6997d9611c34d4a65a1d6a53727f/tools/rust-test-suites.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/0648448fa12d6997d9611c34d4a65a1d6a53727f/.github/workflows/desktop-platform-build.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/verification/rust-test-efficiency-2026-10-09.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/evaluation/README.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/130c3902eea205bc0ca599c9da45635bb8974ede/tools/mon_docs.py"
---

# テストと任意の品質評価

## セキュリティと文書の検査

`make check-docs`と`make check-security`を実行します。準備、検査範囲、例外とCI判定は[DevSecOps運用](devsecops.md)、PRからの流れは[開発フロー](development-workflow.md)を参照してください。

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

## 通常・重い検証・全件の使い分け

Rustランナーの`--suite quick|heavy|full`で実行範囲を選びます。省略時は従来どおり全件です。日常の変更後確認には`quick`を使い、配布物の作成や大容量・多数の中断地点・履歴世代上限を調べる検証は`heavy`で実行します。`full`は両方を実行します。通常側の成功を全件成功とは扱いません。

分類の正本は[スイート定義](../../tools/rust-test-suites.json)です。通常の保存・削除・同意と一覧の検査、音声削除の基本完了、現行バックアップ形式の往復、登録音声の保持、完了音声クリーンアップの基本検証は通常側に残します。ネイティブ保存先の切替やWALコピー、FLAC作業領域の移動後再開、複数世代の再確認、各SQL段階での破損・取消・SIGKILLの組合せは重い側で確認します。文字列だけを扱う履歴上限の検査は通常側です。定義に一致しない新しい通常テストは`quick`に入ります。重いモジュールへの追加はそのモジュールと同じ分類になります。分類はMacとWindowsで共通ですが、OS条件により存在するテストは異なります。

PRとmain更新は通常スイートを使い、QAインストーラー・MSIX・ネイティブ資材の準備を待ちません。日次04:17（日本時間）と全検証の手動実行、リリース前は全件と既存のSwift／Windows連携・配布物検証を実行します。日次の開始時刻はGitHub側で遅れる場合があります。セキュリティ検証の実行条件は変更しません。構成は[開発フロー](development-workflow.md)を参照してください。

全件CIで作るQAインストーラーとMSIXは、ビルド・内容照合の検証用です。バイナリはActions成果物へアップロードせず、ビルド証跡JSONとテスト時間・診断資料を保持します。パッケージ生成の成功を、実機確認や配布条件の承認とは扱いません。

実モデル・実機・専用連携を必要とする既存のignoredテストは、各テストの条件を満たした専用コマンドで呼び出します。`full`でも全ignoredテストを一括実行しません。

## CIのキャッシュ

CIはnpm・検査用pip依存、Playwrightのブラウザー、Rustの依存とワークスペースクレートを再利用します。全件検証ではWindowsのCMake／Vulkanビルド中間資材と固定ダウンロード、macOSのSwift／Rust FFIビルドもキャッシュします。配布用パッケージ、署名状態、インストーラー、モデルの重み、会議データはキャッシュ対象に含めません。

ネイティブ中間資材は[入力定義](../../tools/mon_ci_cache.py)のソース・設定・固定資材定義と、OS・アーキテクチャ・ランナーイメージ・Rust・CMakeまたはSwift／SDK・ビルドフラグを照合した完全一致キーで復元します。別の入力へ広く一致させる復元キーは使いません。復元後も通常のビルド・固定SHA照合・全件検査を実行します。

共有するビルド・ブラウザーキャッシュは、任意のソース指定がないmainのpush・日次・手動実行が成功した場合だけ保存します。PRとリリース検証は復元のみです。初回とキャッシュ復元時の所要時間を区別し、Actionsのキャッシュ命中表示とRust時間診断で確認します。キャッシュ未作成時や入力変更後の短縮は保証しません。

## macOSのRust本体とキャッシュ

```sh
make test-rust-quick
make test-rust-heavy
python3 tools/mon_rust_tests.py --suite full
python3 tools/mon_rust_tests.py recording_segments::tests
make build-rust-tests
make rust-test-cache
```

このランナーはmacOS／Unix専用です。`make test-rust-quick`は共有クレートも含めて確認します。直接ランナーを呼ぶ場合は`--workspace`で共有クレートも対象にし、省略時は従来どおりデスクトップパッケージが対象です。通常・全件は一度ビルドして対象テストとdoc testを実行し、重いスイートではdoc testを繰り返しません。重点検査は指定フィルターのライブラリ検査に限定します。通常側と重い側の間でCargoのビルドプロファイルは変えず、同じキャッシュを使います。

日常用のMakeターゲットは4プロセス×4スレッドで実行します。対象名を列挙して重複なく分け、モジュールを交互に割り当てます。直接呼び出すCLIの既定は従来の1プロセスです。CIの通常側は2プロセス×4スレッドに抑えます。並列数の変更は`--processes`と`--threads`で指定し、失敗の自動再試行は行いません。

`.cache/rust-tests`を所有付き共有キャッシュとして再利用します。標準ビルド並列数は最大4、終了後のソフト予算は10,240 MiB、開始前は2,048 MiB以上の空きを要求します。実行中の厳密な上限ではありません。`--jobs`で1〜16を指定できます。状態確認では回収せず、明示的な整理は`python3 tools/mon_rust_tests.py --clean`です。子プロセスが終了していない、所有不明、リンクがある領域を回収しません。

`make check`はPython・Rust・画面・ブラウザー・Swift録音プローブ・専用連携をまとめて実行します。実マイク録音の品質評価とは別です。

## Windowsのネイティブ検査

通常側はQAビルドなしで実行できます。`TAURI_CONFIG`で配布用リソースの準備を無効化し、デスクトップパッケージのbin・統合検査・doc testも含める場合は`--all-targets`を指定します。共有クレートは別のCargoコマンドで確認します。

Windowsでも通常・重いスイートには`--processes`と`--threads`を指定できます。既定は1プロセスで、CIと同じ並列設定は`--processes 2 --threads 4`です。全件スイートは従来の1プロセスを維持します。

```powershell
$env:TAURI_CONFIG = '{"bundle":{"resources":null,"externalBin":null,"active":false}}'
cargo test --workspace --exclude mon-desktop --locked --lib
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
python tools/mon_ci_native_tests.py --suite quick --all-targets
```

この例で設定した`TAURI_CONFIG`は通常検証用です。QAビルドへ進む前に`Remove-Item Env:TAURI_CONFIG`で解除します。もともと設定があった場合はその値へ戻してください。

重い側・全件では[QAビルド](build-windows.md)で録音ヘルパーと推論ランタイムを準備してから実行します。引数なしの既存コマンドはライブラリ全件、`--all-targets`付きはデスクトップパッケージのその他のテスト対象も含みます。

```powershell
ctest --test-dir probes/windows-capture/build -C Release --output-on-failure --no-tests=error
python tools/mon_ci_native_tests.py --suite heavy
python tools/mon_ci_native_tests.py --suite full --all-targets
```

Windows専用のリンク・実行結果はWindows上で取得します。Macでのクロスターゲット型検査をWASAPI・実画面・インストーラーの合格にしません。

## 所要時間の確認

通常スイートの目標は、依存キャッシュがある端末でコード変更後のビルドから完了まで約5分です。初回コンパイル、配布物の作成、CIの起動待ち、画面・セキュリティ検証を含む全体の5分保証ではありません。テストを分けてもコンパイル対象のソース自体は減らないため、無変更時だけ速い結果を変更後の結果と扱いません。

```sh
python3 tools/mon_rust_tests.py --workspace --suite quick --processes 4 --threads 4 --timings-json /private/tmp/mon-rust-quick.json
```

Windowsは`python tools/mon_ci_native_tests.py --suite quick --all-targets --timings-json "$env:TEMP/mon-rust-quick.json"`でデスクトップ側を計測し、上記の共有クレート検査の時間も合算します。端末のOS・CPU・メモリ・Rust版と、テスト件数、ビルド・実行・合計時間を併せて確認してください。他のビルドを重ねず、代表的な実装変更とテスト変更の後で連続3回300秒以内となった環境だけを達成済みとします。Macの計測をWindowsの達成結果へ転用しません。分類定義に過去の遅い実行の根拠があっても、最新ソースの5分達成を示すものではありません。

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
