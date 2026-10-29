# Mac／Windows共通化の実装記録（2026年10月29日）

対象は[精査記録の27項目](refactoring-review-2026-10-29.md)。`origin/main` の `48356014794abc032ba82b9cfc021f30dffd6d85` を取り込み、`codex/cross-platform-refactor` で実装した。既存会議・音声・ユーザー設定を使ったアプリ起動、録音、インストール、配布は行っていない。

## 利用者から見える変更

- 解析結果の保存と、話者補完・声照合を続ける意図を同じDBトランザクションに保存する。アプリ終了・応答喪失後も同じ操作IDで確認し、設定変更によって過去会議を一律に再解析しない。人が後から編集した場合は自動適用を止める。
- 保存待ちが入力ごとに20秒・16 MiB・4096パケットのいずれかを超えると、音声を捨てて続行せず停止して案内する。
- Mac／Windowsの録音上限、入力復旧、マイク切替要求、列挙結果、音声ファイルの受理条件を共通処理へ接続する。OSの機器取得・プロセス所有・実際のファイルハンドル操作は各OSが担当する。
- 削除の受付が不明な場合は、同じ操作を照合してから次の操作へ進む。表示文言の変更によって再送可否が変わらない。
- 移行前DBコピーは非公開で保存する。未知の既存コピー・リンク・他者が差替えられる保存先には書き込まず、既存データを保持して起動診断で停止する。
- 声の登録・照合の対応可否はネイティブのcapabilityを使う。旧READMEのmacOS限定説明を修正した。

画面の主要操作や既存のモデル定義・receiptのハッシュは維持する。補助windowは役割に必要な画面を遅延読込みする。

## 保存形式と回復

DB schema 36で `post_analysis_intents` を追加した。過去のmigration SQL・checksumは変更しない。正しい履歴を持つschema 35（既存の34→35経路を含む）だけを追加移行し、`before-followup-intents.sqlite3` を移行前に残す。既存会議への自動処理意図は生成しない。コピーは非公開の所有付き一時領域でSQLite backupを作り、ファイルと親を同期して上書きなしで保存する。既存コピーは非公開・単一所有・現在のDBイメージとのSHA一致を確認した場合だけ再利用し、未知の物やリンクは変更しない。

意図は対象解析版・作業内容のSHA・人物登録のSHA・閾値・モデル指紋と実行条件・固定操作IDを保持する。解析保存と意図作成、ジョブ作成と意図の受付更新、補完結果保存と声照合への引継ぎを、それぞれ同じトランザクションで確定する。変更された入力は `stale`、処理を開始できない状態は `blocked` として保持し、通常の再生成から新しく確認する。処理不能なモデルを別のモデルへ自動で置き換えない。

意図の消化はStorage単位で直列化し、会議の作業leaseを保持する。状態更新は読み取ったphase・解析版・作業SHAを条件に行い、補完結果の保存後に古いsnapshotが声照合への引継ぎを上書きしない。消化中に削除された意図は読み飛ばす。

新しいバックアップは形式6・schema36。形式5・schema35はアプリ所有の旧schemaで検証して、schema36へ追加復元する。復元した意図はIDを変換した履歴として `restored` に固定し、新しい自動実行の同意にはしない。端末の実行パスは可搬化し、会議・解析版・ジョブ種別・操作IDの所属も検証する。

耐久性の最低条件はアプリプロセス異常終了からの回復とファイルのflush。POSIXのdirectory同期とWindowsの再起動回復用receiptを区別して表す。電源断に対する同等の保証を確認したものではない。

## 項目ごとの接続先

| ID | 実装した共通経路 |
| --- | --- |
| 01 | `mon-core::capture::raw` の上限・prefix検証・準備済みcommit・tip/hashをSwiftとC++の実writerから使用 |
| 02 | `storage/post_analysis.rs`、migration36、解析保存・補完保存・worker起動時の永続意図消化 |
| 03 | `mon-native::transport` のcurl引数、HTTP200の終端マーカー、容量／SHA／期限、非公開の上限付き診断 |
| 04 | `mon-core::capture::supervisor` の入力復旧・世代・再試行・停止の状態遷移 |
| 05 | `mon-native::durability` のOSごとの保証とreceipt、既存native flushとの接続 |
| 06 | `mon-core::capture::request` のrevision・owner・UID・mode・重複キー検査を両helperへ接続 |
| 07 | `mon-native::archive` のtar／gzip／bzip2ストリーム検証と共有予算、OS別destination adapter |
| 08 | `mon-native::identity` の実体ID／Proofとnative extension trait。OS固有の置換・削除の保証を維持 |
| 09 | `mon-native::process` の用途別起動条件と、guardian／Job Object等の所有付き起動 |
| 10 | Windowsモデル領域の作成・再読取時の非公開DACL検査、既存POSIXのmode／owner検査 |
| 11 | `command_registry.json` の150 handler／190公開引数から登録・権限・呼出キー検査を生成。実署名の引数／応答／error型と実Tauri ACLを照合 |
| 12 | `mon-core::operation` の操作×資源scope判定を実受付とblockerへ接続し、既存leaseの所有・終了待ちを維持 |
| 13 | `mon-contracts` のJobKind／JobState／PauseReason。DB境界の未知値拒否、種別メタデータから音声要否・形式・失敗分類・受付対象を導出 |
| 14 | `job_codec` の型付きresolverと各ドメインの復元codec／結果要約 |
| 15 | `ProcessingError` の品質拒否、`AppError` のcode／operationId／retryDisposition。削除と再起動理由の文言依存を除去 |
| 16 | Sherpa設定とファイル指紋の共有builder。検査用Transientと保存用Retainedを明示 |
| 17 | `mon-core::media` のbounded M4A parser、WAV／FLAC decoder。OS decoderは追加codecのadapter |
| 18 | 各入力の20秒／16 MiB／4096 packet予算、超過時の明示停止 |
| 19 | `model-definitions.json` と `platform-runtimes.json` からtarget別catalogを生成。容量・token limitsも共通定義 |
| 20 | Swift／C++のCLI・options・sessionを分割し、合成driverを製品のlane／prefix経路へ接続 |
| 21 | `mon-core::capture::catalog` の並び順・自プロセス除外・上限128・overflowの契約 |
| 22 | `DeletionExchange` の確認・送信・受付不明・照合・終端の共通状態機械、3種の削除hookへ接続 |
| 23 | snapshot resourceと共通event／focus／poll／in-flight／teardown。録音・データ操作・確認・取り込み・解析・議事録から使用 |
| 24 | Rust schemaからTypeScript型と構造validatorを生成。既存serializerと同じcanonical bytes／SHAの共通golden vector |
| 25 | window別lazy bootstrap、RecorderPanel、capture actions、編集保存付きnavigation、終了／hide controllerを分離 |
| 26 | 自己完結のPlaywright runnerで全96ブラウザースイートをローカル・CIの検証入口へ接続 |
| 27 | 共通tooling／crate／fixture／workflow変更のCI対象を拡張。Python、生成物検査、共通RustとOS固有検証を接続 |

## 検証

- TypeScript：881件中880成功・1skip。型検査・本番ビルド・14 schema生成物の一致が成功。
- Python：223件中203成功・20件は環境依存のためskip。
- 共通Rust：contracts 4件、core 12件、native 13件成功。FFIの生成header一致とrelease静的ライブラリ再構築が成功。
- ネイティブRust全体：2,144件中2,049成功・95件ignored・失敗0。全34バッチとdoc testを完走し、runnerの終了コード0を確認した。
- Swift：XCTest 270件中267成功・3件skip、Swift Testing 1件成功。通常権限で全suiteを実行した。
- Portable C++：共通Rust静的ライブラリを使う5スイート成功。Windows SDKの実行結果ではない。
- WebKitのブラウザー全96件が成功（13.7分）。最後の理由code修正後も関連5件と全単体・ビルドを確認した。
- 専用Swift→Rust原音声連携は204,000フレームのPCM一致からDB確定まで1件成功。移行前コピーの最終6件も成功。実ファイルのschema35→36移行・再起動では、会議・解析結果・設定・既存migration checksum・未知ファイルを保持し、旧schemaの非公開コピーを保存することと、過去会議の自動処理意図を作らないことを確認した。
- 意図消化の同時実行、古い補完snapshotによる引継ぎの上書き防止、削除との競合を回帰試験で確認した。Windows用ソースの型検査も成功したが、Windows SDKでのリンク・実行は含まない。

生成する構造型・validatorはRecipe／Manifest等のモデル定義、ジョブenum、WindowEnvironment、AppErrorの14 schemaが対象。全150 commandの応答DTOを丸ごと自動生成したものではない。command名・公開引数のキーと必須性・Rust署名の型のずれは別のregistry検査で検出する。

既存Viteの脆弱性は7.3.6から[7.3.7](https://github.com/vitejs/vite/releases/tag/v7.3.7)への最小パッチ更新で対応し、更新後のnpm auditは0件。これによる本番アプリの画面仕様の変更はない。

## 実機で確認が必要な範囲

Windows SDKでの最終リンク・WASAPIの実入力／再接続、両OSの長時間録音・実ディスク枯渇、実windowのWKWebView／WebView2、電源断、署名・配布・実更新は、このホストでの合成試験やRust型検査の成功からは判断しない。既存の人間側QAの範囲を維持する。
