# MacとWindowsの実装差分監査

調査日: 2026-10-26 JST。調査開始時のHEADは `20253ba2526a7332887ec631a674089348413ad5`、終了確認時は `c6a49e7292fc6b0f7032501277185b3ac9fe28c2`。その間の設定画面・ブラウザー検査の4ファイルの更新も確認した。対象には開始時点の作業ツリーを含む。

同日の監査後修正は末尾の「監査後の実装」を参照。以下の指摘・再現結果は修正前の記録として保持する。

主要機能の接続は両OSにあるが、録音範囲、録音容量、再接続、音声削除、WAV対応範囲に6件の挙動差が残る。優先するのは、Windowsの音声削除後に変換途中の音声が残る経路と、全体録音で別の出力機器の音声を取得できない経路。

10月24日の監査で未対応だった取り込み、バックアップ、追加復元、会議・音声・モデル削除、保存先変更、声の登録・照合、話者補完、常駐推論、GPU試行は現在のWindowsコードにも接続されている。以前の未対応一覧を現状の結論として流用していない。

## 対象と判定方法

- React画面、145件の独自IPC、RustのOS条件分岐、Swift録音、C++録音、モデルカタログ、配布設定、CIを照合した。IPC登録の14個の条件分岐はいずれもMacとWindowsの両方を含み、登録件数の不足は見つからなかった。
- 主な対象は録音と5秒テスト、入力変更・復帰・停止、FLAC確定と回復、再生、取り込み、解析、削除、バックアップと追加復元、保存先、起動・常駐、ウィンドウ、モデルと配布。
- 「再現」は現行ソースの対象処理を合成入力で実行したもの。「コード経路確認」はproducerから共有処理までの分岐・呼出順を確認したもの。WindowsのGUI、WASAPI実機、インストーラーは今回実行していない。
- この監査では製品コード・既存テストを編集していない。開始時の既存変更にも書込み・巻戻しはしていない。利用者データでの録音・削除・復元、追加モデル取得は行っていない。

パスは特記がない限りリポジトリルートを基準とし、`storage/` で始まる省略Rustパスは `app/src-tauri/src/` を基準とする。行番号は調査時点を指す。

## 修正対象となる挙動差

| ID | 優先度 | 差分と影響 | 確認方法 |
| --- | --- | --- | --- |
| A01 | 高 | Windowsの音声だけの削除で、所有を証明できる未登録FLACや変換途中音声が削除対象に入らない | コード経路とMacの既存回帰試験 |
| A02 | 高 | Windowsの全体録音が最初の既定出力機器に固定され、別機器への出力や既定出力の変更を追えない | コード経路。出力変更の実機確認が必要 |
| A03 | 中 | Windowsの通常録音が1入力あたり1,025個目の時計区間で停止する。Macの通常録音は65,536区間 | C++実ソースで境界再現 |
| A04 | 中 | Windowsは256 KiBを超える録音manifestを保存確定時に拒否する。Macには32 MiBまで扱う経路がある | 容量用JSONとコード経路 |
| A05 | 中 | Windowsのアプリ音声の再接続が最大15秒、マイクの保存・状態更新・停止要求の受付を止める | 呼出順と待機時間のコード経路 |
| A06 | 中 | 同じPCM WAVでも、有効ビット数と格納ビット数が異なるとWindowsだけが拒否する | Mac実復号とWindows実パーサーで再現 |

### A01 Windowsの音声削除にFLAC作業領域の所有検査が接続されていない

Windowsの通常録音も、原音声セグメントがある場合は共通のFLAC確定処理を通る。変換音声を作った後のDBコミット失敗や中断により、管理下に未登録の最終FLACや `.part` 音声が残り得る。

Macでは変換計画・チェックポイント・実体の照合から、そのファイルを `recording_flac` と判定し、明示同意後の「音声だけの削除」に含める。Windowsはこの検査とヒント付与を `#[cfg(unix)]` で除外している。登録資産にも録音セッション直下の既知ファイルにも該当しない出力は `Unclassified` になり、未分類ファイルを保持する方針に従って残る。「未登録の音声も削除」に同意しても未分類ファイルは削除対象にならない。

根拠:

- `app/src-tauri/src/storage/recordings.rs:787` と `:806`: Windowsの原音声セグメント検証とFLAC公開。
- `app/src-tauri/src/storage/recordings/windows.rs:23`: セグメント録音を共通 `flac::Publication` に渡す。
- `app/src-tauri/src/storage/deletion/files.rs:204`, `:428`, `:748`, `:942`: 作業領域の所有検査と `recording_flac` の付与をUnixに限定。
- `app/src-tauri/src/storage/recordings/flac/inventory.rs:9`: チェックポイントに結び付く音声の判定。
- `app/src-tauri/src/storage/deletion/audio.rs:351`: `recording_flac` は音声削除対象、ヒントがなければ未分類。

Macの `flac_workspace_failed_commit_is_read_only_then_explicit_audio_only_deletion_retains_metadata` と `flac_workspace_allocated_inode_is_audio_but_pre_identity_and_replaced_files_are_not_adopted` は今回成功した。しかし、これらを含むテストモジュールはWindowsではコンパイルされない。

修正方向: Windowsでも同じ所有検査を接続する。ファイル名だけで削除範囲を広げず、Windowsの実体・権限・変換チェックポイントを照合する。DB失敗後の未登録FLACと所有確定済み `.part` を削除し、所有未確定・差し替え済みファイルを残す試験をWindowsにも通す。

### A02 Windowsの全体録音は出力機器をまたいで取得できない

Windowsの `system` モードは `GetDefaultAudioEndpoint(eRender, eConsole)` で1台の出力機器を開き、その `IAudioClient` を使い続ける。制御ループの機器選択追従はマイクだけで、PC音声には既定出力IDの比較や変更通知がない。PC音声はエラーで切断されたときだけ開き直す。

WASAPIの通常loopbackが取得するのは選択した出力endpointの音声であり、WASAPIを直接使うアプリの出力切替はアプリ側で実装する必要がある。[Microsoftのloopback仕様](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording)、[出力切替の仕様](https://learn.microsoft.com/en-us/windows/win32/coreaudio/stream-routing)でも確認した。

そのため、会議アプリが別の出力機器を使う場合は「PC全体」でもその音声を取得できない。また、旧出力機器が接続されたまま既定出力をスピーカーからUSBヘッドセットなどへ変えた場合、旧クライアントがエラーにならなければ切替先を取得しない。状態報告は接続中のままになり得る。

Macの製品経路は `stereoGlobalTapButExcludeProcesses` によるプロセス音声の全体取得で、初期の既定出力1台に固定する実装ではない。Windowsのアプリ選択モードもプロセスツリー取得を使うため、この指摘は全体録音の経路に限定する。

根拠:

- `probes/windows-capture/product_capture.h:86`: endpointの選択。
- `probes/windows-capture/product_capture.h:221`: 接続済みlaneの再取得を行わない。
- `probes/windows-capture/product_capture.h:273`: 定期的に選択を確認するのはマイクだけ。
- `probes/macos-capture/Sources/MONMacOSCapture/NativeCoreAudioCaptureDriver.swift:24`: Macの全体tap。
- `probes/windows-capture/main.cpp:96`: Windowsのアプリ選択取得。

修正方向: 「PC全体」の取得範囲を複数出力も含めて定義し、既定出力変更を検出して時計・欠損・履歴を維持したまま追従させる。旧機器を抜かずに既定だけを変える条件と、アプリ別に別機器へ出力する条件を実機試験に含める。既定変更への追従だけでは、同時に複数機器へ出る音声をすべて取得することにはならない。

### A03 Windowsの通常録音に古い時計区間数の上限が残る

Windowsの全体トラック用 `PacketClock::append` は `parts.size() >= 1024` で例外を投げる。同じlaneの後段には65,536区間を想定する検査があるが、その前に失敗する。原音声セグメントごとの時計を初期化しても、全体トラックの `Lane::clock` は初期化されない。

Macは通常録音で原音声セグメントを使用すると、マイクとPC音声の時計容量を65,536区間へ拡張する。Windowsだけ、入力断・明示的な不連続・時計区間を分けるギャップが多い録音を早く打ち切る。

再現は本番の `packet_clock.h` をそのまま読み込む独立C++ハーネスで行った。480フレーム、40 ms間隔の正方向の時計区間を追加すると、1,025回目で `too many recording clock discontinuities` となり、確定済み配列は1,024要素だった。

根拠:

- `probes/windows-capture/packet_clock.h:87`: 1,024区間の上限。
- `probes/windows-capture/product_capture.h:126`: その後に置かれた65,536区間の検査。
- `probes/macos-capture/Sources/MONMacOSCapture/App.swift:1144`: 通常マイク録音の容量選択。
- `probes/macos-capture/Sources/MONMacOSCapture/SystemPCMWriter.swift:30`: PC音声の容量選択。
- `probes/macos-capture/Sources/MONMacOSCapture/CapturedClockMap.swift:21`: 65,536区間。

修正方向: 1つの原音声セグメントと録音全体の上限を分け、通常録音の全体容量を両OSで揃える。原音声・FLAC・DBの既存の各容量検査も通ることを確認する。

### A04 Windowsは大きい録音manifestを保存確定できない

共有Rustコードの `capture_manifest::MAX_BYTES` はMacで32 MiB、Windowsで256 KiB。大きいmanifestについて原音声のownerを検査する経路もUnixだけに限定される。

C++ producerは両入力の全体時計配列を最終manifestへ出すため、A03の1,024区間以内でも両入力の合計で容量制限に達し得る。容量確認用に、各入力1,024区間と原音声ownerを持つJSONを作成したところ、336,818バイトだった。これはWindowsの262,144バイトを超える。比較は容量ゲートの再現であり、音声ファイルを伴うWindowsの保存統合試験ではない。

producerが `saved` として終了しても、Rust側の読取り・保存確定でmanifestを拒否し、通常の保存完了へ進めなくなる。監視側の `manifest_bytes` も同じ制限を使用する。原音声を削除する処理ではないが、利用者には回復を要する録音になる。

根拠:

- `app/src-tauri/src/capture_manifest.rs:5`, `:28`: OS別容量と大容量owner検査。
- `app/src-tauri/src/recorder.rs:1405`: 監視側の容量制限。
- `app/src-tauri/src/storage/recordings.rs`: 保存確定時にも `capture_manifest::validate_size` を使用。
- `probes/windows-capture/product_capture.h:134`, `:305`: 全体時計を最終manifestへ出力。
- `app/src-tauri/src/capture_manifest/tests.rs`: 大容量の2試験はいずれもUnix限定。

修正方向: Windowsにも同じ大容量の所有検査を接続する。容量定数の変更だけで、owner確認のないJSONを許可しない。両入力に多数の区間を持つWindows producerの出力から、停止・保存・回復まで通す。

### A05 Windowsのアプリ音声再接続が制御ループを待たせる

Windowsの `reconnect(system, false)` は、マイクの保存と停止確認を行う同じループから同期的に `activate_process_loopback` を呼ぶ。この処理は `WaitForSingleObject(..., 15000)` で最大15秒待つ。停止ファイルの確認と状態報告は、その呼出しが戻った後にしか進まない。

音声取得用スレッドは分離済みなので、待機中もマイクの取得はキューへ続く。ただし、マイクのPCMと原音声セグメントのディスク保存は滞留し、停止要求やマイク切替、最新状態の反映も遅れる。キューの上限は20秒分で、待機・保存負荷が重なる条件の余裕が小さい。

Macはシステム音声の再取得を所有する非同期Taskに分け、5秒の取得タイムアウト中もマイクと制御ループを継続する。タイムアウト後も元のOS操作が戻るまでは所有を維持し、並行した再取得を増やさない。

根拠:

- `probes/windows-capture/main.cpp:113`: 15秒の同期待機。
- `probes/windows-capture/product_capture.h:87`, `:221`, `:276`, `:300`: 同じループでの再取得、保存、停止確認。
- `probes/windows-capture/packet_queue.h:21`: 20秒分の上限。
- `probes/macos-capture/Sources/MONMacOSCapture/App.swift:746`: 非同期再取得と所有管理。
- `probes/macos-capture/Sources/MONMacOSCapture/SystemAudioRecovery.swift:107`: 5秒のタイムアウト。

修正方向: Windowsも再取得を1件だけ所有する非同期処理へ分離する。取得待機中にマイクの確定セグメントが増えることと、停止受付が待機終了を待たないことを、遅延する模擬WASAPIで確認する。

### A06 有効24ビットを32ビットに格納するWAVをWindowsだけが拒否する

WindowsのPCM WAV実装は、WAVEFORMATEXTENSIBLEの `wValidBitsPerSample` が格納ビット数と等しい場合だけを許可する。MacはOSのPCM復号を使い、有効24ビットを32ビット領域に格納したファイルを読み込める。

同じ独立fixtureについて、現行の `AudioImport.swift` を組み込むファイル専用ハーネスと、現行のWindows `wave` 関数を抽出したハーネスを実行した。マイク・PC音声は使用していない。

| 条件 | 結果 |
| --- | --- |
| 入力 | PCM、48 kHz、mono、480フレーム、格納32ビット、有効24ビット |
| Mac | 成功。`detectedFormat=wav`, `codec=pcm`, `frames=480`, `durationUs=10000` |
| Windowsパーサー | `WAVの有効ビット数が未対応です` |

根拠:

- `app/src-tauri/src/audio_import/pcm.rs:57`: 拡張WAVの読取り。
- `app/src-tauri/src/audio_import/pcm.rs:65`: 有効ビット数と格納ビット数の完全一致を要求。
- `probes/macos-capture/Sources/MONMacOSCapture/AudioImport.swift:77`: Macの実復号。

修正方向: 整数PCMの有効ビット数・配置を検証して対応範囲を揃え、このfixtureの取り込み・再生・PCM値を両OSで比較する。WindowsのUIにはWAV自体を対応形式として表示しているため、現状はOSによって同じファイルの可否が変わる。

## 保護検査と検証体制の差

### 削除済み音声の利用拒否が一部のWindows読取り経路にない

次の7箇所の `deletion::audio::require_source` が `#[cfg(unix)]` のままで、Windowsでは削除完了の型付き履歴を検査しない。

| 経路 | 場所 |
| --- | --- |
| 発言の音声参照とトラック再生 | `storage/playback.rs:559`, `:672` |
| 会議再生の対象検査 | `storage/meeting_playback.rs:367` |
| 録音音声プランと解析音声プラン | `storage/transcription.rs:169`, `:208` |
| 話者分離の受付と固定入力読取り | `storage/diarization.rs:74`, `:266` |

通常の削除後は共有のトラック・資産状態検査やジョブ受付でも拒否するため、これだけで「削除した音声を再生できる」とは判定しない。一方、Macが行う削除完了履歴の整合性検査と、readyフラグだけに依存しない拒否がWindowsのこれらの経路から抜けている。両OSを含む条件へ揃え、破損履歴・不整合な状態・保存済み入力を使う試験をWindowsにも移す必要がある。

### Windowsの全テスト成功でも通らない共通領域がある

以下のテスト群は、実装の多くが共有化された後もUnix限定のコンパイル条件のまま。Windowsで `cargo test --lib` を実行しても、この検証は実行されない。

- `storage/recordings/flac.rs:25`: 今回Macで実行したFLAC公開・作業領域・移動・回復のテスト群。
- `storage/deletion/audio_completion.rs`: 削除完了と利用不可の検証群。
- `capture_manifest/tests.rs`: 大容量manifestの2試験。

Windows専用の合成録音・解析・削除試験は存在する。ただし、連続PCMの長時間試験だけでは、全体時計の高密度化、FLAC公開失敗後の削除、履歴の不整合を検証できない。OS操作部分をfixtureへ分け、同じ保存・削除の条件を両OSで走らせる必要がある。

また `.github/workflows/desktop-platform-build.yml:6` のPR/push対象パスに `probes/macos-capture/**` がなく、Swift録音だけを変更したPRではこの両OSワークフローが起動しない。Mac録音の変更を共通の回帰へつなぐには、対象パスにも追加する必要がある。

## OS固有の実装として区別する差

以下は機能欠落とは判定していない。

| 項目 | Mac | Windows |
| --- | --- | --- |
| 録音基盤 | SwiftとCore Audioのtap | C++とWASAPI、アプリ指定時はプロセスツリー |
| GPU | Metal | Vulkan。実試行・デバイス固定とCPUへの退避経路あり |
| 保存領域の制約 | ローカルAPFS | ローカルNTFS |
| メインタイトルバー | OS標準 | ReactのタイトルバーとWin32/DWMの調整 |
| 常駐入口 | Dockとメニューバー | タスクトレイとウィンドウ |
| 起動失敗からの操作 | 終了後に再起動 | 既存プロセスの表示・確認した対象の再起動UIもある |
| 配布 | appとDMG、Developer IDと公証の工程 | NSIS、Authenticodeの工程 |

モデル候補はMacの共有カタログとWhisper派生定義からWindows用を生成する構成で、両OSとも14候補を提供するコードがある。取り込み、削除、移動、声登録を「Windows未実装」とする現行のIPC除外は見つからなかった。バイナリ・GPU方式・ネイティブOS処理が異なるため、この接続確認は認識精度・速度・実機安定性の同等性を意味しない。

## 今回の実行結果

| 確認 | 結果 |
| --- | --- |
| フロントエンド `npm --prefix app test` | 803成功、0失敗、1除外 |
| OS設定・IPC・権限 `test_mon_tauri_platform.py` | 5成功 |
| Rust `capture_manifest::tests` | 2成功 |
| Rust `storage::recordings::flac::` | 55成功、0失敗、11除外 |
| Rust `storage::deletion::audio::intent::removal::completion::tests` | 24成功、0失敗、1除外 |
| C++ `packet_clock_tests.cpp` と `packet_queue_tests.cpp` | 2実行とも成功。Mac上でOS非依存部分をコンパイル |
| C++全体時計の上限 | 1,025個目で例外、1,024個保持を再現 |
| 両入力の大容量manifest | 各1,024区間で336,818バイト。Windowsの容量上限超過 |
| 有効24ビットの拡張WAV | Macで成功、Windowsの実パーサーで拒否 |

Rustの成功はMac上の実行結果。除外には実モデル、Swiftの専用fixture、専用の異常終了fixtureなどを要求するテストがあり、今回成功に算入していない。TypeScript/Viteの新規ビルド、Rust全体、リモートCI、Windowsのネイティブビルドと実機録音は今回の結果に含まない。

一時的な再現素材と実行ログ:

- `/tmp/mon-platform-audit-20261026/evidence.json`: 時計上限、manifest容量、IPC件数。
- `/tmp/mon-platform-audit-20261026/wav-evidence.json`: 同一WAVに対する両実装の結果。
- 同ディレクトリの `capacity.cpp`, `windows-wave-parser.rs`, `ImportHarness.swift`, `valid24-in32.wav`, `dense-manifest.json`: 再現素材。
- `/tmp/mon-platform-audit-frontend.log`, `/tmp/mon-platform-audit-config.log`, `/tmp/mon-platform-audit-flac.log`, `/tmp/mon-platform-audit-audio-completion.log`: 今回の実行ログ。

一時ファイルはOSの整理で消えるため、判定に必要な条件・数値・ソース位置は本記録にも記載した。

## 監査後の実装（2026-10-26）

更新したプランの実装を同じ作業ツリーへ反映した。既存のモデル削除・FLAC試験の編集を保持した。公開IPC・DB・保存形式・データ移行は変更していない。

| 対象 | 実装と確認範囲 |
| --- | --- |
| A01・削除後の利用拒否 | WindowsでもFLAC変換計画・チェックポイント・実体を照合し、所有確定済みの未登録FLACや途中音声を明示同意後の対象とする。未知・所有未確定・差し替え済みファイルは保持。再生・音声プラン・話者分離の7箇所へ履歴検査を接続。共通fixtureをWindowsでもコンパイル・実行する構成へ変更。Macの重点回帰95成功・12明示除外。 |
| A02 | Windows全体録音を、ヘルパー自身のプロセスツリーを除外するprocess loopbackへ変更。指定アプリは元の選択PID・実体・生存を照合し、そのツリーに限定。全体取得への退避は行わない。対象PIDの決定は可搬テスト、全出力の実録音は未実施。 |
| A03 | `PacketClock`の容量を選択式にし、通常録音65,536、5秒テスト1,024とする。原音声の126区間での早期封印・FLAC既存上限を維持。Mac上の本番ヘッダーを用いたC++境界試験成功。 |
| A04 | 最大32 MiBと両入力のowner/lane検査を共通化。監視・保存・回復・バックアップの既存呼出先も同じ検査を使う。2,048区間のJSON、32 MiB境界、所有不一致、重複フィールドの3試験成功。 |
| A05 | 入力準備と自動マイク選択を制御ループから分離。未完了処理を1件だけ保持し、5秒の期限で採用を取り消す。実際の終了後1秒で再試行可能とする。停止後の遅延結果は破棄。終了を証明できない場合も原音声の末尾を封印し、成功した録音とは扱わない。5秒テストの準備中パケットを保存しない。WindowsのCOM・WASAPI実行は未検証。 |
| A06 | 整数PCMの1～格納ビット数までの有効ビットに対応。左詰めの余剰下位ビットを捨てて正規化し、不正値とfloatの不一致を拒否。24-in-32/20-in-24/12-in-16/unsigned8と既存float等の共通テスト3件成功。Windowsでの完全なCAF出力試験も追加したが未実行。 |

### 利用者から報告された削除画面

会議削除の共通進捗は `20253ba` で追加済みだった。今回、削除中と安全停止を共通ヘッダーで優先し、対象名・現在工程・実測件数を表示する。全体の割合・残り時間は推測しない。

モデル削除は汎用の `available/reason` に代えて、既存の `deletionAvailable/deletionReason` を使用する。通知履歴の書込みだけでは入口を無効にしない。この変更時点ではネイティブ側の共有保守受付が通知処理を拒否していたため、後続の[モデル削除の排他分離](model-deletion-isolation-2026-10-26.md)で専用所有へ接続した。通知処理の解放待ちは行わない。録音・AI・モデル準備・未完了削除などの制限は維持する。旧ネイティブ応答に専用条件がない場合は汎用条件を使う。無効理由と再確認、取得失敗からの復帰、確認取消後の再有効化を検証した。

使用されたQAインストーラーの実バイナリは取得していない。10月26日のWindows QAビルドとネイティブ画面成功の別記録もあるため、使用版を旧版と断定しない。今回のブラウザ10ケースは合成IPCと専用プロファイルによるもので、Windowsネイティブ画面の結果とは区別する。

### 配布物の識別と残件

`build-windows.ps1` はコミット、追跡差分のSHA-256、未追跡ファイルを含むソース一覧、本体・録音ヘルパー・インストーラーのSHA-256を記録する。ビルド中と配布直前のソース一致、各実行ファイル、ネイティブCTestを検査し、成功した場合だけ既存配布物を置換する。記録は `output/windows-qa-work/`、インストーラーは従来の `output/windows-qa/MON-QA-setup.exe` とする。インストール先の照合は独立した結果として保存する。[再実行手順](../windows-setup.md#必要なツール)。

CIはSwift録音のみの変更でも両OSを検証する。ビルド証拠の6試験、Windowsの配布拒否・旧成果物保持試験、入力準備のCTestと証拠アップロードを追加した。現在のホストではWindowsのビルド・CTest・実画面・インストール・実出力機器の検証を実行できないため、QAインストーラーを再生成・置換していない。公開署名や一般公開も実施していない。

Windowsで残る確認は、ネイティブビルドと全体Rust/CTest、複数出力の同時取得、既定出力だけの変更、指定アプリの終了と再取得、準備待ち中のマイク保存と停止、5秒テスト、削除画面と隔離した合成データの最終削除、新しいQAの生成・インストール先照合である。両OSの実機音声・長時間安定性を同等性確認済みとは扱わない。

### 修正後に実行した検証

| 検証 | 結果 |
| --- | --- |
| フロント全体 | 806成功、0失敗、既存のネイティブ専用1件除外 |
| TypeScriptとVite本番ビルド | 成功 |
| MacのRust全体と失敗箇所の再試験 | 実行対象の異なる2,025件が成功、93件は明示除外。初回全体2,015成功・10失敗の後、下記の旧期待値と実行権限を直して関連44件成功・8除外。全体を再実行して全件成功したという意味ではない |
| 既存ブラウザランナーの `model-deletion-admission` | 合成IPCで10ケース成功。正常・拒否理由・再確認・取消・会議削除と安全停止の上部表示を確認 |
| Windows向け `cargo check --tests` | 成功。外部SQLite参照による型検査であり、Windows SDKでのリンク・実行ではない |
| C++の時計・準備所有/スケジュール・取得キュー | Macで3実行成功。`-std=c++20 -Wall -Wextra -Werror -pedantic` |
| 共通PCM WAV | Rustの3試験成功。追加の同一480フレームWAVについて、Macの本番`AudioImport.swift`によるCAF出力とWindowsの本番パーサー/PCM変換をMacでコンパイルした出力が1,920バイトすべて一致 |
| ビルド証拠 | Python 6試験成功。ビルド中・生成後のソース変更、本体/ヘルパー/配布物/インストール先の不一致、証拠欠落を拒否 |
| プラットフォームとIPC | Python 5試験成功 |
| PowerShellの発行試験 | Windows専用6件はMacで明示除外。CIへ追加 |
| `cargo fmt --check` と `git diff --check` | 成功。CIを止めていた既存の全体整形差分も、動作を変えず整形した |

Rust全体の初回で失敗した10件は、候補数の旧期待12件（現行14件）と話者分離の旧引数期待0秒（現行の既定0.5秒）の2件、およびローカルHTTP待受けを実行制限が拒否した8件だった。前2件は10月25日の既存失敗記録と同じ更新漏れであり、製品のモデル候補・話者分離条件は変更せず期待値だけを更新した。後8件はlocalhostの`bind`が`Operation not permitted`になることを確認し、通常の実行権限で所有するHTTP/子プロセスの合成試験を再実行した。全10件を含む関連44件が成功した。今回変更した保存・削除・PCMの試験に失敗はない。

検証ログは `/tmp/mon-ui-parity-tests.log`、`/tmp/mon-ui-parity-build.log`、`/tmp/mon-ui-parity-browser-runner.log`、`/tmp/mon-parity-mac-regression.log`、`/tmp/mon-parity-native-retry.log`、`/tmp/mon-parity-windows-typecheck.log`、`/tmp/mon-parity-build-evidence.log`、`/tmp/mon-parity-platform.log`、`/tmp/mon-parity-fmt.log`。PCMの同一性記録とファイル専用ハーネスは `/tmp/mon-parity-wav-compare/` にある。原音声の取得や利用者データの操作は含まない。
