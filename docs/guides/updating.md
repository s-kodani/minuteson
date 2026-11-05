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

## 初回起動と確認

保存先の所有・配置・履歴とDB形式を検査します。公開前の開発版では、現行の保存形式だけを受け入れます。対象外のDBは元のファイルを保持して起動診断を表示します。現在の形式は[保存仕様](../spec/storage.md#現行形式)を参照してください。

起動検証に失敗した場合はアプリ・DB・台帳を保全し、[障害対応](troubleshooting.md)で段階と診断を確認します。DB、WAL、台帳、設定の個別削除で回避しません。
