---
type: "Specification"
title: "MinutesOnの製品概要"
description: "公開前の機能範囲、対応環境と配布・評価の確認状況。"
tags: ["product", "release"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/tauri.conf.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/macos-distribution.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/verification/windows-store-dpi-process-audit-2026-11-01.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/verification/m4-01-macos-native-qa-preparation-2026-10-07.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/6d0dedff432144125524e2a18584fb4ffba0ba5d/LICENSE"
---

# MinutesOnの製品概要

MinutesOnは会議の録音・文字起こし・話者分離・議事録生成をローカルで行うデスクトップアプリです。1.0.0は公開前の開発版です。実行ファイル・保存形式などの内部識別子には略称monを使います。

## 対応環境

| OS | 対象 |
| --- | --- |
| Windows | Windows 11 24H2以降、x64 |
| macOS | macOS 15以降、Apple Silicon |

モデル導入後のAI処理はPC内で実行します。初回のモデル取得は外部配布元への通信を伴い、モデル本体と取得素材を置く空き容量が必要です。ブラウザーだけで画面を表示した場合はデスクトップ接続がなく、録音・AI処理は利用できません。

## 機能

- [録音・音声取り込み](recording.md)：マイク・PC音声、5秒テスト、ミニパネル、保存音声の取り込み。
- [文字起こし・話者・用語](analysis.md)：自動解析、本文・話者の修正、声の登録と照合、共通・会議別辞書。
- [モデル管理](models.md)：全体設定での選択、ダウンロード、切替、導入済みコピーの管理。
- [議事録](minutes.md)：ローカル生成、Markdownプレビュー・編集・保存・出力。
- [データ管理](storage.md)：手動バックアップ、追加復元、保存先変更、削除、異常終了からの回復。

表示はモノトーン／パープルとシステム／ライト／ダークを独立して設定します。初期設定はモノトーン・システムです。保存した設定は補助ウィンドウと通知にも反映します。

## 配布と評価の確認状況

このリポジトリでは現在、インストーラーを配布していません。配布経路と品質評価の確認状況を個別に記録します。確認記録は次のとおりです。

| 項目 | 確認状況 |
| --- | --- |
| macOS | QAアプリの構築・アドホック署名を確認。2026-11-01にDeveloper ID証明書と公証認証を準備。Developer ID署名・公証済みDMGの成功、Gatekeeper、通常版の実機更新は未確認 |
| Windows | MSIXの生成・内容照合と検証用コピーによるWACK実行を確認。Store提出・認定・一般公開の完了記録はない。WACKの総合結果はWARNINGで、DPIのInfrastructureエラーと任意検査の指摘が残る |
| 品質・性能 | 短い実モデル統合や合成入力の検査記録あり。16GB端末・10話者、60分処理、4時間録音、各機器・会議アプリでの網羅的な評価は未確認 |
| ライセンス | 自作コード・文書・自作画像はMIT。依存通知本文・SDK由来条件等の配布審査は未完了。依存ライブラリ・モデルの条件は個別に適用される |

ソースからのビルドは[macOS](../guides/build-macos.md)／[Windows](../guides/build-windows.md)、配布物の構築・検証は[macOS配布](../guides/distribution-macos.md)、[Windows EXE](../guides/distribution-windows.md)、[Microsoft Store](../guides/distribution-store.md)の手順を参照してください。性能や文字起こしの精度は端末・モデル・音声に依存し、上表の未測定条件について保証値を設けません。
