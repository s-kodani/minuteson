---
type: "Specification"
title: "アプリの構成と処理境界"
description: "React・Tauri・Rust共通処理・ネイティブヘルパーとIPC契約の構成。"
tags: ["architecture", "ipc", "native"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/Cargo.toml"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/build.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/lib.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/Cargo.toml"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/crates/mon-capture-ffi/src/lib.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/probes/windows-capture/CMakeLists.txt"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/probes/macos-capture/Package.swift"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/scripts/gen-contracts.mjs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/verification/cross-platform-refactor-implementation-2026-10-29.md"
---

# アプリの構成と処理境界

## 構成

アプリ側が操作受付・ジョブ・保存を管理し、録音と推論は別プロセスのヘルパーへ委ねます。矢印は主な呼出しとデータの受渡しを表します。

```mermaid
flowchart TB
    subgraph desktop["アプリ側"]
        ui["React / TypeScript<br/>メイン画面・録音パネル・補助画面"]
        ipc["Tauri IPC<br/>役割別の許可・入力検証・起動状態の検査"]
        app["Rust本体<br/>録音制御・ジョブ・編集・モデル管理・復旧"]
        ui <-->|"型付き要求・状態・結果"| ipc
        ipc <--> app
    end

    subgraph helpers["ネイティブヘルパー・ワーカー（別プロセス）"]
        capture["録音ヘルパー<br/>macOS: Swift / Windows: C++"]
        inference["推論・話者特徴量抽出<br/>Whisper / sherpa-onnx / llama.cpp"]
    end

    subgraph local["ローカルデータ"]
        db[("SQLite<br/>会議・解析・編集・処理履歴")]
        audio["管理音声・作業領域<br/>原PCM / CAF・保存FLAC・解析WAV"]
        models["AIモデル・推論ランタイム"]
        logs["内部ログ"]
    end

    app -->|"起動・停止・所有管理"| capture
    capture -->|"状態・manifest"| app
    capture -->|"取得音声・時計"| audio
    app -->|"要求・取消・所有管理"| inference
    inference -->|"進捗・推論結果"| app
    app <-->|"保存・読取り"| db
    app <-->|"確定・検証・再生・復旧"| audio
    app <-->|"取得・照合・配置"| models
    models -->|"検証した資材を使用"| inference
    app -->|"診断を記録"| logs
```

| 領域 | 役割 |
| --- | --- |
| `app/src` | React／TypeScriptの画面、編集状態、通知、生成IPCの利用 |
| `app/src-tauri` | Tauri本体、永続化、ジョブ、操作受付、モデル管理、OS連携 |
| `crates/mon-core` | OSに依存しない録音・モデル・メディア等の共通判定 |
| `crates/mon-native` | OS別のファイル・プロセス・転送などのネイティブ境界 |
| `crates/mon-contracts` | 共通の構造型・契約 |
| `crates/mon-capture-ffi` | 録音のFFI連携 |
| `probes` | Swift／C++の音声取得、推論ワーカー、声の特徴量ヘルパー |
| `tools` | 資材準備、契約生成、配布、テスト・評価ツール |

Rust workspaceと`Cargo.lock`をルートで共有します。録音・解析・データ操作の成功判定はネイティブ側で行い、画面だけで保存済みと推測しません。

## 処理の流れ

```mermaid
flowchart TB
    recording["マイク・PC音声の録音"] --> saved["所有・時刻・保存の確定"]
    imported["音声ファイルの取り込み"] --> saved

    subgraph automatic["保存後の自動解析"]
        wave["会議全体の共通WAV"]
        asr["文字起こし"]
        diarization["話者分離・発言への割当て"]
        publish["発言一覧へ直接保存<br/>後続処理の意図も原子的に記録"]
        followup["話者補完・登録した声との照合"]
        wave --> asr --> diarization --> publish --> followup
    end

    saved --> wave
    followup --> edit["音声を再生し、本文・話者を修正"]
    edit -->|"利用者が生成を実行"| generate["議事録の分割生成・統合"]
    generate --> markdown["Markdownを保存・プレビュー"]
    markdown --> manual["テキスト編集・保存・出力"]
```

各処理は開始時の固定入力と操作IDを持ち、取消・異常終了・再開・公開時の編集競合を検査します。文字起こしやモデル設定の変更だけでは議事録を自動生成せず、明示的な再生成では置換を確認します。

## 共通処理とコード生成

実線はライブラリの依存先、点線はビルド時のコード生成を表します。録音ヘルパーはC ABIを通じてRustの共通判定を使い、Tauri本体と判定を共有します。

```mermaid
flowchart TB
    desktop["Tauri本体<br/>mon-desktop"] --> core["mon-core<br/>OS共通の判定・処理"]
    desktop --> native["mon-native<br/>OS別のファイル・プロセス・転送"]
    desktop --> contracts["mon-contracts<br/>共通構造型・契約"]
    core --> contracts

    mac["macOS録音ヘルパー<br/>Swift"] --> ffi["mon-capture-ffi<br/>C ABI・静的ライブラリ"]
    windows["Windows録音ヘルパー<br/>C++"] --> ffi
    ffi --> core

    contracts -.->|"スキーマを書き出す"| generator["generate:contracts<br/>TypeScript型・バリデーター生成"]
    generator -.-> generated["app/src/generated<br/>画面で使うIPC契約"]
```

## IPCとウィンドウ

IPC型・バリデーターは契約定義から生成します。`npm --prefix app run generate:contracts`で生成し、`check:contracts`で整合性を確認します。コマンドと各ウィンドウの許可は[コマンド登録](../../app/src-tauri/command_registry.json)と[Tauri capabilities](../../app/src-tauri/capabilities/)を参照します。

メイン、録音パネル、録音説明、終了確認は別の役割と許可を持ちます。WebViewへ汎用シェル・任意ファイル操作を公開しません。起動診断時は通常の画面と操作を開始せず、診断の読取り・終了・再起動と、専用の受付条件を満たす明示的な初期化操作を提供します。画面ごとの関係は[画面構成と遷移](screens.md)、保存データの関連は[ER図](data-model.md)を参照してください。

## ネイティブプロセス

録音・推論ヘルパーは同梱パスと必要な資材を検証して起動します。Windowsの推論などはJob Objectで所有し、取消・親終了時に配下の終了を管理します。macOSでも所有する子の停止・確定を待ちます。モデルから得た任意のコマンドをシェルで実行しません。

通常版とQA版は起動識別子・保存設定を分離します。同一版を再度起動した場合は既存のメイン画面を表示し、後発プロセスは終了します。アプリ・ヘルパー・ランタイムは同じソースから揃えて構築します。

設計の理由は[ADR](../adr/index.md)、変更時の確認は[テスト運用](../guides/testing.md)を参照してください。
