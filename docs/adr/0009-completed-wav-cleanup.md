---
type: "ADR"
title: "ADR 0009 完成WAVを残して作業音声を整理する"
description: "WAV確定後の専用作業領域を整理し、外部原本・文書・管理記録・未救出音声を保持する。"
tags: [storage, audio, cleanup]
status: stable
sources:
  - resource: "https://github.com/s-kodani/mon/blob/48fb086d5747961a17f1b4ec7a8c9d34b1ab2824/app/src-tauri/src/storage/audio_publication/cleanup.rs"
  - resource: "https://github.com/s-kodani/mon/blob/48fb086d5747961a17f1b4ec7a8c9d34b1ab2824/app/src-tauri/src/storage/audio_publication/cleanup/relocation.rs"
  - resource: "https://github.com/s-kodani/mon/blob/5fc01fea243b8bdc0c9995ae3c5d804c44a9bbbd/app/src-tauri/src/storage/audio_cleanup_upgrade.rs"
---

# ADR 0009 完成WAVを残して作業音声を整理する

## 背景

録音・取り込みは、連続CAF・復旧用分割CAF・管理コピー・復号音声を一時保存します。正式WAVが完成した後もこれらや強制終了時の途中ファイルが残ると、会議音声より大きな容量を使います。

## 決定

Windows・macOSとも、正式WAVの保存・検証・DB確定後に不要な管理音声を削除します。正常完了した録音・取り込みとWAV出力の専用作業領域では、空・破損した途中ファイル等、管理記録と対応付けられない残骸も整理します。対象を台帳に固定し、実体・ハッシュを再確認してから削除し、中断後は残りから再開します。

旧FLAC変換の出力領域は、入力トラックまたは実体を検証した変換計画から特定します。保存先移動後の再開では、既存の移動履歴に検証済みコピーの実体対応を保存し、整理台帳の対象と照合します。内容が同じという理由だけで別実体を削除しません。

外部オリジナル、最終WAV、登録文書、通知・時刻・管理記録は保持します。未完了録音・部分救出の作業領域は不明ファイルの一括整理に含めません。解析用一時音声は既存のライフサイクルに従います。録音形式・二重保存・取り込みの中間CAFは維持します。

## 理由

音声変換と取得時計を変えずに、正常完了後の冗長な保存容量を回収できます。専用領域への限定と確定済みWAVの検証により、原本・未救出音声と役目を終えた残骸を区別できます。

## 影響

初回起動時に従来の整理済み会議も確認します。正常なWAVは再生成せず、初回整理は後で切り離せる専用モジュールに置きます。DB schemaとバックアップ形式は変更しません。[ADR 0003](0003-storage-recovery.md)の不明資産の保持方針に対する例外は、正常完了した専用作業領域に限定します。

## 参照元

- [保存仕様](../spec/storage.md#wav完成後の音声整理)
- [録音・取り込み仕様](../spec/recording.md)
- [障害対応](../guides/troubleshooting.md#wavは保存できたが音声の整理に失敗する)
