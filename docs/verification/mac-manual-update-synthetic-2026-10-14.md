# Macアプリ置換時の現行データ保持（分離QA）

2026年10月14日。実会議と通常QAを使わず、識別子 `jp.mon.desktop.qa.manualupdatefixture20261014` の分離QAで確認した。会議保存先は `/Users/skodani/Library/Application Support/jp.mon.desktop.qa.manualupdatefixture20261014/data`。初期開発の旧DB形式の移行、署名済み配布物の導入、Gatekeeperは対象にしていない。

## 操作と結果

1. 版0.1.0の `MON Update Fixture QA.app` を起動し、29.610375秒の合成WAVをファイル選択から取り込んだ。会議名は `QA-manual-update-fixture-20261014`。モデル未導入のため自動文字起こしは失敗通知となったが、会議と音声は保存された。
2. 議事録に `# 更新検証用議事録`、`- 合成音声の会議`、`- アプリ置換後も本文を保持` を手入力して明示保存した。通常終了・冷起動後も会議一覧の1件を表示した。
3. アプリの「バックアップを作成」で `/private/tmp/mon-manual-update-qa.OBbwxK/backups/MON-update-fixture-before.monbackup` を保存した。画面表示は会議1件・2 MiB、ファイルのSHA256は `3bb80f7fd28c895601092de2e38bcbc6744a5dc426b7ee6c8a72a44618371f4b`。作成後に通常終了した。復元検査はこの操作では行っていない。
4. 同じ識別子・コードで版番号だけ0.1.1にしたQAアプリを構築し、`codesign --verify --deep --strict` を通過した。旧アプリを `MON Update Fixture QA 0.1.0.app` として退避し、元の `/private/tmp/mon-manual-update-qa.OBbwxK/fixture-before/MON Update Fixture QA.app` に0.1.1を配置した。アプリデータ領域は移動・削除していない。
5. 差し替えたパスの実行プロセスを確認した。会議一覧に同じ1件、詳細に保存音声、議事録プレビューに上記3行が表示された。正常終了後、DB+WALの停止コピーで `PRAGMA integrity_check` は `ok`、`PRAGMA foreign_key_check` は0行。差し替え前後の全SQLite `.dump` は同じSHA256 `aaba3ac15e529e3c09856773d775a0eee11eb955b690baff0988de39d7a4c720` だった。保存された原本WAVとFLACのSHA256も差し替え前後でそれぞれ `ccc3316f158ff9691267f176c32039f51eb49d5faf79b21908f43e6f328bfa93`、`76ffa261fbf0b534248872320caa84118dc0d3692d91676fe1b3c84c1884952b` と一致した。

## 境界

同一コードのメタデータ上の版差による、同一パスでのアプリ本体置換と現行DB31の読戻しを確認した。スキーマ変更を伴う更新、通常版 `jp.mon.desktop` の既存会議・モデル設定・保存先、実際の音声再生、失敗中断時の更新、Developer ID署名・公証・DMG導入とGatekeeperは証明しない。旧形式データの互換処理はユーザー指定により不要。バックアップは予防として保持し、復元や既存データの削除は行っていない。最初に構築した別識別子のQAに未知の会議が表示されたため操作対象から外し、そのデータは触れずに残した。実録音・OS権限変更・CI・Windows固有部分・正式品質評価は行っていない。

## 工程時間

同識別子の版0.1.1バンドル構築は約1分15秒（Rust releaseビルド表示1分13秒）。版0.1.0の初回構築は約84秒。GUIの取り込み・保存・バックアップ・冷起動・置換後読戻し、およびDB・音声の照合はこの二つの構築と別に実施したが、個別秒数は未計測。製品コードの変更はなく、この記録の文書化のみを実装工程とする。
