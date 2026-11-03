---
type: "Guide"
title: "macOSの開発とQAビルド"
description: "Apple Siliconでの前提環境、開発起動と通常版・QAアプリの構築。"
tags: ["macos", "build", "qa"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/build-macos.sh"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/package.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/tauri.macos.conf.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/tauri.qa.conf.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/probes/macos-capture/Package.swift"
---

# macOSの開発とQAビルド

## 前提

macOS 15以降・Apple Silicon、Node.js 24、Rust stable、Python 3.12、CMake、Swift 6.2以上を含むXcodeを用意します。コマンドはリポジトリルートから実行します。

```sh
xcode-select -p
swift --version
node --version
python3 --version
cargo --version
cmake --version
npm ci --prefix app
cargo fetch --locked
```

Xcodeを複数導入している場合はSwiftの版と選択中のDeveloperディレクトリを確認します。依存・固定資材の初回取得には通信が必要です。

## 開発起動

```sh
npm --prefix app run tauri -- dev
```

通常の開発起動は通常版と同じ識別子・保存設定を使い、起動時に回復や解析が進む場合があります。録音・推論に必要なヘルパーとランタイムは下記のビルドフックで準備します。画面だけをブラウザーで表示した場合は録音・AI処理を利用できません。

## QAアプリ

```sh
CARGO_INCREMENTAL=0 npm --prefix app run tauri:qa:build -- --bundles app
```

`app/build-macos.sh`が画面、依存通知文、Whisperランタイム、推論ワーカー、録音・話者特徴量ヘルパーを準備し、Tauriがアプリを構築します。出力は`app/src-tauri/target/release/bundle/macos/MinutesOn QA.app`です。モデルの重みは設定から取得します。

| 項目 | 通常版 | QA版 |
| --- | --- | --- |
| 表示名 | MinutesOn | MinutesOn QA |
| 識別子 | `jp.mon.desktop` | `jp.mon.desktop.qa` |
| 保存設定 | 通常利用用 | QA用 |

QAビルドはアドホック署名を使います。`tauri.qa.conf.json`が名前・識別子・ウィンドウ名を分離し、macOS共通のリソース・最低OS・権限条件を保持します。同じQA識別子の別コピーは同じ保存設定を使うため、検証用コピーのアプリ名を変えただけではデータを分離できません。

```sh
codesign --verify --deep --strict 'app/src-tauri/target/release/bundle/macos/MinutesOn QA.app'
```

構築・署名検査とGUIの実行結果は分けて確認します。QAを永続的な場所に置いて利用する場合は、処理終了後にアプリを正常終了してから入れ替えます。会議保存先には一時ディレクトリを使わないでください。

## 通常版と配布

アドホックの通常版appは`npm --prefix app run tauri -- build --bundles app`で構築できます。出力は同じbundleディレクトリの`MinutesOn.app`です。Developer ID署名・公証・DMGは[配布手順](distribution-macos.md)を使います。

動作確認には[テスト運用](testing.md)を参照してください。通常版のデータをQAへ移す場合は、DBを直接共有せずアプリのバックアップ・復元を使います。
