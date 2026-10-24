# Windows機能差分の実装・検証記録

2026-10-24追補：利用時に判明した5秒テスト起動、録音時刻逆転・欠損、Windows外枠、設定カード、削除メニューの修正と追加検証は [追補記録](windows-capture-window-fixes-2026-10-24.md) を参照。以下の実装時検証だけでは、所有リースを継承した5秒録音の保存完了と実機の録音品質を網羅できていなかった。

対象は `windows-macos-feature-parity-2026-10-24.md` の32項目。監査基準コミット `36431bb6f109b040508d5150f840a8697f8ca621` と既存の未コミット変更を引き継いだ。検証時のHEADは `84bb9f217aea533a7a1758054f65ced472fb6e4b` で、本対応は作業ツリーの変更を含む。実マイク、実PC音声、機器抜き差し、実時間4時間録音、公開証明書による署名は実施していない。下記の合成入力・実モデル試験と区別する。

## 実装の対応表

パスは `app/src-tauri/src/` を基準とする。録音ヘルパーは `probes/windows-capture/`、ビルド処理は `tools/` にある。

| ID | 実装根拠 | 実行した確認・実機残件 |
| --- | --- | --- |
| F01 | `audio_import.rs`、`audio_import/pcm.rs`、`audio_import.h`。PCM WAV／MP3／検証済みFLAC、Media Foundation Source ReaderによるAAC／ALAC。元ファイルを保持して管理コピーと解析用FLACを作成する | 独立FFmpeg製fixtureによる整数／浮動小数WAV、MP3、FLAC、AAC、ALAC。動画入り・2音声トラックを拒否。取消とバックアップ往復も試験対象 |
| F02 | `native_fs.rs`、`windows_files.rs`、`storage/backup/`。実空き容量、NTFS実体・ACL・同期を共通バックアップへ接続 | 実バックアップ作成、取消、既存出力の変更拒否、上書き、破損・容量上限、未知ファイル保持の共通試験 |
| F03 | `storage/backup/publication/`、`restore_ids.rs`、`restore_fixed/`。Windows公開所有ジャーナルと追加物の回収 | 追加復元、同一操作の再試行、途中公開の取消、DB失敗、改ざんされた所有記録、既存会議保持 |
| F04 | `storage/deletion/fs_windows.rs` と共通削除処理、既存4コマンドをWindowsにも登録 | Windows会議削除の実行、他会議保持。NTFS実体差し替え・ハードリンク拒否 |
| F05 | 共通 `storage/deletion/audio*` をWindowsへ接続 | 実音声取り込み後の音声削除、取消、会議保持、外部原本保持、削除済み会議のバックアップ往復 |
| F06 | 共通 `storage/deletion/model*`、Windowsのモデル実体・予約名操作 | 実モデル本体の削除と取消、配置・互換性履歴と他パッケージの保持 |
| F07 | `bootstrap/windows_io.rs`、`bootstrap/movement.rs`、`placement/`、`move_recovery.rs` | コピー・内容検証・切替・再起動後読込み、移動前DB保持。コピー中断後の再起動・保持判断も試験対象 |
| F08 | `lib.rs` 起動処理、`startup_attention.rs`、保存先管理2コマンド、フロントのWindows絶対パス検査 | 隔離fixtureでWindows従来配置（設定先とデータ先が同一）のDB実体・会議を保持し、新規DBへ移し替えないことを確認。移動後再起動も確認 |
| F09 | `diarization/embedding.rs`、`windows_runtime.rs`、Windows特徴量ヘルパー | 実Sherpa 1.13.8のCPUヘルパーと常駐抽出器で同一の512次元特徴量。声登録の共通13試験 |
| F10 | `voice.rs`、`storage/voice.rs` のWindows制限・無処理分岐を解消 | 自動照合条件、過去会議照合、候補・手動確認、曖昧な照合、モデル違い、手動名保持の共通試験 |
| F11 | `storage/speaker_completion.rs` の実行をWindowsへ接続 | 補完の共通12試験。手動の消去・分割・割当を保護し、本文編集は保持することを確認 |
| F12 | `mon_windows_runtime.py`。重み・モデル条件を共通カタログから生成し全14候補を提供 | 全14候補で実素材のSHA、正式な配置、実推論による互換性検査、標準設定への適用が成功。品質評価とは区別 |
| C01 | `input_binding.h`。対象アプリと子プロセスの稼働中入力セッションを列挙。プロセスハンドル・作成時刻でPID再利用を除外 | 一意な機器のみ採用し、不明・曖昧・対象終了は理由付きOS既定入力。実会議アプリによる確認は利用者側 |
| C02 | `Binding` とマイク独立再取得。固定UIDを保持、接続世代を更新 | 合成のマイク切断・復帰と、その間のPC音声保存、履歴・欠損を確認。実抜き差しは未実施 |
| C03 | PC音声を独立再取得。元のアプリ実体を保持し、別アプリや全体へ変更しない | 合成のPC音声切断・復帰とマイク継続を確認。実アプリ終了・デバイス変更は利用者側 |
| C04 | 実遷移から状態・世代・中断回数・理由・最大64イベントを生成 | 合成の両入力の履歴、停止状態、固定世代の解消を確認 |
| C05 | `input-events.json`、`input-journal-status.json`、停止manifestの保存診断 | 同期済み履歴と要求／確定revision、停止時封印、失敗時unconfirmed。共通の破損・欠損診断試験 |
| C06 | 既存の停止後履歴・通知・確認台帳へ、Windows producerとバックアップの履歴を接続 | 停止・再起動・復元後も元producerのownerを保持。確認済み通知の共通状態試験。実UI操作は未実施 |
| C07 | `capture_test/journal_windows.rs`。Windowsローカル版1の所有台帳と子へ継承する排他リース | 親ハンドル消失後も子の生存中は回収拒否、子終了後のみ回収、未知payloadを保持。実5秒録音は利用者側 |
| C08 | `raw_segments.h`、`recording_segments.rs`、共通 `recordings/flac/`。4秒PCM、SHA連鎖、時計端点、同期済みtipとFLAC確定台帳 | 4時間相当の単一mono laneで691,200,000フレーム・3,600区間、Rustで全SHA連鎖・時計対応・240 FLAC群を検証 |
| C09 | `IdleSleepGuard` の `ES_SYSTEM_REQUIRED`。終了時RAII解除、画面消灯は抑止しない | コード経路と合成終了を確認。明示スリープ後は取得再開と欠損として扱う。実スリープ／復帰は未実施 |
| C10 | 権限拒否・機器不在・再接続・容量不足・保存失敗の理由、起動応答platformとWindows日本語案内 | フロント試験、合成状態遷移、容量・失敗分岐。実OS権限画面での確認は利用者側 |
| R01 | `windows_files/`、`native_fs.rs`、`RootAnchor`、bootstrap。Volume／File ID、作成・変更時刻、DACL・所有SIDの完全一致、親ハンドル。短縮した互換用UIDだけで非公開所有を許可しない | DB実体と従来DB保持、差し替え・複数リンク拒否、SQLite継続書込み。保存先は書込み可能なローカルNTFS |
| R02 | `storage/backup/scratch*` のWindows永続所有。既知の旧Windows版2は完全なSQL・ルート証拠を確認して版4へ移行 | 取消・失敗・再起動・未知追加物・所有証拠の改ざん・保持判断を共通試験で確認 |
| R03 | `publication/journal*` のNTFS識別とハンドルによる回収 | 未公開追加物のみ回収、参照済み・差し替え・未登録作成物は保持する共通試験 |
| R04 | 音声利用不可の可搬履歴とWindows側の再確認経路を接続 | 音声削除済み会議を復元し、会議は存在し音声は利用不可のまま。Mac実機との往復は未実施 |
| R05 | `input_snapshots/source.rs` のWindows所有確認、可搬履歴生成 | Windows合成録音のバックアップ・復元で元ownerと停止履歴を保持。端末固有のハンドル・権限は可搬データへ入れない |
| R06 | 会議・音声・モデル削除の起動時照合、再確認・残り範囲・保持処理をWindowsへ接続 | 共通の中断・再確認状態試験とWindowsの実削除試験。Windowsで会議削除を中断し、再起動・残り範囲の再確認・完了を確認。実機強制終了の全タイミング網羅は未実施 |
| R07 | 書き出し、復元、取り込み回収、処理出力、ログ、モデル・音声検証にNTFS実体確認を接続 | 変更時刻を戻した変更検知、File ID差し替え・ハードリンク・親変更・共有ロックを試験。未知ファイルを再帰削除しない |
| P01 | 固定Whisper CPU／Vulkan worker、ジョブ開始時の実GPU試行とバックエンド固定 | tiny実モデルでCPU常駐再利用とIntel Iris XeのVulkan推論が成功。全6種のGPU品質・速度評価は未実施 |
| P02 | llama.cppのVulkan試行、実デバイスUUID・正のoffload・JSON結果検証、失敗時CPU、再開時同一GPU要求 | Qwen 2.5 1.5BでIntel Iris Xeへ29/29層をoffloadしJSON生成。GPU試行を失敗させた後のCPU生成も成功。Qwen 3.5 9Bの通し試験では実メモリ不足を検出し、16/33層の成功構成を固定・再利用して議事録生成まで完了 |
| P03 | 共通resident v1、上限付きパイプ、Windows Job Object、固定DLL、UTF-8・長いパス対応。Windowsでは完了通知前に区間ログの書込みハンドルを解放する | Whisperで同じPIDとモデルの再利用、CPU特徴量の同一結果。完了ログを排他的な読取りで封印する試験を追加。プロセス所有・取消の共通／Windows試験。旧protocol 2を変更しない |

## 補足の工程

- 外観: `windows_appearance.rs` が録音・説明・停止確認の物理サイズとDPIに追従してクリップする。既存タイトルバーを維持し、説明ウィンドウのWebViewフォーカスをWindowsへ接続。実デスクトップ描画・複数画面は未実施。
- 公開署名: `sign-windows.ps1` と `build-windows.ps1`。指定証明書の秘密鍵・有効期間・Code Signing用途、HTTPS RFC3161、EXE／DLL／インストーラーの署名・検証を要求する。証明書なしで公開ビルドが開始されず、既存成果物も変更されない自動試験2件が成功。実証明書による署名は未実施。
- 開発環境: Pythonの対象テストを明示UTF-8化。Windowsで動く共通fixtureへ修正し、声登録・補完試験もWindowsで実行。CIはMac固有試験とWindows固有試験を分け、両OSの共通Rust・フロント検査を実行する構成へ拡張。リモートCIの実行成功は未確認。
- 依存通知: 391パッケージを収集。不足していた `tauri-plugin@2.7.1` と `webview2-com` 系3パッケージは、Cargo checksum・VCSコミット・宣言ライセンスを一致させて上流のMIT本文を補い、`missingNoticeTexts=[]` を確認した。`third-party/notices/index.json` に出典・本文SHA・バイト数を保存。通知生成の完了と公開配布の法的確認は区別し、既存の `licenseReview=not_verified`、`distributionReady=false` は維持する。

## 実測記録

- 全14モデルの配置・互換性・適用: 1件の統合試験で全候補成功、2,153.03秒。モデル重み16素材を含む25固定素材のSHAを確認。
- GPU: Intel(R) Iris(R) Xe Graphics、UUID `8680a0a7040000000002000000000000`。Whisper tinyのVulkan実行・CPU常駐再利用・完了ログ封印は、最終SID補強後も21.12秒の統合試験で成功。llamaの実GPU試行・GPU次区間での再利用・試行失敗後CPU生成は23.76秒で成功。これは試験時間でありベンチマーク値ではない。
- 原PCM: 4時間相当mono、保留PCM最大768,000 bytes、producerのピークWorking Set 9,453,568 bytes。RustのSHA連鎖・時計・群検証は73.49秒で成功。実音声録音および2 lane同時実時間4時間は未実施。
- 声: CPUの単体ヘルパー／常駐抽出器から同じ512次元の正規化結果。人物識別精度の実測ではない。
- 通し解析: 保存済み22秒の日本語合成音声から、Whisper large-v3-turboの文字起こし、CPU話者分離、Qwen 3.5 9Bの議事録生成まで414.98秒で成功。生成した347文字の日本語議事録に、音声内の担当者・次回曜日・正しい再生区間リンクがあることを追加確認した。実マイク録音や一般の議事録品質評価とは区別する。
- 追加取得: 台帳 `.cache/windows-parity/download-ledger.json` に失敗・再取得分を含め26,042,589,935 bytes（約24.25GiB）。30GiB以内。モデル、固定ビルド素材、独立音声fixture作成用FFmpeg、補足通知本文を含む。取得済み原素材と配置済みモデルを保持し、完了したQA用二重コピーのみ回収した。
- 大きいコンテキスト: 追加の実モデルの通し試験で見つかった旧60,000入力上限の矛盾を解消し、実行エンジンと同じ262,144の絶対上限へ揃えた。既存プランの内容・保存済み上限は変更しない。プラン回帰8件が成功。

## 自動試験の集計

| 対象 | 結果 | ローカル実行記録 |
| --- | --- | --- |
| Windows共通・ネイティブRust | SID完全一致の補強後も756成功、0失敗、32明示除外。255.20秒 | `.cache/windows-parity-rust-owner-final.log` |
| フロントエンド | 773成功、0失敗、1明示除外。TypeScript／Viteビルド成功 | `.cache/windows-parity-frontend-final.log`、`.cache/windows-parity-frontend-build-final.log` |
| OS設定・権限 | Python 5成功 | `.cache/windows-parity-platform-final.log` |
| 公開署名の異常系 | Python 2成功 | `.cache/windows-parity-signing-tests-final.log` |
| 依存通知生成 | Python 15成功、3明示除外。除外は実シンボリックリンク作成権限が必要な検査 | `.cache/windows-parity-notice-tests-final.log` |
| ネイティブ時計 | CTest 1成功 | `.cache/windows-parity-ctest.log` |
| Windows話者分離DLL名の最終修正 | 話者分離6成功、1明示除外 | `.cache/windows-parity-sherpa-final.log` |
| 取り込み・取消・復元往復 | 6種類の実音声fixtureの統合試験成功、44.28秒 | `.cache/windows-parity-final-windows_real_import_four_formats.log` |
| 最終常駐CPU／Vulkan・特徴量 | 実モデルの3統合試験成功。完了ログ封印と次区間再利用を含む | `.cache/windows-parity-owner-real-worker.log`、`.cache/windows-parity-publish-windows_real_llama_vulkan.log`、`.cache/windows-parity-publish-windows_real_speaker_embedding.log` |
| 保存音声の通し解析 | 文字起こし・話者分離・議事録生成の実統合試験成功 | `.cache/windows-parity-speech-large-context.log`、`output/windows-qa/parity-final-speech-evidence.json` |
| 起動・モデル選択UI | Edgeのブラウザfixtureで起動診断8ケース、全14候補の選択、3エンジンの適用操作が成功。適用操作は模擬IPC | `output/windows-qa/parity-final-browser-results.json`、`output/windows-qa/parity-final-model-options-ui.png` |
| Rust整形・差分 | `cargo fmt --check` と `git diff --check` 成功 | `.cache/windows-parity-format-release.log` |

32件のRust除外には、実モデルや独立fixture、手動機器操作を必要とする試験が含まれる。全14モデル・Vulkan・特徴量・取り込み・4時間相当PCMなど、本記録に個別の実行結果を記したものは別途明示実行した。Mac実機回帰、リモートCI、実デスクトップでのインストール・更新時保持、実音声録音、正式署名は未実施であり、上記の成功数には含めない。

## QA配布物

- `app/build-windows.ps1 -Qa` が成功。2026-10-24 06:16:21〜06:23:11 JSTに生成したWindows x64版。
- 成果物: `app/src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/MON QA_0.1.0_x64-setup.exe`。
- サイズ: 268,303,540 bytes（255.87MiB）。SHA-256: `bca4ff92144dc4e1c48fc75f3f512f26bc94557ddd04c3d12a916f57baf8a289`。
- Authenticode: `NotSigned`。公開証明書による署名済み製品ではない。インストーラーの実行、既存インストールへの更新、実デスクトップでの描画確認は未実施。
- 最終集計と実行部品・通知のSHA照合は `output/windows-qa/parity-final/summary.json`、個別の試験ログは同ディレクトリに保存する。

既存の未コミット変更を含めて実装し、コミット・push・公開署名は実行していない。
