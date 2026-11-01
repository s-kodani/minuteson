# Windows Store版のDPI宣言とプロセス起動の確認

対象はMinutesOn 1.0.0／MSIX 1.0.0.0、Windows x64。`runtime.tar`の構成・取得レシピ・モデル準備処理は維持する。プロセス起動の指摘を消す目的でバイナリの文字列を改変したり、必要なヘルパー機能を除去したりしない。

## DPI宣言

修正前の実EXEからWindows SDKの`mt.exe`で抽出したマニフェストには、Common Controls v6の依存だけがあり、DPI宣言はなかった。Taoには起動時にAPIでPer-Monitor V2を有効化する実装があるため、その設定とWACKが読む静的宣言を揃える。

`app/src-tauri/windows.manifest`を`build.rs`の`WindowsAttributes`から埋め込む。`dpiAwareness=PerMonitorV2`と`dpiAware=true/pm`、Common Controls v6、`asInvoker`／`uiAccess=false`を明示する。SDKで検証できる完全なマニフェストにするため、固定のネイティブactivation-context identityも宣言する。この識別子はTauri／MSIX識別子・製品バージョンとは別で、保存先や単一起動の識別は変更しない。

旧EXEに存在しなかったUTF-8コードページ・longPathAware・OS互換性フラグは追加しない。ヘッドレス推論ワーカーも変更しない。検査はソースXML、SDKによる検証、再リンクしたEXEからの抽出を対象とし、CIでもビルド後に実EXEを確認する。

Microsoftは[DPI対応をマニフェストで宣言する方法](https://learn.microsoft.com/en-us/windows/win32/hidpi/setting-the-default-dpi-awareness-for-a-process)を推奨している。実際の複数モニター間移動・表示倍率の変更は今回の静的宣言検査とは別の確認になる。

## 再ビルドとWACK再検査

2026-11-01 15:32〜15:42（日本時間）に本番MSIXを再生成した。ネイティブCTestは8件成功、DPI検査はソースXML・SDK検証・実EXE抽出の3件すべて成功。`cargo fmt --all --check`と`git diff --check`も成功した。

- 原本：`output/windows-store/MinutesOn_1.0.0.0_x64.msix`、202,247,576 bytes。
- MSIX SHA-256：`f347520ad05bb8da0979d615de7bd067e7dee06f6c4f35b02f9849cb3b792421`。
- ビルドID：`20261101T063247Z-6ba210faf82c43fb949fee20bbbbb8f0`、同ディレクトリの`build.json`／`source.json`に証跡。
- ソース一覧SHA-256：`9277b5889a6c1dca8e66f17d1e1a7820ca55b88b957195b2d8a5891b23975a0b`。コミット前の作業状態を記録するため、コミット後のID照合とは区別する。
- 公開用コピーの`verify --require-current-source`はコミット前に成功し、`currentSourceMatches: true`、同梱113ファイルの一致を確認した。
- 同梱ファイルの追加・削除はなし。`runtime.tar`は従来どおり同梱・利用する。再ビルドでバイナリと索引・TARのハッシュは変わるが、ランタイム構成・取得処理は変更していない。

15:45〜15:47にWACK 10.0.26100.7705をWindows 11 Pro 10.0.26200.0で再実行した。**総合結果はWARNINGのまま（21 PASS、任意2 FAIL、必須DPI1 WARNING）。DPI警告の解消済みとは扱わない。** `PARTIAL_RUN=FALSE`、終了コード0。ArchiveFilesUsageとDetectBlockedExesは引き続き`OPTIONAL=TRUE`で、新旧のプロセス起動メッセージ37件は全件一致した。

結果は`output/windows-wack/20261101T064429Z/`に保存した。

- `wack-report.htm`／`wack-report.xml`：公式レポート。
- `wack-summary.json`：全24検査の区分・結果・メッセージ。
- `kit-logs/`：46ファイルの個別ログ。
- `wack-run.json`：インストール後の113ファイルの照合、実行、削除結果。
- `dpi-installed-diagnostics.json`：後述の同一EXEに対する追加診断。

### 残るDPI警告の切り分け

WACKのDPIログは、`BinaryInfoClass.IsPerMonitorV2DPIAwarenessSet()`の`Infrastructure`例外／`HRESULT E_FAIL`を記録している。`SetProcessDpiAwarenessContext`のAPI参照自体は検出済み。検査DLLの処理を確認すると、この例外の場合はAPI参照による判定へ進まず警告になる。

15:51の追加診断では、同じSHA-256（`e6593a492addf4aba6bb6911fb6f18289c8c15684d2776efda85067249abdebb`）のEXEを管理者セッション内で比較した。各COM判定は新しいオブジェクトで`LoadBinary`直後に行った。

| 対象 | WACKのCOM判定 | WindowsのCreateActCtx／QueryActCtxSettings |
| --- | --- | --- |
| ビルド出力のEXE | PerMonitorV2判定`True` | `PerMonitorV2`を取得 |
| WindowsAppsにインストールしたEXE | `E_FAIL` | CreateActCtxでアクセス拒否（設定取得は未実行） |

埋め込み宣言の欠落は修正でき、通常パスではOSとWACKの読み取りAPIの双方が認識する。残る警告はこの環境のインストール先でのアクセス条件に依存して再現する。ただし、具体的なアクセス権項目やOS／キットのどの不具合かまでは特定していない。WindowsAppsのACL・所有者・システムポリシーを変更する対応は行わない。

Microsoftの[現在のローカル検証の説明](https://learn.microsoft.com/en-us/windows/msix/package/packaging-uwp-apps#validate-your-app-package-locally)では、WACKは非推奨・保守終了の任意検査で、正式な認証はPartner Centerへの提出時に実施される。DPIの最終確認は同じMSIXを別の検証環境で再検査するか、提出時の正式な検査で行う。今回の結果はStore審査通過を意味しない。

再検査と追加診断の両方で、インストール後113ファイルの容量・SHAを照合した。各実行後に検証用パッケージと一時信頼登録を削除し、秘密鍵は署名直後に削除済み。独立した照合でも証明書・信頼登録の不存在と原本MSIXのSHA不変を確認した。Storeへの提出は行っていない。

## プロセス起動検出の内訳

初回WACKの`TEST INDEX=88`は`OPTIONAL=TRUE`。検査ログにはAPI参照8件と名前一致29件、合計37件がある。これはプロセス起動の動的トレースではなく、PEインポートと大文字小文字を区別しない名前の静的走査である。

| API検出先 | 検出API | 件数 |
| --- | --- | ---: |
| 本体`mon-desktop.exe` | CreateProcessW／ShellExecuteW | 2 |
| asr/libとsherpaのsherpa-onnx-c-api.dll | CreateProcessW | 2 |
| llamaとllama11960のllama-common.dll | CreateProcessW | 2 |
| llamaとllama11960のmtmd.dll | CreateProcessW | 2 |

| 名前検出先（ログのbasename表記） | 名前 | 件数 |
| --- | --- | ---: |
| mon-desktop.exe | rEG、DNx、cmd、cmd.exe、\cmd.exe、CsI | 6 |
| runtime.tar | bash、reg、CdB、cmd.exe | 4 |
| onnxruntime.dll | CdB | 1 |
| sherpa-onnx-c-api.dll | cmd.exe | 1 |
| ggml-base.dll | reg | 1 |
| llama-common.dll | bash | 1 |
| llama-server-impl.dll | cMD、dNx、rEg、CSI | 4 |
| llama.dll | reg | 1 |
| whisper-cli.exe | bash、reg | 2 |
| mon-llama-worker.exe／mon-llama-vulkan-worker.exe | bash、reg | 各2 |
| mon-whisper-worker.exe／mon-whisper-vulkan-worker.exe | bash、reg | 各2 |

名前検出部分はbasenameだけで、同名DLLの全コピーに個別帰属できない。今回の実行条件で見つかった一致と、その文字列が実際の起動先として使われることは区別する。

## アプリが実際に使う起動経路

Windows製品コードと同梱ヘルパーの呼び出しを確認した。`cmd.exe`、PowerShell、bash、regを指定して直接起動する製品コードは見つからなかった。テスト専用・macOS専用の呼び出しは分けて確認した。

| 用途 | 起動先・主な根拠 | 制御 |
| --- | --- | --- |
| 録音・入力一覧・音声取り込み | mon-windows-capture.exe、recorder.rs／capture_inputs.rs／audio_import.rs | 同梱パスを解決・検証し、固定オプションとパスを個別引数にする |
| 推論・特徴量計算 | Whisper／Llama／Sherpa CLI、同梱embedding helper、常駐worker | 実行ファイル・関連資材の容量／SHA照合、固定worker名、所有する子プロセスの管理 |
| 再起動・監督 | current_exeから自分自身 | 親PID・作成世代・実行ファイル・名前付きパイプの送信元を確認 |
| モデル取得 | %SystemRoot%/System32/curl.exe、model_download/transport_windows.rs | 意図的なOS標準ツールの起動。シェルを介さず、HTTPS限定・URLの独立引数・時間／容量／ハッシュ制限を使う |

推論・特徴量計算・モデル取得などで`OwnedCommand`を使う経路では、`local_engine.rs`が`Command::new(program)`を使い、WindowsではJob Objectへ割り当てる。録音・入力一覧・再起動には直接`Command`を生成する箇所もある。`mon-native/src/process.rs`の環境設定と`transport.rs`のcurl設定を確認した。curlはアプリ同梱EXEではなく、Windows標準の実行ファイルである。`--disable`を先頭にしてcurlrcを無効化し、`--globoff`・HTTPSリダイレクト制限・`--url`の独立引数を使う。

### Rust／Tauri内の一致

本体EXE内の`cmd.exe`／`\cmd.exe`の周辺には、Rust標準ライブラリのWindowsバッチ起動処理に対応する`/e:ON /v:OFF /d /c`や診断文がある。`cmd`はTauri JavaScript IPCのフィールド／引数名にも存在する。`rEG`、`DNx`、`CsI`は命令列・バイナリ表にも一致しており、これらだけで実行先を特定できない。

Tauri Runtime Wry 2.12.1の`ShellExecuteW`呼び出しは、WebView2 Runtimeがない場合のダイアログからMicrosoftの案内リンクを開く処理（`src/dialog/windows.rs:55–64`、`src/lib.rs:4680–4683`）にある。アプリが任意コマンドを組み立てて渡す箇所ではない。

### Sherpaの任意パイプ機能

Sherpaの固定ソース`11afbd009a7f8c08f4bcf2fc1b265d0df4670fbf`と、そこから固定されるKaldi decoder 0.3.0／KaldiFST 1.8.0を確認した。後二者の取得アーカイブはCMakeに記載されたSHA-256との一致も確認した。資料は公式[k2-fsa/sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx/tree/11afbd009a7f8c08f4bcf2fc1b265d0df4670fbf)、[kaldi-decoder v0.3.0](https://github.com/k2-fsa/kaldi-decoder/tree/v0.3.0)、[kaldifst v1.8.0](https://github.com/k2-fsa/kaldifst/tree/v1.8.0)に対応する。

任意の`rule_fsts`／homophone rulesからTextNormalizerを作り、KaldiFSTのInputに末尾`|`のファイル指定を渡すと、Windowsの`_popen`を使う経路が存在する。DLL内のシェル起動能力は実在する。

現在の`probes/inference-worker/transducer-main.cpp:98–111`はC設定をゼロ初期化し、モデル・tokens・provider・threads・復号設定を与える。`rule_fsts`やhomophone rulesは設定せず、C API側では空として扱われるため、当該オプション経路を選択しない。話者分離と特徴量計算はONNXモデルを`Ort::Session`へ渡す経路である。

### llama／Whisper内の一致

Whisperには生成用スクリプトの`#!/bin/bash`や言語名bashkir、llamaには`--completion-bash`等の通常文字列がある。`reg`はbackend registryや正規表現等にも存在する。WACKの内部前処理と各一致offsetが公開ログにないため、通常文字列の存在だけで全検出の直接の一致元と断定しない。

llamaには実際のsubprocess作成機能があり、mtmdの任意動画機能にはffmpeg／ffprobeを起動するソースもある。MinutesOn workerは`LLAMA_BUILD_MTMD=OFF`・`LLAMA_BUILD_APP=OFF`で構築し、固定ソースの`tools/completion/completion.cpp`に当該subprocess呼び出しは見つからなかった。上流から同梱する追加DLLもWACKの走査対象になるが、事前ビルド版mtmdの厳密なコード帰属までは証明していない。

## 判断範囲

同梱ヘルパー・自身・Windows標準curlの起動は製品機能として意図した経路であり、禁止名を直接起動する製品コードは確認されなかった。第三者ライブラリに任意のシェル起動機能が存在することも確認できたため、検出全体を「誤検知」とは扱わない。

これは固定ソース・現在の設定・PEインポート・バイナリの確認であり、全モデル・全入力の動的子プロセストレースによる証明ではない。[MicrosoftのDesktop Bridge検査説明](https://learn.microsoft.com/en-us/windows/uwp/debug-test-perf/windows-desktop-bridge-app-tests)も、実際の起動先とアプリ同梱プログラムかどうかを確認するよう案内している。
