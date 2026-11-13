---
type: "Specification"
title: "録音と音声取り込み"
description: "録音範囲、入力選択、5秒テストと保存音声の取り込み。"
tags: ["capture", "audio"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/409e99b3b0f2b02a80d838debf6813fc3cc3ae91/app/src-tauri/src/storage/audio_publication/cleanup.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/crates/mon-core/src/capture/limits.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/recorder.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/audio_import.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src/audio-import.tsx"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/verification/windows-macos-parity-audit-2026-10-26.md"
---

# 録音と音声取り込み

## 録音範囲と入力

マイクとPC音声を別トラックで取得します。全体録音は全出力機器の音声を対象とします。Windowsは各出力機器のWASAPIループバックを直接取得し、共通のQPC時刻で重ね合わせます。macOSは録音ヘルパー自身のプロセスツリーを除外します。アプリ選択録音は指定したアプリのプロセスツリーに限定し、取得失敗時に全体録音へ切り替えません。

Windowsの全体録音は、Teamsデスクトップ版の音声がプロセス単位のループバックでは無音になる[Microsoftのサンプルへの報告](https://github.com/microsoft/Windows-classic-samples/issues/414)を踏まえ、アプリ選択録音とは取得方式を分けます。出力機器間の合成待ちは250 ms、バッファは10 msのブロック100個までとし、音声が届いていない区間を無音で補いません。出力機器の接続・切断は録音入力の切断と再取得として扱います。

マイクは自動選択と機器の固定指定を使い分けます。自動選択で会議アプリの使用入力を特定できない、または候補が複数ある場合はOS既定入力へ戻して通知します。固定指定では別機器へ無断で切り替えません。機器の切断・再取得や取得できなかった区間は状態・通知へ反映します。

macOSの取得はAVAudioEngine、ScreenCaptureKit／Core Audio Tapsを使い、WindowsはWASAPIを使います。権限・出力経路・会議アプリの実行構成により取得範囲が異なるため、最初に[5秒テスト](../guides/usage.md#録音する)で入力を確認します。

## テストと録音操作

「5秒録音して確認」の音声は一時確認用です。確認画面を閉じると実行と再生を終了してから削除します。PC側が無音でサンプルを受信しないことと、保存の失敗を区別します。

通常録音には固定の4時間停止期限を設けません。メイン画面が非アクティブになるとミニパネルを表示し、入力状態と停止操作を提供します。パネルを隠す操作と録音停止は別です。停止の確定後は、保存処理の終了を待ちます。

## 音声の保存

元の取得フレームと会議時刻の対応を保持し、欠損を正常な無音で埋めたり、時間を詰めたりしません。未確定の録音は保存完了と表示しません。保存待ちが1入力あたり20秒・16 MiB・4,096パケットのいずれかを超えると、音声を捨てて続けず停止して案内します。

録音中は、取得した音声を入力ごとに16 kHz・モノラル・PCM16へ変換し、同じPCMを連続CAFと復旧用の分割CAFへ並行保存します。取り込み中も、原本の管理コピーから復号した小さな区間を順次変換して、同じ低容量形式の中間CAFを直接作成します。高音域・ステレオ情報は中間音声に保持しません。マイクとPC音声の区別、取得時刻と欠損区間は保持します。録音・取り込み後は16 kHz・モノラル・PCM16の会議全体の共通WAVを正式音声として保存し、再生・解析に使います。元音声・時計・欠損・ミックス条件に対応する内容を照合して再利用します。WAV完成後は不要な音声と途中ファイルを整理します。取り込み元のオリジナルは保持します。Windows・macOS共通の削除範囲と回復は[データ管理](storage.md#wav完成後の音声整理)を参照してください。

録音中の音声本体はマイク・PC音声と各復旧用を合わせて約460.8 MB/時、取り込みの中間CAFと正式WAVはそれぞれ約115.2 MB/時です（十進表記。メタデータ等は別途）。取り込みの必要容量には原本の管理コピー、中間CAF、正式WAVと安全余裕を含めます。原本の容量表示は変換前の値を使います。

## 音声ファイルの取り込み

WAV・MP3・AAC/ALAC M4A・FLACを1件ずつ選択またはドロップして会議を作成します。拡張子だけでなく実際の形式を検査し、長さ・容量・保存先の空きと必要容量を表示します。会議名と取り込み時の用語を設定できます。

WAVは対応する整数／浮動小数点PCMを扱い、WAVEFORMATEXTENSIBLEの整数有効ビットは左詰めとして解釈します。FLACはCRC・PCMのMD5を含めて検査します。新規取り込みの復号中間音声は16 kHz・モノラル・PCM16のCAFを使用し、原本の形式・長さ・フレーム数と保存後の形式・フレーム数を別々に検証します。確定済みの既存音声を形式変更だけのために再生成しません。原本を検査した証拠と所有を保持し、取り込み元の利用者ファイルを削除しません。
