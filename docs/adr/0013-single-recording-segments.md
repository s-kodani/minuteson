---
type: "ADR"
title: "ADR 0013 録音音声を分割CAFへ一本化する"
description: "録音中の重複書込みを廃止し、取得・変換と同期保存を分離する。"
tags: [capture, storage, recovery]
status: stable
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/844cb35b7bb166dda26b541cf19da168e4cfe280/probes/windows-capture/raw_segments.h"
  - resource: "https://github.com/s-kodani/minuteson/blob/844cb35b7bb166dda26b541cf19da168e4cfe280/probes/macos-capture/Sources/MONMacOSCapture/RawRecordingSegments.swift"
---

# ADR 0013 録音音声を分割CAFへ一本化する

## 決定と理由

参照元は変更前の実装です。連続CAFと復旧用CAFへの重複保存、および取得と同期保存の待ちの関係を見直します。

Windows・macOSとも、16 kHz・モノラル・PCM16の復旧用分割CAFを録音中の音声の正本とします。同じPCMの連続CAFへの書込みを廃止し、音声本体の書込み量を半減します。変換と時計対応は変更しません。

約4秒分のPCMをまとめて保存します。取得と変換は、同期保存・読戻し検証・receipt・tip公開を行う専用ワーカーから切り離します。ディスク待ちで大容量の取得PCMが滞留する構造を避け、状態確認と停止受付を継続できるようにします。

確定済み音声の救出条件と通常時の確定間隔は維持します。未保存音声は入力ごとに最大120秒までメモリで保持します。取得から確定までの全段を計上し、保存確定後にだけ予算を解放します。保存先の停止中は未確定部分が増えるため、異常終了で失われる音声が増える場合があります。容量上限・書込み不能では、確定済み音声を保持して停止します。

## 停止と復旧

停止後は、取得済み音声と変換末尾を排出し、保存ワーカーの完了を確認してから入力の終了を公開します。分割音声を検証済みの可逆FLACへ束ねて既存の正式WAV作成へ渡します。連続CAFは再生成しません。正常終了と部分救出の公開計画を分け、正常終了の時計・終了証拠を部分救出から推測しません。

途中の音声はWAVの同期・検証・DB確定まで保持します。確定後の整理には[ADR 0009](0009-completed-wav-cleanup.md)を適用します。既存の完成WAVを再生成せず、旧中断録音は既存の形式アダプターで検証します。

## 検証

旧経路と新経路の最終WAV、時計対応、欠損区間を比較します。保存遅延と各確定段階の強制終了を注入し、入力処理の継続、待機上限、確定済みprefixだけの救出、再実行時の重複防止を確認します。

[録音仕様](../spec/recording.md)・[保存仕様](../spec/storage.md)
