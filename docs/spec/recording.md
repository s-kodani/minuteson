---
type: "Specification"
title: "録音と音声取り込み"
description: "録音範囲、入力選択、5秒テストと保存音声の取り込み。"
tags: ["capture", "audio"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/5fc01fea243b8bdc0c9995ae3c5d804c44a9bbbd/app/src-tauri/src/storage/audio_publication/cleanup.rs"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/crates/mon-core/src/capture/limits.rs"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src-tauri/src/recorder.rs"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src-tauri/src/audio_import.rs"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src/audio-import.tsx"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/verification/windows-macos-parity-audit-2026-10-26.md"
---

# 録音と音声取り込み

## 録音範囲と入力

マイクとPC音声を別トラックで取得します。全体録音は全出力機器の音声を対象とし、録音ヘルパー自身のプロセスツリーを除外します。アプリ選択録音は指定したアプリのプロセスツリーに限定し、取得失敗時に全体録音へ切り替えません。

マイクは自動選択と機器の固定指定を使い分けます。自動選択で会議アプリの使用入力を特定できない、または候補が複数ある場合はOS既定入力へ戻して通知します。固定指定では別機器へ無断で切り替えません。機器の切断・再取得や取得できなかった区間は状態・通知へ反映します。

macOSの取得はAVAudioEngine、ScreenCaptureKit／Core Audio Tapsを使い、WindowsはWASAPIを使います。権限・出力経路・会議アプリの実行構成により取得範囲が異なるため、最初に[5秒テスト](../guides/usage.md#録音する)で入力を確認します。

## テストと録音操作

「5秒録音して確認」の音声は一時確認用です。確認画面を閉じると実行と再生を終了してから削除します。PC側が無音でサンプルを受信しないことと、保存の失敗を区別します。

通常録音には固定の4時間停止期限を設けません。メイン画面が非アクティブになるとミニパネルを表示し、入力状態と停止操作を提供します。パネルを隠す操作と録音停止は別です。停止の確定後は、保存処理の終了を待ちます。

## 音声の保存

元の取得フレームと会議時刻の対応を保持し、欠損を正常な無音で埋めたり、時間を詰めたりしません。未確定の録音は保存完了と表示しません。保存待ちが1入力あたり20秒・16 MiB・4,096パケットのいずれかを超えると、音声を捨てて続けず停止して案内します。

録音中は連続CAFと復旧用の分割CAFを並行保存し、取り込み中は管理コピーから中間CAFを作成します。録音・取り込み後は16 kHz・モノラル・PCM16の会議全体の共通WAVを正式音声として保存し、再生・解析に使います。元音声・時計・欠損・ミックス条件に対応する内容を照合して再利用します。WAV完成後は不要な音声と途中ファイルを整理します。取り込み元のオリジナルは保持します。Windows・macOS共通の削除範囲と回復は[データ管理](storage.md#wav完成後の音声整理)を参照してください。

## 音声ファイルの取り込み

WAV・MP3・AAC/ALAC M4A・FLACを1件ずつ選択またはドロップして会議を作成します。拡張子だけでなく実際の形式を検査し、長さ・容量・保存先の空きと必要容量を表示します。会議名と取り込み時の用語を設定できます。

WAVは対応する整数／浮動小数点PCMを扱い、WAVEFORMATEXTENSIBLEの整数有効ビットは左詰めとして解釈します。FLACはCRC・PCMのMD5を含めて検査します。新規取り込みの復号中間音声はCAFを使用し、確定済みの既存音声を形式変更だけのために再生成しません。原本を検査した証拠と所有を保持し、取り込み元の利用者ファイルを削除しません。
