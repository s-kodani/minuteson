---
type: ADR
title: "ADR 0015 リリースの確認状況を文書へ記載しない"
description: "版ごとのリリース準備・配布・評価の確認状況を製品概要から外し、公開状況はGitHub Releasesで確認する。"
tags: [release, documentation]
status: stable
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/f3702c6feec077f59ebcd711d866c138eb5736db/docs/adr/0008-release-documentation.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/f3702c6feec077f59ebcd711d866c138eb5736db/docs/spec/overview.md"
---

# ADR 0015 リリースの確認状況を文書へ記載しない

## 背景

[ADR 0008](0008-release-documentation.md)では、署名・公証・Store公開や性能評価の確認状況を製品概要に残すとしました。その結果、製品概要には版ごとのリリース準備と公開記録が積み重なり、リリースのたびに版番号の更新と合わせて記録を書き換える必要がありました。製品の機能範囲と対応環境を説明する仕様文書に、版ごとの作業記録が大半を占める状態になっていました。

## 決定

2026-11-26のユーザー指定により、版ごとのリリース準備・配布・評価の確認状況を製品概要を含む通常の文書へ記載しません。公開状況とダウンロードは[GitHub Releases](https://github.com/s-kodani/minuteson/releases)で確認します。製品概要は機能範囲と対応環境の説明に絞ります。

## 理由

公開状況の正本はGitHub Releasesにあり、文書へ同じ内容を複製すると更新漏れや食い違いが起きるためです。リリース作業のたびに仕様文書を書き換える必要もなくなります。

## 影響

製品概要の「配布と評価の確認状況」節を削除し、同節への参照をGitHub Releasesか該当なしへ置き換えました。過去の確認記録はGit履歴とコミット固定の出典で辿ります。署名・公証・Store提出・実機確認の手順そのものは、配布ガイドを正本として維持します。ADR 0008のうち、現行仕様・手順・判断理由をOKFへ集約する方針は変更しません。

## 参照元

- [ADR 0008 完成版の位置付けとOKF文書管理](0008-release-documentation.md)
- [製品概要](../spec/overview.md)
- [文書管理](../guides/documentation.md)
