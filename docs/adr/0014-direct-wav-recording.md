---
type: ADR
title: "ADR 0014 録音と取り込みを直接WAVへ保存する"
description: "単一WAVへの録音中の合成と、確定証拠・旧音声の救出・互換性の境界。"
tags: [capture, storage, recovery, migration]
status: stable
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/6c53589c71264ad0754adb230524224b3e5b8cc1/docs/adr/0013-single-recording-segments.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/6c53589c71264ad0754adb230524224b3e5b8cc1/app/src-tauri/src/storage/recordings/flac.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/6c53589c71264ad0754adb230524224b3e5b8cc1/app/src-tauri/src/storage/audio_publication/writer.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/6c53589c71264ad0754adb230524224b3e5b8cc1/app/src-tauri/src/storage/imports/legacy.rs"
---

# ADR 0014 録音と取り込みを直接WAVへ保存する

## 背景

[ADR 0013](0013-single-recording-segments.md)では、録音中の正本を分割CAFへ一本化しました。出典の変更前実装では、その音声を停止後にFLACへ集約してから正式WAVを作成し、取り込みにも復号CAFを保持していました。完成音声がWAVであるため、中間音声の符号化・復号・書込みを経由せず保存できる構成へ変更します。

## 決定

通常録音はマイクとPC音声を入力ごとに16 kHz・モノラル・PCM16へ正規化し、共通Rustミキサーで録音中から単一WAVへ合成します。区間ごとの取得入力数で平均し、受信したゼロ値PCMと未受信を区別します。入力の接続世代・連番とFIFOの処理完了を確認し、取得・変換・合成と同期保存を分離します。既存の合成待ちと無通信猶予を使い、追加の一律4秒の整列待ちは設けません。

単一WAVの所有実体を予約し、約4秒ごとにPCMの同期・読戻し照合、範囲ハッシュと時計・取得／欠損範囲の記録、確定位置の公開を行います。80バイトのヘッダー領域を確保し、必要時にRIFFの予約領域をRF64へ切り替えます。停止後は末尾とヘッダーを確定し、同じWAVを検証してDBへ登録します。全体の再ミックスやPCM本体の再書込みは行いません。

取り込みは復号・正規化したWAVをそのまま正式音声へ登録し、5秒テストは入力別の短いWAVで個別再生を維持します。共通化はPCM・WAV処理を中心とし、取り込み・テストへ録音用の永続チェックポイントを一律導入しません。

既存の所有検証・ファイル公開・DB確定・整理を再利用します。汎用の移行／ジョブ基盤や、遅着音声用の別WAVは新設しません。既存の公開型と関数を維持し、FFI ABI 1への関数追加で両OSの処理を共有します。

## 理由

完成音声を録音中に作成することで、中間CAF・FLACと停止後の全体変換を省けます。安全性はファイル拡張子へ依存させず、所有実体、確定範囲、同期保存、読戻しとハッシュの照合で保ちます。保存待ちの上限は引き継ぎ、ディスク停止を取得済みPCMの時刻異常や先行確定へ変換しません。

## 影響

録音manifestはversion 5、取り込みmanifestはversion 4へ更新します。旧未完了CAF・FLACは専用の救出・移行入口で検証し、新たな中間CAF・FLACを作らず正式WAVへ渡します。旧音声はWAVの同期・検証・DB確定まで保持します。完成済みWAVは再生成せず、旧録音の救出では従来の合成条件を維持します。

取り込み確定条件がCAFだけに固定されていたため、会議DBはschema 39から40へ専用移行します。既存のSQLチェックサムと実構造が一致する場合だけ、トリガー1件と版の記録を原子的に更新し、表・列・既存データは維持します。バックアップformat 8は維持し、旧schema 39と新schema 40を明示的に扱います。新版での旧データ利用と新版同士のバックアップ往復を対象とします。旧1.0.7で新しいWAV・manifest・バックアップを読むことは保証しません。登録した声の短いWAVや、外部原本・モデル・人手編集の保存形式は変更しません。方向別の条件は[保存仕様](../spec/storage.md#107との互換性)に集約します。

正常停止と部分救出の終了証拠を区別し、異常終了後は確認できる確定範囲だけを修復・公開します。WindowsのBluetooth再接続や受信遅延で、受信済みPCMの時刻が既に合成した位置より古くなる場合は、取得側で区間全体の時刻を補正して保持します。元の時刻・補正先を診断へ残し、保存済みWAVと共通ミキサーの順序検査は維持します。不正なPCM、所有・連番の不整合、容量超過、実際の保存失敗は引き続き異常として扱います。配布候補にはWAV化と既存の性能改善を同じSHAから含め、全件CI・実機・更新確認の実施状況を個別に記録します。

## 参照元

- [録音仕様](../spec/recording.md)・[保存仕様](../spec/storage.md)
- [共通WAV・ミックス・確定記録](../../crates/mon-core/src/capture/wav.rs)
- [単一WAVの検証と公開](../../app/src-tauri/src/storage/recordings/wav.rs)
- [旧録音の直接WAV救出](../../app/src-tauri/src/storage/recordings/legacy_wav.rs)
- [録音保存の検査](../guides/testing.md#単一wav録音と旧音声の救出)
