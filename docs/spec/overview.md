---
type: "Specification"
title: "MinutesOnの製品概要"
description: "公開前の機能範囲、対応環境と配布・評価の確認状況。"
tags: ["product", "release"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/d251874f4b7d4a65238a1bb4248c89525f4f7a01/app/src-tauri/src/startup_window.rs"
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

メインウィンドウは起動時に1280 × 820論理ピクセルを上限とし、OSの枠を含めて画面の作業領域の90%以内に収まるよう縮小して中央に表示します。通常の最小サイズは960 × 640ですが、小さい画面では必要な寸法だけ最小値を下げます。表示倍率と作業領域を取得して毎回プロセス起動時に計算し、常駐からの再表示では手動変更したサイズ・位置を維持します。前回プロセスのサイズ・位置は保存しません。通常版・QA版、macOS・Windowsで同じ方針を使います。

## 配布と評価の確認状況

2026-11-11に、コミット`d34c198e92c95e9ce9ea8d20c6db2d6225e9d0fd`を対象とする1.0.0のdry runが成功しました。インストーラーはGitHub Actionsの成果物として保存され、`v1.0.0`のGitHub Releaseは下書きのままです。配布経路と品質評価の確認状況は次のとおりです。

| 項目 | 確認状況 |
| --- | --- |
| 自動検査 | 対象コミットのmain CI・セキュリティ検査、dry run内の全件CI・セキュリティ検査が成功。Windows・macOSのブラウザー／ネイティブ回帰テストと、代表モデルを使ったランタイム検証を確認 |
| macOS | 通常版アプリとDMGのDeveloper ID署名、Apple公証、チケット添付・検証が成功（`notarization: tickets_validated`）。ダウンロードしたDMGからのGatekeeper確認、実機での権限・録音・モデル導入、通常版の更新とデータ保持は未確認 |
| Windows | 未署名EXEとStore用未署名MSIXの生成・成果物収集、MSIXの内容照合、リンク済みEXEのDPI宣言検査が成功。対象クライアントOSでの今回の配布物の導入・実機操作・更新、Store提出・認定・一般公開は未確認。過去の検証用コピーによるWACKの総合結果はWARNINGで、DPIのInfrastructureエラーと任意検査の指摘は今回のCIでは解消確認していない |
| 配布証跡 | 全プラットフォームの成果物統合・ハッシュ照合、SBOM・チェックサム・GitHubのビルド来歴証明の生成が成功。GitHub Releaseの公開とStore提出は未実施 |
| 品質・性能 | 短い実モデル統合や合成入力の検査記録あり。16GB端末・10話者、60分処理、4時間録音、各機器・会議アプリでの網羅的な評価は未確認。CI上のランタイム検証は通常版の実機操作確認とは別に扱う |
| ライセンス | 自作コード・文書・自作画像はMIT。依存通知本文・SDK由来条件等の配布審査は未完了。依存ライブラリ・モデルの条件は個別に適用される |

確認根拠は[main CI](https://github.com/s-kodani/minuteson/actions/runs/42958731033)、[mainのセキュリティ検査](https://github.com/s-kodani/minuteson/actions/runs/42958731072)、[1.0.0のdry run](https://github.com/s-kodani/minuteson/actions/runs/42958883715)です。dry runでは[macOS署名・公証ジョブ](https://github.com/s-kodani/minuteson/actions/runs/42958883715/job/130358948004)と[成果物統合・証明生成ジョブ](https://github.com/s-kodani/minuteson/actions/runs/42958883715/job/130359659284)が成功し、後者の公開ステップは`dry_run=true`の条件でスキップされました。

添付予定の配布物は[verified-release-preview](https://github.com/s-kodani/minuteson/actions/runs/42958883715/artifacts/13833696360)、Store提出用のMSIXは[release-store](https://github.com/s-kodani/minuteson/actions/runs/42958883715/artifacts/13832676423)に保存されています。これらは30日保持のActions成果物で、保持期限は2026-12-11（UTC）です。確認時点で`RELEASE_PUBLISH_ENABLED=false`、`STORE_PUBLISH_ENABLED=false`を維持しています。

このdry runの証跡は、`rustix`を1.1.6へ更新する[依存更新PR #67](https://github.com/s-kodani/minuteson/pull/67)より前の固定SHAでの記録です。依存更新後のSHAに対して、全件CI・パッケージ生成・署名・公証・証明生成までを含むdry runは未実施です。

署名・公証と自動検査の成功だけでは、実機確認・ライセンス審査を確認済みにしません。macOSの構築記録では`runtimeQA`・`gatekeeper`・`licenseReview`・`manualUpdate`が`not_verified`、MSIXの構築記録では`installedVerification`・`certification`が`pending`であり、いずれも`distributionReady`は`false`のままです。

ソースからのビルドは[macOS](../guides/build-macos.md)／[Windows](../guides/build-windows.md)、配布物の構築・検証は[macOS配布](../guides/distribution-macos.md)、[Windows EXE](../guides/distribution-windows.md)、[Microsoft Store](../guides/distribution-store.md)の手順を参照してください。性能や文字起こしの精度は端末・モデル・音声に依存し、上表の未測定条件について保証値を設けません。
