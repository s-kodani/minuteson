---
type: "ADR"
title: "ADR 0002 共通Rust処理と生成IPC契約"
description: "共通判定・ネイティブ境界・構造型を分離し、IPCとウィンドウ権限を照合する。"
tags: ["architecture", "ipc"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/Cargo.toml"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/tools/mon_contracts.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/scripts/gen-contracts.mjs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/verification/cross-platform-refactor-implementation-2026-10-29.md"
---

# ADR 0002 共通Rust処理と生成IPC契約

2026-11-02に現行コードと既存記録から整理したADRです。原決定日はこの整理日と区別します。

## 背景

OS間で同じ操作を提供するため、録音・モデル・データ操作の判定と、ファイル・プロセス等の差異を分ける必要があります。

## 決定

Rust workspaceでcore・native・contracts・capture FFIを共有し、Tauri本体から利用します。IPC登録・構造型・バリデーターとウィンドウ別の許可を生成・照合します。

## 理由

処理と契約の重複を減らし、OS差異や画面と本体の型・権限のずれを変更時に確認できるためです。

## 影響

共通処理の変更は両OSへ影響します。ネイティブ実行の成功は各OSで確認し、生成物の整合性検査と明示的な権限を維持します。

## 参照元

- [アーキテクチャ](../spec/architecture.md)
- [テスト運用](../guides/testing.md)
