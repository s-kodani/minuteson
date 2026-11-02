---
type: "ADR"
title: "ADR 0006 Markdownを議事録の正本とする"
description: "議事録をMarkdownプレビューとテキスト編集へ絞り、生成は明示操作にする。"
tags: ["minutes", "markdown"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/minutes-simplification.md"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/verification/minutes-plain-production-ui-2026-10-12.md"
---

# ADR 0006 Markdownを議事録の正本とする

2026-11-02に現行コードと既存記録から整理したADRです。原決定日はこの整理日と区別します。

## 背景

当初の構造化議事録・根拠管理・候補採用・リッチ編集は、生成後の文書を利用者が調整する操作に対して複雑でした。

## 決定

Markdownを正本とし、生成・プレビュー・プレーンテキスト編集・保存・出力を提供します。再生成は確認後に現在のMarkdownを置き換え、旧版の自動バックアップや直前版復元を作りません。

## 理由

生成後は利用者が本文を編集する使い方へ集約し、本文と別の構造化項目・根拠状態を同期する操作をなくすためです。

## 影響

必要な旧本文は事前出力か会議バックアップで保持します。プレビューの音声区間は所定のMarkdown記法から扱い、文字起こし変更で本文を自動修正しません。

## 参照元

- [議事録仕様](../spec/minutes.md)
- [基本操作](../guides/usage.md)
