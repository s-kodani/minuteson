---
type: "ADR"
title: "ADR 0004 全体モデル設定と実行条件固定"
description: "モデル選択を全体設定へ集約し、開始済み処理の資材と条件を固定する。"
tags: ["models", "jobs"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/verification/simple-models-and-transcription-2026-10-12.md"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src-tauri/src/transcription/asr.rs"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/probes/inference-worker/README.md"
---

# ADR 0004 全体モデル設定と実行条件固定

2026-11-02に現行コードと既存記録から整理したADRです。原決定日はこの整理日と区別します。

## 背景

会議ごとのモデル設定や取得・配置・適用の個別画面は操作を複雑にします。一方、処理の再開には元の入力・モデル・実行条件の再現が必要です。

## 決定

全体設定で選択・ダウンロード・使用を行い、準備操作の中で照合・配置・CPU確認・適用を進めます。ジョブは開始時のモデル・実行ファイル・入力・条件を固定し、変更は新しい処理から適用します。

## 理由

通常操作を簡潔にしながら、異常終了や再試行で別のモデル・ワーカーへ無断に置き換わることを防ぐためです。

## 影響

手動モデル取り込みや会議別選択は提供しません。同じ重みを検証して再利用できますが、固定済みのworkerが欠損したジョブへ別バイトの実行ファイルを代入しません。

## 参照元

- [モデル仕様](../spec/models.md)
- [推論ワーカー](../../probes/inference-worker/README.md)
