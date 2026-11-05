---
type: "ADR"
title: "ADR 0007 限定的な前進更新と追加復元"
description: "実装が明示的に対応する旧形式だけ更新・復元し、既存データを保持する。"
tags: ["migration", "backup"]
status: "deprecated"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/storage/schema_upgrade.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/storage/backup/archive.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/manual-update-macos.md"
---

# ADR 0007 限定的な前進更新と追加復元

2026-11-02に現行コードと既存記録から整理したADRです。原決定日はこの整理日と区別します。

公開前の保存方針は[現行形式](../spec/storage.md#現行形式)を参照してください。

## 背景

初期開発では旧形式互換を不要としていましたが、現行コードはschema 34／35／36から37への更新と一部の旧バックアップの受入れを持っていました。過去の互換性不要の指定を現在の動作として書くと矛盾します。

## 決定

2026-11-03の簡素化でバックアップは現在内容だけを扱うformat 8に変更し、旧format 5／6／7の受入れを終了します。DB schema 38と既存DBの前進更新は保持します。現在のコードが検査する更新元とバックアップ形式を仕様に明示します。移行前コピーと既存migrationのchecksumを保持し、復元は既存データを置換しない追加方式とします。その他の旧形式を推測で変換しません。

## 理由

検証済みの更新経路で現行データを保護し、広範な自動変換や、失敗時の初期化による情報消失を避けるためです。

## 影響

対応範囲の正本はコードと保存仕様です。名称変更のための再初期化や、通常版・QA・Store版のDB直接共有は行いません。形式を変更する場合は対応元・復元・中断時の保持を確認します。

## 参照元

- [保存形式](../spec/storage.md#現行形式)
- [更新手順](../guides/updating.md)
