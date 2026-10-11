# ファイル選択式のモデル取り込み

2026-10-11。M4-02のモデル導入を進める限定実装。正式評価とWindows固有部分は保留。一般利用者向けの公式モデルカタログ／アーカイブ初回取得の完成とはしない。

## 実装

- 設定 → ローカルから取り込むで、文字起こし／話者分離／議事録生成を選び、必要な実行ファイルとモデルをMac標準のファイル選択から指定する。JSON作成・絶対パスの入力・元ファイルを同じフォルダーへ移す作業は不要。
- 共通の親フォルダーと相対パスを導出するが、フォルダー全体の走査／コピーはしない。選択された通常ファイルだけを検査・コピーする。共有ライブラリの相対配置を保持し、任意のVADと依存ライブラリは詳細内で明示選択する。依存の自動検出はない。
- 構成はメモリ上で生成し、元フォルダーの `mon-model.json` を作成／上書きしない。既存manifestを採用しない新経路と、既存manifestを読み込む旧経路を分離。モデル名はファイル名、エンジン版／モデル版は「未申告」。配布元認証・ライセンス確認・性能／精度の評価ではない。
- コピーは別確認後のみ。検査内容の構成・ファイル別SHA／容量・役割を再照合し、管理領域にだけcanonical manifestを生成する。元ファイルは変更しない。標準／高精度への適用とCPU試行は従来どおり別の確認とし、選択・検査・コピーでは実行しない。
- 新経路の由来は `manual_import`。v1配置記録を保持し、生成したmanifestバイト列のSHAで構成を固定する。このSHAを配布元の提供値や元フォルダーに実在したJSONのSHAと呼ばない。既存のdownload由来／読戻し／意図保存／中断保全を変更しない。
- 種類・選択変更で検査結果を無効化し、キャンセルは前の選択を保持。未mount／状態変更／古い選択結果を受け入れない。Macのネイティブ選択終了後、同じモーダルが有効で操作可能な場合だけ、元の選択ボタンへfocusを戻す。

## 自動検証

- フロント812件成功、失敗／skip 0、2.029284833秒。純粋な宣言生成／不正選択／検査応答の結び付け5件と、focus／コピー宣言引継ぎのソース契約2件を追加。ソース契約は実GUIの証拠ではない。
- 新しいRust5件は成功。manifest不要／既存manifest保持／3エンジン／取消／不正パス／symlink拒否／管理コピーの読戻し／同じ操作の再確認／構成変更・素材変更の拒否を確認。構築187.83秒、runner実行7.92秒（libtest0.19秒）。
- 関連回帰は検査12件（構築0.58秒・実行0.04秒）、配置13件（0.26秒・1.10秒）、既存取得35件（77.58秒・10.24秒）、すべて成功。新5件はこの60件に含むため加算しない。取得の合成ループバック通信は許可された環境で実行し、外部モデルの追加取得はしていない。
- Rust cacheは最後に8317.3MiB、約8.12GiB。上限10GiB内に再生成可能な古いcacheを回収。元音声・モデル・会議は回収対象ではない。全Rust／全browser driverを最新ソースで再実行した証拠ではない。
- `cargo fmt --check`／`git diff --check`成功。Sonnerのuse-client、既存大chunk、Swift読み取り専用cache、12件のnotice本文不足、正式署名／公証の未準備は既存の残件。

ログ（私的領域、Gitには含めない）:

| 内容 | `/private/tmp/` 内のファイル | SHA-256 |
|---|---|---|
| フロント | `mon-guided-model-ui-20261011.log` | `57f8e117157d50265d7017d27d0b098f6b49927fb6e0f807030966df572eb89f` |
| Rust追加5件 | `mon-guided-model-rust-20261011.log` | `f8b391018c53cb132cf8819a5057b15780bf2f227603357c9d751c77b2f94662` |
| 検査回帰 | `mon-guided-model-inspection-regression-20261011.log` | `e34fbd90f6955e43b261b4ec6ba26cfc5b4bffe64410133706c3464065d13171` |
| 配置回帰 | `mon-guided-model-placement-regression-20261011.log` | `945d789f260708986a63ad1f45baf3cbe1301c5066d2bd3016be37448020859d` |
| 取得回帰 | `mon-guided-model-download-regression-20261011.log` | `95e105f2a0c55ae5bbcc77c468c3b294ef7a7bcf3bb2d87d11883b51270387cb` |

## 合成GUIとMac実ファイル検査

合成画面の2ファイル選択 → 検査 → 別のコピー確認 → 合成コピーを通過。表示される操作数は検査1／コピー1／CPU0／適用0。最終応答整合guard追加後にも選択→検査が成功し、話者分離・LLMへの種類変更で選択・検査結果・コピー入口が解除された。合成fixtureは実ファイルや実プロセスを操作しない。ここからMacのコピー・実行成功を推測しない。

Macの最初のQA（`/private/tmp/mon-guided-model-qa.JtPmHo/MON QA.app`、main SHA `e7069e6677bb87b41f90781d017fca0ae7843d142ec3d4b86c0c36133de17923`、deep strict ad-hoc署名検査成功）で、選択取消後の未選択保持、実際の2ファイル選択と検査を確認。再取得済み原本の `whisper-build/bin/whisper-cli` 2,501,672bytesと `ggml-tiny.bin` 77,691,713bytes、合計80,193,385bytes。元の共通親フォルダーにmanifestがない状態で、画面へパス・JSONを入力せず成功した。native AXのSHAと事前の独立SHAが一致:

- CLI: `9f631df18105fb3246bfb66fb4c349372c9c338c0c34ed40271a682eab61b7c0`
- モデル: `be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21`
- 生成manifestのSHA: `4dbaff0bb61dae2a4cf6f72ccc61d58c4f5398cfb6457f7e7c3448104b1f7b53`

Macではコピー／CPU試行／適用を行っていない。選択後のfocus欠落をここで発見し、終了後にfocus復帰と説明のinfo移動を補修。最終QAでの補修再確認は後段に記録する。

QA終了確認後、元の2ファイルとbootstrapのSHAは不変。会議DBの全下書き・出力／全解析版・議事録版・会議head／設定・jobs／用語の4組が開始前と一致:

- 下書き・出力: `af00b3854243d5690c74e6b404cce22cff4479edf4423bc532c7a3ecd15ed035`
- 版・会議head: `6bdb8534aa2495a4ff042106f68c079adbafa448e8bb9201bff766fa0f542961`
- 設定・jobs: `213509aef1d22a5aa172f73463d19be7651103e056d52a6c876ca88651a26226`
- 用語: `e6882c0c2721d29b3babf5bcac02f17dc6c036d4dc7af77b90c9dfd0c5f34aee`

## 最終focus補修と再確認

最初の補修QA（main SHA `88c27e0ea65fe8113bacbfc2b52d68a051bc4ffad7e733c1008dc3c635093e28`、Vite1.51秒／Cargo1分10秒）では、取消後もnative AXのfocusがHTMLへ戻る状態を確認。即時の1フレームだけで解消したと扱わず、ボタン再有効化のcommit後と150ms後の限定補修へ変更した。変更後も同じ操作・可視な同じ最上位モーダル・元ボタンが有効な条件だけで行い、利用者が別の有効な入力を選んでいれば奪わない。

最終 `/private/tmp/mon-guided-model-focus-qa.dSPjjM/MON QA.app` はmain SHA `213a141c3add9219885ee4cda1ee8398db230f18e1d7f83a828fa0999c4a9689`、deep strict ad-hoc検査成功。Vite1.53秒／Cargo1分12秒、build log `mon-guided-model-qa-focus-build-20261011.log` SHA `154c29a777cdd047390fc16d2e09f31cc3d0140039c23cf3b5ee0ceb49ea48be`。Macで実行ファイル選択→取消→元の実行ファイルボタンfocus、Tab→モデル選択ボタン、モデル選択→取消→元のモデルボタンfocus、Escape→取り込み入口focusを確認。選択は未選択のままで、実行／コピー／適用を行っていない。

最新フロント812件は再実行成功、2.118716667秒。`mon-guided-model-ui-final-20261011.log` SHA `c2c5c41cfda0aa7949f036597f188c4ac65021f0eb93b4e76a8e12149095a7c1`。上表の旧ログは初回812件の履歴として残す。JSON旧経路のbrowser driverも既存設定の開閉とportaled modalに合わせて更新し、構文検査成功。最終コードの合成GUIで旧入力→検査と新選択→検査、選択ボタンfocusを確認したが、driver全ケース実行の証明とはしない。

最終QA終了後も保護対象4組／bootstrapは開始前と一致。画像は私的領域の `mon-guided-model-native-inspection-20261011.png`（実ファイル検査）と `mon-guided-model-native-final-20261011.png`（最終簡素化画面）。録音・追加取得・本番データ変更・CI・コミット／Pushなし。

## 引き続き残る対応

公式カタログからの選択式初回取得、実行依存を含む公式アーカイブの安全な展開、採用構成／利用条件の確定が残る。今回の簡素化は手元の実行ファイルとモデルを用意できる利用者向けで、一般利用者のワンクリック導入完了ではない。Macの新ガイド経由の3エンジン全コピー・CPU・適用の統合確認も未完了。実録音／4時間、全IME／VoiceOver、正式署名／公証・先行試用等の監査残件は維持。CI／コミット／Pushなし。

## 追実装：固定カタログとアーカイブ取得

上段の「公式カタログ／アーカイブ未実装」は先行時点の履歴。設定に「モデルをダウンロード」を追加し、llama.cpp b11533＋Qwen2.5-1.5B-Instruct Q4_K_Mと、sherpa-onnx v1.13.8＋pyannote segmentation 3.0／3D-Speaker ERes2Net baseを固定定義から選べるようにした。JSON・パス入力は不要。文字起こしの自動導入は未実装で「準備中」と表示し、一般利用者向け全3種類の導入完了にはしない。

形式版1のバイト列／SHAと旧配置由来を保持したまま、形式版2にアーカイブ全体の容量／SHAとmemberPathを追加。同じURLは1回の通信として扱い、保存するモデル容量・通信容量・保持する圧縮原本容量を区別する。macOSシステムlibarchiveのTARとnone／gzip／bzip2だけを有効にし、OSのパス展開APIは使わない。選択した通常ファイルだけをprivate create-newハンドルへ保存し、全体と内部ファイルの両SHA、パス・重複・項目数・膨張容量・取消・制限時間・親の実体を確認する。選択しないスクリプトやライブラリは保存／実行しない。

`notice`の役割を追加し、利用条件の本文も検査・コピー対象へ含める。実行引数のfingerprintとは区別するが、パッケージ全体の再検査からは除外しない。取得／コピーは実行・適用を行わず、CPU試行・設定適用は別の確認。由来は`download`のまま形式版2を保持し、手動取り込みへ付け替えない。

| 選択肢 | 通信bytes | payload bytes | 保持するアーカイブbytes | 定義SHA-256 |
|---|---:|---:|---:|---|
| LLM | 1,129,407,950 | 1,136,172,053 | 12,075,871 | e4aaf7c71da23a1cadc141d92208d6eac4274eb12fa1901383f7753234f210ba |
| 話者分離 | 65,153,215 | 75,117,137 | 25,210,612 | 0996bb77a0fab2f604c97af2bde017cebe813e83529228be92778c4befd4a7c8 |

LLMの共有ライブラリは実際の`otool -L`から必要な9件を選び、アーカイブの通常ファイルをロード先の名前へ配置する（リンクの展開を許可しない）。話者分離はbin／libの相対配置を保持する。ONNX Runtimeは固定sherpaソースのCMakeとライブラリ内の版文字列で1.28.2を照合。LLMはMIT／Qwen Apache-2.0の本文2件、話者分離はsherpa／3D-Speaker Apache-2.0、segmentation／ONNX Runtime MIT、ONNX Runtime ThirdPartyNoticesの5件を保持する。再配布承認・配布元署名認証・全依存条件の最終監査ではない。

公式取得はLLMアーカイブ12,075,871bytes、Qwen本文11,343bytes、話者分離関係の本文4件348,842bytesを追加。既存のモデル・sherpa／segmentationアーカイブは再利用。既知の完了取得ファイルの累計は2,112,224,861bytes（約1.97GiB）で許可4GiB内。メタデータ／本文の読み取り通信すべてを含むネットワーク総転送量の測定ではない。ダウンロードした本文のSHAは独立算出、segmentationアーカイブも配布元digest未提供の独立算出値であり、公開署名と呼ばない。

### 実LLMで判明したCPU条件の補修

最初の実素材試行ではstage／コピーが通過してもCPU確認が失敗した。診断付き再実行のstderrで、gpuLayers=0にもかかわらずMetalのcommand queue／context生成を試みて失敗することを確認した。公式CLIの対応引数を確認し、CPU指定時だけ`--device none --no-kv-offload --no-op-offload`を追加。固定したジョブ条件がメモリ都合で変わらないよう`--fit off`も明示する。GPU指定時はCPU専用引数を追加せず、fake CLIの契約検査を追加した。実GPUの成功とはしない。旧CPU専用のソースビルドが成功したことから、公式共有ビルドもCPU動作するとは推定しなかった。

修正後の実素材試行は終了0：stage8.057秒、管理コピー11.477秒、固定JSONを返すCPU確認15.878秒。構築195.50秒、runner実行43.23秒（libtest35.51秒）。取得済み約1.1GBのGGUFを使用し通信なし、test-owned Storageとprivate TempDirだけを操作した。この初回成功は適用前までの証拠であり、後段の2種類最終試行と混同しない。

### 検査の履歴と限定範囲

アーカイブ追加後の取得回帰39件成功・実素材1件ignored（構築162.23秒／runner10.96秒）。CPU補修後の検査／配置／取得／互換性／設定／由来バックアップ等122件成功・専用2件ignored（構築0.59秒／実行38.21秒）。これらのignoredを成功件数へ加算しない。最終2選択肢のフロント815件成功（1.96064425秒）、型検査／Vite build成功（1.59秒）。全Rust・全browser driverや仕様全体の一括再検証ではない。

合成GUIでJSONなしの選択→確認→取得→確認コピーをLLM／話者分離について確認。話者分離では取得1・コピー1・CPU0・適用0、コピー確認のチェック欄は再度未選択、各終了後に恒常的なモデル選択入口へfocus復帰。既知の取得結果だけで高度なJSON入口へ「要確認」を付ける表示も除いた。新しい実ファイル／実プロセスをこのbrowserから動かしたものではない。画像`/private/tmp/mon-two-model-catalog-browser-20261011.png`、agentタブは終了しユーザーのタブは保持した。

| 私的領域のログ | SHA-256 |
|---|---|
| mon-catalog-download-regression-20261011.log | 1a9419451427669d7705cf3efb8c1771503b094144cb110d5cc94545de166108 |
| mon-catalog-real-llm-diagnostic-20261011.log（失敗の証拠） | 32115d90babf6382c059249ca4fddb5544223dcda6300aeccb5a194eb4d68c67 |
| mon-catalog-real-llm-cpu-final-20261011.log | b1c6f352ba8d0df0c4e62f754cba400ab4a8ec6d864804ac193caf0cd3b4c7b9 |
| mon-catalog-related-final-20261011.log | 429a0e33f5f17c5100eb1ee6fa3e98bbc9d6c06b6e0ae193cc4c025d27432198 |
| mon-catalog-two-ui-final-20261011.log | aa4a56b901bb4b37de44fa9bf1e1fa32db24146367607cbf859c4398a81ff07f |
| mon-catalog-two-ui-final-build-20261011.log | bc201eec95204a2aeb179d1151eeb0cf9a3fdb4089f322069bc00ebb5141ee56 |

文字起こしの簡単導入にはMac用whisper-cliのアプリ側準備が必要。確認した公式v1.9.4／b5130の配布一覧にMac CLI TARはなく、XCFrameworkをCLIであるかのように指定しない。利用者にXcode／CMakeを導入させる案へ置き換えず、アプリ作成時の固定版ビルド／同梱を引き続き実装する。通常GUIの実TLS/CDN取得・全3種類導入、全ガイド経路、正式品質・速度／10話者、全Mac最低OS／GPU、実録音／4時間、実IME／VoiceOver、正式署名／公証・notice最終承認・試用等の監査残件を維持する。CI・コミット／Pushなし。

### 2種類の実素材での最終native試行

固定カタログの2定義をそのまま使い、既存原本をno-link Sourceで再照合しながらstageへ供給。ネットワークを使わず、gzip／bzip2の実アーカイブから必要ファイルだけを取り出した。取得済み素材の由来をdownload形式版2として管理コピーし、CPU試行→別の適用確認をtest-owned Storageで通過。標準側の1コンポーネントだけを適用し、「全3種類の準備完了」にはならないこと、高精度が不変・会議0件、通常本文ファイルの完全一致を確認。QA／本番のDB・設定への適用ではなく、TempDirの検証用コピーは終了時に回収する。

- LLM：stage8.496秒、管理コピー12.728秒、合成の固定JSON CPU試行16.049秒。その後の適用／保持条件も成功。
- 話者分離：stage2.457秒、管理コピー0.927秒、合成10秒無音のCPU試行1.831秒。その後の適用／保持条件も成功。実発話の認識や人数／重複の品質を証明しない。
- 実素材2件成功・0失敗、libtest50.50秒。構築314.94秒、runner実行58.17秒（cacheの終了後計測／回収も含む）。再生成可能な古いcache4484.5MiBを回収し、最後8366.4MiB、10GiB上限内。元モデルや音声は回収対象ではない。
- ログ`/private/tmp/mon-catalog-real-two-engines-final-20261011.log`、SHA-256 `d852f908cb2bfb25d3779bdc9294656f1bae1480ae5eca8e61dc543e147acef2`。
- 最終ソースの関連回帰も122件成功・0失敗・専用3件ignored。構築0.52秒／runner38.41秒（libtest38.40秒）、`mon-catalog-two-related-final-20261011.log`。上段の旧2件ignoredとは対象版が違う。専用2実素材成功と通常回帰は別実行であり、全Rustの結果に加算しない。

この成功は製品のnative関数と実エンジンを通した証拠だが、通常GUIによる実TLS取得や実会議処理、正式品質評価とは分ける。最終アプリの画面確認は後続に記録する。

### Mac通常GUIの実HTTPS取得と冷起動後の照合

最初のカタログQAは `/private/tmp/mon-model-catalog-qa.UtHVZo/MON QA.app`、main SHA `81afcf482eaac8436c9666f36a4c8144d2192947ba35c7bc5c03eae5a01cde80`、deep strict ad-hoc検査成功。Vite1.66秒／Cargo1分11秒、build log SHA `09363474a8167d73f5a8be9c9fa60e677b225603d9b245e24b3b6c5892309699`。通常画面から2選択肢とASR準備中の表示、LLMの確認取消、話者分離の確認→実HTTPS取得を確認。合成IPCへ差し替えていない。

話者分離の取得IDは `26310293-d601-4466-83e2-215e23b93379`。完了記録の定義SHAはカタログと一致し、payload75,117,137bytes、保持する圧縮原本25,210,612bytes。通信対象ファイル容量65,153,215bytesを追加し、既知の完了取得ファイル累計は2,177,378,076bytes（約2.03GiB）、許可4GiB以内。HTTPSのヘッダー／転送／メタデータをすべて含むwire trafficを別途測定した値ではない。終了後の独立読取りでもpayload全9件と圧縮原本2件の容量／SHAが固定定義に一致した。未配置stage約96MiBは再利用用に保持し、自動削除しない。

QAを通常の終了操作で止め、旧PID95497と同じ実行パスのプロセスが消えたことを読取り確認してから再起動。「完了記録あり（実体未照合）」が同じIDで保持され、自動再取得／コピー／実行／適用は起こらなかった。初回の明示的照合では、結果が取得記録モーダルの背後に隠れる導線を発見したため、その時点の画面確認を完了扱いにせず補修した。

### 取得記録の結果表示・戻り位置の補修

取得記録をcontrolled modalに変更し、完了した実体照合だけが記録を閉じて恒常的な「照合済み素材のコピーを確認」へfocusを戻す。モーダルを利用者が先に閉じた場合は後からfocusを奪わない。未完了と失敗は同じモーダル内で表示する。これは表示の遷移だけで、コピー・CPU・適用の呼出しを追加しない。

合成画面では完了→コピー入口focus、照合失敗→同じモーダル内の要対応／説明、未完了→保全メッセージとモーダル保持を確認。最後も合成取得1／コピー0／CPU0／適用0、未完了ではコピー入口なし、閉じると取得記録へfocus復帰。全browser driverの一括実行ではない。

最終QAは `/private/tmp/mon-catalog-record-qa.1r6MKT/MON QA.app`、main SHA `02d0d9cdea911783f561197cf5bfcbd8dd990f2abc5abd8d1bbd2b14244197d3`、deep strict ad-hoc検査成功。Vite1.49秒／Cargo1分10秒、bundle36.68MiB。冷起動したこのQAから同じ取得IDを再照合し、結果表示・コピー入口focusが成功。コピー確認はチェック0／実行ボタンdisabled、Escape取消で同じコピー入口へ戻った。追加取得・管理コピー・CPU・設定適用・会議解析はこのQAで行っていない。

最終フロント816件成功・0失敗・0skip、1.931697083秒、型検査／Vite build成功（1.50秒）。今回の表示補修はRustを変更せず、上段の最終関連122件／実素材専用2件を再加算・全Rust再実行と主張しない。fmt／diff空白検査成功。実装・調査・画面操作・記録の個別時間は未分離で、構築と検査の値を単純合算して経過時間にはしない。

| 私的領域の最終ログ | SHA-256 |
|---|---|
| mon-catalog-record-ui-final-20261011.log | ddce7a31cac86867b3e0732b8353e14478b34cd510f4c36831b72d0ec51db872 |
| mon-catalog-record-ui-final-build-20261011.log | c74052af7869b3bedc74daae12e6a6a0a57af3a2e76a91fe8d15d191e3a4e2b5 |
| mon-catalog-record-qa-build-20261011.log | 01445a9ec71871e0e3f61cee0122277881f4ee65b415642c513dee4233720a81 |

最終QA終了を同じ実行パスのプロセス消失で確認してから、readonly SQLで開始前後を比較。11会議／bootstrap SHA `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` は不変。今回はsqlite3既定list出力を同じqueryでhash化しており、上段の異なる出力形式の値とは比較しない。

- 全下書き・出力：`acce9d590c669a10c3638fcd404a906ce250389e1e44156f1af037102db71e95`
- 全解析／議事録版・会議head：`da1b16ab22ba1bc56612c314c75287f1c4edb315ef2c5e5bc1dae0962796dafe`
- 全設定・jobs：`2c7dd00ac1b7cf37e60596e4733c652179efad902e8b68d20b07b01eafdb6a20`
- 全用語関連：`8805da5d5a3469e2afe5237bdf6bfc97f090a0cf156ff45727391eea7c873d5e`

実機画像 `/private/tmp/mon-catalog-record-native-final-20261011.png`、SHA `eca0c17d1c89d11d735691fad8edaf9cbf95dd8d19908476ef0fadc0692009ce`。合成未完了画像は `mon-catalog-record-incomplete-synthetic-20261011.png`。agentのbrowserタブは終了し、ユーザーのタブは保持。LLMの診断だけに使った展開済み重複約31MiBを限定回収、原アーカイブは保持して再生成可能。元モデル／音声／会議は削除していない。

残る簡単導入は文字起こしCLIのアプリ側準備、全3種類の統合導線、LLMの通常GUI実TLS確認等。正式品質／速度・10話者、Windows、最低OS／GPU、実録音／4時間、全IME／VoiceOver、Developer ID／公証・notice最終承認・試用等の監査残件は維持。既存のSonner／chunk／Swift cache警告、12件notice不足を解消したとはしない。CI・コミット／Pushなし、目標全体は未完了。

### 文字起こし実行ファイルのビルド時同梱

利用者がCMake／XcodeやMac用CLIを別途準備する状態を解消するため、アプリ作成時の固定版ビルドを追加した。画面との接続は次の工程で、ASRの自動取得を完了にしない。`tools/mon_whisper_runtime.py`をMac before-buildから呼び、アプリResourcesへsealed TARとindexだけを同梱。モデル重み・開発ツール・ソース・ビルド中間物は含めず、起動時の自動展開／実行／適用も追加していない。

固定ソースは[whisper.cpp v1.9.4](https://github.com/ggml-org/whisper.cpp/releases/tag/v1.9.4)、commit `927cfce34f31707e17f2bff35c349632fb9e2c3a`。公式codeloadのgzipは9,357,755bytes、独立算出SHA `41b664fee09e79176ac277b5237debec34f8d74af3c7d71f333f1ec67989ecde`。既存原本を再利用し、追加取得0bytes、累計取得2,177,378,076bytesを維持。開いた同一handleで全量SHA→安全な通常ファイル展開→実体identity再確認を行い、リンク／不正パス／大小文字重複／10,000件超／128MiB超を拒否。初回ネットワーク経路にもHTTPS限定・秘密環境なし・親側のbyte／時間制限と所有子終了待ちを設け、Content-Lengthだけに依存しない。実外部通信をこの工程で再実行したものではない。

Release static、arm64、minimum macOS15.0、CPU armv8-a／native OFF、Metal／OpenMP／KleidiAI／動的backend／CoreML／OpenVINO OFF、Accelerate＋Apple BLAS ON。FetchContent切断、CLIだけを対象に構築。`lipo`／`otool -L`／`otool -l`／codesignでarm64単独、15.0、依存がlibSystem／libc++／Accelerateだけであることを確認した。開発用CMakeは既存のKitware公式4.1.2を絶対パスで指定、グローバル導入なし。正式GPU・最低OS実機の成功や品質合格ではない。

構築・封印31.654秒、TAR2,478,080bytes（SHA `4a5968c63b56993be5ffc263844e03833742290b46bfcb7fb92298bc74d5493c`）。内容はCLI2,470,960bytes（SHA `bb8a270033f83570737f5644dc08913f294b82d603d5ab88d12215c026a38a15`）、whisper MIT本文1,078bytes、構築元／条件JSON1,068bytes。index SHA `038839273676291a2e7472ef490461827f8dedf650bd08c99a021d66d613480b`。アドホックCLIをTARへ封印しており、外側アプリを署名しても内包CLIがDeveloper ID署名済みになるとは扱わない。正式配布準備のreportへ独立した未接続／署名・公証未検証の状態を追加し、欠落／改変なら署名／アップロード前に拒否する。モデル・エンジンの配布条件とcompiled dependenciesの最終監査は残る。

cacheは固定ソース＋封印物で11,564KiB（約11.3MiB）。構築source／中間物は新しい私的TempDirだけを終了時回収し、未知・改変cacheを自動上書き／削除しない。384MiB空きを構築前に確認。2回目は全封印ファイルの照合後に再コンパイル／追加通信なし、同じ成果物を保持した。Rust共有cache8320.9MiBは変更対象ではなく、今回の処理は同じRust test launcherを再構築しない。

最終Python151件中150成功・0失敗・専用native1skip、11.406秒。新runtime16件とrelease追加2件はこの合計に含まれる。合成TAR／改変／非対応source／容量・時間超過／取消対象子／保存先保護等の結果であり、公式モデルやGUIの成功へ拡張しない。専用nativeは別途実行し、開発用封印物のCLIと既存tiny／合成10秒無音で終了0（3.553秒）。本体のモデル原本は容量／mtime／SHAを保持。これはwhisper CLIのJSON出力smokeで、Rust worker／実会議／認識品質の統合評価ではない。

最終QA `/private/tmp/mon-asr-runtime-qa.3pwtP6/MON QA.app` は39.05MiB、main SHA `0d333c53702d1d9721af8a570d6f05a83cbc82935dc3c61afda77db4b18f8b9d`、deep strict ad-hoc署名検査成功。Vite1.53秒／Cargo1分10秒、before-buildで同じcache再利用・追加通信0。アプリ内TAR／indexが元封印物とbyte単位で一致し、全member／版／容量／SHAも再照合成功。この同梱TARから私的TempDirへ取り出した実CLIで、既存tiny＋合成10秒無音の専用native1件を再実行して成功（3.296秒）、終わりにTempDirを回収。通常appのGUI起動・モデル取得・managed copy・CPU記録・設定適用・会議DB変更・録音は今回実行していない。

| 私的領域のログ | SHA-256 |
|---|---|
| mon-whisper-runtime-prepare-20261011.log | 6be4e58c32bd8829d7c80bf77a64ceee08aa96d0f01f1adb389b0a00b03b6e69 |
| mon-whisper-runtime-python-final-20261011.log | ce49928cc80a8f7696d0545cccfa95cca66dc44be1332a1c88664f4b7784830b |
| mon-whisper-runtime-bundled-native-20261011.log | 8e2d17015dcf9bba6fc982115cb695b61f77bcd248e04fe7e8519b8ad99ce854 |
| mon-whisper-runtime-qa-build-20261011.log | 9daae454c6973c56a5b23fab8cd4643f3c3c2133cdea289d697e772bb0780634 |
| mon-whisper-runtime-source-proof-20261011.log | bc4ce9dd405d815f2e9c4fc682476988bda43c2afe5156749b0891cfc2167c73 |

既存sourceをcacheへコピーする処理も容量＋1byteで親側制限し、JSONのtrue／falseを整数1／0と取り違えない確認へ補強後に再実行。最新はPython151件中150成功・0失敗・専用1skip（11.116秒）、同梱CLI専用1件成功（3.519秒）。上段の11.406／3.296秒はこの最後の再実行前の履歴で、ログSHA表は最新を示す。条件・TAR／indexの内容とQA本体は不変で、追加取得／QA再構築は不要だった。

Python構文・diff検査成功。before-buildの型検査／frontend buildは成功したが、画面ソースを変更していないため前段の816件をこの工程で再実行したとは主張しない。既存のSonner／chunk／Swift cache警告、notice12件不足・正式署名／公証は残る。実装／調査／記録の個別時間は未分離。次は同梱素材とモデル取得の安全な混在定義・由来・明示操作をつなぎ、通常GUIの全3種類導入を検証する。正式評価／Windows、録音等の保留／監査残件とCI・コミット／Push停止は維持、目標全体は未完了。
