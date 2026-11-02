# MinutesOn

![MinutesOn — アプリアイコン、文字起こし画面、議事録プレビューを紹介するバナー](assets/readme/hero.png)

<sub>画面内の会議は公開用のサンプルデータです。</sub>

**会議の記録を、あなたのPCで。**

MinutesOn（略称 **mon**）は、会議の録音・文字起こし・話者分離・議事録生成をローカルで行う、Windows／macOS向けデスクトップアプリです。AIモデルの導入後は、録音した会議や取り込んだ音声をオフラインで解析できます。

[主な機能](#主な機能) · [対応環境](#対応環境) · [使い方](#使い方) · [開発を始める](#開発を始める) · [開発に参加する](#開発に参加する) · [ドキュメント](#ドキュメント)

> **開発中のプロジェクトです。** 録音から議事録出力までの機能を接続し、短い音声での実モデル統合を確認しています。実録音・機器変更・長時間の安定性、正式な品質・性能評価、署名済み配布物と実機更新の検証は残っています。現在の実装範囲と残件は[実装状況](docs/implementation-status.md)を参照してください。

## 主な機能

| 機能 | できること |
| --- | --- |
| 会議を録音 | マイクとPC音声の取得、録音前の5秒テスト、録音中のミニパネル表示 |
| 音声を取り込む | 保存済みの音声ファイルを会議として取り込み、解析 |
| 文字起こし | ローカルAIで発言をテキスト化し、音声を再生しながら本文を修正 |
| 話者を整理 | 話者ごとの区間分離、話者名の修正、声の登録・照合 |
| 議事録を作る | ローカルLLMでMarkdownを生成し、プレビュー・テキスト編集・保存・出力 |
| 用語を登録 | 共通・会議別の用語辞書で固有名詞や専門用語を指定 |
| データを管理 | 手動バックアップ、既存データへ追加する復元、会議データの保存先変更 |
| 表示を選ぶ | モノトーン／パープル、システム／ライト／ダークを設定 |

AIモデルは全体設定で選択します。文字起こしにはWhisperを使用でき、macOSではReazonSpeech・Haas-ja-v1.0も選択できます。モデルの種類や実行方法はOSによって異なるため、詳細は[モデル定義](app/model-catalog/)と[モデル導入ガイド](docs/model-download-macos.md)を参照してください。

## 対応環境

| OS | 対象 |
| --- | --- |
| Windows | Windows 11 24H2以降・x64 |
| macOS | macOS 15以降・Apple Silicon |

処理時間とメモリ使用量はモデルや端末に依存します。16GB端末・10話者会議での正式評価は未完了です。初回のモデル取得にはネットワーク接続と、モデル本体・取得素材を保存する空き容量が必要です。

## 使い方

1. 下記の[開発手順](#開発を始める)またはOS別のビルド手順でアプリを準備します。
2. 「設定」から文字起こし・話者分離・議事録用のモデルを選び、「ダウンロードして使用」で導入します。
3. 会議を作成し、マイク・PC音声を選んで録音します。録音前に「5秒録音して確認」で入力を確認できます。保存済みの音声は取り込みから始められます。
4. 解析後の発言一覧で、音声を聞きながら本文と話者を修正します。
5. 議事録を生成し、Markdownを確認・編集して出力します。

文字起こしと議事録は再生成できます。置換の確認があり、処理中に行った編集を無断で上書きしません。議事録の再生成では旧版の自動バックアップを作らないため、必要な内容は事前に出力してください。

### データとプライバシー

- 準備済みモデルによるAI処理はPC内で行います。モデルの初回取得には外部の配布元へ通信します。
- 会議情報はSQLite、保存音声はFLACで管理します。会議データとAIモデルの保存先は別で、会議保存先の変更ではモデルを移動しません。
- 内部ログには会議本文やローカルパスが含まれる場合があります。ログは会議データとは別に最大30日・合計200 MiB保存し、バックアップには含めません。会議を削除しても保持期限まで残る場合があります。

## 開発を始める

### 前提

Node.js 24（npmを含む）、Rust stable、Python 3.12と、OS別のネイティブ開発環境を用意してください。

- **Windows:** Visual Studio Build Tools 2022のC++開発環境、Windows SDK、CMake 3.21以上、WebView2。導入と事前検査は[Windows開発環境](docs/windows-setup.md)を参照してください。
- **macOS:** Xcode（Swift 6.2以上）とCMake。録音ヘルパーや推論ランタイムを含むアプリの準備は[MinutesOn QAの準備手順](docs/verification/m4-01-macos-native-qa-preparation-2026-10-07.md)を参照してください。

### ソースの取得と開発起動

```sh
git clone https://github.com/s-kodani/mon.git
cd mon
npm ci --prefix app
npm --prefix app run tauri -- dev
```

WindowsのPowerShellでnpmの実行ポリシーにより起動できない場合は、`npm`を`npm.cmd`に読み替えてください。

このコマンドは画面とRust本体の開発起動です。録音ヘルパー・推論ランタイムを含めたビルドは、上記のOS別手順に従って準備してください。ブラウザーだけで表示した場合は「デスクトップ接続なし」と表示され、録音・AI処理は利用できません。

**通常の開発起動は、通常版と同じ識別子・保存設定を使います。** 起動時に回復処理や解析が進む場合があるため、既存データと分離して実機検証するときはMinutesOn QAを使用してください。

### 検査

以下はリポジトリルートから実行します。フロントエンドの検査には、上記の`npm ci`が必要です。

```sh
# 画面の型検査とビルド
npm --prefix app run build

# フロントエンドのテスト
npm --prefix app test

# 生成済みのIPC型・バリデーターの整合性
npm --prefix app run check:contracts
```

Rustの共通ライブラリのテストは両OSで実行できます。

```sh
cargo test --workspace --exclude mon-desktop --locked --lib
```

macOSの本体テストは、共有キャッシュを管理するランナーから実行します。

```sh
python3 tools/mon_rust_tests.py

# 変更箇所を絞って実行する例
python3 tools/mon_rust_tests.py recording_segments::tests
```

このランナーはmacOS／Unix専用です。キャッシュの容量管理は[Rustテストの運用](docs/verification/rust-test-efficiency-2026-10-09.md)を参照してください。Windowsの本体テストは、QAビルドで録音ヘルパーと推論ランタイムを準備してから`python tools/mon_ci_native_tests.py`で実行します。OS別の検査条件は[CIの定義](.github/workflows/desktop-platform-build.yml)にも記載しています。

macOSでは`make check`でPython・Rust・画面・ブラウザー・ネイティブ録音プローブの検査をまとめて実行できます。`make check-browser`は合成IPCを使う画面検査です。実機の録音やAI品質の確認は、[評価手順](docs/verification/m1-01-evaluation-protocol.md)に沿って別途行います。

### リポジトリ構成

```text
app/          React / TypeScriptの画面とTauriのデスクトップ本体
crates/       共通の処理・契約型・ネイティブ連携・録音FFI
probes/       OS別の音声取得・推論ワーカー・話者特徴量の実装
tools/        ビルド・モデル準備・検査・評価用ツール
tests/        Pythonテストとフィクスチャ
evaluation/   評価条件・データ管理・評価結果の形式
docs/         仕様・開発計画・操作と配布の手順・検証記録
assets/       アプリアイコンとREADMEの画像
```

Rust workspaceと`Cargo.lock`はルートで共有しています。IPC型・バリデーターは`npm --prefix app run generate:contracts`で生成します。実行ファイル名や保存形式などの内部識別子には略称`mon`を使用します。

## 開発に参加する

不具合報告や改善提案は[Issues](https://github.com/s-kodani/mon/issues)、修正はPull Requestで受け付けます。大きな機能追加や設計変更は、まずIssueで目的と範囲を共有してください。

- **不具合報告:** OS・アーキテクチャ、使用モデル、再現手順、期待した動作と実際の動作を記載してください。
- **修正:** 変更の目的と影響範囲、実行した検査、未確認の環境をPull Requestに記載してください。
- **検証データ:** 再現用の合成音声や匿名化した例を使用してください。ログ・画像・音声を共有する前に、会議本文、個人情報、ローカルパス、認証情報が含まれていないか確認してください。

実装の優先順位と完了条件は[開発タスク計画](docs/development-plan.md)、実装済みの範囲は[実装状況](docs/implementation-status.md)で確認できます。

## ドキュメント

| 読みたいこと | ドキュメント |
| --- | --- |
| 要件・データ設計・UI | [初期仕様書 v0.8](docs/specification-v0.8.md) |
| 実装の進捗と残件 | [実装状況](docs/implementation-status.md)・[開発タスク計画](docs/development-plan.md) |
| Windowsの開発・QAビルド | [Windows開発環境](docs/windows-setup.md) |
| macOSの実機QA | [MinutesOn QAの準備手順](docs/verification/m4-01-macos-native-qa-preparation-2026-10-07.md) |
| macOSのモデル導入 | [モデル導入ガイド](docs/model-download-macos.md) |
| WindowsのStore向けビルド | [MSIXの生成・検証・提出準備](docs/windows-store.md) |
| macOSの署名・公証・DMG | [本番ビルド手順](docs/macos-distribution.md) |
| macOSの手動更新・データ保持 | [手動更新ガイド](docs/manual-update-macos.md) |
| 音声・AIの品質と性能の評価 | [M1評価手順](docs/verification/m1-01-evaluation-protocol.md) |
| 起動時の保存先診断 | [保存先の起動拒否時の案内](docs/verification/storage-startup-attention-2026-10-10.md) |
| 画面の操作イメージ | [HTMLモック](prototypes/meeting-app-preview.html)（ダウンロードしてブラウザーで表示） |

HTMLモックの録音・再生・AI処理はデモです。配布用ビルドの生成と、署名・公証・実機QA・利用条件の確認は、それぞれ別の工程です。

## ライセンス

本体のライセンスは未設定です。現時点でリポジトリルートに`LICENSE`はありません。OSSとしての公開に向けて、採用するライセンスの明示が必要です。

依存ライブラリ・AIモデルにはそれぞれの利用条件が適用されます。[第三者ライセンス資料](third-party/notices/README.md)と[モデル定義](app/model-catalog/)を参照してください。
