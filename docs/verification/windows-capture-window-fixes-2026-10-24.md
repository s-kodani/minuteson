# Windows録音・パネル・会議操作の追補修正

2026-10-24。既存の機能差分実装と利用者データを保持した追補。実マイク・PC音声の取得、利用者データの削除、インストール済みアプリの更新・終了は行っていない。

## 5秒録音テストの開始直後の失敗

本番と同じ `owned_command` と永続所有台帳を使う試験で、録音前の所有確認が失敗することを再現した。従来の5秒合成試験は所有ハンドルを継承しない経路であり、リース回復試験は保存完了を確認していなかった。

`GetFinalPathNameByHandleW` の拡張パスと `std::filesystem` が正規化したパスを文字列比較していたことが原因。同じディレクトリでも不一致になっていた。親ディレクトリを再解析ポイントを開かないハンドルで確認し、Volume Serial・File Index・作成時刻を照合する方式へ変更した。空の通常ファイル、単一リンク、所有IDに対応したファイル名、継承リースの検査は維持している。

- `windows_owned_five_second_helper_completes_with_inherited_lease`：日本語パスで本番起動経路から合成PCMを保存し、240,000フレームを共通検証して所有回収まで確認。
- `windows_owned_test_reports_early_lease_failure_and_rejects_another_parent`：別の親ディレクトリは拒否し、外部対象を変更せず、開始前エラーを画面へ返す経路を確認。
- `capture_test.rs`：Windowsではマニフェスト作成前の標準エラーも最大8,192バイト・非ブロッキングで読み、取消とは区別して表示する。

## `recording packet start timestamp reversed`

実際の `capture-clock.json` を読み取り、48kHz出力のパケット間隔よりQPCの進みが短く、4秒の原PCM分割後の区間開始が次のパケット開始より後になるケースを確認した。実例は2区間と55フレームを保存した後の停止。元QPCが増加していても、パケットの途中で分割した後には逆転が起こり得た。

`packet_clock.h` で、等値・逆行・無効・未来時刻に加えて名目区間の重なりも推定時計へ切り替える。保存済み区間を遡らず、受け取ったPCMをすべて保持する。元のQPC・端末位置・理由は診断に残し、推定と不確かさを明記する。端末位置と48kHz出力の単位が一致すると確認できないストリームでは、端末位置から欠損フレーム数を推定しない。

- `packet-clock`：連続、重なり、欠損、等値・逆行・0・未来・エラーフラグ、時計リセット、端数、再取得、異なる端末レートを検証。
- `windows_resampled_packets_cross_raw_seal_without_reversing_or_losing_pcm`：479フレーム単位で元QPCが7.5ms進む合成入力を500回与える。両入力239,500フレームの保存、4秒分割、Rust側のSHA連鎖・時計・FLAC群と推定理由を検証。

Microsoftの [GetBuffer仕様](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer) を参照。Macと異なるWindowsヘルパーの時刻処理にあった不備であり、Windowsで必ず発生するものではない。

## 音声の「ブツブツ」への対策

既存の診断には実際に `AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY` が残っていた。失敗したマイク録音の直近32パケットでは6回、取得間隔の最大値は36msだった。ただしこの情報だけで聞こえたノイズの全原因は断定できない。利用者音声の内容は読み取っていない。

従来のWindows取得ループではWASAPIの読取りと、原PCM・履歴・診断ファイルの同期保存が同じスレッドだった。保存待ちが入力を止めないよう、`async_capture.h` と `packet_queue.h` で次を実装した。

- マイクとPC音声それぞれに独立した取得スレッドを置く。`GetBuffer` と `ReleaseBuffer` は同じスレッドで完結する。デバイスのバッファは解放前にコピーする。
- MMCSSのAudioタスクへの登録を要求し、共有モードの要求バッファ容量を500msにする。容量を増やす指定であり、取得を500ms待機する処理ではない。
- 片入力につき20秒・4,096パケットまでのキューを使用する。キュー内のmono＋stereo PCM上限は約11MiB（取得中の1パケット分は別）。上限到達時は既存音声を捨てず、明示的な保存待ちエラーにする。
- 停止・切断時は取得スレッドを終了し、既に受け取った音声を保存してから切断を反映する。欠損回数とキューの最大待機フレームを診断へ追加した。

`packet-queue` は保存側が遅れても連続PCM・元時刻・順序を保持し、切断エラーより先に待機PCMを回収することと上限拒否を確認する。`async-capture` は本番の `AsyncCapture` を模擬WASAPI COMデバイスで動かし、保存側を止めた状態の取得継続、解放前コピー、連続ステレオPCM、無音パケット、同一スレッドでのバッファ解放、停止・切断を検証する。実デバイスは使用しない。

[Initializeのバッファ仕様](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-initialize)、[MMCSS仕様](https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service) を参照。実マイク・PC音声での音質確認と長時間確認は利用者側に残る。既存の録音に混入済みの欠損・ノイズを自動修復したものではない。

## Windows標準の外枠

Taoの装飾なしウィンドウでも既定の影とネイティブのcaption・境界用スタイルが残っていた。`window_chrome.rs` で録音・説明・停止確認の3ウィンドウを影なし・透明背景とし、`windows_appearance.rs` でcaption・resize・外周スタイルの除去とフレーム再計算を行う。再計算はスタイル変更時のみで、サイズ通知の再帰を避ける。DWMの外周色を無効にし、従来のDPI対応の角丸クリップは維持する。Windowsの該当ページのみHTML・body・rootも透明にする。

- 通常・QA双方のマージ済み設定の回帰試験。
- 非表示のWin32ウィンドウで3種類×100/150/200%相当のサイズ・角丸を検証し、外側とクライアント領域が一致、caption・境界スタイルがないことを確認。
- ミニパネル、説明、停止確認の既存合成ブラウザー試験と透明背景を確認。インストール後の実画面・複数モニター間のDPI移動は別途確認が必要。

## 設定カードと削除メニュー

「導入済みモデル管理」だけ `margin-top:8px` で、ほかのカードの20pxより狭かった。固有指定を外し、実DOMで全3カードの外側上部20px・内側上部24pxを確認した。

削除は録音・音声保存・AI処理・モデル準備などの共通排他、再起動待ち、未完了の削除、未確認状態では無効になる。添付画像だけではその時点の理由は特定できない。既存の受付条件を保ち、`deletion-review-reason.ts` と `meeting-deletion-menu.tsx` から実際の無効理由をメニューに表示し、該当ボタンと `aria-describedby` で関連付ける。共有理由は一度だけ表示する。ボタンは透明背景・落ち着いた文字色とし、無効な破壊操作に赤文字と灰色の塗りを重ねる表示を解消した。

合成UIでAI処理中の理由、無効状態でクリック・キーから削除を送らないこと、アイドル復帰後の有効化、削除確認と取消、フォーカス復帰を確認した。利用者データの削除は実行していない。

## 検証・配布

- CTest：3件成功。ログ `output/windows-qa-work/capture-fix-native-clock-queue.log`。
- Win32外枠：1試験内9条件成功。ログ `output/windows-qa-work/capture-fix-native-frames.log`。
- フロントエンド：775成功・1除外。TypeScript検査成功。
- 合成UI：ミニパネル、説明、停止確認、および削除メニューと設定カードの実DOM検査。ログ・画像は `output/windows-qa-work/capture-fix-*`。
- Rust全体回帰：最終759成功・失敗0・33除外（340.92秒）。ログ `output/windows-qa-work/capture-fix-rust-tests-final.log`。初回の旧時刻補正回数を要求していた1件は、新しい推定理由・全240,000フレーム保持の検査へ更新し、単独・全体の両方で再確認した。
- 修正版QAインストーラー：生成成功。`app/src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/MON QA_0.1.0_x64-setup.exe`、268,294,816バイト（255.87MiB）。SHA-256 `938140c24dd87c2008301d5e1291e61eddc2bbbd88b60e3daefec8de0a7eabd4`。署名状態は `NotSigned`（QA）。公開署名・実録音の確認を成功扱いにはしない。
- 同梱録音ヘルパー：723,456バイト、SHA-256 `e758941cf27875138ae639f0eb4c1f161a65af043ee77cecbd1eef0a4cee1539`。試験したヘルパー、sidecar入力、release配布ファイルの3点が一致する。
- ビルド記録：`output/windows-qa-work/capture-fix-qa-build.log` と `capture-fix-build-report.json`。追加ダウンロード0バイト、累計26,042,589,935バイトで30GiB以内。インストール済みアプリの更新・利用者データへの書込みは行っていない。
