# macOS実機検証用MON QAの準備

2026-10-10時点の最新QAは `/private/tmp/mon-settings-compact-final-qa.WFa7PG/MON QA.app`、main SHA `1a21a28e5822e483cb595515b9c255e7cfc11912b6f11a6f814e125e693a0102`、識別子 `jp.mon.desktop.qa`。正常な空状態だけ設定の中断・削除5カードを折りたたみ、対象／失敗／未知状態を隠さない表示整理と終了時の未完了データ再読取を含む。フロント641件、合成ブラウザの限定異常系／modal操作、録音なしMacの通常表示を確認。コピー一致・strict deepアドホック署名・全arm64・通知一覧一致・capture help確認済み。実録音・実モデル・実機異常系・公証は未検証。[範囲・工程時間・残件](m4-01-native-nonrecording-ui-2026-10-10.md#設定の通常表示と中断対応の分離)を参照。以下の「最新」は当時の履歴である。

2026-10-10時点の最新QAは `/private/tmp/mon-prototype-simple-qa-artifact.WuwVHb/MON QA.app`、main SHA `5468530636a8b82918153403195daf9c1918aff91fbc173e417fc9a8a56ab897`、識別子 `jp.mon.desktop.qa`。モック準拠の会議一覧・3入口・設定内だけのテーマ切替、CSS／Mac小窓背景の角丸、明示的な説明のWebView焦点復帰を含む。コピー一致・アドホック署名・全arm64・capture help、録音なしの冷起動／検索／2タブ／準備から戻る／小窓を確認。反復focusの変動、全画面、実録音／モデル、配布署名／公証は残す。[範囲と工程時間](m4-01-native-nonrecording-ui-2026-10-10.md#モック準拠と画面操作の簡素化)を参照。以下の「最新」は当時の履歴である。

MON QAは、通常利用のMONとは別のアプリ識別子とウィンドウ名で実機検証するためのビルド設定である。メイン・録音ミニパネル・録音確認・録音説明ウィンドウの操作経路と権限制限は本番と同じにする。起動や録音は設定追加だけでは行わず、操作権限と検証範囲の許可後に進める。M4-01と計画全体は進行中である。

現在の最新QAは `/private/tmp/mon-raw-duration-qa-artifact.nHNweS/MON QA.app`、main SHA `cd38dc862662d68d404d2e84a1d10a8efebc68cf7adcf0e33c3c02cdf2ca6d31`。共通dialogのネイティブmodal状態への統一に加えて、[未処理PCMの500ms上限を端数切捨てで超えない修正](live-raw-recording-segments-2026-10-09.md#2026-10-10の未処理pcm時間の端数保護)を含む。最新Swift全体・合成Swift／Rust連携・QA構築・コピー一致・アドホック署名・全arm64・識別子・capture help・未承認通知文照合を確認した。操作可能となった後、新QAの冷起動・既存3会議・復元プレビューの限定Tab／Escapeを確認し終了した。同条件の旧側もAX取得に成功したため、以前の欠落は今回再現できず、原因／解消の断定・VoiceOver等の全体合格にはしない。実復元・録音・実モデル・公証は未検証。[今回の実機比較](m4-01-native-nonrecording-ui-2026-10-10.md#再操作可能になった後の最新qaと旧qaの比較)を参照。以下の「最新」表記は当時の履歴として扱う。

直前のQAは `/private/tmp/mon-modal-modality-qa-artifact.u8TS4Y/MON QA.app`、main SHA `1ed7f67296cdde68fa33c3d1e5d33c202b4a5ef0491cb06a7ddda506967da3f1`。共通dialogの固定ARIA宣言だけを外し、ネイティブmodal状態へ統一した。フロント634件と合成ブラウザの限定回帰、QA構築・コピー一致・アドホック署名・全arm64・識別子・capture help・未承認通知文照合を確認。Macの再ロックにより復元画面AXの旧／新A/Bは解除待ちで、実機不具合解消・録音・実モデル・公証は未検証。[今回の範囲・時間・解除後手順](m4-01-native-nonrecording-ui-2026-10-10.md#ネイティブモーダル状態への統一と解除待ち)を参照。

最新QAは `/private/tmp/mon-legacy-seal-qa-artifact.rI1uKt/MON QA.app`。直前の読取コピー案内等に加えて、[形式版1の会議参照を完全削除前に保護する修正](legacy-deletion-seal-detachment-2026-10-10.md)を含む。構築・全コピー一致・アドホック署名・全arm64・識別子・capture help・未承認通知文照合を確認した。新しい画面機能はない。ロック解除後、この成果物で移動済み保存先の冷起動・日本語会議名／用語の保持・合成6秒M4Aの取り込み／再生／冷起動後保持を実画面で確認し、終了した。追加復元確認のAX操作要素欠落は2回再現し、Escapeで取消、復元は実行していない。原因は未解決で、録音・実モデル・公証・モーダル全体の合格とはしない。[最新QAの再確認](m4-01-native-nonrecording-ui-2026-10-10.md#ロック解除後の最新qa再確認)を参照。以下のロック待ち・未起動記述は、その時点の履歴である。

2026-10-10にユーザーからQAの起動・画面操作許可を得て、録音なしの合成WAV取り込み・保存・再生、日本語会議名と会議別用語、バックアップ・追加復元と冷起動後の保持を実画面で確認した。保存先変更後のQA起動設定は `/private/tmp/mon-native-ui-smoke.SOSoB9/MON-meetings-f352507d-7b63-4335-b953-6b063348c5fe` を選択している。旧QA領域と検証素材は保持する。直前の成果物は `/private/tmp/mon-read-cache-guidance-qa-artifact.tYCkOU/MON QA.app` で、[読取用一時領域の起動拒否案内](read-cache-startup-guidance-2026-10-10.md)、[所有付きのWAL読取コピーと中断回収](owned-read-cache-recovery-2026-10-10.md)、[マイクの再取得中の原因保持修正](microphone-retry-diagnosis-2026-10-10.md)と[版固定で補完した通知文と未承認の一覧・MPLソース案内](macos-notice-supplements-2026-10-10.md)を含む。全コピー一致・アドホック署名・全arm64・QA識別子・同梱help・通知文照合は確認済み。Macのロック解除待ちにより修正後のGUI再確認は未完了。録音・TCC許可・実モデル・長時間・配布署名／公証は未検証。詳細は[録音なしMac実機検証](m4-01-native-nonrecording-ui-2026-10-10.md)を参照。以下の未起動記述は、その時点の履歴として扱う。

## 本番との分離

| 対象 | 通常利用 | 検証用 |
|---|---|---|
| アプリ名 | mon | MON QA |
| 識別子 | jp.mon.desktop | jp.mon.desktop.qa |
| ウィンドウ名 | mon | MON QAで始まる名前 |
| ウィンドウの役割 | main、recorder、recording-confirmation、recorder-info | 同じ役割とURL |

`app/src-tauri/tauri.qa.conf.json`は識別子・アプリ名・ウィンドウ名だけを変更する。Tauriの既存のmacOS設定は自動的に読み込まれるため、録音・復号・再生ヘルパーと最低OS条件を維持する。通常の設定ファイルとnpmの`tauri`コマンドは変更しない。

起動処理はTauriの`app_config_dir`と`app_data_dir`を使い、インストール済みTauri 2.12.1では既定の領域にアプリ識別子を付ける。QA設定には領域の上書きを置かない。ただし、既存QAの起動設定で保存先が変更されている場合はその保存先が使われる。起動前にQAの設定・DBと本番との分離を確認し、本番の保存先をQAへ指定したり、本番のbootstrap設定をコピーしたりしない。

通常利用のMONを起動すると、未完了の音声取り込み・録音回復や解析待ち行列が動くことがある。検証目的で本番アプリを起動せず、QAのウィンドウ名・バンドル識別子と保存先を確認する。

## ビルドと起動前確認

対象はmacOS 15以降のApple Siliconである。空き容量とNode.js・Rust・Swift・Xcodeの既存環境を確認する。必要容量を確保できない場合はビルドを始めない。リポジトリのtargetへ大容量キャッシュを戻さず、今回専用の一時領域を指定する。

```sh
cd /Users/skodani/Codebase/mon/app
qa_target=$(mktemp -d /private/tmp/mon-qa-build.XXXXXX)
CARGO_TARGET_DIR="$qa_target" CARGO_INCREMENTAL=0 npm run tauri:qa:build
```

このコマンドはローカルビルドだけを行い、アプリやCIを起動しない。生成されたアプリの`Contents/Info.plist`で`CFBundleIdentifier`が`jp.mon.desktop.qa`、アプリ名がMON QAであることを確認する。出力場所は実際のビルド結果で確定する。署名検査も行うが、既存設定のアドホック署名は配布用署名・公証の完了ではない。

アプリ操作の許可後にQAだけを起動する。最初は録音を行わず、利用を許可された音声のコピーで取り込み・再生・編集・保存・再起動を検証する。初回QAに本番の会議や設定が表示された場合は操作を止め、領域の分離を再確認する。録音の検証は、対象音声と時間・取得範囲を確認し、マイク・システム音声の権限承認後に別途行う。

ビルド・検証のプロセスがすべて終了してから、生成アプリと必要なログを保持し、今回作成した専用キャッシュだけを整理する。既存の会議・音声・モデル・本番設定は削除しない。

## 静的検査と実機の確認項目

2026-10-07にQA設定の追加4件を含むフロントエンド268件が成功し、失敗・skipは0件となった。通常のフロントエンドビルドも成功した。検査は識別子の分離、全ウィンドウの役割・URL・寸法・表示条件の一致、保存先・権限・署名・ヘルパー設定を上書きしないこと、起動を含まないビルドコマンドを対象とする。ログは`/private/tmp/mon-native-qa-config-units.log`と`/private/tmp/mon-native-qa-config-build.log`に保存した。

2026-10-07時点ではネイティブバンドルは作成・起動していなかった。2026-10-10に初回のMON QAバンドル構築と識別子・署名・リソースの確認を完了した。構築前に話者特徴量ヘルパーの同梱指定漏れを修正し、本番・QA共通のMac設定へ追加した。詳細と保持場所は[MON QAのネイティブバンドル検証](m4-01-native-qa-bundle-2026-10-10.md)を参照。アプリの起動は行っていない。

実際の設定・DB領域、WebViewとネイティブ操作、録音権限と取得条件、通常・異常時の統合は実機の確認項目として残る。通知実装の本番Reactと合成IPCによる全38組・450ケースは別の検査であり、ネイティブ統合の代わりにしない。正式評価とWindows固有部分は保留し、Windows CIは停止したままとする。

2026-10-10の直前のQAは[release録音ヘルパーとCLI入口の分離](m4-02-capture-release-bundle-2026-10-10.md)で再構築した。最新のPCM形式ガードとreleaseヘルパーを含み、同梱CLIの5形式取り込み・再起動再生は合成音声で確認済み。保持場所は`/private/tmp/mon-native-qa-release-artifact.bs0dqS/MON QA.app`。

直前のQAは[録音パネルの独立した説明画面](m3-08-recorder-info-window-2026-10-10.md)を含めて構築し、`/private/tmp/mon-recorder-info-qa-artifact.p91QMy/MON QA.app`に保持した。

直前のQAは[録音パネルの操作通知と原因保持](m3-08-recorder-operation-notifications-2026-10-10.md)も含めて再構築し、`/private/tmp/mon-operation-notices-qa-artifact.oerdTa/MON QA.app`に保持した。

直前のQAは[保存先の起動拒否時の案内](storage-startup-attention-2026-10-10.md)とミニ画面の両入力ラベル幅の修正も含め、`/private/tmp/mon-startup-attention-final-qa-artifact.LbAFro/MON QA.app`に保持した。

直前のQAは[手動更新時のDB移行の一括確定](manual-update-migration-2026-10-10.md)も含め、`/private/tmp/mon-migration-update-qa-artifact.V6THu6/MON QA.app`に保持した。

直前のQAは[初回ダウンロードの通信・一時配置基盤](model-download-transport-2026-10-10.md)を含め、`/private/tmp/mon-model-download-final-qa-artifact.k0WiiN/MON QA.app`に保持した。

直前のQAは[ダウンロードの読戻しと配置由来](model-download-readback-2026-10-10.md)も含め、`/private/tmp/mon-download-readback-qa-artifact.UsausP/MON QA.app`に保持した。この時点では基盤はIPC非公開で、設定画面のダウンロードはまだ利用できなかった。

直前のQAは[初回取得の画面・IPC・停止待ち・結果確認](model-download-ui-2026-10-10.md)を含め、`/private/tmp/mon-download-ui-final-qa-artifact.6FTOqF/MON QA.app`に保持した。取得・管理コピー・CPU実行・適用は別々の明示操作であり、一覧の完了記録だけでは実体を利用可能にしない。署名・識別子・両ヘルパーは確認済みだが、GUI起動・保存先分離・実録音・実モデル／配布元・公証は未検証のままである。専用UIへの切替・通常IPCの拒否・OSの終了とデータ移行を実機で確認するためにも、本番と分離したQA領域を使う。

直前のQAは[時計復帰の録音制御・入力通知接続](capture-clock-recovery-control-2026-10-10.md)を含み、`/private/tmp/mon-clock-control-qa-artifact.wpTMjP/MON QA.app`に保持した。通常の所有付き確認済みSCStream録音の両入力停止・新epoch・再取得経路を含む。アドホック署名・識別子・全arm64・両ヘルパー・同梱CLIのhelpは確認済みで、GUI・TCC・実スリープ・実録音・実モデル・配布署名／公証の成功にはしない。

直前のQAは[PC音声の旧出力解除の失敗経路修正](system-output-retirement-2026-10-10.md)も含み、`/private/tmp/mon-output-retirement-qa-artifact.0oNZjp/MON QA.app`に保持した。旧出力の解除成功とcallback排出まで所有を保ち、解除失敗では新streamを開始しない。成功した停止を繰返さず、形式変更のblockも維持する。全コピー一致・アドホック署名・識別子・全arm64・両ヘルパー・同梱CLIのhelpは確認済みだが、GUI／TCC／実SCStreamの停止と解除／実録音／実モデル／配布署名と公証は未検証のままである。

直前のQAは[解析プロセスの親寿命と所有付き終了](local-engine-parent-lifetime-2026-10-10.md)も含み、`/private/tmp/mon-owned-engine-qa-artifact.OwfWTg/MON QA.app`に保持した。同じ製品実行ファイルの私有headless入口をGUI初期化前に分岐させ、合成engineの正確な出力・終了コード、継承端／親の拒否、通常終了後の残子、通信切断、実親SIGKILLと無関係な対照プロセスの保持の6ケースを確認した。アドホック署名・識別子・全arm64・同梱help・全コピー一致も確認済み。私有入口の検査は製品GUIの起動、実会議、実モデル、録音優先、OSスリープ、Developer ID署名と公証の成功とは区別する。

最新QAは[録音確認の回収ロックと並列検査](capture-test-quiet-ownership-2026-10-10.md)の修正も含み、`/private/tmp/mon-quiet-ownership-qa-artifact.5SgmBI/MON QA.app`へ保持した。通常Rustの1／4プロセス全件と専用Swift継承lease連携が成功し、回収用descriptionだけを明示解除して、取得ヘルパーの継承所有は保つ。全コピー一致・アドホック署名・識別子・全arm64・同梱helpと、同じmainのheadless6ケースを再確認した。GUI・TCC・実録音・実モデル・長時間取得・配布用署名／公証は未検証のままである。

GUIを起動しない専用検査は、同じ新QAが用意されている場合に限り、リポジトリで以下を実行する。古いQAには私有入口がなく、同じ引数でも通常GUIを起動し得るので使わない。

```sh
node tools/mon_owned_engine_check.mjs --executable '/private/tmp/mon-quiet-ownership-qa-artifact.5SgmBI/MON QA.app/Contents/MacOS/mon-desktop' --sha256 bc45033b70acef09922b97297133f8931e8e396ce5996f2edee55f8376abccbe
```
