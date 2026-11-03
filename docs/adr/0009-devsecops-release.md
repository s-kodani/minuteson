---
type: "ADR"
title: "ADR 0009 PR検査と下書きからの自動公開"
description: "重大な指摘を停止条件とし、タグをReleases画面で管理して検証後に公開する。"
tags: ["security", "release", "github"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/e789585e6d40b4e20ba8719b0e118e9c289f0d0e/.github/workflows/security.yml"
  - resource: "https://github.com/s-kodani/mon/blob/e789585e6d40b4e20ba8719b0e118e9c289f0d0e/.github/workflows/release-controller.yml"
  - resource: "https://github.com/s-kodani/mon/blob/e789585e6d40b4e20ba8719b0e118e9c289f0d0e/.github/workflows/release-build.yml"
---

# ADR 0009 PR検査と下書きからの自動公開

## 背景

既存のOS別CIと配布物照合を活かし、保守中の依存・秘密情報・コード・文書の検査と自動配布を統合します。

## 決定

PRでは常時完了する最終ゲートを設け、High以上と重大度不明の脆弱性、秘密情報、検査失敗で停止します。例外は根拠・担当者・最長30日の期限を必須とします。1名保守では他者承認を必須にしません。

タグはGitHub Releases画面で指定します。公開開始を指定した下書きを定期検出し、ソースSHAを固定します。全検査と署名・公証・照合後に成果物を添付して公開します。Windows EXEは当面未署名です。Storeは初回手動公開後の更新をAPIで自動提出します。

## 理由

下書きの作成・編集ではActionsのreleaseイベントが発生しないため、公開前に検査を完了するための定期検出を採用します。既存の署名・配布照合を再利用し、ビルドと資格情報を扱う処理を分離します。

## 影響

実機確認は公開の必須条件にせず、既存の未確認項目を確認済みに書き換えません。関連文書は実装と同じPRで更新します。公開設定と認証の初期登録は所有者が行い、未設定の自動配布は無効です。本体ライセンスやアプリ内更新方式は変更しません。

## 参照元

- [GitHubでの開発とリリース](../guides/development-workflow.md)
- [DevSecOps運用](../guides/devsecops.md)
- [既存の配布と文書管理の判断](0008-release-documentation.md)
- [GitHubのreleaseイベント](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#release)
