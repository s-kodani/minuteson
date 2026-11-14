---
type: "ADR"
title: "ADR 0010 範囲別PR検査とmain成果物の公開"
description: "重大な指摘を停止条件とし、mainで正式ビルドを検証し、同じ成果物を手動公開、承認後にStoreへ送信する。"
tags: ["security", "release", "github"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/.github/workflows/security.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/.github/workflows/release-controller.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/.github/workflows/release-build.yml"
---

# ADR 0010 範囲別PR検査とmain成果物の公開

## 背景

既存のOS別CIと配布物照合を活かし、保守中の依存・秘密情報・コード・文書の検査と自動配布を統合します。

## 決定

PRでは常時完了する最終ゲートを設け、High以上と重大度不明の脆弱性、秘密情報、検査失敗で停止します。例外は根拠・担当者・最長30日の期限を必須とします。1名保守では他者承認を必須にしません。

PRは変更範囲別に検査し、mainのコード変更で全件検証と正式配布物作成を一度行います。同じ配布物をローカル確認した後、固定SHAのRelease下書きへ自動添付し、担当者がPublish releaseします。新規タグは公開時に作成し、公開処理で再検査・再ビルドはしません。Windows EXEは当面未署名です。Storeは初回手動公開後、Actions Environmentの承認後に同じMSIXをAPI送信します。定期照会は既存の提出IDに対するGETだけです。

## 理由

一度作成し検証した成果物を昇格させることで、手元の確認と公開物を一致させます。下書きの作成・編集ではActionsのreleaseイベントが発生しないため、保存済み成果物の添付に定期検出を使います。既存の署名・配布照合を再利用し、ビルドと資格情報を扱う処理を分離します。

## 影響

担当者はmainの成果物で最終ローカル確認して公開します。機械的な重複チェックは追加せず、既存の未確認項目を確認済みに書き換えません。同一SHAの日次ネイティブ全件実行を廃止し、手動fullと日次脆弱性検査を残します。関連文書は実装と同じPRで更新します。公開設定と認証の初期登録は所有者が行い、未設定の自動配布は無効です。本体ライセンスやアプリ内更新方式は変更しません。

## 参照元

- [GitHubでの開発とリリース](../guides/development-workflow.md)
- [DevSecOps運用](../guides/devsecops.md)
- [既存の配布と文書管理の判断](0008-release-documentation.md)
- [GitHubのreleaseイベント](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#release)

- [GitHubのimmutable releasesと下書き運用](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases)
- [Releaseフォームへの固定コミットの入力](https://docs.github.com/en/repositories/releasing-projects-on-github/automation-for-release-forms-with-query-parameters)
- [Environmentの承認](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/review-deployments)
