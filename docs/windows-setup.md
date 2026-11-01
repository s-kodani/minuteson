# MinutesOnのWindows開発環境とMinutesOn QAビルド

Microsoft Store向けの本番MSIXは[Store用MSIXの生成・検証・提出準備](windows-store.md)を参照。直接配布版は署名済み`MinutesOn-setup.exe`、明示的な未署名版は`MinutesOn-unsigned-setup.exe`、QA版は`MON-QA-setup.exe`として作成する。

Windows 11 24H2以降・x64向けのローカルビルド手順。リポジトリのルートでPowerShellから実行する。WASAPI録音、取り込み、バックアップ・復元、削除、保存先変更、声の登録・照合・補完とCPU／Vulkan推論を接続した。最新の32項目別の実装根拠・試験・実機残件は[対応記録](verification/windows-parity-implementation-2026-10-24.md)を参照。以下の過去のビルド記録にある未対応の説明は、その時点の状態を示す。

公開用は `powershell -File app/build-windows.ps1` を使い、`MON_WINDOWS_CERTIFICATE_THUMBPRINT` にCurrentUser\\Myの指定Code Signing証明書、`MON_WINDOWS_TIMESTAMP_URL` に証明書提供元のHTTPS RFC3161サービスを指定する。秘密鍵・証明書・署名・タイムスタンプ・検証のいずれかが欠ければ公開ビルドを失敗とする。証明書が未提供のため、実際の公開署名は実施していない。QA用は `-Qa` を指定し、未署名成果物として区別する。

通常のMinutesOnを未署名のNSISで作る場合は、`npm.cmd --prefix app run tauri:windows:unsigned:build`、または`powershell -File app/build-windows.ps1 -Unsigned`を明示する。製品名・識別子・機能は通常版と同じで、QA版への切替ではない。署名用証明書は不要だが、Windowsの発行元表示は未確認の発行元となる。MSIXは作成・変更しない。

未署名ビルドは専用の`app/src-tauri/target-unsigned/`で本体を構築し、継承した`TAURI_CONFIG`と署名指定を無効化する。本体・録音／推論ヘルパー・インストーラーのAuthenticode状態とPE証明書テーブルが未署名であることを確認した場合だけ配布物を更新する。第三者配布物のDLL署名を削除・変更しない。`-Qa`と`-Unsigned`の同時指定は出力作成前に拒否する。既定の署名必須ビルドとQAビルドは引き続き個別に使える。

## 必要なツール

10月26日以降のビルドは、開始時のコミット・追跡差分SHA-256・未追跡ファイルを含むソース一覧と、本体・録音ヘルパー・インストーラーのSHA-256を `output/windows-qa-work/<build-id>-source.json` と `<build-id>-build.json` に保存する。ビルド中および配布直前のソース照合とネイティブCTestに成功した場合だけ `output/windows-qa/MON-QA-setup.exe` を置き換える。ビルド記録の `installedVerification: pending` はインストール先の実行ファイルが未照合であることを示す。

隔離したQA環境へのインストール後、次のコマンドでインストール先の本体と録音ヘルパーを照合する。アプリ本体の実ファイル名はインストール先に合わせて指定する。確認結果は別の新規ファイルへ保存する。これはバイナリ照合だけを行い、アプリの起動やデータ削除は行わない。

```powershell
python tools/mon_windows_build_evidence.py verify --manifest output/windows-qa-work/<build-id>-build.json --installed-app '<隔離環境のアプリ.exe>' --installed-helper '<隔離環境のmon-windows-capture.exe>' --published-installer output/windows-qa/MON-QA-setup.exe --output output/windows-qa-work/<build-id>-installed.json
```

`currentSourceMatches` は現在のソースとの一致、`installed` の `app` と `helper` はインストール先の照合結果を表す。本体・ヘルパー・配布物の3パスを明示すれば、元のビルドディレクトリがないQA端末でも照合できる。過去のインストールを確認するときは現在のソースと違っていてもそのビルドとの対応を表示する。配布時は `--require-current-source` と元のビルド成果物を必須にして不一致を拒否する。今回のMacでの修正ではWindows QAを再生成しておらず、以前のビルド成功記録は今回のコードの検証結果へ流用しない。

| ツール | 使用する系列・構成 |
| --- | --- |
| Git | Windows版Git |
| Node.js | 24（CIと同じ系列。npmを含む） |
| Python | 3.12（通知文書生成に標準ライブラリのtomllibを使用） |
| Rust | stable、ホスト・ターゲットともにx86_64-pc-windows-msvc |
| Visual Studio Build Tools | 2022、C++開発ワークロード、MSVC x64/x86、Windows SDK |
| CMake | 3.21以上 |
| WebView2 | Microsoft Edge WebView2 Runtime |

`app/prepare-windows.ps1` がCMakeの `Visual Studio 17 2022` ジェネレーターを指定するため、Visual Studio 2022を導入する。Tauriの公式前提条件は <https://v2.tauri.app/start/prerequisites/> を参照。

## 開発ツールの導入

winget（アプリ インストーラー）が利用できることを確認する。管理者権限を求められた場合はWindowsの確認画面で許可する。各コマンドの終了を待って次へ進む。

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --source winget --silent --accept-package-agreements --accept-source-agreements --disable-interactivity --override '--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
winget install --id OpenJS.NodeJS.LTS --version 24.22.1 --exact --source winget --silent --accept-package-agreements --accept-source-agreements --disable-interactivity
winget install --id Python.Python.3.12 --exact --source winget --scope user --silent --accept-package-agreements --accept-source-agreements --disable-interactivity
winget install --id Rustlang.Rustup --exact --source winget --silent --accept-package-agreements --accept-source-agreements --disable-interactivity
```

Node.jsは24系を選ぶ。上のバージョンは今回のセットアップで選択した値。将来 `OpenJS.NodeJS.LTS` の系列が変わっても、無指定で別系列へ更新しない。

CMakeが見つからない場合は追加する。

```powershell
winget install --id Kitware.CMake --exact --source winget --silent --accept-package-agreements --accept-source-agreements --disable-interactivity
```

Visual Studio同梱のCMakeを使う場合は、追加導入の代わりに次のディレクトリをビルド用PATHへ加える。

```powershell
$env:PATH = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin;' + $env:PATH
```

導入後はターミナルを開き直す。親アプリが古いPATHを保持している場合は、そのアプリも再起動する。再起動要求が出た場合は端末の再起動後に続行する。

```powershell
node --version
npm.cmd --version
python --version
rustup show
cargo --version
cmake --version
rustup target add x86_64-pc-windows-msvc
```

`python` がMicrosoft Storeを開く場合は実体のPythonへPATHを通す。ユーザー単位の標準導入先は `$env:LOCALAPPDATA\Programs\Python\Python312`。Rustは `$env:USERPROFILE\.cargo\bin`、Node.jsは `C:\Program Files\nodejs` が標準の実行ファイル配置先。

PowerShellの実行ポリシーで `npm.ps1` が拒否される場合は `npm.cmd` を使う。以下の `npm` コマンドも同様に読み替えられる。Windows用ビルドスクリプト自体は既存のnpmスクリプトから `-ExecutionPolicy Bypass` を付けた子PowerShellで実行される。

### 既存のnvmリンクが使えない場合

今回の端末では `C:\Program Files\nodejs` が別ユーザーの古いnvm環境へのシンボリックリンクだった。wingetが導入成功を返しても `node` を実行できなかったため、リンクを変更せず公式ZIPを別の場所に展開した。同じ症状なら `Get-Item 'C:\Program Files\nodejs' -Force | Format-List LinkType,Target` で確認する。

リポジトリ直下から以下を実行する。`.cache/` はGit管理対象外。既に同じZIPを展開済みなら、再展開せずPATH設定から続行する。

```powershell
$ErrorActionPreference = 'Stop'
$nodeTools = Join-Path (Get-Location) '.cache/windows-tools'
New-Item -ItemType Directory -Force -Path $nodeTools | Out-Null
Invoke-WebRequest 'https://nodejs.org/dist/v24.22.1/node-v24.22.1-win-x64.zip' -OutFile "$nodeTools/node-v24.22.1-win-x64.zip"
Invoke-WebRequest 'https://nodejs.org/dist/v24.22.1/SHASUMS256.txt' -OutFile "$nodeTools/SHASUMS256.txt"
$actual = (Get-FileHash "$nodeTools/node-v24.22.1-win-x64.zip" -Algorithm SHA256).Hash
$expected = ((Get-Content "$nodeTools/SHASUMS256.txt" | Where-Object { $_ -match '  node-v24.22.1-win-x64.zip$' }) -split '\s+')[0]
if ($actual -ne $expected) { throw 'Node ZIP checksum mismatch' }
Expand-Archive -LiteralPath "$nodeTools/node-v24.22.1-win-x64.zip" -DestinationPath $nodeTools
$env:PATH = "$nodeTools/node-v24.22.1-win-x64;" + $env:PATH
node --version
```

このPATH追加は現在のシェルとその子プロセスにだけ適用される。新しいターミナルでも同じ配置先をPATHへ加える。

## 依存の取得と事前検査

```powershell
npm.cmd ci --prefix app
cargo fetch --manifest-path app/src-tauri/Cargo.toml --locked
python -m unittest discover -s tests -p test_mon_tauri_platform.py -v
npm.cmd --prefix app test
```

各コマンドの終了コードが0であることを確認する。npm・Cargoの依存取得と初回のバンドル用ツール取得にはネットワーク接続が必要。ロックファイルは更新しない。

## MinutesOn QAのビルド

```powershell
npm.cmd --prefix app run tauri:windows:qa:build
```

このコマンドは以下を順に構築する。

1. 固定ハッシュのnlohmann/jsonヘッダーとライセンスの取得。
2. 固定版Windows CPU推論ランタイムの準備。Whisper v1.9.4はCMakeでソースからビルドし、llama.cpp b11533とsherpa-onnx v1.13.8は公式Windows配布物の容量・SHA-256を照合して取り込む。必要なVCランタイムDLLもアプリ内へ配置する。
3. TypeScript型検査とViteによるフロントエンド。
4. Windowsターゲットの依存ライセンス通知（`.cache/windows-third-party-notices/`）。
5. CMakeによるRelease版WASAPI録音ヘルパーと、ターゲット名付きサイドカーの配置。
6. Rust本体と、推論ランタイム・通知・WebView2オフライン導入資材を含むNSISインストーラー。モデルの重みは同梱せず、設定画面から取得する。

QAの製品名は `MinutesOn QA`、識別子は `jp.mon.desktop.qa`。通常版と区別され、インストーラーはユーザー単位の構成になる。公開配布用署名はこの手順に含めない。

配布用の出力先（ビルド成功後に最新版1つだけを配置）：

```text
QA版: output/windows-qa/MON-QA-setup.exe
公開版: output/windows-release/MinutesOn-setup.exe
未署名通常版: output/windows-unsigned/MinutesOn-unsigned-setup.exe
```

`app/build-windows.ps1` は、ビルド成功・公開版の署名確認後に `tools/publish-windows-installer.ps1` で配布先を更新する。配布先にはログ・画像・検証スクリプト・ハッシュファイルを置かない。既知の過去のMONインストーラーは削除し、それ以外のファイルは `output/windows-qa-work/` または `output/windows-release-work/` に移す。検証結果やビルドログは最初からこれらの作業用フォルダーに保存する。ビルド内部の生成物は引き続き `app/src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/` にある。

未署名通常版の配布と証跡は`output/windows-unsigned/`と`output/windows-unsigned-work/`に分離する。`<build-id>-unsigned.json`に未署名検査の各パス・SHA-256を保存し、既存のソース／本体／ヘルパー／インストーラー照合とCTestも通す。失敗時は既存の配布物を保持する。内部のNSIS出力は`app/src-tauri/target-unsigned/x86_64-pc-windows-msvc/release/bundle/nsis/`となる。

```powershell
Get-ChildItem -LiteralPath output/windows-qa |
    Select-Object FullName, Length, LastWriteTime
Get-FileHash -LiteralPath output/windows-qa/MON-QA-setup.exe -Algorithm SHA256
git rev-parse HEAD
git status --short
```

生成日時が今回のビルドに対応することも確認する。ビルドコマンドはアプリをインストール・起動しない。実機確認では、インストール・起動、録音、モデル取得、AI処理を別途記録する。

## ビルド時の確認事項

- `supplemental notice content/provenance changed`：`third-party/notices/index.json` のバイト数・SHA-256と原文が一致していない。今回の原因は通知原文のCRLF変換だった。`.gitattributes` で通知ディレクトリの改行変換を無効にしている。古いチェックアウトでは、未保存の変更がないことを確認し、Git原本と登録ハッシュを照合してから原文を復元する。ハッシュ検証を無効化したり、変換後の内容に合わせて登録値を書き換えたりしない。
- 録音プローブの `numeric_limits<...>::max()` でC2589／C2059：Windows SDKの `max` マクロとの衝突。CMakeのターゲット定義に `NOMINMAX` を指定する。本手順と同時に追加済み。
- フロントエンドのソース検査でLF固定の正規表現が失敗：CRLFも受け入れる `\r?\n` を使う。今回 `minutes-removed-features.test.ts` の2箇所を修正した。
- Viteのチャンクサイズ・`use client` 警告はビルド失敗と区別する。npmの依存監査結果も、ロックファイルを自動更新せず記録する。

## 2026年10月23日のセットアップ記録

- OS：Windows 11 25H2、ビルド26200、x64。
- Node.js 24.22.1（公式ZIP）、npm 11.19.1、Python 3.12.10、Cargo 1.99.1（Rust stable MSVC）。
- Visual Studio Build Tools 2022 17.14.42、MSVC 19.44.35229、Windows SDK 10.0.26100.0、Visual Studio同梱CMake 3.31.6-msvc6。
- WebView2 155.0.4283.62は導入済み。追加導入していない。
- 基準コミット：`36431bb6f109b040508d5150f840a8697f8ca621`。上記のビルド・テスト互換修正とこの手順書は未コミット。
- 事前検査：プラットフォーム設定5件、フロントエンド770件が成功。
- 依存取得時のnpm監査はhigh 1件。自動修正はしていない。同梱通知の生成は成功し、既存の未承認ライセンス項目・不足通知4件は解消済みとは扱わない。
- 追加実行した通知生成のPythonテスト18件には、Windowsの改行・パス区切り・POSIX権限・シンボリックリンク作成権限の前提による失敗が残った（8成功、3失敗、7エラー）。この追加テスト全体のWindows対応は今回実施していない。実依存391件の通知生成と原文ハッシュ照合は別途成功している。

この端末のビルド用PATH設定・各工程のログは、Git管理対象外の `output/windows-qa-work/` に保存する。新規セットアップでは上記の公開手順を使い、このローカル出力への依存は不要。

## Windows録音・モデル解析の実装と検証（2026年10月24日）

録音開始時に同梱の `mon-windows-capture.exe` を起動し、マイクとPC全体または指定アプリの音声をWASAPIで取得する。録音停止後は元CAFを検証し、時計区間を維持したFLACチャンクを保存して解析用音声を作る。PC音声が一度も届かなかった場合は未受信として扱う。取得していない音声を保存済みと表示しない。

モデル候補はWindows用の9件を表示する。Whisperの多言語モデル6種、Pyannote + ERes2Net base、Qwen 2.5 1.5B Q4_K_M、Qwen 3 4B Q4_K_M。ダウンロードしたファイルと同梱エンジンは容量・SHA-256を確認し、Windows用の配置・互換性検査・適用を経て使用する。今回のWindows推論はCPUで実行し、エンジンを処理単位で起動する。GPUや常駐ワーカーの性能は確認していない。

インストール後は「設定 → AIモデル」で文字起こし、話者分離、議事録のモデルを選び、「ダウンロードして使用」で準備する。今回の短い日本語検証ではWhisper base、Pyannote + ERes2Net base、Qwen 2.5 1.5Bを使用した。Whisper tinyも取得・配置・互換性検査・適用に成功している。他の候補の重み取得・実解析は未実施。録音自体はモデルの準備前でも開始できる。

実行済みの検証：

- フロントエンド770件、Pythonプラットフォーム設定5件が成功。
- Windows統合テスト3件が成功。新規ファイルのみのモデル配置、ハードリンク・親ディレクトリ差し替え拒否、モデル一覧検証、合成PCMからの録音停止・入力状態読取り・FLAC保存・解析用ミックス作成を確認。
- 録音前5秒テストの合成PCM取得・保存検証・一時音声削除が成功。空の一時ディレクトリはWindowsの差し替え防止ハンドルで保持し、終了後も音声を含まない空ディレクトリとして残る。
- Windows Job Objectによる子孫プロセス終了の2件、話者分離の固定形式・出力パーサー5件が成功。Rust書式検査が成功。
- 固定モデル4件の実ダウンロード・配置・ネイティブ実行による互換性検査・設定適用が成功。検証先は通常アプリの保存先から分離した `.cache/windows-native-smoke/`。
- Microsoft Harukaでファイルへ直接生成した日本語合成音声を使い、保存 → Whisper文字起こし → Sherpa話者分離 → Qwen議事録生成とMarkdown保存まで成功（44秒）。音声の再生・マイク入力は使用していない。

自動検証のコマンド（先にQAビルドでヘルパー・ランタイムを準備する）：

```powershell
cargo test --manifest-path app/src-tauri/Cargo.toml --locked --lib windows_integration_tests
cargo test --manifest-path app/src-tauri/Cargo.toml --locked --lib windows_five_second_synthetic_capture_validates_and_releases_audio
cargo test --manifest-path app/src-tauri/Cargo.toml --locked --lib local_engine::windows::tests
cargo test --manifest-path app/src-tauri/Cargo.toml --locked --lib diarization::sherpa::tests
cargo fmt --manifest-path app/src-tauri/Cargo.toml --check
```

通常の統合テストではネットワークやマイクを使わず、実モデル検証2件をignoreする。明示的な実モデル検証には十分なディスク容量とネットワーク接続が必要で、通常アプリと異なる専用フォルダーを指定する。

```powershell
$env:MON_WINDOWS_INTEGRATION_ROOT = Join-Path (Get-Location) '.cache/windows-native-smoke'
cargo test --manifest-path app/src-tauri/Cargo.toml --locked --lib windows_real_models_download_place_check_and_apply -- --ignored --nocapture
# 別途用意した日本語・PCM16 WAVファイルを指定する。実録音は不要。
$env:MON_WINDOWS_QA_SAMPLE = Join-Path (Get-Location) 'output/windows-qa-work/japanese-speech.wav'
cargo test --manifest-path app/src-tauri/Cargo.toml --locked --lib windows_saved_speech_runs_transcription_diarization_and_minutes -- --ignored --nocapture
```

主要ログは `output/windows-qa-work/` の `windows-native-tests-final.log`、`windows-five-second-final.log`、`windows-real-models-base.log`、`windows-japanese-analysis.log`、`frontend-tests-final.log`、`platform-tests-final.log` に保存。実マイク・PC音声・対象アプリの実録音、機器切替、長時間安定性の確認は利用者が行う。今回の自動検証では実録音・再生、新版アプリのインストール・起動は実施していない。

### 起動直後・5秒テスト・モデル一覧・状態更新の修正

初回のWindows機能追加後に報告された4点を修正した。

- `missing field captureTest`：録音ヘルパーの最初の `starting` 記録にテスト所有者・5秒枠・空のトラック情報を含める。初期化失敗時も同じ形式を維持し、権限・機器などの本来のエラーを表示する。保存完了後だけでなく、初期化中から製品の監視処理で読み取る回帰テストを追加。
- モデル一覧が空で「取得の定義・操作・応答が一致しません」：画面側の検査・容量計算がWindows同梱URIに未対応だった。固定のWindows同梱URI・対象OS・素材の役割・アーカイブを照合し、ネットワーク取得量と同梱量を区別する。Rustが返す実際の9候補をTypeScriptの製品側検査へ渡すテストを追加。
- `metadata publication failed`：利用者の保存先に残った更新途中の `input-status.json.pending` を確認。同じ状態ファイルを短時間だけ削除共有なしで開く条件では、旧同梱ヘルパーが同じエラーで失敗し、修正版は保存まで成功した。書込み・flush済みの同じ一時ファイルについて共有違反・ロック違反・アクセス拒否を最大2秒再試行し、それでも失敗した場合はファイル名とWindowsエラー番号を残す。
- 毎回「保存先の起動確認」で止まる：起動結果の最初の読取りが `starting` の場合だけメモリ内の結果を再読取りする。準備中は短い起動表示にし、`ready` が確認できたら通常画面へ進む。確定した起動拒否や不正応答からは通常操作を開始せず、診断表示を維持する。

修正後の検証は、画面側773件・プラットフォーム設定5件、Windows統合3件、5秒テスト2件とRust書式検査が成功。ブラウザーでは起動の8条件、Windows実定義9候補の表示・選択、3エンジンの取得・配置・検査・適用の画面操作を確認した。画面操作の取得以降は合成IPCであり、実エンジン・重みの先行検証とは区別する。今回も実録音は行っていない。

ログ・記録は `output/windows-qa-work/fix-five-second-tests.log`、`fix-windows-integration-tests.log`、`fix-frontend-tests.log`、`fix-platform-tests.log`、`fix-browser-results.json`、`fix-metadata-before-after.json`。ブラウザーは検証終了後に閉じる。利用者が使っている旧版を自動で終了・更新せず、再ビルドしたインストーラーから明示的に更新する。

修正版の再ビルドは2026年10月24日01:08:00（日本時間）、終了コード0で成功。生成日時と、最新ヘルパーの同梱元SHA-256一致を確認した。

| 項目 | 修正版の結果 |
| --- | --- |
| 成果物の絶対パス | `C:\Users\local_user\work\mon\app\src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis\MON QA_0.1.0_x64-setup.exe` |
| サイズ | 251,520,882 bytes（239.87 MiB） |
| SHA-256 | `ff86452e06a5c1a0ef542f669464dc78e1ca107d2595e17b905b1f2b7d6841a4` |
| ビルド対象 | `codex/windows-platform-foundation`、`36431bb6f109b040508d5150f840a8697f8ca621` と今回までの未コミット変更 |
| 再ビルド所要時間 | 315.90秒（約5分16秒、01:02:44〜01:08:00。調査・修正・先行検証は含まない） |
| ログと機械可読記録 | `output/windows-qa-work/fix-rebuild.log`、`fix-build-report.json` |

旧MON QAを終了してから同名の修正版インストーラーで更新する。今回も新版のインストール・起動・実録音は行っていない。

### 録音パケットの同値・逆行と診断ログ

`recording packet start timestamp reversed` への追加対応では、未補正のWASAPI時刻をそのまま保存用時計へ渡さず、パケット単位で先に正規化する。時刻が同じ場合と逆行する場合に加え、ゼロ・`TIMESTAMP_ERROR`・到着より未来の時刻も推定へ切り替える。最初のパケットだけ原点を作れない場合は、到着QPCからそのパケット長を引いて推定する。

- 同じストリームのデバイス位置が前パケットの終わり以降へ進み、不連続フラグがない場合は、フレーム位置で継続する。位置から判明する60秒以下の欠損は保存時計の空白として維持する。
- 位置のリセット・不連続・過大な飛びでは、前の補正済み終端と取得フレーム数で継続する。推定が始まったら、入力を開き直すまでその時計を維持する。端数の100ns換算を繰り越し、繰り返す異常で区間数が増え続けないようにする。
- 補正だけを理由にPCMを捨てたり、原本に無音を挿入したりしない。5秒テストでは、補正後の共通5秒枠の外だけを切り取る。推定を使ったmanifestは `timingEstimated: true` とし、通常の1ms精度を主張せず、録音全体の長さを含む保守的な不確実性値にする。これは実測精度の保証ではなく、未知の欠損や入力間同期の実機確認は残る。
- WindowsのFLAC分割間にも `capture_timestamp_gap` を記録し、既知の欠損位置を保ったまま解析用ミックスへ渡す。発言・声紋用の連続区間読取りが欠損を拒否する検査は維持する。

通常録音では `<設定した保存先>\meetings\<会議ID>\audio\<録音ID>\capture-clock.json` に診断を残す。開始中、最初の推定発生時、その後は約1秒ごと、保存・失敗時に原子的に更新する。失敗時も取得済みのフレーム数と本来のエラーを記録する。

| 診断項目 | 内容 |
| --- | --- |
| `rawQpc100ns` / `previousRawQpc100ns` | 元の先頭QPCと前の値。単位は100ns |
| `devicePositionFrames` / `previousDevicePositionFrames` | 元のデバイス位置と前の値 |
| `flags` | WASAPIフラグ。1は不連続、2は無音、4は時刻取得エラー |
| `arrivalQpc100ns` / `mappedStart100ns` / `mappedEnd100ns` | 到着時刻と補正結果 |
| `source` / `reason` | QPC・デバイス位置・取得フレーム数・到着推定のどれを使ったかと理由 |
| 各種件数 / `savedFrames` | 同値・逆行・時刻エラー・各推定方法の回数と実保存フレーム数 |
| `firstAnomalies` / `recentPackets` | 最初の異常16件と直近32パケットだけを保持 |

ログには音声サンプル・会議名・機器名を含めない。5秒テストでは専用一時フォルダー内に同じ診断を作り、テスト音声の解放時に併せて削除する。恒久的な会議録音の診断は録音フォルダーに残る。

検証は実マイク・PC音声を使わず、同値・逆行・ゼロとエラーフラグ・位置リセット・既知の20ms欠損を合成して行う。C++時計テスト、5秒録音の実監視・検証・解放3件、通常録音の停止・FLAC保存・欠損記録・解析用ミックス1件が成功した。5秒枠外以外にフレームが失われていないこと、ログの件数・サイズ上限、コンソール非表示、初期化失敗時のエラー維持も確認した。合成診断の実ファイルは `output/windows-qa-work/packet-synthetic-aiyu3zo8/`、その検証結果は `packet-diagnostic-check.json` にある。検証時の診断は30,154 bytesだった。

最新化では `git pull --ff-only --autostash origin codex/windows-platform-foundation` が成功し、`84bb9f217aea533a7a1758054f65ced472fb6e4b` を取得した。既存変更は競合なく復元し、新規コミット・pushは行っていない。以下の旧版記録にある先頭時刻逆行の拒否は、この対応で置き換えた。

この最新コードで、フロントエンド773件、プラットフォーム設定5件、Rust書式検査が成功した。QAへ同梱する最終ヘルパーでもC++時計テスト1件、5秒録音3件、通常録音の保存・解析用ミックス1件が成功した。起動時回復のRustテストはWindowsでは対象がなく0件で、成功した実行テストには数えない。全体Rustテスト・macOS回帰・CIは今回実行していない。

同値・逆行対応版は2026年10月24日01:57:06（日本時間）、終了コード0で再ビルドが成功。生成日時が今回のビルド開始後であること、NSISのMON QA名・QA識別子・ユーザー単位構成・WebView2オフライン導入資材・録音ヘルパー配置を確認した。検証した最終ヘルパーと同梱元のSHA-256は `96a966a4a0a6b2cba21d0e086efcfba65c4bb3869bb935a668d93349779c1425` で一致する。

| 項目 | 同値・逆行対応版の結果 |
| --- | --- |
| 成果物の絶対パス | `C:\Users\local_user\work\mon\app\src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis\MON QA_0.1.0_x64-setup.exe` |
| サイズ | 251,523,949 bytes（239.87 MiB） |
| SHA-256 | `c2869a449575562c45fffa28027f60659bff8cd923d1bc4c6598d31b13840b85` |
| ビルド対象 | `codex/windows-platform-foundation`、`84bb9f217aea533a7a1758054f65ced472fb6e4b` と今回までの未コミット変更 |
| 再ビルド所要時間 | 393.00秒（6分33秒、01:50:33〜01:57:06。調査・修正・先行検証は含まない） |
| ビルドログ・機械可読記録 | `output/windows-qa-work/packet-rebuild.log`、`packet-build-report.json` |
| 最終ヘルパーの検証ログ | `packet-final-native-tests.log`、`packet-final-five-second-tests.log`、`packet-final-recording-tests.log` |
| その他の検証ログ | `packet-frontend-tests.log`、`packet-platform-tests.log`、`packet-startup-recovery-tests.log`（回復テストは対象0件） |

同名の旧インストーラーとは生成日時・SHA-256が異なる。旧MON QAを終了してから、この版で更新する。新版のインストール・起動・実録音、公開用署名、コミット・pushは行っていない。実機での録音確認は利用者が行う。

### 録音時刻・コンソール・起動中表示の追加修正

通常録音と5秒テストの `recording clock reversed` を調査し、取得した次パケットの開始時刻と、前パケットのフレーム数・公称48kHzから計算した終了時刻を比較していた判定を修正した。[WASAPI GetBufferの仕様](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)ではQPCはパケット先頭フレームの取得時刻（100ns単位）である。開始時刻が前へ進んでいる場合、微小な境界の重なりで録音を中止せず、直前の線形対応の終点を新しい観測時刻へ合わせる。全PCMフレームを保持し、正の時間欠損は消さず、補正回数・最大補正量と時計の不確実性をmanifestへ残す。先頭時刻そのものの逆行や不正な変換比は引き続き拒否する。

5秒テストの子プロセスにも `CREATE_NO_WINDOW` を指定した。合成テストでは実際に起動したヘルパーの `GetConsoleWindow()` が空であることを保存記録から検証する。通常録音の起動経路には既に同じ非表示設定がある。

「MONを起動しています…」は起動専用の領域を画面全体に取り、縦横中央へ配置した。ブラウザーで1280×820時の中心が(640,410)、960×640時の中心が(480,320)であることを計測し、画像も確認した。正常起動と起動拒否の遷移は従来の検証を維持する。

追加検証はC++パケット時計テスト（境界の微小重なり、繰り返す不連続、正の欠損、実際の逆行拒否、連続音声）が成功。同じ時刻の重なりを合成した5秒テスト2件と通常録音の停止・FLAC保存・解析用ミックス1件が成功した。フロント773件、プラットフォーム設定5件、起動ブラウザー8条件、Rust書式検査が成功。実マイク・PC音声の取得は行っていない。

ログは `output/windows-qa-work/clock-native-tests.log`、`clock-five-second-tests.log`、`clock-recording-tests.log`、`clock-frontend-tests.log`、`clock-platform-tests.log`、`clock-startup-browser.json`。起動表示の画像は `startup-center-1280.png` と `startup-center-960.png`。

追加修正版の再ビルドは2026年10月24日01:26:10（日本時間）、終了コード0で成功した。同梱元ヘルパーと検証したヘルパーのSHA-256一致を確認している。

| 項目 | 追加修正版の結果 |
| --- | --- |
| 成果物の絶対パス | `C:\Users\local_user\work\mon\app\src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis\MON QA_0.1.0_x64-setup.exe` |
| サイズ | 251,519,960 bytes（239.87 MiB） |
| SHA-256 | `4bca267e573c9869106ea79e96305e6553a1897875a86f156ce7aac6ff0318cb` |
| ビルド対象 | `codex/windows-platform-foundation`、`36431bb6f109b040508d5150f840a8697f8ca621` と今回までの未コミット変更 |
| 再ビルド所要時間 | 325.21秒（約5分25秒、01:20:45〜01:26:10。調査・修正・先行検証は含まない） |
| ログと機械可読記録 | `output/windows-qa-work/clock-rebuild.log`、`clock-build-report.json` |

旧MON QAを終了してから、この生成時刻のインストーラーで更新する。新版のインストール・起動・実録音、コミット・pushは実施していない。

#### 会議の削除メニューについて

現時点のWindows版は会議削除が未対応のため、録音の成功・失敗にかかわらず削除メニューが無効になる。ネイティブの削除操作がUnix向けに限定され、画面側も対応確認がない場合は無効にするためである。今回の録音時刻・表示の修正には会議削除のWindows実装を含めていない。削除するためにDBや録音フォルダーを手作業で変更する手順は提供していない。

### 録音・モデル対応後の初回再ビルド結果

2026年10月24日00:48:09（日本時間）、終了コード0で再ビルドが成功した。録音ヘルパーのビルド出力と同梱元のSHA-256一致、NSISスクリプトのヘルパー・Windowsモデル一覧・ランタイム・通知の配置、ユーザー単位構成とWebView2オフライン導入資材を確認した。

| 項目 | 結果 |
| --- | --- |
| 成果物の絶対パス | `C:\Users\local_user\work\mon\app\src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis\MON QA_0.1.0_x64-setup.exe` |
| サイズ | 251,519,491 bytes（239.87 MiB） |
| SHA-256 | `3a66f6763ec373e1d37fc65e779cd7c3a2d13e4818d57ec2ccfcd151919abffa` |
| ビルド対象 | `codex/windows-platform-foundation`、基準コミット `36431bb6f109b040508d5150f840a8697f8ca621` と今回の未コミット変更 |
| 再ビルド所要時間 | 395.46秒（約6分35秒、00:41:33〜00:48:09。環境準備・実装・先行検証は含まない） |
| ログ | `output/windows-qa-work/rebuild-windows-functional.log` |
| 機械可読記録 | `output/windows-qa-work/windows-functional-build-report.json` |

生成したインストーラーの実行・新版アプリの起動・実録音は実施していない。利用者が旧MON QAを終了してこのインストーラーで更新し、録音と必要なモデルの導入を確認する。公開用署名、コミット・pushは行っていない。

### 初回生成結果（録音・モデル対応を追加する前）

2026年10月23日23:36（日本時間）、QAインストーラーの生成が終了コード0で成功した。

| 項目 | 結果 |
| --- | --- |
| 成果物 | `app/src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/MON QA_0.1.0_x64-setup.exe` |
| サイズ | 224,173,441 bytes（213.79 MiB） |
| SHA-256 | `958eb8f730467d3b82e766fbed657c5264c3b8476eb69827605f701f14f50b0d` |
| 成功したビルドの経過 | 389.47秒（約6分29秒、Rust release工程は4分18秒） |
| 先行した失敗試行 | 通知ハッシュ不一致19.61秒、C++マクロ衝突33.74秒 |
| 環境確認開始から生成結果の記録まで | 約25分（23:11:50〜23:36:51。環境導入・取得・修正・検査・待機を含む） |
| ログ | `output/windows-qa-work/build-attempt3.log` |
| 機械可読記録 | `output/windows-qa-work/summary.json` |

ツール導入・取得には並行した工程があるため、工程時間の合計を総経過時間とは扱わない。細かな修正作業の個別時間は未計測。インストーラーの実行、アプリ起動、録音・AI処理、公開用署名、コミット・pushは行っていない。
