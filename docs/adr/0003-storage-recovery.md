---
type: "ADR"
title: "ADR 0003 保存資産の所有と復旧"
description: "SQLiteと管理音声を所有・内容・操作IDで照合し、不明な状態を保持する。"
tags: ["storage", "recovery"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/storage"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/bootstrap"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/verification/storage-startup-attention-2026-10-10.md"
---

# ADR 0003 保存資産の所有と復旧

2026-11-02に現行コードと既存記録から整理したADRです。原決定日はこの整理日と区別します。

## 背景

音声ファイルの確定とDB更新は異なる境界を持ち、異常終了・保存先変更・削除の途中でも既存データを保護する必要があります。

## 決定

会議情報をSQLite、保存音声を管理資産として扱います。正常完了した会議の正式音声は共通WAVとし、完成後の不要音声整理と専用領域の残骸の扱いは[ADR 0009](0009-completed-wav-cleanup.md)で定めます。所有・実体・ハッシュ・配置台帳・操作IDを照合し、確定できない操作は結果不明として保持します。起動検証に失敗した場合は通常操作を開始しません。

## 理由

ファイルの存在や画面応答だけでは保存・削除の確定を判断できず、同じ操作の再開と不明な資産の保護が必要なためです。

## 影響

診断画面は自動修復や空DBへの置換をしません。回復・取消・削除は所有する範囲だけに限定し、起動中のDBへ外部のSQLite接続を追加しません。

当初の「データ復元」は、全対象を回収・検証して元の保存先を初期化した後、回収したデータを再登録する方式でした。現在は、確認できたデータを新しい保存先へ引き継ぐ方式に切り替えました。旧版ですでに開始した操作は、旧初期化ジャーナルに従って起動時に処理します。[現行のデータ復元](../spec/storage.md#データ復元)

確定済みのモデル構成は、再構成先で準備中の状態から構成部品を登録し、その後に元の状態へ確定します。通常の構成登録と同じ順序・不変条件を使い、元DBの状態や制約を変更しません。

## 参照元

- [保存仕様](../spec/storage.md)
- [障害対応](../guides/troubleshooting.md)
