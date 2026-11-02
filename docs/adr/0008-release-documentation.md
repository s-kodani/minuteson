---
type: "ADR"
title: "ADR 0008 完成版の位置付けとOKF文書管理"
description: "1.0.1を完成版として扱い、現行仕様・手順・判断理由をOKFへ集約する。"
tags: ["release", "documentation", "okf"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/README.md"
  - resource: "https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md"
---

# ADR 0008 完成版の位置付けとOKF文書管理

2026-11-02に現行コードと既存記録から整理したADRです。原決定日はこの整理日と区別します。

## 背景

開発中の計画・実装状況・検証日誌に過去の仕様と残件が蓄積し、現行動作を読むために多数の変更履歴を辿る状態でした。

## 決定

2026-11-02のユーザー指定により、現行1.0.1を正式リリースとして説明します。文書はOKF v0.2で最新仕様・利用／保守手順・ADRへ集約し、情報を移した旧資料は削除します。過去資料はGit履歴とコミット固定の参照で辿ります。

## 理由

利用者と保守担当者が現在の動作と必要な操作を短い導線で確認でき、設計判断の理由も失わない構成にするためです。

## 影響

アーカイブ、開発完了までの残工数一覧、工程ごとの検証日誌は増やしません。署名・公証・Store公開や性能評価の確認状況は事実どおり製品概要に残します。今回の整理でアプリのコード・API・保存形式・版番号は変更しません。

## 参照元

- [製品概要](../spec/overview.md)
- [文書管理](../guides/documentation.md)
- [ドキュメント目次](../index.md)
