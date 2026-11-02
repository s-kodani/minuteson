---
type: "Specification"
title: "保存とバックアップと復旧"
description: "会議・音声・モデル・ログの保存、追加復元と対応する更新形式。"
tags: ["storage", "backup", "recovery", "privacy"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src-tauri/src/current_formats.rs"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src-tauri/src/storage/schema_upgrade.rs"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src-tauri/src/storage/backup/archive.rs"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src-tauri/src/internal_logs.rs"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src-tauri/src/bootstrap"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src/startup-reset-form.tsx"
---

# 保存とバックアップと復旧

## 保存する情報

主要テーブルのキーと関連は[データモデルとER図](data-model.md)を参照してください。

会議情報・解析・編集・処理履歴をSQLiteで管理し、保存音声はFLACを基本に扱います。取得中の原PCM／CAF、解析用WAV、処理途中の記録は用途に応じて保持し、所有と確定状態を検証します。会議の時間欠損を隠して音声を連結しません。

会議データとAIモデルの保存先は別です。設定から会議保存先を変更してもモデルは移動しません。通常版・QA版・Windows Store版はアプリ識別子で保存設定を分離します。具体的な保存先はアプリの表示とOSのアプリ領域を確認し、他端末の固定パスを流用しません。

## バックアップと追加復元

アプリのバックアップ操作で`.monbackup`を作成し、復元では現在のデータを置き換えずに会議を追加します。管理音声・本文・話者・用語・登録した声・必要な処理由来を保持し、モデル本体、端末固有の設定・機器、内部ログは含めません。復元された未完了処理を無断で再開しません。

準備・検査・追加確定の各段階で、参照・内容ハッシュ・所有・容量を照合します。同一の復元バッチの再送で二重追加せず、利用者が別の復元として選ぶ場合は別会議として扱います。既存の会議・モデル選択・設定は保持します。

会議削除と音声だけの削除を区別します。削除対象・範囲を確認し、対象の所有と同意を照合してから進めます。途中の不明なファイルや他の保存先まで一括削除しません。登録済み声の特徴量と内部ログには、それぞれ独立した保存・削除の操作／期限があります。

## 現行形式と対応する更新元

| 対象 | 現行／対応範囲 |
| --- | --- |
| DB | schema 37。新規初期化と、履歴を検証した34／35／36からの限定的な前進更新 |
| バックアップ出力 | format 6・schema 37 |
| 追加復元 | format 6・schema 37／36、およびformat 5・schema 35 |

35→36の前に`before-followup-intents.sqlite3`、36→37の前に`before-pcm-import.sqlite3`を残します。過去のmigrationとchecksumは変更せず、過去会議に新しい解析意図を一律追加しません。その他の旧形式を一律変換しないため、未対応版・改変・履歴欠落は拒否します。正本は[更新処理](../../app/src-tauri/src/storage/schema_upgrade.rs)と[バックアップ形式検査](../../app/src-tauri/src/storage/backup/archive.rs)です。

## 異常時と排他

会議DB・配置台帳・起動設定・ファイルの所有と実体を確認し、起動検証に失敗した場合は通常操作を開始せず診断を表示します。診断の再読込みはメモリに保持した結果を読み、保存先の再検査・修復・削除を行いません。

診断画面には、別の明示操作として「起動をやり直す」と「データを初期化」があります。初期化の確認では対象の保存先と退避の有無を選びます。会議・音声・文書・モデル・設定が対象となり、退避を選んでもモデル本体・取得途中のモデルは削除されます。退避フォルダーは通常のバックアップ形式とは異なります。[画面遷移](screens.md#起動と診断)と[障害対応](../guides/troubleshooting.md)を確認してください。

処理の所有・操作ID・入力・内容ハッシュ・版を固定し、結果不明の操作を別IDで無断再送しません。解析結果と後続処理意図を原子的に記録します。会議削除・モデル削除・全体保守は専用の所有を区別し、競合する実処理との相互排他を維持します。取消・アプリ終了時は所有する実処理の終了を待ちます。

起動中の会議DBへ診断用の外部SQLite接続を追加しません。アプリ内で確認するか正常終了後に調べます。復旧手順は[障害対応](../guides/troubleshooting.md)を参照してください。

## ログとプライバシー

「内部ログ」で処理経過とAIエンジンの診断を確認できます。自動更新の一時停止、日時・処理・ID・本文検索、表示中ログのコピーに対応します。会議データと別に最大30日・合計200 MiBを保存し、上限を超えると古いログを削除します。会議本文やローカルパスが含まれる場合があり、会議削除後も保持期限まで残ります。ログはバックアップ対象外です。
