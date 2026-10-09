# release録音ヘルパーとCLI入口の分離

- 対象: M4-01のMac検証用バンドル、M4-02の配布構成
- 更新日: 2026-10-10
- 状態: 最終MON QA構築・署名・両Swiftランナー・音声連携・同梱CLIの実復号成功。GUI・録音・公証は未検証

## 配布構成と入口

MacのbeforeBuildは、Rustアプリ本体がreleaseでも録音ヘルパーをSwiftの既定debug構成で生成していた。`app/build-macos.sh`で`--configuration release`を明示する。共通bundleスクリプトは構築と`--show-bin-path`に同じ引数を渡し、その実行ファイルを配置する。単体の検証プローブの既定構成やMakeのdebug検査は変更しない。

先行release版のXCTestは142件成功・3件skipだったが、後続Swift Testingで製品CLIが`--test-bundle-path`を受け取り、コマンド全体が失敗した。テスト対象のexecutableモジュールにも`@main`があり、最適化したランナーへ製品入口が混入する構成だった。Swift Testingを無効化したり、製品CLIでランナー引数を無視したりして回避しない。

`MONMacOSCapture`を内部実装・検証用の通常targetへ変更し、`MONMacOSCaptureCLI`に小さな`@main`入口を分離した。product名`mon-macos-capture`と既存テストのimport名、出力パス・CLI引数は維持する。入口は公開した`CaptureCommand.run()`へ委譲し、async-mainの実行をMainActorへ明示する。録音・権限・保存・同期・所有・時計・終了処理は変更しない。

Swift Testingのスモーク1件を追加し、CLIを開始せずにcoreのOptionsを読む。XCTestにランナー引数が未知のCLI引数として拒否される1件を追加した。フロントの設定検査もrelease指定・同じ構成の実行ファイル配置と入口の分離の2件を追加した。

## 最終結果

- 設定7件とフロントエンド575件成功。初期の設定6件・全体574件も成功し、同じ検査を新規件数へ二重加算しない。
- 最終release Swift: XCTest146件中143件成功・専用3件skip、Swift Testing1件成功、コマンド終了コード0。
- 最終debug Swift: XCTest143件成功・専用3件skip、Swift Testing1件成功。Python86件、書式・eval前提・plist、CAF全4,800framesと専用音声連携が成功。
- 変更した入口に関係する専用Rustラッパー19回はすべて終了コード0。通常／部分救出・原音声・時計・通知・所有付き削除／移動SIGKILL・特徴量・保存区間worker等を維持した。
- 同梱した署名済みrelease CLIを実行するRustの実取り込み1件はsandbox外で成功。合成音声のWAV・MP3・AAC M4A・ALAC M4A・FLACを内容で判定し、原本不変・移動後の冷起動再生・取り込み済み原本のSHA一致を確認した。動画付きM4Aと破損入力は会議を作らず拒否する。録音・実会議・実モデル・GUI統合の証明にはしない。

変更範囲に合わせて`make lint test build-macos-probe check-caf-playback check-capture-clock check-input-levels check-input-status check-input-history check-input-journal check-capture-test check-microphone-recovery check-system-recovery check-capture-ending check-recording-raw-prefix check-speaker-embedding`を実行した。終了コード0、経過202.89秒。型検査・フロント構築はnative beforeBuild、全フロントとRust書式は別に確認した。通常Rust1,739件・doc test全体は今回再実行していない。直前の全体成功は[PCM形式照合](microphone-callback-format-2026-10-10.md)の証跡であり、今回の件数・時間に混ぜない。

## バンドルと実CLI

最終MON QAは`/private/tmp/mon-native-qa-release-artifact.bs0dqS/MON QA.app`へdittoで保持した。元との全ファイル差分なし、コピー後のdeep・strict署名成功。33,992KiB（約33.20MiB）。一時領域であり永続配布物ではない。以前のdebugヘルパー付きQAを最新だとは扱わない。

Info.plistはQA識別子`jp.mon.desktop.qa`・表示名MON QA・minimum macOS 15.0。本体と録音ヘルパーはMach-O arm64、両ヘルパーを実ファイルで同梱し、個別strict署名も成功した。録音ヘルパーの権限説明・PrivacyInfo.xcprivacyを保持しplist lintも成功。本体署名は既存のアドホックで、公証は資格情報なしでスキップされた。Developer ID署名・公証・Gatekeeper通過・必須OSの実機成功を保証しない。

SHA-256は本体`12669c25c92fdfb952ae850677539d020784dea76f515cf91ed366ccf5c224ec`、録音`4c897545854088bb3822123a0114ea0b48b09a29145ca9a0919c6c598036320f`、特徴量`d9b217f5ebaced3957ebf77b3e35874fc21be06cf474eab0f550c313a21ba8a9`。録音の全体SHAは生成した署名済みbundle内と保持QA内で一致した。SwiftPMの元実行ファイルはbundle署名と異なる署名を持つため全体SHAが異なるが、最終release元と同梱CLIの`otool -s __TEXT __text`（ファイル名行を除く）SHAは共に`f8d83aa214d598794af10a84ea3179e705bae1ab1d3bd04ee51b16054e0e1dbf`だった。

同梱CLIの`--help`は正常終了し、`--test-bundle-path`は未知引数として既存の終了コード2で拒否した。最初の確認で誤って終了コード1を期待したため確認コマンドを失敗させたが、製品の終了コードを変更せず確認を修正した。MON QAのGUI起動・実録音・TCC承認はしていない。録音ヘルパーは約1.25MiBとなったが、処理速度・長時間安定性の改善率は実測していない。

## 失敗・再試行と工程時間

最初のrelease Swiftは構築35.02秒・XCTest本体9.052秒、コマンド45.39秒で失敗。入口分離後は構築21.92秒・XCTest8.855秒・Swift Testing0.001秒、コマンド32.03秒で成功した。文脈不一致のapply_patchを1回拒否されたが、その呼出しは未適用で、実際のPackage.swiftを確認して適用した。

実取り込みのsandbox実行は、最終releaseと以前のdebugの両方でAACの`configure PCM failed: 1718449215`となった。製品コードを変更せず、同じreleaseヘルパーと合成音声検査をsandbox外へ切り替えると成功した。実行環境の制限への依存を示す結果であり、特定のOSサービスが原因だと直接証明したものではない。音声・モデルの外部送信、録音、権限承認は行っていない。

| 工程 | 参考小計 | 範囲 |
| --- | --- | --- |
| 実装・調査・修正・記録 | 約15〜25分（概算） | release指定、入口分離、回帰検査、実CLIの環境切り分け、署名・記録等。構築待ち全体を純作業へ含めない |
| 計測済み構築 | 431.45秒（7分11.45秒） | native2回のコマンド全体311.80秒、Swift検査構築65.28秒、Rust専用・失敗再試行の構築54.37秒 |
| 検査実行区間 | 146.49秒（2分26.49秒） | Rust実行区間102.89秒、失敗を含むrelease/debug XCTest・Swift Testing、Python10.556秒、先行と全フロント本体2.691315秒 |

native先行193.78秒、最終118.02秒は型検査・Vite・ヘルパー生成・Rust release・バンドル・署名込み。内部Swift14.45秒／10.27秒、Vite1.39秒／1.38秒は内数で別加算しない。Swift検査構築はrelease2回とdebug・8 fixtureの計測を含む。Rustの専用構築ではルートcrateの再構築49.26秒が実際に発生した。

取り込み失敗releaseのコマンド35.65秒は共有キャッシュのロック待ち等を含み、構築0.52秒・実行区間1.57秒を純作業の35.65秒とはしない。debug対照は構築0.23秒・実行2.01秒・コマンド2.56秒、sandbox外の最終は構築0.22秒・実行2.90秒・コマンド3.39秒。失敗・再試行は参考小計に含む。

署名・CLI help／未知引数・plist・コピー等は個別未計測。Swiftスモーク・内部子構築等も未分離。Rust実行区間は内部構築等を含むため純粋な検査本体だけではない。native構築・release検査・フロント検査、専用連携とロック待ち・記録に重なりがあるため工程の和を経過時間としない。

## 回収と残タスク

最終ソース7ファイルのSHA一致と、関連コンパイラ・バンドラー・ヘルパー・ffmpegの終了を確認した。保持アプリとログを残し、今回作成した専用native target1,229,088KiBとrelease検査scratch226,688KiBの合計約1.39GiBだけを削除した。必要時は新しい専用targetで再構築できる。共有テストキャッシュ10,084.3MiB（約9.85GiB）、製品target38,664KiB（約38MiB）、Swiftの既存プロジェクトキャッシュを保持する。録音・モデル・既存release・ユーザーViteは変更・削除していない。Windows capture buildの`disabled_manually`をAPIで確認し、有効化・手動実行していない。

取得形式・残境界レビュー0.5〜1.5時間、実モデル・残UI・配布6〜12時間、Mac実機・長時間4〜8時間以上、計10.5〜21.5時間以上の暫定残工数を維持する。実モデル配置先・GUI操作許可・署名／公証・先行試用の外部準備待ちは別。正式評価・Windows固有部分とWindows CIは保留。M3-01・M4-01・M4-02と計画全体は進行中。

ログ: `/private/tmp/mon-native-release-build-v1.log`・`build-v2.log`、同prefixの`swift-v1.log`・`swift-v2.log`、`config-v1.log`・`config-v2.log`、`frontend-v1.log`・`frontend-v2.log`、`capture-check-v1.log`、`import-v1.log`・`import-debug-baseline-v1.log`・`import-unsandboxed-v2.log`。CLIは`/private/tmp/mon-native-release-cli-help.log`・`mon-native-release-cli-unknown.log`・`mon-native-release-cli-unknown-v2.log`。
