---
type: "Guide"
title: "OKFドキュメントの管理"
description: "文書の分類、OKFメタデータ、ADRと仕様を更新するためのルール。"
tags: ["documentation", "okf", "maintenance"]
status: "stable"
sources:
  - resource: "https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/development-plan.md"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/implementation-status.md"
---

# OKFドキュメントの管理

## 読む順序と正本

[ルート目次](../index.md)から利用手順・現行仕様・ADRへ進みます。現行動作は仕様、実行方法はガイド、継続する設計判断と理由はADRを正本とします。コードと文書に不一致がある場合はコード・設定・テストを確認し、文書を修正します。過去資料が現行コードより優先されるとは扱いません。

`docs`全体をOpen Knowledge Format v0.2のバンドルとします。[公式仕様](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md)に従い、MarkdownとYAML、相互リンクだけで管理します。読み取りにSDKやクラウドは不要です。

## 文書の作り方

一つのテーマを一つの文書で説明し、ファイル名は英小文字とハイフンにします。`index.md`と`log.md`は予約名です。目次は配下の文書への相対リンクと短い説明を列挙します。ルート目次だけに`okf_version: "0.2"`を置き、子の目次にはフロントマターを付けません。更新履歴はGitで管理し、`log.md`は作りません。

コンセプト文書の共通項目は次のとおりです。`type`のみがOKFの常時必須項目で、残りはこのプロジェクトの規約です。

```yaml
---
type: Specification
title: 文書の題名
description: 内容を説明する一文。
tags: [storage, backup]
status: stable
sources:
  - resource: https://github.com/s-kodani/mon/blob/<commit>/app/src-tauri/src/storage.rs
---
```

`type`は`Specification`・`Guide`・`ADR`を使います。`status`は`draft`・`stable`・`deprecated`です。内容への相対リンクはGitHubでそのまま辿れる形にします。本文で参照するファイルと、フロントマターの根拠は用途を分けます。

`sources`には確認したコード・設定・過去の判断記録を記載します。GitHubの出典は完全なコミットSHAへ固定します。削除する旧資料は整理前コミットのURLで参照できます。日付・理由・検証結果を推測して補いません。作成者を記録する場合は`generated`、独立して内容確認した場合だけ`verified`を使い、実際のactorとタイムゾーン付き日時を指定します。

## ADR

番号と要旨をファイル名に付け、「背景・決定・理由・影響・参照元」の順で書きます。初期ADRは2026-11-02に現行コードと既存記録から整理したもので、原決定日が不明なものは遡って採用日を作りません。

採用方針を変更する場合は新しいADRを追加し、旧ADRと新ADRを相互にリンクします。最新仕様は採用済みの内容へ更新し、変更前後の全文を併記しません。利用者の操作変更・データ形式・OS境界・ジョブ条件・配布方式が変わる場合は関連ガイドも同時に更新します。

## 更新と確認

仕様・操作・制約の説明先を一つにし、候補一覧・ハッシュ・版番号をコードから無制限に複製しません。文書変更時はYAML、空でないtype、目次形式、リンク・アンカー、コード参照の実在、目次からの到達を確認します。

構成や処理の流れはMermaidのコードブロックで記述し、アーキテクチャの説明と一緒に更新します。実行時のデータ受渡し、ライブラリ依存、コード生成の矢印は、各図で意味を明示します。

開発計画・進捗日誌・工程ごとのテスト件数や一時パスは通常の文書へ戻しません。過去の経緯は`git log -- docs`やコミット固定の出典を参照します。配布の確認状況と性能保証は[製品概要](../spec/overview.md#配布と評価の確認状況)に集約し、確認が進んだ時点で更新します。

文書も実装と同じPRで更新し、更新先または不要の理由をPRへ記載します。`make check-docs`が上記の形式・参照・到達性を検査します。本文と実装の意味の一致は実装担当者が確認します。[開発フロー](development-workflow.md)と[DevSecOps運用](devsecops.md)に従ってください。
