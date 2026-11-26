---
type: "Guide"
title: "Windowsの開発とQAビルド"
description: "Windows x64の開発ツール、依存準備とQAインストーラーの構築。"
tags: ["windows", "build", "qa"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/prepare-windows.ps1"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/build-windows.ps1"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/tauri.windows.conf.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/windows-setup.md"
---

# Windowsの開発とQAビルド

## 前提

Windows 11 24H2以降・x64でPowerShellを使います。

| ツール | 構成 |
| --- | --- |
| Git・Node.js | Windows版Git、Node.js 24とnpm |
| Python | 3.12 |
| Rust | [固定版](development-workflow.md#rustの版の固定)、`x86_64-pc-windows-msvc` |
| Visual Studio Build Tools | 2022、C++開発、MSVC x64/x86、Windows SDK |
| CMake | 3.21以上。CTestも必要 |
| WebView2 | Microsoft Edge WebView2 Runtime |

Visual Studio 2022の`Visual Studio 17 2022`ジェネレーターを使います。ツール導入後はターミナルを開き直し、以下を確認します。PowerShellのポリシーが`npm.ps1`を拒否する環境では`npm.cmd`を使います。

```powershell
node --version
npm.cmd --version
python --version
rustup show
cmake --version
ctest --version
rustup target add x86_64-pc-windows-msvc
```

CMakeがPATHにない場合、ビルドスクリプトはVisual Studio 2022同梱のCMake／CTestも探索します。手動で使う場合はその`Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin`をビルド用PATHに追加します。古いnvmへのリンクが残る場合はリンク先を調べ、利用するNode.js 24の実体へPATHを通します。

## 依存と事前検査

リポジトリルートで実行します。

```powershell
npm.cmd ci --prefix app
cargo fetch --manifest-path app/src-tauri/Cargo.toml --locked
python -m unittest discover -s tests -p test_mon_tauri_platform.py -v
npm.cmd --prefix app test
```

各コマンドの終了コードを確認してから進めます。初回のnpm・Cargo依存と同梱資材の取得は通信を伴います。ロックファイルを自動更新しません。

## QAビルド

```powershell
npm.cmd --prefix app run tauri:windows:qa:build
```

ビルドフックが固定資材を照合し、画面、Windows依存通知文、CPU／Vulkan推論ランタイム、ASR資材、WASAPI録音ヘルパーとRust本体を準備します。QAの識別子は`jp.mon.desktop.qa`で、ユーザー単位のNSISを作ります。WebView2の導入方法は[WindowsのEXE配布](distribution-windows.md#webview2の導入と更新)を参照してください。モデルの重みは設定からダウンロードします。

- 配布用QA：`output/windows-qa/MON-QA-setup.exe`
- ソースと構築の証跡：`output/windows-qa-work/<build-id>-source.json`、`<build-id>-build.json`
- Tauri内部出力：`app/src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/`

ソース・本体・ヘルパー・インストーラーの一致とネイティブCTestが成功した後に配布用コピーを更新します。失敗時は以前のコピーを保持するため、古いファイルを今回の成功と取り違えないよう証跡のビルドIDを確認します。ビルド自体はインストール・起動しません。

## 起動回復画面をQA版で再現する

Windows QA版の専用保存先で、一時的なファイルロックにより起動検証を停止させます。DB・音声・モデルを書き換えずに、実際の起動回復画面を表示できます。

1. `MinutesOn QA`を起動し、設定から**検証専用の空フォルダー**を会議保存先に選びます。通常版の会議保存先とは分けます。
2. QA版をトレイのメニューから完全終了します。
3. PowerShellで次を実行し、選んだ検証用の会議保存先を入力します。QA版が作成した既存の空ロックファイルを一時的に保持します。

   ```powershell
   $recoveryRoot = (Read-Host 'QA版で選んだ検証用の会議保存先（絶対パス）').Trim('"')
   $recoveryLockPath = Join-Path $recoveryRoot '.mon-startup-reset.lock'
   $recoveryShare = [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
   $recoveryLock = [IO.File]::Open($recoveryLockPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, $recoveryShare)
   try {
       $recoveryLock.Lock(0, 1)
       Read-Host 'MinutesOn QAを起動し、起動回復画面が表示されたら、ここでEnterを押してください'
   } finally {
       $recoveryLock.Dispose()
   }
   ```

4. PowerShellが入力待ちの間にQA版を起動します。同じ保存先の使用中または排他取得の失敗が原因に表示され、「起動をやり直す」を選べることを確認します。
5. PowerShellに戻ってEnterを押し、一時ロックを解放します。QA版で「起動をやり直す」を選び、通常画面が開くことを確認します。

一時ロックはPowerShellを閉じても解放されます。会議保存先の選択を戻す場合は、QA版の設定で元の保存先を選び直します。

一時ロックでは、診断から通常起動へ戻る経路を確認します。データ復元の入口と、DBや音声の異常がある場合にデータを新しい保存先へ引き継ぐ処理は、隔離したネイティブテストで検証します。

## CIでの準備済み資材の再利用

全件CIは`app/prepare-windows.ps1 -Receipt <絶対パス>`で資材を先に準備し、native検証が終わってからQA／MSIXを作ります。receiptは同じActions jobの`RUNNER_TEMP`内に作成し、`MON_WINDOWS_PREPARATION_RECEIPT`でビルドフックへ渡します。ソース、実際のcompiler、run／attempt／job、unsignedモード、frontend・runtime・helper・通知文のサイズとSHAが一致する場合だけ再準備を省略します。

receiptを渡さない通常のQA／Storeビルドは従来どおり必要資材を準備します。指定済みreceiptの欠落・不整合は停止し、署名付きビルドではunsignedの準備結果を流用しません。Windows runtimeの一時構築・検証と正式packageへの置換が完了してからreceiptを確定します。実装は[準備証跡ツール](../../tools/mon_windows_preparation.py)にあります。

## 開発起動と通常版

ヘルパー準備後の開発起動は`npm.cmd --prefix app run tauri -- dev`です。通常版と同じ保存設定を使うため、実データと分けた検証はQAを使います。

通常版の署名／未署名EXEは[Windows配布](distribution-windows.md)、MSIXは[Store配布](distribution-store.md)、ネイティブ検査は[テスト運用](testing.md)を参照してください。
