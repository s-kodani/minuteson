# Mac実装とWindows実装の機能差分監査

この文書は修正前の監査記録。以後の実装根拠・検証結果・実機残件は [Windows機能差分の実装記録](windows-parity-implementation-2026-10-24.md) に記録する。

調査日: 2026-10-24（日本時間）  
対象: `codex/windows-platform-foundation`、基準コミット `36431bb6f109b040508d5150f840a8697f8ca621` **および調査開始時点の未コミット・未追跡の実装すべて**。

## 結論

WindowsではWASAPI録音、5秒テスト、モデル9候補の取得・配置・適用、CPUによる文字起こし・話者分離・議事録生成が接続済み。一方、取り込み、バックアップ、追加復元、3種類の削除、保存先変更、声の登録・照合、話者補完、モデル5候補、録音の復帰・履歴・異常終了対策、GPU・常駐推論に実装差が残る。

特に、**バックアップ・追加復元はダイアログがあっても完了できない**。共通の空き容量取得がWindowsで常にエラーを返すためで、既存Windowsテストバイナリでもバックアップの失敗を再現した。

以下は「未対応を示す文言」の検索結果だけではなく、画面 → IPC → OS分岐 → ネイティブ処理・永続化の経路から判定した一覧。未接続機能と、実装済みだが実機確認が残る機能を区別する。製品コードの修正は行っていない。

## 判定方法と読み方

- `lib.rs` の独自IPC 135件と `app_commands.rs`、共通React画面、RustのOS条件分岐、MacのSwift録音実装とWindowsのC++製品録音経路、モデル定義、ビルド・CIを照合した。
- Windowsには135件中14件が登録されず、登録される121件の中にもWindowsでエラー・処理省略になるものがある。ACLの許可リストに名前があることは機能実装の証明にはならない。
- 「未対応」は処理経路がない／必ず拒否するもの。「部分対応」は基本経路はあるがMac側の動作・保護・診断が欠けるもの。「基盤差」は通常動作だけでは現れない耐障害性・性能の差。
- 優先度「高」は基本操作・保存・録音継続、「中」は追加機能・互換性・性能。「根拠」の行番号はこの調査時点のソースを指す。
- Macの実装存在を確認したことと、Macで実機品質・4時間運用・正式署名が検証済みであることは別。今回は実録音、音声再生、インストール、利用中アプリの操作、モデル追加ダウンロードは行っていない。

## 1. 通常操作・解析機能の不足

| ID | 機能 | Windowsの判定・利用者への影響 | 根拠 | 優先度 |
| --- | --- | --- | --- | --- |
| F01 | 音声ファイルの取り込み | **未対応**。選択ダイアログはあるが完了経路がない。WAV・M4A・FLACの検査／復号に使うヘルパーの `--import-audio` がWindows製品経路にない。MP3には共通Rust復号があるが、その後の空き容量取得が必ず失敗するため、MP3も製品の取り込みとして完了しない。 | `import_controller.rs:176`、`audio_import.rs:131,239`、`platform_dialog.rs:51`、`probes/windows-capture/product_capture.h:96`。Mac側は `AudioImport.swift` と `App.swift` の取り込み分岐。 | 高 |
| F02 | 手動バックアップ作成 | **未対応**。保存先の選択・プレビューは可能でも、スナップショット作成時の容量検査で停止する。エラー文が「音声取り込みはまだ接続されていません」となり、操作内容とも一致しない。 | `storage/backup.rs:295,328,457` → `audio_import.rs:239`。既存テストで実行確認（後述）。 | 高 |
| F03 | バックアップからの追加復元 | **未対応**。正常な現行バックアップも、アーカイブ読取り時の容量検査で停止する。さらに公開時の所有記録にも別の未対応箇所がある（R03）。容量取得だけの修正では完成しない。 | `storage/backup/operations.rs:342` → `storage/backup/archive.rs:605` → `storage/backup.rs:457`。公開は `storage/backup/publication.rs:239` → `publication/journal.rs:291`。 | 高 |
| F04 | 会議全体の削除 | **未対応**。対象一覧・削除範囲確認・実行・未完了削除の再確認がWindowsで除外される。UIは応答の対応項目の有無を見て操作を無効化する。 | `lib.rs:2301` 付近の `#[cfg(unix)]`、`storage/deletion.rs:21`、`data-operation.ts:36`。 | 高 |
| F05 | 音声だけの削除 | **未対応**。本文・議事録を残して音声のみを削除する一連の処理、残り範囲の再同意・継続もWindowsには接続されない。 | `lib.rs:2317` 付近、`storage/deletion.rs:3`、`data_controller.rs` の `AudioDeletion` 分岐。 | 高 |
| F06 | 導入済みモデル本体の削除 | **未対応**。設定・履歴を残してモデル一式を削除する処理、未完了削除の再確認もWindowsで除外される。モデル取得は可能でもアプリ内で容量を戻す操作が不足する。 | `lib.rs:2309` 付近、`storage/deletion.rs:27`、`storage/model_placements.rs:269`。 | 高 |
| F07 | 会議保存先の変更 | **未対応**。フォルダー選択ダイアログの実装はあるが、その後 `Kind::Relocation` が拒否される。コピー・検証・切替・再起動・未完了移動の確認というMacの実装がない。 | `lib.rs:8,2325`、`data_controller/work.rs:185,439`、`data_controller.rs:829`。Mac側は `bootstrap/`、`storage/backup/relocation/`。 | 高 |
| F08 | 保存先の現在位置・起動設定の表示と回復 | **未対応／部分対応**。`get_storage_bootstrap` がWindowsで未登録なのに保存設定画面から呼ばれる。Macの保存先選択履歴・移動中状態・元保存先を保持する回復経路はない。Windowsにも「起動失敗を表示して通常操作を拒否する」共通診断はある。 | `data-save.tsx:49`、`lib.rs:2190,2220,2325`、`startup_attention.rs:208`。 | 高 |
| F09 | 声の登録 | **未対応**。人物に対して発言候補1〜2区間を選び、特徴量を抽出して声を登録する操作を拒否する。話者分離と、話者名の手入力による編集は別機能として利用できる。 | `storage/voice.rs:394`、`voice.rs:128`、`diarization/embedding.rs:56`、`voice.tsx:84`。抽出ヘルパー探索は `diarization/known.rs:23` でMac署名・dylibに依存。 | 中 |
| F10 | 登録した声と会議話者の照合 | **未対応**。新しい解析後の自動照合、過去会議の明示的な再照合、照合候補を作る工程がない。人物一覧等の共通データ操作まで全廃されているという意味ではない。 | `voice.rs:128,325`。Windowsでは自動照合を `Ok(())` で省略。画面側も `context.supported` で制限。 | 中 |
| F11 | 既知話者の声による「不明」発言の補完 | **未対応（無処理で終了）**。話者分離後に既知話者の特徴量から不明発言を補完する工程を実行しない。設定の取得・保存が共通でも補完実行は対応済みにならない。 | `storage/speaker_completion.rs:345,352` でMac以外は `Ok(None)`。特徴量抽出にもF09の制限。 | 中 |
| F12 | Macと同じモデル候補を選択・導入すること | **一部未対応**。Whisper6種はあるが、Macにある議事録4候補・話者分離1候補がWindowsカタログから除外される。内訳は次表。 | `model_official_catalog.rs:60,70,110`、`tools/mon_windows_runtime.py:108` 付近の3候補だけの許可条件。 | 中 |

### Windowsにないモデル5候補

Mac: Whisper6候補 + `macos.json` の8候補 = **14候補**。Windows: Whisper6候補 + 議事録2候補 + 話者分離1候補 = **9候補**。候補として提供されることを比較したもので、全モデルの実解析成功・品質を意味しない。

| 用途 | WindowsにないMacの候補 | Mac定義のID・行 |
| --- | --- | --- |
| 議事録 | Qwen 2.5 1.5B · Q8_0 | `llama-b11533-qwen25-15b-q8`、`app/model-catalog/macos.json:94` |
| 話者分離 | Pyannote int8 + ERes2Net base | `sherpa-1138-pyannote3-int8-eres2net-base`、同 `:187` |
| 議事録 | Qwen 3 8B · Q4_K_M | `llama-b11533-qwen3-8b-q4km`、同 `:245` |
| 議事録 | Qwen 3.5 4B · Q4_K_M | `llama-b11533-qwen35-4b-q4km`、同 `:693` |
| 議事録 | Qwen 3.5 9B · Q4_K_M | `llama-b11533-qwen35-9b-q4km`、同 `:917` |

Windowsで提供される議事録候補はQwen 2.5 1.5B Q4_K_MとQwen 3 4B Q4_K_M、話者分離は非int8のPyannote + ERes2Net base。モデル重みだけを追加すれば完成するとは限らず、実行部品、固定ハッシュ、トークン条件、互換性検査・適用をWindows向けに確定する必要がある。

## 2. 録音継続・入力状態・診断の不足

| ID | 機能 | Windowsの判定・不足する動作 | 根拠 | 優先度 |
| --- | --- | --- | --- | --- |
| C01 | 指定会議アプリが使うマイクの自動選択 | **部分対応**。Windowsの自動選択はOS既定の通信入力を使う。Macは対象プロセスの入力機器を照合し、一意ならそのマイク、確認できなければ理由付きでOS既定へ戻す。Windowsは対象アプリ指定時も `mon_direct_recording` として報告する。固定マイクの選択・手動切替は実装済み。 | Mac `InputDevices.swift:95,120`、Win `product_capture.h:140,147,172`。 | 高 |
| C02 | マイク切断・再接続からの自動復帰 | **未対応**。Macの同一機器の再探索、世代の更新、古いコールバックの拒否、再接続・復帰の状態管理に相当する経路がない。Winの `pump()` / `open()` のエラーは外側の例外処理へ出て録音全体を失敗終了する。 | Mac `MicrophoneRecovery.swift` と `App.swift` の制御ループ。Win `product_capture.h:63,88,157,188`。 | 高 |
| C03 | PC音声取得の中断・再接続からの自動復帰 | **未対応**。同じ取得範囲を保持した再取得、非同期開始の所有、タイムアウト・再試行がない。対象アプリの終了・実体変更も例外で録音全体を終了する。 | Mac `SystemAudioRecovery.swift:26,50,92,107`。Win `product_capture.h:151,157,188`。 | 高 |
| C04 | 入力状態遷移と切断・復帰通知 | **部分対応**。入力レベルの実測はあるが、稼働中の接続状態は両入力とも `connected / sourceEpoch=1 / interruptions=0` を固定出力する。手動切替時も遷移履歴を作らない。共通UIの更新停止検知はあるが、切断理由や復帰履歴を生成する実装が欠ける。 | Win `product_capture.h:148,161,162`。Mac `MicrophoneRecovery.swift`、`SystemAudioRecovery.swift`、`InputEventHistory.swift`。 | 高 |
| C05 | 入力イベント履歴の永続保存・停止時の保存診断 | **未対応**。Windows製品ヘルパーは `input-status.json` の空履歴だけを出し、Macの `input-events.json`、保存要求／確定revision、停止時の封印・保存失敗診断を出力しない。 | Win `product_capture.h:148,162,180`。Mac `InputEventJournal.swift`、`InputEventJournalFile.swift`、`App.swift:1880` 付近。消費側は `input_status/journal.rs`、`input_status/persistence.rs`。 | 高 |
| C06 | 録音終了後の入力通知の再表示・確認完了 | **部分対応／処理不能**。共通の通知IPCはあるが、保存後通知のディレクトリ同一性検査がWindowsでは常にエラー。新規キャッシュ作成後の再検査や再読取りで利用不可になる。C05の履歴生成不足とは別に修正が必要。 | `input_status/archived.rs:63,78,118,166,307` の `same_directory` と `entry` / `verify` / `acknowledge`。 | 中 |
| C07 | 異常終了した5秒テスト音声の次回起動時回収 | **未対応**。通常完了・取消時の音声削除は実装済みだが、Macの永続所有台帳と次回起動時の回収がWindowsでは省略され、回復状態を `clear` にする。異常終了で残った音声を継続追跡できない。 | `capture_test.rs:4,453,460`、`capture_test/recovery.rs:30,68,95,109`、`capture_test/files.rs:151`。 | 高 |
| C08 | 録音途中の原PCM区間の同期・所有・確定証拠 | **基盤差（未対応）**。Macは `RawRecordingSegments` で原PCM区間・SHA・時計・確定範囲を逐次保存する。Windows製品経路は各入力を単一CAFに追記し、停止時だけ長さ・flush・時計manifestを確定する。途中の耐久確定範囲とMac同等の救出基盤がない。 | Mac `RawRecordingSegments.swift`、`App.swift:1894`、`storage/recordings.rs:544,740`。Win `product_capture.h:71,79,89,178`、`storage/recordings/windows.rs`。 | 高 |
| C09 | 録音中のアイドルスリープ抑止 | **未対応**。Macは取得プロセスが電源assertionを保持・解放する。Windows側に同等の電源要求APIの呼出しがない。通常の画面消灯とユーザーが明示したスリープをどう扱うかも移植対象。 | Mac `RecordingPower.swift:10`、`App.swift:1948`。Windows製品C++・Rust全体で `SetThreadExecutionState` / `PowerSetRequest` / `PowerCreateRequest` の使用なし。 | 高 |
| C10 | 権限・機器・保存失敗を区別したWindows向け案内 | **部分対応**。Windowsのマイク権限一覧は常に `unknown`。主な録音失敗はHRESULT等を含む英語の例外文字列で、Macの分類済み診断・日本語の復旧手順に相当する案内がない。共通画面にはMacのシステム設定を案内する文言が残る。 | Win `product_capture.h:54,188`。Mac `CaptureFailureGuide.swift`。共通UI `capture-inputs.tsx:47`、`capture-applications.tsx:43`。 | 中 |

C08の範囲に関する注意: Macの `review_partial_recording*` と `recover_partial_recording*` は現行ソースで `#[cfg(test)]` のものがあり、製品IPCにも公開されていない。**「Macで利用できる部分救出画面をWindowsに移植する」という独立した製品機能としては数えていない**。Macに存在する区間保存・検証・移動時検査の基盤との差である。また、正常に `saved` まで到達した録音を起動後に再確定する `recover_completed_recordings()` は両OSにある。

## 3. 保存・復元・ファイル操作の堅牢性の不足

F02/F03の容量取得を直した後にも残る項目を明記する。

| ID | 機能 | Windowsの判定・不足する動作 | 根拠 | 優先度 |
| --- | --- | --- | --- | --- |
| R01 | 保存先全体とDBの実体を保持する書込み保護 | **基盤差**。Macの保存先起動設定、管理ルートとDBの実体保持、削除・移動履歴との照合がWindowsにはない。`managed_anchor`、起動時の削除台帳検査・`seal_controls()` はUnixだけで、書込み解除時の実体検査もWindowsでは `true`。モデル配置等にはWindows用ハンドル保護が既にあるが、保存領域全体には広がっていない。 | `storage.rs:175,421,434,477`、`storage/deletion/fence.rs:195`、`lib.rs:2190,2220`、`windows_files.rs`。 | 高 |
| R02 | バックアップ等の一時領域の永続所有・異常終了後回収 | **未対応**。Windowsは `scratch=None` で一時領域をTempDirとして使う。通常終了時の解放はあるが、Macの永続所有台帳・再起動後の回収／保持確認がない。対応する `Identity::read` もWindowsでは拒否する。 | `storage.rs:451,463,512`、`storage/backup/scratch.rs:74,818`、`storage/backup/recovery.rs:192,210`。 | 高 |
| R03 | 追加復元の公開ジャーナルと失敗時の追加物回収 | **未対応**。公開前に作る `Journal::begin()` が呼ぶ実体識別取得はWindowsで必ずエラー。ロールバック用の `same_inode()` も常にfalseなので、追加物を所有確認して片付ける経路は完成していない。容量検査と別のブロッカー。 | `storage/backup/publication.rs:239` → `publication/journal.rs:29,45,291`、`publication/files.rs:16,22`。 | 高 |
| R04 | 音声削除済み会議を含むバックアップの復元 | **未対応**。Macの「本文・議事録は残り、音声は明示的に利用不可」という履歴をWindowsへ追加復元する分岐が拒否される。一般復元とは別に可搬データの互換性を用意する必要がある。 | `storage/backup/restore_ids.rs:218`、`storage/backup/publication.rs:165`、`storage/backup/exclusions.rs:39`。 | 中 |
| R05 | 自端末で録音した入力履歴・未確認通知をバックアップへ含めること | **部分対応／未対応**。既存の復元済みスナップショットを保持する共通コードはあるが、Windowsで新しく録音したディレクトリからの入力履歴取り出しは `None`。C05/C06を修正しても、このバックアップ生成側の接続が別途必要。 | `storage/backup/input_snapshots.rs:153,188,215,256`。 | 中 |
| R06 | 中断した削除処理の起動時整理・再同意・継続 | **未対応**。会議・音声・モデルの未完了削除を照合して自動整理する処理、残り対象を確認して続行する経路はWindowsで除外。F04〜F06の正常系だけを足してもこの復旧は揃わない。 | `lib.rs:2249` 付近の `finish_interrupted_deletions`、`storage/deletion.rs`、`data_controller.rs` のUnix限定操作。 | 高 |
| R07 | 書き出し・一時ファイル等の同一実体／リンク／非公開権限の検査 | **基盤差**。Markdown等の書き出しのハードリンク拒否・inode比較・親ディレクトリ実体比較、一時復元領域の非公開権限確認等はUnix限定。Windowsの同等なFile ID・共有モード・ACL検査はこれら全経路に適用されていない。通常のMarkdown新規保存が未対応という意味ではない。 | `exports.rs:34,52,106,223`、`storage/backup/scratch.rs:130,151`、`storage/backup/restore_fixed/workspace/seal.rs:32,47`。同種のUnix限定検査は `internal_logs/store.rs`、`restored_processing.rs` にもある。 | 高 |

## 4. AI実行速度・負荷の差

| ID | 機能 | Windowsの判定・不足する動作 | 根拠 | 優先度 |
| --- | --- | --- | --- | --- |
| P01 | WhisperのGPU利用 | **未対応**。MacのMetal利用方針に対しWindowsはCPU構成。Windows用WhisperビルドでVulkanを明示的に無効化し、他のGPUエンジンを同梱・選択する実装もない。単にGPU実機評価が済んでいないという状態ではない。 | `transcription.rs:275,640`、`tools/mon_windows_runtime.py:60` の `GGML_VULKAN=OFF`、Windowsカタログ。 | 中 |
| P02 | 議事録生成のGPU自動試行・CPUへの切替 | **未対応**。Macは対応モデルでMetalの試行、利用実態・メモリの確認、CPUへの切替を行う。Windowsに同等のGPUポリシーとGPU同梱ランタイムがない。CPU生成は実装済み。 | `generation/acceleration.rs:30,91,106,144`、`tools/mon_windows_runtime.py:22,116`。 | 中 |
| P03 | モデルを保持する常駐推論ワーカー | **未対応**。Windowsのprotocol 2は `Scope::enter()` で常駐セッションを作らず、工程・区間ごとにCLIを起動する。Macのprotocol 1と同じモデルロードの再利用、常駐ワーカーの進捗・寿命管理はない。Job Objectによる子プロセス終了管理自体は実装済み。 | `local_engine/resident.rs:29,39,253,269`、`local_engine/resident/installation.rs:180`、`tools/mon_windows_runtime.py:143` 付近。 | 中 |

補助的な差分として、共通会議WAVのコピーはMacが `clonefile` を先に試すのに対し、Windowsは逐次コピーへ進む（`storage/meeting_mix.rs:341,357`）。**同じ機能を実行できるため機能未対応には数えない**が、長い会議の準備時間・一時容量の差になり得る。実測差は今回計測していない。

## 5. 外観・配布・検証の差分

これらは上記の機能未接続と区別する。

| 項目 | 確認した差・扱い |
| --- | --- |
| ミニ録音パネル等のネイティブ角丸・透明背景 | Macは `rounded_windows.rs:6` で録音・説明・停止確認ウィンドウのネイティブ描画をクリップし、背景を透明化する。Windowsに相当処理はない。CSSや共通ウィンドウはあるため、**実際のWindows外観の欠け方は実機描画で要確認**。パネル機能全体を未対応としない。 |
| 公開配布向けの署名ビルド手順 | Macには `tools/mon_macos_release.py`、署名・公証設定と検査の工程がある。Windowsは現行スクリプト・NSIS設定に公開用署名の同等工程がなく、`windows-setup.md` でも対象外と明記。Macも実Developer ID署名・公証の成功は未検証なので、「Macだけ正式配布完了」とは判定しない。 |
| 自動テストのWindows対応範囲 | Unixシェル・Mac音声fixture・所有台帳等の試験が `#[cfg(all(test, unix))]` で除外される一方、共通のままWindowsで実際に失敗するバックアップ試験もある。Windowsの全Rustテスト・通知生成等を一通り通せる状態には至っていない。 |
| Pythonテストの既定文字コード | 今回、通常起動の `test_mon_tauri_platform.py` はCP932の `read_text()` で1件エラー。`python -X utf8` では5件成功。OS既定のまま動く開発手順にも差が残る。機能未実装件数には含めない。 |

macOSのCmd+Q・Dock・ネイティブメニュー・Core Audio・Metal・codesign等のAPIそのものをWindowsへ移すことは目的ではない。同じ利用者向け動作・保護がWindowsに必要かを基準にした。共通の終了確認・保存待ち・子プロセス終了処理は存在するため、Windowsの終了機能全体を未対応とはしていない。

## 6. Windowsにも実装があり、未対応一覧から除いた機能

「実装が接続されている」という判定であり、以下の全項目の実機操作成功を今回確認したわけではない。

- マイク＋PC全体／指定アプリのWASAPI録音、開始・停止、対象アプリ実体の照合、マイク一覧・固定選択・手動切替。
- 5秒テストの通常取得・検査・個別試聴用PCMの準備・通常完了／取消時の削除、入力レベル表示。
- 停止後のCAF検査・FLAC保存、時計区間・欠損を保持した共通会議ミックスの生成、正常完了manifestからの起動後再確定。
- Windows用9モデル候補の表示、取得・容量／SHA検証・配置・互換性検査・設定適用。Macの残り5候補とGPUは別途不足。
- CPUによる文字起こし・話者分離・議事録生成、共通の処理待ち・中断・再開・キャンセル・通知。追加の声照合・不明話者補完は除く。
- 会議一覧・改名・保存、本文・話者の手動修正、発言／会議／Markdown区間の音声再生準備、議事録の表示・編集保存・Markdown出力。
- 共通／会議別用語、議事録プロンプト、カラー・表示モード、内部ログ、録音ミニパネル・説明・停止確認、共通の安全な終了処理。
- Windowsのファイル選択・保存ダイアログ、起動失敗診断、WebViewのIPC権限制限、推論子プロセスのJob Object管理、QA用NSISビルドとWebView2資材同梱。

旧進捗にある「WASAPI製品未接続」「モデル取得未対応」「インストーラー未生成」は2026-10-24時点のコード・ローカルビルド記録と一致しないため、今回の未対応項目へ転記していない。

## 7. Windowsで実行した確認

テストでは既存の `app/src-tauri/target/debug/deps/mon_desktop_lib-4ad56abe7f3482c5.exe` を使用。更新日時は **2026-10-24 01:19:53 JST**、調査時点でこれより新しい `app/src-tauri/src/**/*.rs` は0件。今回はRustの再ビルドをしていないため、結果を新規ビルドの成功とは扱わない。

| 確認 | 結果 |
| --- | --- |
| `python -m unittest discover -s tests -p test_mon_tauri_platform.py -v` | 4成功、1エラー。CP932で `src/lib.rs` を読む際の `UnicodeDecodeError`。 |
| `python -X utf8 -m unittest discover -s tests -p test_mon_tauri_platform.py -v` | **5成功**。設定・ACL整合性が通ることと、各機能が実行できることは別。 |
| 上記Rustバイナリ `storage::windows_tests --nocapture` | **2成功**。日本語パスでの会議保存・再読込み、未知DBの拒否と既存ファイル保持。 |
| 上記Rustバイナリ `storage::backup::tests::snapshot_only_stages_registered_files_and_does_not_publish_raw_database --exact --nocapture` | **1失敗**。`src/storage/backup_tests.rs:181`、`prepare_backup_snapshot(...).unwrap()` で `Err("このOSの音声取り込みはまだ接続されていません")`。F02の容量取得ブロッカーを確認。 |
| IPC登録の機械照合 | 総数135、Windows除外14。下表のとおり。 |
| モデル定義の照合 | Mac固定8＋Whisper6、Windows生成済みカタログ9。Windows生成スクリプトの候補フィルターと一致。 |

### Windowsで登録されない14コマンド

| 分野 | コマンド |
| --- | --- |
| 会議削除 | `get_deletion_items`、`review_meeting_deletion`、`review_pending_deletion`、`get_deletion_paths` |
| モデル削除 | `get_model_deletion_items`、`review_model_body_deletion`、`review_pending_model_deletion`、`get_model_deletion_paths` |
| 音声削除 | `get_audio_deletion_items`、`review_meeting_audio_deletion`、`review_pending_audio_deletion`、`get_audio_deletion_paths` |
| 保存先管理 | `get_storage_bootstrap`、`review_storage_move` |

## 8. 実装済みだが実機検証が残るもの

コードがない項目とは別の検証残件。

- 実マイク・PC音声・Meet／Zoom／Teamsの指定アプリ録音、Windowsの権限拒否・排他使用・Bluetooth／USB機器変更。
- 録音中の手動マイク切替、PC音声未受信、実デバイスでの時計揺れと欠損、4時間安定性。C02/C03等の未実装項目は、検証だけで解消しない。
- WindowsネイティブUIのフォーカス・モーダル・複数画面・DPI・ミニパネル・音声再生・終了待ち。
- 9候補すべての重み取得・実解析と性能評価。既存記録の実モデル確認はtiny/base、標準話者分離、Qwen 2.5 1.5Bに限られる。
- 更新後の実機操作、CI全体の成功、公開署名、正式品質評価。ローカルNSIS生成済み記録はあるが、それだけでインストール・更新・実録音成功を証明しない。

根拠: `docs/windows-setup.md`、`docs/implementation-status.md` の2026-10-24追記と `output/windows-qa/fix-build-report.json`。後者はローカル出力でありGit追跡対象外。

## 9. 対応の順序

1. **保存・復元の基礎を揃える**: Windowsの空き容量取得、保存領域・DB・書き出し先の実体保持、復元ジャーナル・一時領域の所有／回収。F02/F03は容量取得だけで完了扱いにしない。
2. **録音の継続と証拠を揃える**: マイク／PC音声の再取得、状態遷移・永続履歴、原PCMの区間確定、5秒テストの異常終了回収、電源制御。
3. **基本操作を開通する**: 取り込みのネイティブ復号、会議・音声・モデル削除と未完了回復、保存先変更・起動設定。
4. **解析の同等機能を揃える**: 特徴量抽出ヘルパーのWindows化、声の登録・照合・不明話者補完、欠けた5モデルのWindows定義・検査。
5. **性能・配布を揃える**: GPU実行と試行・CPU切替、常駐ワーカー、ネイティブ外観、Windows向け診断・署名工程と実機評価。

新機能の実装着手時には、各行を「正常系」「取消」「アプリ異常終了」「再起動後の回復」「既存データ保持」に展開して検証する。本監査は修正済み宣言ではなく、その作業対象と根拠を確定したもの。
