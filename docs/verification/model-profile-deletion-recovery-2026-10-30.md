# 削除済みモデルを参照するプロファイルの読み込み回復

## 発生条件と原因

WindowsのMON QAで、選択中の議事録モデルQwen3.5-9Bの本体を削除した後、起動時に「プロファイルと配置モデルの構成が一致しません」と表示され、導入済みモデル一覧も表示できなくなった。

対象DBはschema 36で、`PRAGMA quick_check` は `ok`。配置記録・成功した互換性確認・モデル適用履歴は残っており、削除完了記録も存在した。プロファイル検証が、履歴上の配置成功ではなく、削除を反映した現在の利用可否を構成の整合性と比較していた。画面は配置一覧とプロファイルをまとめて読み込むため、プロファイルの失敗によって一覧全体が表示されなかった。

## 修正

- プロファイルの履歴検証は `model_placements::load_history` を使用し、明示的な本体削除後も設定を読み込めるようにした。配置・設定・SHA・互換性証拠の検証は維持する。
- 表示用の現在状態には本体削除を反映し、削除済みモデルを利用可能・構成完了とは判定しない。
- 適用プレビュー・適用結果の再確認・新規ジョブのモデル選択は、別途現在の利用可否を確認する。削除済み本体は実行対象にできない。
- 別工程のモデル再適用や、削除済みモデルの置き換えでは、以前の適用履歴を保持したまま新しい版を追加する。

DBの直接修復・設定の初期化・モデルの再取得は行っていない。実データの検証にはSQLiteの読み取り専用接続から作成したコピーを使用する。

## 検証

- TypeScriptのモデル表示・選択・適用状態の単体検査: 33件成功。
- 画面の型検査・本番ビルド: 成功。
- Chromiumの `simple-models` スイート: 成功。
- Windowsのモデル削除後・再起動・モデル置き換えの回帰テスト: 成功。
- 実データのDBコピーを使う読み取り専用診断: 成功。文字起こし・話者分離は設定と現在の互換性記録が一致し、削除済みの議事録モデルだけが利用不可になることを確認。
- 既存のWindowsモデル削除検査: 21件成功、明示実行用2件ignored。
- Windows QAインストーラー: 生成成功。同梱録音ヘルパーのCTest 8件成功、ビルド元ソース一致と配布先コピーのSHA-256一致を確認。既存環境へのインストール・起動確認は未実施。

初回のモデル修正時点（下記main取り込み前）の配布物: `output/windows-qa/MON-QA-setup.exe`。SHA-256: `fdcff40e06c4fd85e6455e4c1fe5dc076dd1c994b1c35e32c0b6d11b6c2a61d3`。

ビルド記録: `output/windows-qa-work/20261030T091707Z-c923e5f747b8420a90b955b7e78884d5-build.json`。

回帰テスト `deleted_selected_model_keeps_profiles_readable_and_allows_replacement_after_restart` は、一時的な合成モデル本体と記録済み試行証拠を使う。実エンジンは起動しない。選択中の議事録モデルを製品の削除経路で削除し、再起動後の一覧・プロファイル読み込み、履歴と設定の保持、削除モデルの再適用拒否、文字起こしモデルの再適用、新しい議事録モデルへの切り替えを確認する。

明示実行の `saved_profiles_can_be_inspected_without_opening_or_mutating_app_storage` は、指定されたDBコピーをSQLiteの読み取り専用・`query_only` で開く。通常のアプリ起動・回復処理を実行せずに、製品のプロファイル検証と表示状態の生成を確認できる。

## 最新main取り込み後の統合検証

`origin/main` の `9f49872`（進行中ジョブのバナーをコンパクト化し、詳細を開閉する変更）をfast-forwardで取り込み、モデル修正との競合がないことを確認した。追加の製品不具合は検出されなかった。

- Windows本体のRust全体検査: 標準のtest profileで955件成功、失敗0、明示実行用48件ignored。デバッグ検査・整数オーバーフロー検査を確認するguardも成功。
- 共通Rust: 28件成功。
- TypeScript: 全881件の初回実行で880件成功・IPC素材が必要な1件skip。Windowsネイティブ検査が生成した素材を指定した追加実行で、該当する3件すべてが成功した。
- Chromium: `compact-progress`、`operation-progress`、`meeting-mix-progress`、`notification-menu`、`model-deletion-admission`、`simple-models`、`toast-production`、`minutes-progress-toast`、`toast` の9スイートが成功。
- 画面の型検査・本番ビルド、IPC定義・モデルカタログの生成物チェック、Rust contracts 14件の一致、Rust整形、差分チェック: 成功。

Windows本体の実行コマンドは `cargo test --locked --target x86_64-pc-windows-msvc -p mon-desktop --lib -- --test-threads=8`。この全体検査には、必須のデバッグ検査が無効になるrelease profileを使わない。
