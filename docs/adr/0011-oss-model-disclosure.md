---
type: "ADR"
title: "ADR 0011 モデル利用条件の確認とStore成果物の出自検証"
description: "モデルごとの条件表示とオフライン原文確認、Store提出前の成果物出自検証。"
tags: ["license", "models", "security", "release"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/6d0dedff432144125524e2a18584fb4ffba0ba5d/LICENSE"
  - resource: "https://github.com/s-kodani/minuteson/blob/6d0dedff432144125524e2a18584fb4ffba0ba5d/app/model-catalog/model-license-policy.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/6d0dedff432144125524e2a18584fb4ffba0ba5d/app/src-tauri/src/model_license.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/6d0dedff432144125524e2a18584fb4ffba0ba5d/app/src/model-inspection.tsx"
  - resource: "https://github.com/s-kodani/minuteson/blob/6d0dedff432144125524e2a18584fb4ffba0ba5d/tools/mon_store_provenance.py"
---

# ADR 0011 モデル利用条件の確認とStore成果物の出自検証

## 背景

アプリ本体と第三者資産には、それぞれの利用条件が適用されます。モデル導入では、本体のライセンスだけでモデルを利用できると誤認しない表示が必要です。Store提出では、編集可能なRelease本文や未認証のmanifestを成果物の出自として信頼できません。

## 決定

自作コード・文書・自作画像はMITとし、著作権表示は`Copyright (c) 2026 s-kodani`です。第三者のLICENSE・NOTICEとモデル・実行エンジンの条件はそれぞれ保持します。

モデルの重み・通知原文の容量とSHA・出典に結び付いた、アプリ所有の利用条件カタログを追加します。既存の取得レシピ・パッケージ同一性は変更しません。選択モデルの概要を常時表示し、オフラインの原文閲覧と、導入・切替・VAD追加・ランタイム更新前の確認を専用ダイアログで提供します。確認欄は毎回未選択で開き、実行直前に選択・パッケージ・条件を読み戻します。確認履歴は保存せず、DB移行・年商等の収集は行いません。公式カタログとの一致を確認できないパッケージは条件未確認と表示します。

閲覧・出典コピー・取消は準備を開始しません。中断回復は結果の照合だけを行い、新しい確認や処理の自動再実行を行いません。既に使用中のモデルと開始済み解析は継続します。同じ通知原文をアプリのクレジットにも表示します。

Store変更の直前にmanifest実バイトのGitHub attestationを検証し、現在の数値リポジトリID、mainの正式ワークフロー、署名証明書が示すrun・attempt、実公開ステップの成功を要求します。ワークフロー版とビルド対象のSHAを区別し、署名済みreceipt、タグ・main包含、MSIXの容量とSHAを照合します。Release本文の状態は認証の根拠にしません。PendingCommitの再開にも同じ検証を要求し、その後の提出状況確認は成果物の保持期間と切り離して継続します。

## 理由

モデル・条件・実行対象を照合することで、確認表示と異なる処理の開始を防ぎます。確認履歴を持たない方式により、保存形式と既存モデルの動作を維持します。Storeは認証した成果物から提出する境界に絞り、既存の状態確認と中断再開を保持します。

## 影響

第三者資産の再配布条件は個別に確認します。依存通知本文、SDK・CRT等の条件、実機・署名・Store公開の未確認事項を保持し、未確認の項目は`licenseReview: not_verified`と`distributionReady: false`を維持します。バイナリ配布・Store自動提出の有効化は、各経路の検査と配布条件の確認を踏まえて判断します。

## 参照元

- [AIモデルの導入と実行](../spec/models.md)
- [利用条件原文の管理](../../third-party/model-licenses/README.md)
- [DevSecOpsの検査と運用](../guides/devsecops.md)
- [ADR 0010 範囲別PR検査とmain成果物の公開](0010-devsecops-release.md)
