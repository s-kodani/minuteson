---
type: "Guide"
title: "アプリ更新とデータの保持"
description: "通常版・QA・Storeの更新と、保存形式の検査・データ保持の確認。"
tags: ["update", "backup", "migration"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/409e99b3b0f2b02a80d838debf6813fc3cc3ae91/app/src-tauri/src/storage/audio_publication/cleanup.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/manual-update-macos.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/storage/schema_upgrade.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/tauri.windows.conf.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/windows-store-test.md"
---

# アプリ更新とデータの保持

## アプリ内の更新確認

WindowsのMicrosoft Store版とmacOS通常版は、正常な起動準備の完了後と24時間ごとに新版を確認します。「設定 → このアプリについて」からも確認できます。成功した確認結果は30分間再利用し、通信失敗を「更新なし」と表示しません。Windows EXE版、QA版、開発実行は対象外で、確認通信を行いません。

新版がある場合は、録音状態に関係なくメイン画面の各画面へバナーを表示します。入力や録音・解析を中断せず、バナーを閉じるとその起動中は再表示しません。アプリ情報画面では引き続き結果を確認できます。失敗した再確認では前回の更新情報と確認時刻を保持し、失敗した旨を表示します。

Store版はStore APIで現在の利用者に利用可能な更新を確認し、「Microsoft Storeを開く」で更新とダウンロードの画面へ進みます。配信の遅れやStore側のキャッシュがあるため、結果だけで最新版・取得済み・適用済みとは断定しません。macOSは公開GitHub Releasesの安定版を確認し、Apple Silicon向けDMGがある新版のリリースページへ案内します。

確認時はMicrosoft StoreまたはGitHubへ通信します。macOSの確認は認証なしの公開APIを使い、会議・音声・モデル情報は送信しません。この機能からダウンロード・更新の適用・アプリ終了・再起動は実行しません。

## 更新前

録音・テスト・取り込み・解析・書き出し・データ操作の終了を待ち、アプリでバックアップを保存します。会議名・件数、発言と話者、Markdown、用語と全体モデル設定、選択している保存先を確認します。起動診断や未確定の保守操作がある場合は先に状態を確認します。

ウィンドウを閉じただけで常駐している場合があるため、アプリの明示終了で保存と停止を待ちます。強制終了を通常の更新手順にしません。

## アプリの置換

| 配布経路 | 更新方法 |
| --- | --- |
| macOS通常版 | 終了後にDMGの`MinutesOn.app`をApplicationsへ置く |
| Windows EXE | 同じ通常版の新しいインストーラーで更新する |
| Windows Store | Microsoft Storeの更新を使う |
| QA | 同じQA識別子のアプリ／インストーラーを正常終了後に更新する |

通常版とQA版の識別子は異なります。EXE版とStore版の移行にはバックアップ・追加復元を使い、DBを直接共有しません。

Storeの同一版・異なる署名からの切替は[ローカル検証手順](distribution-store.md#原本と同じ内容のローカル検証)に従います。

Store版のパッケージは`UpdateWhileInUse=defer`を宣言します。Windows 11 24H2以降では、録音・解析中に限らず起動中の更新適用を延期する方針です。トレイ常駐も起動中に含まれます。作業と保存の完了後に明示終了し、Storeの再試行や次回起動時の適用を確認してください。OS更新やシステム再起動の終了を防ぐ設定ではありません。この設定を初めて導入する旧版からの更新は、既存版での保護を実機確認前に保証しません。

## 初回起動と確認

保存先の所有・配置・履歴とDB形式を検査します。公開前の開発版では、現行の保存形式だけを受け入れます。対象外のDBは元のファイルを保持して起動診断を表示します。現在の形式は[保存仕様](../spec/storage.md#現行形式)を参照してください。

起動検証に失敗した場合はアプリ・DB・台帳を保全し、[障害対応](troubleshooting.md)で段階と診断を確認します。DB、WAL、台帳、設定の個別削除で回避しません。
