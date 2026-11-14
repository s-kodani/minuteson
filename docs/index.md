---
okf_version: "0.2"
---

# MinutesOn ドキュメント

公開前のMinutesOn 1.0.0を基準に、利用と保守に必要な情報をまとめています。各文書はOpen Knowledge Format v0.2で管理します。

## 初めて使う

- [使い方](guides/usage.md) — モデルの準備、録音・取り込み、修正、議事録出力。
- [製品概要](spec/overview.md) — 機能範囲、対応環境、配布と評価の確認状況。
- [障害対応](guides/troubleshooting.md) — 起動診断、録音・解析・モデル準備の失敗。

## 保守と開発

- [GitHubでの開発とリリース](guides/development-workflow.md) — PR、文書更新、必須検査、main成果物の検証、手動公開と承認後のStore送信。
- [DevSecOpsの検査と運用](guides/devsecops.md) — セキュリティ検査と初期設定・障害対応。

- [現行仕様](spec/index.md) — 録音・解析・モデル・議事録・保存とアーキテクチャ。
- [利用と保守の手順](guides/index.md) — OS別ビルド、配布、更新、テストと文書管理。
- [ADR](adr/index.md) — 継続する設計判断、その理由と影響。

## 設計図

- [アーキテクチャ図](spec/architecture.md) — 構成、処理フロー、共通ライブラリとコード生成。
- [ER図](spec/data-model.md) — 主要テーブルのキーと関連。
- [画面遷移図](spec/screens.md) — メイン画面、確認ダイアログ、補助ウィンドウ。

## 文書の管理

- [OKFドキュメントの管理](guides/documentation.md) — 分類、メタデータ、出典、更新ルール。

旧仕様・開発計画・進捗・検証日誌はGit履歴にあります。現行仕様とADRの出典は、確認時点のコミットへ固定しています。
