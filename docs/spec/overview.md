---
type: "Specification"
title: "MinutesOnの製品概要"
description: "機能範囲と対応環境。"
tags: ["product"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/d251874f4b7d4a65238a1bb4248c89525f4f7a01/app/src-tauri/src/startup_window.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/tauri.conf.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/macos-distribution.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/6d0dedff432144125524e2a18584fb4ffba0ba5d/LICENSE"
---

# MinutesOnの製品概要

MinutesOnは会議の録音・文字起こし・話者分離・議事録生成をローカルで行うデスクトップアプリです。公開版は[GitHub Releases](https://github.com/s-kodani/minuteson/releases)で確認できます。実行ファイル・保存形式などの内部識別子には略称monを使います。

## 対応環境

| OS | 対象 |
| --- | --- |
| Windows | Windows 11 24H2以降、x64 |
| macOS | macOS 15以降、Apple Silicon |

モデル導入後のAI処理はPC内で実行します。初回のモデル取得は外部配布元への通信を伴い、モデル本体と取得素材を置く空き容量が必要です。ブラウザーだけで画面を表示した場合はデスクトップ接続がなく、録音・AI処理は利用できません。

## 機能

- [録音・音声取り込み](recording.md)：マイク・PC音声、5秒テスト、ミニパネル、保存音声の取り込み。
- [文字起こし・話者・用語](analysis.md)：自動解析、本文・話者の修正、声の登録と照合、共通・会議別辞書。
- [モデル管理](models.md)：全体設定での選択、ダウンロード、切替、導入済みモデルの自動整理。
- [議事録](minutes.md)：ローカル生成、Markdownプレビュー・編集・保存・出力。
- [データ管理](storage.md)：手動バックアップ、追加復元、保存先変更、削除、異常終了からの回復。

表示はモノトーン／パープルとシステム／ライト／ダークを独立して設定します。初期設定はモノトーン・システムです。保存した設定は補助ウィンドウと通知にも反映します。

メインウィンドウは起動時に1280 × 820論理ピクセルを上限とし、OSの枠を含めて画面の作業領域の90%以内に収まるよう縮小して中央に表示します。通常の最小サイズは960 × 640ですが、小さい画面では必要な寸法だけ最小値を下げます。表示倍率と作業領域を取得して毎回プロセス起動時に計算し、常駐からの再表示では手動変更したサイズ・位置を維持します。前回プロセスのサイズ・位置は保存しません。通常版・QA版、macOS・Windowsで同じ方針を使います。
