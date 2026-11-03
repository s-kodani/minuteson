---
type: "Specification"
title: "AIモデルの導入と実行"
description: "全体モデル設定、取得・切替・削除とASRバックエンドの契約。"
tags: ["models", "asr", "runtime"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/model-catalog/model-definitions.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/src/transcription/asr.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/development/asr-adapters.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/tools/mon_windows_runtime.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/tools/mon_model_catalog.py"
---

# AIモデルの導入と実行

## 導入と切替

「設定 → ローカルAI」で文字起こし・話者分離・議事録生成のモデルを選びます。未導入なら「ダウンロードして使用」、導入済みなら「使用」で切り替えます。取得、容量・SHA-256照合、配置、CPUでの短い動作確認、設定適用を一つの準備操作で行います。実行中のAI処理とは競合させず、必要なら完了を待ちます。

選択欄の近くに利用条件の概要を表示し、「利用条件を見る」から同梱の全文と出典を確認できます。表示だけでは取得通信やモデル実行を始めません。導入・切替・VAD追加・ランタイム更新の明示操作ごとに、モデルと実行エンジンの条件を確認してから準備を開始します。確認済み状態は保存しません。取消・状態読戻し・中断回復では新しい確認や処理の自動再実行を行いません。既に使用中のモデルと開始済み解析は継続します。

LFM3のLFM Open License v1.0には、関連会社等を含む組織の定義と年商1,000万米ドルを基準とした商用利用条件があります。原文の第1条・第5条を確認してください。本体のMITとは別の条件です。年商や組織情報は収集しません。公式モデルと一致しない導入済みコピーでは条件未確認と導入元情報を表示し、既存の互換性・利用可否の判定を維持します。

モデルは全体設定で指定し、その後に開始するジョブへ適用します。開始済みジョブのモデル・実行ファイル・条件は変更しません。ローカルファイルからの手動導入や取得定義JSONの入力は提供しません。

## 候補と実行方式

| 用途 | モデル系列 |
| --- | --- |
| 文字起こし | Whisper、ReazonSpeech-k2-v2、Haas-ja-v1.0 |
| 話者分離 | Pyannote segmentationとERes2Net、標準／int8構成 |
| 議事録 | Qwen 2.5／3／3.5、Gemma 4 E4B IT、Phi-4-mini-instruct、LFM3 |

実際に選択できる量子化形式・OS別候補・容量・取得元・SHA・利用条件の正本は[モデルカタログ](../../app/model-catalog/)です。利用条件の対応は`model-license-policy.json`で重みと通知原文の容量・SHA・出典へ結び付け、既存の取得レシピとは分けて管理します。[同梱原文](../../third-party/model-licenses/README.md)はクレジットでも表示します。UIはその定義と同梱ランタイムから候補を構成します。候補総数や全ファイルのハッシュを文書へ複製しません。

Whisperや議事録エンジンでは対応するGPUを試行し、条件に従ってCPUへ切り替えます。macOSはMetal、Windowsは同梱CPU／Vulkanワーカーを使います。ReazonSpeechとHaasはCPUで動作します。CPUでの導入確認はGPU動作や認識品質の保証ではありません。

議事録はモデルの入力・出力・共有コンテキスト上限と、登録されたトークン計数方式を使います。未知の上限をモデル名から推測して実行しません。メモリ配分と長文処理は[議事録仕様](minutes.md)を参照してください。

## コピーと取得失敗

導入済みの同じモデルの複数コピーは1候補にまとめ、コピー数・合計容量と各コピーの管理操作を提供します。選択中であることだけでは削除を禁止せず、実際に競合する処理や状態取得失敗などの理由を表示します。

モデル本体の同一性はバックエンドと完全なモデルファイル集合から決めます。配布レシピやランタイム更新だけで本体の同一性を変えず、検証できた同じ重みは再利用します。由来不明・未完了・破損した素材を使用可能にしません。

準備の中断・応答喪失時は元の処理の停止と保存結果を照合します。設定画面を開いただけでは取得を開始しません。失敗の確定後は通常の導入・切替操作から再試行でき、成功した取得は再利用します。

## ASRを保守・拡張する場合

ASRは共通の`AsrAdapter`へ登録し、選択・準備・取消・チェックポイント・結果公開を共有します。バックエンドはビルド時に登録し、モデル名によるUIやジョブ分岐を追加しません。

- 既存バックエンドへのモデル追加は`model-definitions.json`に固定出典・容量・SHAを定義し、`python3 tools/mon_model_catalog.py`でプラットフォームカタログを再生成します。
- 新バックエンドはID・revision・worker・必須スロット・GPU/VAD能力・入力上限・辞書方式を宣言し、入力検証・推論・共通Segmentへの変換を実装します。
- 実行ファイルとモデルの役割・全構成を検査します。モデル本体は`asr`、付属資材は`asr_asset`、実行依存は`runtime_library`を使います。
- ReazonSpeechは最大30秒、Haasは最大29秒の復号区間を扱います。Haasの`encoder.data`はencoderの隣で名前を保ち、他のモデル資材と同じくハッシュを固定します。
- HaasのBPEヒントは境界あり／なしを可逆なworker専用シンボルへ変換し、公開前に元の語彙へ戻します。中間日本語ヒントと、ヒントなしのtoken・時刻・JSON一致の回帰を維持します。

ASR用資材の生成・出典は[第三者ASR資材](../../third-party/asr-assets/)に保持します。確認手順は[テスト運用](../guides/testing.md)を参照してください。
