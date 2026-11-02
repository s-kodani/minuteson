---
type: "ADR"
title: "ADR 0001 ローカル処理とOS別連携"
description: "会議の解析をPC内で行い、音声取得と実行環境をOS別に実装する。"
tags: ["local-ai", "native"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src-tauri/tauri.macos.conf.json"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/prepare-windows.ps1"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/probes/macos-capture"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/probes/windows-capture"
---

# ADR 0001 ローカル処理とOS別連携

2026-11-02に現行コードと既存記録から整理したADRです。原決定日はこの整理日と区別します。

## 背景

WindowsとmacOSで会議の録音・解析を行います。音声取得API、権限、実行ファイル、GPUの利用方法はOSごとに異なります。

## 決定

モデル準備後の解析はローカルで行い、Rust／Tauriの共通操作からOS別ヘルパーを呼び出します。モデルの初回取得と配布用の署名・公証は、それぞれ明示した通信経路を使います。

## 理由

会議本文・保存音声を外部推論サービスへ送ることなく、OS固有の取得方式を利用できる構成です。

## 影響

モデル容量と処理資源は端末が負担します。録音範囲・権限・機器変更とCPU／GPUの動作はOSごとに確認します。「外部通信なし」とは説明しません。

## 参照元

- [録音仕様](../spec/recording.md)
- [モデル仕様](../spec/models.md)
- [構成](../spec/architecture.md)
