---
type: "Guide"
title: "Microsoft StoreのMSIX配布"
description: "Store原本の生成・照合、同一内容のローカル検証と提出時の確認。"
tags: ["windows", "store", "msix"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/build-windows-store.ps1"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/windows-store.json"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/tools/prepare-windows-store-test.ps1"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/windows-store-test.md"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/verification/windows-store-dpi-process-audit-2026-11-01.md"
---

# Microsoft StoreのMSIX配布

## パッケージ生成

[Windowsビルド環境](build-windows.md)に、Windows SDKのx64 MakeAppxとMakePRIを追加します。リポジトリルートで実行します。

```powershell
powershell -NoProfile -File app/build-windows-store.ps1 -ValidateOnly
npm.cmd --prefix app run tauri:windows:store:build
```

Partner CenterのName・Publisher・PublisherDisplayNameは[Store設定](../../app/windows-store.json)に完全一致で指定します。アプリの版は共通Tauri設定を使い、MSIXは末尾0を付けます。Store用の末尾は0で予約し、更新では先頭3要素を増やします。

生成はソース記録、本体・ランタイム構築、CTest、ステージ作成、MakePRI索引、MakeAppx検証、全ペイロードのSHA照合、配布用コピー更新の順です。Store原本は未署名で作り、自前の公開署名を要求しません。

- 原本：`output/windows-store/MinutesOn_<version>.0_x64.msix`
- ソース・構築記録とステージ：`output/windows-store-work/<build-id>/`
- 検証：同ディレクトリの`source.json`と`build.json`

```powershell
python tools/mon_windows_msix.py verify --manifest 'output/windows-store-work/<build-id>/build.json' --package 'output/windows-store/MinutesOn_<version>.0_x64.msix' --require-current-source
```

`installedVerification`と`certification`は`pending`、`distributionReady`は`false`として生成します。実際の検証・認定は別に記録します。

## 原本と同じ内容のローカル検証

Windows 11 24H2以降の隔離検証環境を使います。Windows Serverの構築成功は対象クライアントOSへのインストール確認の代わりにはなりません。

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/prepare-windows-store-test.ps1 -BuildWorkspace '<変更のないビルドworktree>' -BuildManifest '<完了したbuild.jsonの絶対パス>' -ExpectedCommit '<完全なコミットSHA>' -StorePackage '<未署名原本MSIXの絶対パス>' -CreateCertificate
```

初回だけ`-CreateCertificate`を付けます。検証用証明書は有効期間90日、CurrentUser\Myの非エクスポート可能な鍵として保持し、次回は同じ鍵を再利用します。複数候補がある場合は`-CertificateThumbprint`で指定します。

準備ツールは原本をコピーして署名し、元の版数・マニフェスト・全ペイロードが一致することを検査します。出力は`output/windows-store-test/<準備ID>/`で、`test-manifest.json`の`payloadChanges`が空、`originalVersion`と`testVersion`が一致することを確認します。秘密鍵は出力しません。

準備だけでは信頼登録・インストール・アプリ停止・データ変更は行いません。初回の公開証明書の拇印を確認して検証端末のLocalMachine\TrustedPeopleへ登録します。アプリを正常終了してから署名済みコピーを`Add-AppxPackage -Path '<署名済みMSIX>' -ErrorAction Stop`で導入し、スタートメニューから確認します。

同じ版数で内容が違うパッケージを通常更新として使わず、データを保つ更新では同一Identityのまま版数を上げます。ローカル署名とStore再署名の同一版への切替が拒否される場合は、バックアップと別環境、または後続の高いStore版による切替を計画します。アンインストールやリセットで回避しません。

## データとアイコン

Store版の識別子は`jp.mon.desktop.store`です。EXE版・QA版と保存先・起動を分離し、移行にはアプリのバックアップと追加復元を使います。モデル本体は設定から取得し、録音ヘルパー・推論ランタイムは同梱します。WebView2 Evergreenが削除されている場合は修復が必要です。

背景なしタスクバーアイコンは既存SVGから`node app/scripts/generate-windows-unplated-icons.mjs`で再生成できます。生成済みPNGと各サイズ・テーマのMakePRI索引を同梱し、通常のMSIXビルド中に画像を再描画しません。

## 提出と実機確認

起動と権限、実マイク・PC／アプリ音声、取り込み、モデル導入・CPU／Vulkan推論、常駐と全プロセス終了、EXE／QAとのデータ分離、更新と保持を確認します。Partner Centerへは検証用署名コピーではなく未署名原本を提出します。

名称・説明・スクリーンショット、録音と声の特徴量・ログの保存／削除、モデル取得の通信を説明するプライバシーポリシー、第三者条件を準備します。存在を確認していない公開URLを記載しません。

WACK実行記録にはDPIのInfrastructureエラーと任意のプロセス検出指摘が残っています。静的な名前一致だけで実際のシェル起動や誤検知を断定せず、同梱プログラムの呼出し元と実動作を確認します。検査を通すためにバイナリ文字列やWindowsAppsのACLを変更しません。過去の詳細はフロントマターの出典、現在の公開状況は[製品概要](../spec/overview.md#配布と評価の確認状況)を参照してください。
