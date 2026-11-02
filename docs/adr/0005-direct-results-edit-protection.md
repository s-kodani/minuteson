---
type: "ADR"
title: "ADR 0005 解析結果の直接保存と編集保護"
description: "結果を発言一覧へ直接保存し、明示再生成時の競合と後続処理を管理する。"
tags: ["analysis", "editing"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/verification/simple-models-and-transcription-2026-10-12.md"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/verification/cross-platform-refactor-implementation-2026-10-29.md"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src-tauri/src/analysis_flow.rs"
---

# ADR 0005 解析結果の直接保存と編集保護

2026-11-02に現行コードと既存記録から整理したADRです。原決定日はこの整理日と区別します。

## 背景

録音・取り込み後は発言をすぐ確認・修正できる必要があります。長い解析中や応答喪失時にも、人間の編集を失わず後続処理を再開できることが必要です。

## 決定

解析結果は候補採用を挟まず発言一覧へ直接保存します。再生成は利用者の確認を経て行い、開始後の編集との版・内容の一致を照合します。結果の保存と後続の話者補完・声照合の意図を原子的に記録します。

## 理由

候補比較の操作を減らしつつ、処理結果で後続の手動編集を上書きしたり、再起動で後続処理を重複・欠落させたりしないためです。

## 影響

全体モデル・用語の変更で既存本文を自動更新しません。再送は同じ操作IDと固定条件で確認し、失敗・結果不明・競合を成功扱いにしません。

## 参照元

- [解析と編集](../spec/analysis.md)
- [保存と復旧](../spec/storage.md)
