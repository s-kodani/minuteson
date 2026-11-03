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

旧名`mon.app`から移る場合はアプリバンドルだけを置き換え、Dock等の参照を確認します。名称変更だけのために会議・モデル・設定を再初期化しません。通常版とQA版の識別子は異なります。EXE版とStore版の移行にはバックアップ・追加復元を使い、DBを直接共有しません。

Storeの同一版・異なる署名からの切替は[ローカル検証手順](distribution-store.md#原本と同じ内容のローカル検証)に従います。

## 初回起動と確認

保存先の所有・配置・履歴とDB形式を検査します。対応する更新元とバックアップ形式は[保存仕様](../spec/storage.md#現行形式と対応する更新元)を正本とします。移行前コピーを保持し、非対応版や改変履歴を空DBで置き換えません。

Windows・macOSとも、更新後の初回起動で、最終WAVが完成した既存会議の専用作業領域に残る不要音声を整理します。正常なWAVは再生成せず、外部の取り込み元原本・文書・管理記録・未救出音声は保持します。整理が中断した場合は次回起動で残りから再開します。通常起動後に未完了ジョブや録音等の回復が進む場合があります。更新前の会議・本文・話者・Markdown・用語・モデル設定を照合し、許可された音声で再生・編集保存・再起動後の読戻しを確認します。議事録の旧版復元や候補採用は確認項目に含めません。

起動検証に失敗した場合はアプリ・DB・台帳を保全し、[障害対応](troubleshooting.md)で段階と診断を確認します。DB、WAL、台帳、設定の個別削除で回避しません。
