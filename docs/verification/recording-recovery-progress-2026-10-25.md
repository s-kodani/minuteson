# 保存回復の進捗表示（2026-10-25）

## 動作

保存途中でプロセスを終了した録音について、次回起動時の自動回復を画面上部、ジョブ一覧、対象会議の状態欄へ表示する。通常の停止・保存と、再起動後の回復は別の状態として扱う。

- 回復対象の確認、会議ごとの待機／実行中／成功／失敗／対象外を、実行IDと更新番号付きのネイティブメモリー状態に保持する。開始時刻は会議の処理を始めた時点で記録し、進捗率は推測しない。
- `get_recording_recovery` はメイン画面専用の参照IPC。取得時に音声やSQLiteを読まない。`recording-recovery-changed` はメイン画面への更新通知で、画面は通知後の取得と1秒間隔の取得、focus／visibilitychangeで正規の状態を読み直す。古い更新番号を採用しない。
- 保存処理の既存戻り値は録音セッションIDのまま維持し、画面への `recordings-recovered` 通知では会議IDに変換して重複を除く。完了イベントを取り逃しても、回復状態の定期取得から会議一覧と選択中会議を更新する。
- 上部には実行中の回復を優先表示する。複数処理の件数とジョブ一覧への導線も表示する。ジョブ一覧の空表示には、解析・回復の両方の状態取得成功が必要。
- 取得失敗時は前回状態を残して確認不能を表示する。回復失敗時は警告と「録音未完了・要確認」を表示する。回復失敗後に保存済みとなった会議を、古い失敗情報で未完了表示へ戻さない。
- 会議の削除可否は従来のネイティブ判定を使用する。回復中の録音所有によって拒否される場合だけ具体的な理由を表示し、表示上の待機状態だけでは削除禁止にしない。待機中に削除済み／回復対象外となった録音は採用しない。
- WindowsのトレイとmacOSのメニューバーは同じ回復状態を参照する。回復中だけの場合も、×で収納するときにバックグラウンド継続を確認する。完全終了や監視終了の既存方式は変更していない。

DB形式、保存完了の検証、音声内容、部分音声救出の条件、自動解析の開始条件は変更していない。利用者の会議・音声・設定は、この検証では変更・削除していない。

## 実施した検証

Windows上で、合成データと合成IPCを使って検証した。

- 新規ネイティブ3件：状態の独立性と更新番号、複数録音の成功・失敗、SQLiteロック外の通知、待機対象削除と対象なし。
- 既存ネイティブ17件：実Tauri権限3、会議別削除6、起動制御6、保存完了録音の再起動回復1、不正音声を未完了のまま保持する検査1。
- フロントエンド全体798件成功、失敗0件、既存のWindowsネイティブfixture依存1件スキップ。TypeScript型検査、Vite製品ビルドも成功。件数は `output/recording-recovery-frontend-final.log` に記録した。
- Windows表示の新規ブラウザー6ケース、既存削除8、保存中終了8、既存終了確認22。常駐の既存9ケースに、Windows／Macそれぞれ回復中だけの継続確認を加えた11ケースも成功。
- Mac表示を模擬した新規ブラウザー6ケース：共通進捗・会議一覧・ジョブ一覧、再読込、削除理由と待機対象の許可、取得失敗と復帰、イベントなしの完了検知、古い応答の拒否と回復失敗の表示。
- Pythonのコマンド登録・配布境界5件、`git diff --check`。

画面検査のソースは `app/tests/recording-recovery-browser-check.js` と専用HTML／合成IPC fixture。既存と同じく、Viteの127.0.0.1:1420でPlaywrightから検査関数へpageを渡して実行する。第3引数に`macos`を渡すとMac表示を模擬する。専用fixtureは実ファイル操作を行わない。

結果とログはGit管理外の `output/recording-recovery-*` に保存した。Vite開発用WebSocketの接続警告があるが、製品ビルドと画面検査は成功している。最初のネイティブ試行はサンドボックスの一時SQLite作成制限で失敗し、ワークスペース内の専用TEMPを指定して検証した。ブラウザー検査は検証サーバーとChromeを同じ実行環境で起動した。

## macOSで残る検証

この端末で利用できるRustターゲットはWindowsのみ。Macのネイティブビルド、実WebView、メニューバー・Dock、スリープ復帰、署名・公証は未検証。Mac表示の模擬IPC成功をMac実機の成功とは扱わない。

Macでは既存のmacOS 15以降・Apple Silicon用開発環境で次を実施する。

```sh
cd app
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --lib --locked recording_recovery -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --lib --locked ipc_acl::tests -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --lib --locked meeting_access_tests -- --test-threads=1
npm run tauri:qa:build -- --config '{"productName":"MON Recovery QA","identifier":"jp.mon.desktop.qa.recovery20261025"}' --bundles app
```

私有QA領域の合成録音を使い、保存中断後の起動で対象会議名・回復中表示・完了／失敗を確認する。回復中の赤い×で継続確認が出て、収納後も処理が続き、メニューバー／Dockからの再表示で最新状態が出ることを確認する。⌘Qが既存の終了経路を維持すること、スリープ復帰後に状態が再取得されることも確認する。通常保存中を回復中と誤表示しないことも確認する。

## Windowsインストーラー再作成

同日22:03（日本時間）に `app/build-windows.ps1 -Qa` で再作成し、`output/windows-qa/MON-QA-setup.exe` を更新した。容量268,497,697 bytes、SHA-256は `077a7c2742277eb3a35bf4768f4c07d1f61f4a4d58057727722696e9cc4074db`。

ソース667ファイルがビルド中に変化していないこと、最新の画面資産と回復IPC・更新通知が本体に含まれること、本体のx64形式、元のNSIS生成物と配布先のハッシュ一致を確認した。ログと検証結果は `output/windows-qa-work/recording-recovery-20261025-build.log` と `recording-recovery-20261025-report.json` に保存した。最初の制限環境ではMSBuildの並列実行が進まなかったため、そのビルドを停止して関連プロセスの終了を確認し、通常の実行環境で成功した。

QA版は未署名。インストーラーの実行とインストール済みアプリの差し替えは行っていない。
