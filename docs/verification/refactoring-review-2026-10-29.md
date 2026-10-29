# MON リファクタリング精査 2026年10月29日

MacとWindowsの機能差異防止、実装重複、メンテナビリティ、拡張性、セキュリティ境界を対象に、27件の改修候補を整理した。優先するのは、録音データの生成と読取りの契約、解析結果の保存確定後の工程登録、通信エラーの扱い、入力復旧の状態遷移、ファイル同期の保証である。これらには、現在のコードで確認できる契約差や中断時の工程抜けがある。

共通化の中心は製品ポリシー、保存形式、状態遷移、検証条件に置く。Core Audio、WASAPI、AudioToolbox、Media Foundation、OS固有のプロセス所有権・ファイルハンドル操作は、それぞれのadapterとして維持する。

## 対象と判定方法

- 対象コミットは `3589e2c3569e2f0a1a54e461f83ce0840eead97d`。開始時の作業ツリーに変更はなかった。
- React／TypeScript、共有Rust、macOS Swift helper、Windows C++ helper、推論worker、モデル生成ツール、ビルド・テスト経路を、4系統で分担して静的に精査した。以下の行番号はこのコミットに対応する。
- 短縮したRustパスは `app/src-tauri/src/`、Swiftパスは `probes/macos-capture/Sources/MONMacOSCapture/`、Windows captureのC++パスは `probes/windows-capture/` を基準とする。同一項目内のファイル名だけの参照は、直前のディレクトリを引き継ぐ。
- P1は運用前に優先して契約や回復性を修正・検証する項目、P2は仕様追加と保守に先立って段階的に整理する項目を表す。脆弱性の深刻度ではない。P0に相当する緊急事象は今回確認していない。
- 「確認済み」はソースに存在する違いや結合を指す。停止注入、長時間録音、Windows実機、電源断による再現は実施していない。セキュリティ項目も、悪用を実証した脆弱性一覧ではない。
- 製品コードは変更していない。実会議データ、稼働中アプリ、録音機器、外部サービスを操作していない。

## 優先度一覧

| ID | 優先度 | 対象 | 確認した問題または構造 |
| --- | --- | --- | --- |
| 01 | P1 | 原音声の分割保存 | Windows producerがRust readerの上限を共有しない |
| 02 | P1 | 解析後の自動処理 | 解析結果の保存確定と後続ジョブ登録の間に回復対象外の隙間がある |
| 03 | P1 | モデル取得 | HTTP成功条件とstderr公開条件がOS間で異なる |
| 04 | P1 | 入力復旧 | 再取得中・失敗の状態と理由コードがOS間で異なる |
| 05 | P1 | 永続化 | Windows内でDirectory syncの保証が一致しない |
| 06 | P2 | helper要求 | マイク切替とCLI引数のvalidatorが非対称 |
| 07 | P2 | アーカイブ | 検証範囲、全体予算、deadlineがOSごとに異なる |
| 08 | P2 | ファイル操作 | 同名APIの保証差とProof検証の重複がある |
| 09 | P2 | 子プロセス | 環境変数やstdioの制約を各呼び出し元が設定する |
| 10 | P2 | モデル管理領域 | Windows側の非公開条件が親ACL継承へ依存する |
| 11 | P2 | IPC権限 | 登録・権限・window roleが複数の宣言とガードへ分散する |
| 12 | P2 | 操作受付 | 競合可否と所有権の条件が複数controllerへ分散する |
| 13 | P2 | ジョブ | enumを途中で文字列へ戻し、種別ごとの列挙を繰り返す |
| 14 | P2 | バックアップ復元 | 復元側が各ドメインのJSON内部構造を直接知る |
| 15 | P2 | エラー | 表示文言やprefixが再認識・受付判定を制御する |
| 16 | P2 | 実行計画 | モデル設定とファイル指紋の組み立てが重複する |
| 17 | P2 | 音声取込み | container検証と製品の受理条件がSwift／C++／Rustへ分散する |
| 18 | P2 | 録音予算 | queueとtimeoutの値に共通の製品保証がない |
| 19 | P2 | モデルcatalog | macOS catalogがWindowsのモデル定義・生成上限の情報源を兼ねる |
| 20 | P2 | 録音helper | CLI、評価driver、製品session制御が混在する |
| 21 | P2 | 機器とアプリ列挙 | overflow、整列、自己除外の方針が異なる |
| 22 | P2 | 削除UI | 受付不明・再確認・取消・完了処理を3系統で実装する |
| 23 | P2 | 状態購読 | event、poll、世代管理、再読込の制御が重複する |
| 24 | P2 | IPCデータ契約 | RustとTypeScriptで型・検証・canonical化を手で同期する |
| 25 | P2 | 画面構成 | MainWindowの業務調停と全windowの起動・bundleが結合する |
| 26 | P2 | 回帰検証 | ブラウザー統合検査が通常のテスト・CIから分離する |
| 27 | P2 | CI起動条件 | 共通runtime生成ツールの変更だけではplatform CIが起動しない |

## 優先して修正する契約と回復性

### 01 原音声セグメントの上限を共通契約にする

**根拠**：`app/src-tauri/src/recording_segments.rs:16` は32,768セグメント、総JSON 32MiB、65,536 clock mapを上限とし、`:639` と `:668` で超過した広告済みprefixを拒否する。Swiftの `probes/macos-capture/Sources/MONMacOSCapture/RawRecordingSegments.swift:318` と `:420` は次の互換性のないprefixを公開する前に停止する。一方、`probes/windows-capture/raw_segments.h:21` はmap数だけを確認し、CAF・receiptを書き、sequenceとtipを更新する。

**影響**：Windowsが共通readerの受理できない録音prefixを公開できる。物理ファイルは残るが、通常の確定・原音声回復を妨げる。4秒区切りの件数上限は約36.4時間であり、clock mapの密度によっては総JSON上限が先に効く。

**改修と検証**：最初に保存形式の上限を一つの仕様から生成し、Windowsのcommit前チェックを揃える。次にreceipt・tipの生成を共有coreへ寄せる。PCM取得のcallbackまで共通化する必要はない。両producerの出力を同じRust readerへ通し、件数・総JSON・map数の上限直前／上限／超過、途中停止を確認する。

### 02 解析結果の保存確定後のジョブ登録を永続化する

解析結果の保存確定とは、結果をその会議で使用する文字起こしとしてDBに保存し、画面の表示対象にする処理を指す。

**根拠**：`storage/analysis_flows.rs:242` は会議で使用する解析版と終端の `review` を同一トランザクションで確定する。その後、`analysis_worker.rs:150` と再生成の `lib.rs:1043` が `speaker_completion::after_analysis` を呼ぶ。`storage/analysis_flows.rs:314` の回復対象はpending／transcription／diarizationで、reviewを含まない。`lib.rs:2517` 以降の起動にも、この後続登録を再実行する経路がない。話者補完成功から声照合登録への `analysis_worker.rs:252` にも同じ境界がある。ここでの相対パスは `app/src-tauri/src/` 配下を指す。

**影響**：解析結果の保存確定後・後続登録前の終了や登録失敗で、文字起こしは保存されているのに話者補完・自動声照合のジョブが存在しない状態が残り得る。初回処理と再生成で失敗の通知方法も異なる。既存の重複防止は、未登録の工程を回復する仕組みにはならない。

**改修と検証**：解析結果の保存確定と同じトランザクションに後続処理の意図を記録し、起動・workerが冪等に消化する。`storage/speaker_completion.rs:420` と `storage/voice.rs:328` の重複防止を利用する。対象の解析版と保存確定時の設定・自動処理意図を固定し、現在設定で過去会議を一律再処理しない。保存確定直後、登録直後、補完確定直後の停止注入で、工程の取りこぼしと二重登録、後続編集の保護を検証する。

### 03 ダウンロードの成功条件と診断公開を共有する

**根拠**：`app/src-tauri/src/model_download/transport.rs:208` はcurl stderrを公開せず、終了成功・HTTP 200固定marker・容量・SHAを要求する。`transport_windows.rs:33` のcurl引数にHTTP status取得がなく、`:149` は終了コード・容量・SHAで判定する。`:162` は失敗時にstderr先頭1000文字をエラーへ追加する。`model_download_ipc.rs:82` の処理から呼び出し元へ返る。

**影響**：同じ応答に対する受理条件が違い、Windowsの利用者向けエラーにはURL/CDN tokenやproxy・serverの診断が混入し得る。現在の公開catalogから秘密情報が漏れたことを示すものではない。SHA検証があるため、HTTP条件差をそのまま任意モデル受理と解釈してはいけない。

**改修と検証**：共有する `TransportPolicy` でHTTP終端条件、出力量、deadline、取消、公開可能なエラーを定義する。OS部は起動とpipe待機を担当する。tarとcurlの共用 `stream()` に診断公開方針を任せない。200／206／204／redirect、同じ本文で異なるstatus、巨大stderr、疑似token入りstderr、取消の同じfixtureを両OSへ適用する。

### 04 入力復旧の状態遷移を共通にする

**根拠**：Swiftの `MicrophoneRecovery.swift:68` と `SystemAudioRecovery.swift:49` は再試行開始を `reconnecting` として公開する。Windowsは `input_binding.h:17` でloss後にdisconnected／unavailableとし、`product_capture.h:273` の再準備開始でreconnectingに戻さない。`app/src/recording-input-state.ts:102` はそれぞれを別の文言で表示する。timeoutもSwift `SystemAudioRecovery.swift:107` のacquisition_timeout／unavailableに対し、Windows `product_capture.h:278` はrestart_failed／disconnectedである。nativeファイルは各 `probes/*-capture` 配下。

**影響**：同じ再取得中でも片方は「再取得中」、片方は切断・取得不能の表示になり得る。理由コードも異なり、案内・履歴・将来の自動判断を共通化しにくい。

**改修と検証**：scope固定、取得epoch、割込み、再試行、timeout、stop、遅延結果の破棄を一つの遷移仕様へまとめる。OS adapterは取得開始・成功・失敗・喪失を返す。同じイベント列から同じstatusとhistoryを出す契約テストを作り、timeout後の遅延成功とstop競合を含める。

### 05 ファイル同期の保証を揃える

**根拠**：`app/src-tauri/src/model_placement/fs_windows.rs:121` の `Directory::sync()` は実体確認だけを行う。一方、`windows_files.rs:84` は書込み権限付きdirectory handleでsyncを実装し、`native_fs.rs` が使用する。macOSの配置側 `model_placement/fs.rs:192` は実際に `sync_all()` する。`model_download/archive_windows.rs:67` などの呼び出し元には同じsyncという名前で見える。

**影響**：永続化を要求するAPIの保証がOS間だけでなくWindows内でも一致しない。どちらのWindows実装が実際の環境で成立するかは、通常権限・対象filesystemでの確認が必要であり、電源断による消失を実証した指摘ではない。

**改修と検証**：実体確認と永続化を別操作にし、ファイル・rename・directory entryそれぞれの保証水準を明示する。共通のdurability backendへ集約し、利用不能時の扱いも定義する。通常権限のNTFSでの同期、失敗注入、各commit境界からの再起動回復を確認する。

## OS差異を防ぐnative境界

### 06 マイク切替とCLI要求のvalidatorを揃える

**根拠**：`app/src-tauri/src/recorder.rs:188` はmode／UID／安全整数revisionを送る。Swift `MicrophoneSelectionRequest.swift:13` と `:34` はその組合せ・上限・通常ファイル・nofollow・容量を検証する。Windows `product_capture.h:407` は容量・schema・ownerを検証するが、modeを使わずUIDと安全整数上限の同等検証がない。CLIもSwift `App.swift:99` の重複引数・UUID v4検証と、Windows `product_capture.h:195` の後勝ち・文字種中心の検証が異なる。

**改修と検証**：要求・応答schema、ID形式、数値範囲、未知field、重複引数の扱いを共通定義し、validatorまたはfixture corpusを生成する。通常の親プロセスが作る要求は既に制約されるため、現行の利用者操作での脆弱性とは扱わない。破損要求、modeとUIDの矛盾、制御文字、revision超過、非通常ファイルを両OSで同じ結果にする。

### 07 アーカイブの検証と全体予算を共有する

**根拠**：`app/src-tauri/src/model_download/archive.rs:123` は全entryの名前・重複・件数10,000件・総展開32GiB、選択memberの種別、全体600秒を検査する。Windows `archive_windows.rs:36` は選択memberごとに `tar.exe -xOf` を起動し、それぞれに600秒を与える。全体件数・未選択entryの同等検証がなく、member数だけ再走査する。`model_download/tests.rs:10` の危険archiveテスト群はmacOS条件付きである。

**改修と検証**：archiveのhash、entry集合、重複・種別、総量、全体deadlineを共通policyへ分離し、readerを差し替える。両側ともarchiveと選択出力のSHA・容量を確認し、tarに任意パスへ展開させていないため、現状を任意ファイル上書きと断定しない。同一TAR corpusで受理・拒否、取消、時間予算を検証する。

### 08 安全なファイル操作の保証をAPIで表す

**根拠**：`app/src-tauri/src/storage/deletion/fs.rs:157` と `fs_windows.rs:16` はIdentity／Proof、時刻、rename後比較、directory scopeの多くを二重実装する。`native_fs.rs:268` の `remove_owned()` はUnixではobservedのidentityを使わないpath削除、Windowsでは保持identityを使うhandle削除となる。

**改修と検証**：Proofの意味・serialization・比較ロジックを共有し、OS部はidentity採取とhandle操作へ限定する。同名APIは同じ保証を満たすか、別の名前・型にする。現状の `remove_owned` 用途は内部ログ整理などであり、これをユーザーデータ削除の攻撃成立と解釈しない。symlink／reparse／hardlink、親差替え、実体差替え、WAL／SHMロックの共通契約ケースを用意する。モデル配置、読取り、削除の権限を万能APIへ統合する必要はない。

### 09 子プロセスを用途別builderから起動する

**根拠**：`app/src-tauri/src/local_engine.rs:113` は `OwnedCommand.command` を公開し、環境、cwd、stdioを各呼び出し元へ委ねる。推論の `transcription/whisper.rs:255`、`diarization/embedding.rs:329`、`local_engine/resident.rs:102` は `env_clear()` を使うが、`audio_import.rs:157` は親環境を継承する。録音、列挙、テストも `recorder.rs:76`、`capture_inputs.rs:97`、`capture_test.rs:166` で構築する。`native_environment()` はWindowsの必要変数を追加するがクリアはしない。

**改修と検証**：既存のプロセス所有権を維持し、capture／import／inference／system tool別builderで、環境allowlist、cwd、stdio、出力上限、timeoutを既定適用する。秘密環境変数やloader関連変数が不要な子へ渡らず、必要なSystemRoot等は残ることを確認する。環境継承からDLL注入等の悪用成立を実証したものではない。

### 10 モデル管理領域の非公開条件を明示する

**根拠**：Unix `app/src-tauri/src/model_placement/fs.rs:47` はモデル管理directoryの公開permissionを拒否し、0700／0600で作成する。Windows `fs_windows.rs:13` と `:48` は親固定・reparse等を検査するが、DACLのprivate検証・設定を行わない。一方、`windows_files/security.rs:71` と `:130` にSIDを使うprivate検証・設定が既にある。

**改修と検証**：`PrivateDirectory` を明示的な前提としてモデル配置にも適用する。既存公開directoryを拒否するか修正するかを決め、親の安全性に依存するならその証拠をAPIへ渡す。通常の非公開親からACLを継承する場合は既に保護されるため、現在の情報漏えいとは断定しない。継承停止・追加SID・公開親のfixtureを含める。

### 17 音声containerの受理条件を共通化する

**根拠**：`app/src-tauri/src/audio_import.rs:133` はMP3を共通処理、Windows WAV／FLACをRust、macOS WAV／FLACをnativeへ振り分ける。WAV判定はRust `audio_import/pcm.rs:17` とSwift `AudioImport.swift:130` に分かれる。M4Aのbox走査・track ID・chapter参照検証もSwift `AudioImport.swift:19` とWindows `m4a_container.h:14` に重複する。

**改修と検証**：製品としてのcontainer preflightと受理条件をRustへ寄せ、OS decoderを必要なcodecの復号に限定する。decoder固有の正常差と拒否条件を区別する。同じ音声corpusでaccept／reject、rate、channels、framesを比較し、PCMの許容差を定義する。特定の正常音声が片方だけ失敗する実機事例を確認したという指摘ではない。

### 18 保存遅延と停止応答の製品予算を定義する

**根拠**：Swift `RawRecordingSegments.swift:34` と `:197` のqueueは半秒・16MiB、Windows `packet_queue.h:17` は20秒・4,096 packetの上限を持つ。timeout／retryはWindows `input_preparation.h:15`、Swift `InitialCapturePreparation.swift:15` と `SystemAudioRecovery.swift:49` に分散する。

**改修と検証**：queueの位置が違うため数値を一律に揃えず、最大保存遅延、停止応答時間、メモリ予算、回復可能なtail量を共通policyにして各段へ配分する。writer遅延、disk full、取得遅延、stop競合を注入し、各OSで製品保証を満たすか測る。現時点の値の違いだけで性能不良と判定しない。

### 20 録音sessionとCLIと評価driverを分離する

**根拠**：macOS `Sources/MONMacOSCapture/App.swift` はCLI options、manifest、system／microphone recorder、全体実行を持ち、`:24` のフラグ組合せでprobeと製品経路を選ぶ。Windowsも `product_capture.h:226` の `run()` にsynthetic生成、取得、保存、状態公開、機器変更、終了が集中する。

**改修と検証**：`CaptureSession`、`InputSupervisor`、`PcmSink`、`MetadataPublisher` を分け、CLIとsynthetic／実機driverを薄くする。ファイルを小さくするだけではなく、状態遷移と保存契約の所有者を明確にする。既存fake driverを同じcoreへ接続し、manifest、history、原音声prefixの一致を確認する。

### 21 機器とアプリの列挙結果を共通の契約で整える

**根拠**：Swift `CaptureApplications.swift:74` は親・自分を除外し、128件超過をエラーにし、名前順にする。Windows `product_capture.h:61` はvisible windowのPID集合から128件で打ち切る。マイクもSwift `InputDevices.swift:39` は超過・重複を検証して整列するが、Windows `product_capture.h:68` は先頭128件である。

**改修と検証**：列挙APIはnativeに残し、返却metadataの検証、自己除外、整列、overflow表示を共通層へ寄せる。多機器・多アプリ時に一方が全体失敗、一方が説明なく欠落する差を解消する。129件、重複、同名、無効名、親・自分、列挙途中で消滅したPIDを同じfixtureで確認する。

## 共通Rust層の責務と型

### 11 IPC登録とwindow権限を一つの宣言から生成する

**根拠**：`app/src-tauri/app_commands.rs:1`、`src/lib.rs:2610` 以降のhandler、`permissions/application.toml`、各commandのwindow判定を同期する必要がある。ガードも `src/lib.rs:311` のdata_app、`src/model_download_ipc.rs:28` のmain、`src/internal_logs.rs:303` のrequire_main等に分かれる。

**改修と検証**：command宣言にroleと引数・応答を付け、handler、manifest、permissionsを生成する。境界で検証した `WindowRole`／`MainWindow` contextを内部へ渡す。現行は `tests/test_mon_tauri_platform.py:49` と `src/ipc_acl.rs:23` が宣言一致・実ACLを検証しており、権限迂回を発見したものではない。全command×window×originの許可行列を生成し、実ACLと照合する。

### 12 操作競合を資源単位の受付とリースへ集約する

**根拠**：`app/src-tauri/src/analysis_worker.rs:19` の状態に加え、録音 `:353`、取り込み `:394`、`analysis_worker/model_preparation.rs:70`、`data_maintenance.rs:269`、`:374`、`:414` が別々の条件を列挙する。SessionStore、ImportController、CaptureTests、DB上の所有権も呼び出し元が組み合わせる。終了の停止順序と巻戻しは `lib.rs:1579` に集中する。

**改修と検証**：`OperationKind × ResourceScope` の競合表と、受付・待機・取消・解放を扱う共通リースを導入する。意図的な併存条件を保ち、単一busyへ潰さない。最初に既存条件を表へ写し、全組合せ、待機中取消、panic、shutdown中受付、古いリース解放を検証する。UIのblockerも同じ判定結果から作る。

### 13 ジョブ種別と状態をenumのまま通す

**根拠**：`app/src-tauri/src/storage/jobs.rs:10` のJobSpecはenumを持つが、`:56` のJobSnapshotではkind／state／pause_reasonが文字列になる。実行と失敗分類は `analysis_worker.rs:205`、受付対象SQLは `storage/jobs.rs:426`、音声要否は `:1158`、形式判定は `current_formats.rs:14` にそれぞれ列挙される。

**改修と検証**：DB境界でJobKind／JobState／PauseReasonへ変換し、音声要否、runner、保存形式、失敗分類を網羅的matchへ寄せる。IPCの既存文字列表現は保持できる。種別追加時にqueuedのまま実行されない、既定のtranscription_failedへ落ちる等の更新漏れを型検査で防ぐ。全enum値のDB・JSON往復、未知値拒否、遷移表を確認する。

### 14 ジョブの復元codecを各ドメインに置く

**根拠**：`app/src-tauri/src/storage/backup/restore_fixed/config.rs:24` がASR・分離・声照合・補完の内部JSONキーとIDを直接変換する。checkpointの変換は `checkpoints.rs:67`、通知用の出力解釈は `storage/jobs.rs:111` に別途存在する。ASR等には既にドメイン所有の復元codecも一部ある。

**改修と検証**：各ドメインがdecode／validate／参照ID変換／結果要約を持ち、backupは型付きresolverを渡す。履歴を保存することと、実行可能なplanを再構築することを分ける。現在形式だけを受け付ける既存方針を維持し、旧形式移行の追加を前提にしない。全種別のbackup→ID変更復元、参照追加・欠落・改変、別会議参照、削除音声を含む履歴を検証する。

### 15 エラー分類を表示文言から分離する

**根拠**：`app/src-tauri/src/transcription/guard.rs:221` と `:246` が日本語prefix入りエラーを作り、`transcription.rs:842` と `analysis_worker.rs:308` がprefixで再認識や失敗分類を決める。UIも `app/src/meeting-deletion.tsx:69`、`model-deletion.tsx:65`、`audio-deletion.tsx:80` がnot_started prefixを解析し、再起動理由は日本語の完全一致で判定する箇所がある。

**改修と検証**：QualityRejected、Cancelled、EngineFailure、NotStarted、ResultUnknown、RestartRequired等を型・固定codeで返し、利用者向け文言は最後に生成する。表示文の修正やラップで自動再認識・再送可否が変わらないことを検証する。既存 `guard::Reason` を保持して渡せば、失敗理由の情報も失わない。

### 16 モデル選択から固定実行計画を作るbuilderを共有する

**根拠**：`app/src-tauri/src/analysis_flow.rs:111`、`voice.rs:127`、`diarization.rs:52` がSherpaConfigのファイル・設定を転記し、`model_compatibility/probe.rs:241` にも組立てがある。ファイル指紋の取得や取消可能性も呼び出しごとに異なる。

**改修と検証**：選択設定から `CapturedExecutionPlan` を作るbuilderに、必須ファイル、設定転記、指紋取得を集約する。probeと通常実行の検証強度は明示的な引数にする。後段検証が既にあるため、現状の転記重複を検証欠落そのものと扱わない。同じ選択に由来する共通のエンジン設定・ファイル指紋の一致、新項目の反映、取消、実体差替え・削除の拒否を確認する。用途と選択プロファイルが違う計画全体を同一にする必要はない。

### 19 モデル定義とplatform runtimeを分ける

**根拠**：`tools/mon_windows_runtime.py:92` がmacos.jsonを加工してWindows catalogを作る。`app/src-tauri/src/model_official_catalog.rs:27` と `:82` のChoice→OptionView変換も重複する。さらに `generation/model_budget.rs:59` はWindowsでもmacos.jsonからLLMのhash・sizeに対応するtoken上限を探す。

**影響と改修**：同じLLMを共有する現在構成では成立するが、macos.jsonにhash・sizeのないWindows限定LLM重みを追加すると、列挙・選択できても生成上限が未確認として失敗する構造である。既存重みを再利用するruntime構成の追加が必ず失敗するという意味ではない。重み、license、token limitsをOS非依存のModelDefinitionへ、exe／DLL／Metal／VulkanをPlatformRuntimeへ分け、同じcompilerでtarget別catalogを生成する。全targetで列挙→review→capacity解決→runtime bindingの整合性を確認する。

## ReactとIPC契約

### 22 削除操作の受付と復旧を共通controllerへ寄せる

**根拠**：`app/src/meeting-deletion.tsx:19`、`model-deletion.tsx:18`、`audio-deletion.tsx:26` 付近に、rpcBusy、unknown、pending request、choice、completed、openedの管理がある。編集保存・再生停止、review、送信、受付不明時の照合、取消、receipt処理を各々実装している。例えばmodelのsendはすべての失敗をunknownとする一方、meeting／audioではnot_startedを区別する。

**改修と検証**：`reviewing → ready → submitting → unknown → terminal` を共通controllerへ置き、対象ID、receipt、編集保存、再生停止、確認文面、追加同意をstrategyとして渡す。音声のみ削除の同意等を共通化で消さない。応答喪失、遅延event、古いrevision、対象変更、再読込、取消を3対象共通のテストへする。先に既存の差を仕様として分類してから統合する。

### 23 eventとpollを共有する状態購読機構にする

**根拠**：`app/src/session.ts:29`、`data-operation.ts:29`、`recording-confirmation.tsx:19`、`audio-import.tsx:44`、`analysis.tsx:37`、`minutes.tsx:87` に、購読、poll、focus再取得、disposed、世代判定、in-flight制御が繰り返される。sessionはin-flight抑制がなく、analysis／minutesは再読込の再要求も調停する等、実装が異なる。

**改修と検証**：resource keyごとのsnapshot storeで、検証、単調revision、event先行購読、poll fallback、in-flight調停、再接続、可視性を共有する。副作用を伴う解析結果の保存確定・操作確認は購読処理から分ける。古い要求の失敗が新しい成功表示を上書きするケース、eventとpollの逆転、windowのmount／unmountをfake clockとtransportで検証する。poll間隔を一律に揃える必要はない。

### 24 型とcanonical serializationの手同期を減らす

**根拠**：`app/src/model-download-state.ts:6` の型、`:30` のcanonicalRecipeは、`app/src-tauri/src/model_download_recipe.rs:17` のstruct、`:139` のserialization、`:169` の同梱素材検証と対応する。TypeScript側にはRustのstruct順序を保ってSHAへ結び付ける旨のコメントがあり、field順、bundle URI、engine版、roleを両側で更新する必要がある。

**改修と検証**：Rust等の単一schemaからIPC型と構造validatorを生成し、業務上の検証はnativeを正とする。hash対象のbytesはnativeが返す方法、または明文化したcanonical形式とcross-languageのgolden vectorで固定する。単なる型生成だけではhash互換性を保証できない。既存receipt・backupに結び付くhashを無断で変えず、null／省略、field順、非ASCII、数値、backend mapを往復検証する。

### 25 MainWindowと各windowの起動を分割する

**根拠**：`app/src/main.tsx:163` から、録音準備、マイク切替、編集保存、削除、navigation、close調停、各画面を同じcomponentが扱う。`:270` のlock合成と `:320` 以降の終了処理は業務上の依存関係を持つ。`:81` と `:843` は全windowの入口も同じmoduleに置く。OSはStartupGateのPlatformContextと `navigator.platform` の二系統で判定される。全画面が静的importされ、本番ビルドのJSは一つの約1,035kB chunkになった。

**改修と検証**：録音操作、navigation／編集保存、終了調停を用途別controllerへ分け、MainWindowをcompositionへ寄せる。window role別entryまたはdynamic importで浮動windowに必要な機能を限定する。起動診断より前に必要なwindow chromeの情報は、storage起動から独立したnative情報にする。chunk量だけで体感遅延を断定せず、各windowの読み込み・接続・停止保護と初期化時間を測る。

**関連する現行差**：`app/src/voice-state.ts:20` とREADMEのmacOS限定説明に対し、`app/src-tauri/src/storage/voice.rs:394` は両OS対応である。これは現在のWindows機能欠落ではなく古い説明である。対応可否と理由codeをbackendのcapabilityとして返し、OS名を業務UIへ埋め込まない。

## 差異を検出する検証とCI

### 26 ブラウザー統合検査を通常の検証入口へ接続する

**根拠**：`app/package.json` のtestは `tests/*.test.ts` を実行する。追跡対象の133ファイル中56でreadFileSyncを使用し、例えば `native-save-dialog.test.ts` は実装文字列と順序を検査する。一方、純粋な状態遷移テストも多数あり、これら56ファイルすべてが静的検査だけという意味ではない。`app/tests/run-browser-checks.mjs:19` に96スイートが登録されているが、既存browser sessionとCLIを別途要求し、通常のnpm test／Makefile／platform CIからは呼ばれない。残る単独driver等を一律に未登録の回帰検査とは扱わない。

**改修と検証**：既存runnerを再利用して、合成IPCでの主要操作をCI向けに自己完結して起動する入口を作る。文字列テストはACL宣言の整合性などに残し、保存・取消・応答喪失・focus・keyboardの条件はbehaviorで検証する。MacのMetaとWindowsのControlをparameter化する。合成ブラウザー検査と、WKWebView／WebView2の実window・native検証を区別し、それぞれの実行条件と最低限の対象を明文化する。

### 27 CIの対象パスと検証commandをまとめる

**根拠**：`.github/workflows/desktop-platform-build.yml:6` と `:26` は `tools/mon_windows_*.py` と一部の固定ファイルを対象にするが、`tools/mon_asr_bundle.py`、`mon_inference_runtime.py`、`mon_whisper_runtime.py`、Makefile等は含まない。Windowsは `tools/mon_windows_runtime.py:171` → `mon_windows_workers.py:12` → mon_inference_runtimeに依存し、mon_asr_bundleも使用する。Macは `app/build-macos.sh:8` からmon_whisper_runtimeを使用する。これらだけの変更では対応するplatform CIが起動しない。Windows captureの別workflowは手動起動のみである。評価用のmon_audio等と対応する大半のPythonテストも、現在の自動検証対象には含まれない。

**改修と検証**：共通tooling・fixture変更で必要なplatform検証が起動するようpath集合を広げ、OS非依存Python検査は軽量jobへ接続する。ローカルとCIで同じ検証entryを使い、OS固有の準備だけwrapperへ置く。build script全体を無理に同じ言語へ書き直す必要はない。代表的な変更ファイル集合に対するCI選択結果を検査し、workflow自体の変更時にも確認する。

## 段階的な実装順序

1. **現行契約を固定する。** 01、03、04、05、06、07について、両OSへ同じ異常系・境界値corpusを適用する。検証の通常入口とCI対象も26、27で整える。
2. **狭い範囲の不一致を修正する。** Windowsのsegment予算、通信診断、復旧statusを先に揃える。fs同期は実環境で保証を確認してからAPI契約を確定する。大規模統合まで修正を待たない。
3. **永続工程を補う。** 02の意図記録と回復を導入し、保存確定した解析結果・人間の後続編集・保存確定時の設定への結び付きを停止注入で検証する。
4. **受付と型を整える。** 11〜16、24でcommand、job、error、execution planの契約を揃える。現在の権限、取消、idempotency、保存形式を保つ。
5. **重複するcontrollerを移す。** 22、23、25のUI調停と、20のnative sessionを段階的に分離する。既存経路との出力比較を通して切り替える。
6. **拡張の情報源を整理する。** 17〜19、21でcontainer受理条件、予算、catalog、列挙policyを共通化する。既存OSの差を明示したうえで、新しいモデル・機器・targetを追加できる形にする。

## 維持する設計と今回の検証

既存のTauri capabilityとwindow別制限、native側の再検証、SHAと実体identityの検証、プロセス所有権、解析結果の保存確定時の楽観的ロック、人間の後続編集保護は維持する。共通化のためにこれらを削らない。Core AudioとWASAPI、MetalとVulkan、POSIX権限とDACL等の必要なOS差も明示的に残す。

内部ログが生の会議本文やlocal pathを含み、会議削除後も最大30日残る点はREADMEと実装で告知された現行仕様である。これを未告知の漏えいとして扱わない。03は通信失敗時の未分類stderrを利用者向けエラーへ追加する方針差を指す。

今回実行した検証は以下のとおり。

| 検証 | 結果 |
| --- | --- |
| `npm test` | 838件中837成功、1スキップ、失敗なし |
| `npm run build` | TypeScript検査・Viteビルド成功 |
| `python3 -m unittest discover -s tests -p test_mon_tauri_platform.py -v` | 6件成功 |

ビルドには既存のSonner directiveとchunk容量の警告がある。今回、Rust全体テスト、Swift／C++ nativeテスト、browser統合runner、実録音・実モデル・installer・電源断試験は実行していない。各項目の改修完了判定には、その項目に記載した契約テストと必要なOS実機確認を用いる。
