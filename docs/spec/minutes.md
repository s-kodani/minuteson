---
type: "Specification"
title: "Markdown議事録の生成と編集"
description: "議事録の正本、生成・再生成・保存と長文処理の動作。"
tags: ["minutes", "markdown", "generation"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/minutes-simplification.md"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src/minutes-audio-state.ts"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src/minutes-prompt-settings.tsx"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src-tauri/src/generation"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/verification/minutes-model-maximum-allocation-2026-10-15.md"
---

# Markdown議事録の生成と編集

## 正本と操作

議事録の正本はMarkdownです。生成結果をプレビューし、プレーンテキストとして編集・保存・Markdown出力できます。高度な書式エディター、構造化項目編集、候補採用、根拠発言の管理パネル、直前版の復元は提供しません。

文字起こしやモデル設定を変更しても保存済みの議事録を自動で書き換えません。「議事録を生成」から明示的に再生成する際は置換を確認します。処理中に行った人間の後続編集は無断で上書きしません。

再生成時に旧版の自動バックアップは作りません。必要な内容は事前にMarkdown出力または会議のバックアップで保存します。手動保存・生成の結果を確認できない場合は未確定状態を保持し、成功したことにしません。

## 音声区間のプレビュー

Markdown内の所定の音声区間記法から、プレビューに再生操作を表示します。例えば`[該当音声](audio:00:01:00-00:01:30)`は1分から1分30秒までを表します。時刻には小数点以下6桁まで指定できます。記法の構文と検証は[パーサー](../../app/src/minutes-markdown-state.ts)を参照してください。内部の発言IDや引用の確認状態は管理しません。音声が削除済み・欠損の場合は再生できると推測しません。

## 長文と実行予算

指示文を含む入力とモデルの能力上限、利用可能メモリから処理枠を決めます。長い入力は分割して抽出・統合し、先頭だけを残して省略しません。登録されたモデルごとの計数方式と最大入力・出力・共有コンテキストに従います。

GPUもCPUで扱える処理枠を使います。GPU試行に失敗した場合は解放後のメモリを確認してCPUへ切り替え、固定した枠を確保できなければ必要量を通知します。開始済みジョブの固定条件は維持します。未完成の出力は保存せず、保存済みの人間の本文を保持します。

モデルは[全体設定](models.md)で選びます。議事録の指示文設定は新しく開始する生成に使い、開始済みの処理へ遡って適用しません。内容の正確さは元音声・文字起こしと照合して確認してください。
