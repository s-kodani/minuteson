# 文字起こしの同梱実行ファイル＋HTTPSモデル準備

2026-10-11。M4-02の残る文字起こし導入を進める変更。正式評価／Windowsは保留、CI・コミット・Pushなし。既存のQA会議・設定へSQLを書き込まない。

## 接続と境界

- 固定ビルド定義を `app/model-catalog/whisper-runtime-build.json` に置き、Python builderとの一致を通常テストで検査する。Rustは同梱indexの必須フィールド／型／未知フィールド／版／OS／全CPU条件／容量・SHA／3memberを検査する。indexを読むだけでCLIやモデルを実行しない。
- モデル選択のネイティブ応答に文字起こしを追加。実行ファイルのSHAは各アプリの同梱indexから導出するため、固定ネットカタログへ偽の共通binary SHAを置かない。LLMと話者分離の既存定義・SHAは変更しない。
- 新しい形式版3は専用の同梱IDとHTTPSのASRモデルを必要とする。形式版1／2で同梱IDを使わず、未知ID／file URL／任意のresource pathを読み取らない。通常の手動取り込みとは由来を区別する。
- native reviewと取得は同じ固定ネット定義＋このアプリのindexから構成を再生成し、全recipeが確認した内容と一致する必要がある。取得の共有lease内で同梱TAR全体の事前SHAを確認し、indexとTARのno-link handle・identityを保持、有限時間／容量／取消付きでcreate-newの私的TARへコピーする。既存libarchiveの選択member検査と全payload検査・同期・完了記録へ接続。実行／設定適用はしない。
- 通信容量は同梱TARを除外する。UIの確認詳細には同梱分とネット取得を区別し、CPU版・QA署名／正式配布未検証を明記する。実体／管理コピー／TARの共存容量は従来どおり見積もる。
- 履歴の検査はportable recipeのSHAへ結び付けるだけで現在のresourceを要求しない。backup／restore／再backupの合成履歴検査を追加し、記録からCLIを起動・配置・使用設定へ適用しない。

## 固定配布情報の確認

[whisper.cpp公式取得script](https://github.com/ggml-org/whisper.cpp/blob/927cfce34f31707e17f2bff35c349632fb9e2c3a/models/download-ggml-model.sh)が参照するメンテナーのGGML配布から多言語smallを選ぶ。正式品質・性能の選定ではなく、tiny smokeを通常品質の証拠としない。モデルの版は固定revision `5359861c739e955e79d9a303bcbc70fb988958b1`。

| 素材 | 固定容量 | SHA-256の由来 |
|---|---:|---|
| ggml-small.bin | 487,601,967 | HF固定revisionのAPI LFS oid `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b`。初期情報確認時点は実体未照合、後述のMac取得後に全バイトの独立算出も一致 |
| [モデルカード](https://huggingface.co/ggerganov/whisper.cpp/blob/5359861c739e955e79d9a303bcbc70fb988958b1/README.md) | 3,196 | 固定本文をHTTPS転送後に独立算出 `21fd967098804f33fc84e803fb0e5ab7666d71801f4027cf28a65e7af09c1758`。カードのMIT申告は正式な再配布審査ではない |
| [OpenAI MIT本文](https://github.com/openai/whisper/blob/31243bad24cc746f07d4c8bfdd2d974872cb1803/LICENSE) | 1,063 | 公式タグv20250625が指す固定commitをAPIで確認し、本文を独立算出 `b5d65a59060e68c4ff940e1eddfa6f94b2d68fdf58ed7f4dd57721c997e35e9d` |

計487,606,226bytes。SHAは配布元の署名認証／モデル品質／再配布権の審査ではない。初期情報確認段階では大容量モデルを新規取得していなかった（配布情報の読み取り通信は実施）。後述のMac GUIで実際にモデル＋notice全3件を取得し、従来の完成ファイル取得合計2,177,378,076bytesとの合計は2,664,984,302bytes（約2.48GiB）で、承認の4GiB以内。これは測定した全wire通信量の集計ではない。

## 途中の検証と残対応

画面側818件成功／0失敗／0skip、1.910798084秒。Python152件中151成功・専用1skip、12.993秒。これらは後続fixture／release出力修正前の途中結果で、最終ソースの合格と混同しない。フロント型検査・Vite build（1.60秒）成功。既存Sonner use-clientとchunk warningは残る。

ブラウザー模擬IPCで3選択肢、文字起こし確認の約0.45GiB通信／約0.93GiB空き、詳細の同梱通信除外、確認前ボタンdisabled、確認後の合成取得1／コピー0／CPU0／適用0、完了結果とコピー入口を確認。模擬runtime SHA・byte数であり実TAR・ネット取得・CPUの証拠ではない。検証画像 `/private/tmp/mon-asr-onboarding-browser-20261011.png`。

Rustの最終関連回帰、最新QAでの実HTTPS取得・冷起動後の記録再照合、同梱＋smallの実展開→管理コピー→CPU→test-owned適用は未検証。元の23項目の完了監査を狭めず、全UI／全3種の通常導入／録音・4時間／正式署名・公証・notice等の残件を維持する。

### 最終自動回帰

最後の関連Rustは130成功／0失敗／4専用ignored、libtest39.45秒。構築432.49秒／runner実行47.32秒、終了0。形式版3の実体照合・通信容量の同梱除外・管理コピーの由来、同梱indexの型／版／CPU条件／リンク／改変拒否、全3選択肢と旧2定義SHA保持、バックアップ→復元→再バックアップの履歴保持、既存モデル検査・配置・CPU・使用設定・llama回帰を含む。全Rust通常／ignored実モデル／Mac GUIを実行したという主張ではない。初回38成功・8失敗・2ignoredは合成TCP listenerのsandbox PermissionDeniedで終了101。検査を省略せず許可された環境で最終回帰を再実行した。初回の構築441.62秒／実行10.97秒は最終と区別する。

最終フロント818成功／0失敗／0skip、1.908517542秒。Python152中151成功・専用1skip、11.516秒。型検査／最終Vite1.65秒、format／diff check成功。Rust cacheは8367.7MiB（約8.17GiB）、上限10GiB。最終runnerは自分が管理する再生成可能な古いcache4591.1MiBを回収、モデル原本・会議・QA取得記録は対象外。

| 最終ログ（`/private/tmp/`） | SHA-256 |
|---|---|
| mon-asr-onboarding-rust-final-20261011.log | 517e2ee963a78f6e538b9951daba350ade1f884ade0803e2212e97a5f25ba047 |
| mon-asr-onboarding-frontend-final-20261011.log | 9d465c2eeb0636c0293a90878cd5ca1c6fca5bdadcdd8a1db376405f6f33838a |
| mon-asr-onboarding-python-final-20261011.log | 326a048f6054da96677336d9318260047bfd4c18be11552dfd52c969d7740d24 |
| mon-asr-onboarding-ui-final-build-20261011.log | 830e1c2fecf3dd74b832bd1f8fb9239a89ab89664c867245bec5521ef2e5bddd |

上の「Rustの最終関連回帰は未検証」は途中時点。実機経路・small実CPUも後述の限定範囲で確認した。模擬画面画像SHA `32f9cdd55a725888ce9e592767ee39625293d4f82c3ca8fca28ce5d8c48af46c`。QA起動前の既存11会議・下書き／出力・版／head・設定／jobs・用語4組とbootstrapのSHAは前回と一致（SQL read-onlyのみ）。

## Mac実取得・冷起動と専用実CPU

実機用の新QAは `/private/tmp/mon-asr-onboarding-qa.Do6mYe/MON QA.app`。39.21MiB、main SHA `878f9cd4f783a31abdc5a846ba6c2378d35e7fdd95ee72b2a8dffe3d9c72423f`、deep strictアドホック照合成功。Vite1.55秒／Cargo1分10秒。通常の設定画面から文字起こし・議事録生成・話者分離の3選択肢を確認し、文字起こしの確認画面でチェック前disabled／固定容量・配布元・同梱物・未実行の境界を確認した。

明示確認後の実HTTPS取得が成功。取得ID `fba38c93-32bb-452a-9f54-1a429961e884`、recipe SHA `25fb94331716ab82ae54ecbd1b06d08356482138d5c04225842f7b0aa9d8ab3b`、完了review SHA `046f6240b17ed6a4a73fbaac8cfd52efa48aa84524b15a72b44abdb386c67006`。payload6件合計490,079,332bytes、ネット素材487,606,226bytes、アプリ同梱TAR2,478,080bytes。保持領域は約485MiBで、再取得せず再利用する。開始観測UTC04:45:40、完了は04:46:49以前に確認したが、通信の正確な完了時刻・正式速度は測定していない。

取得意図JSONのcanonical digestと完了記録の結び付きを独立確認。全6件の実容量・SHAを別の読み取り計算でも照合した。

| payload | bytes | SHA-256 |
|---|---:|---|
| bin/whisper-cli | 2,470,960 | bb8a270033f83570737f5644dc08913f294b82d603d5ab88d12215c026a38a15 |
| notices/whisper-MIT.txt | 1,078 | 94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d |
| build-info.json | 1,068 | f817b6062e57cacb8e86fc75edc16d9a15ee17a2df93e73fc87a1502b672582f |
| models/ggml-small.bin | 487,601,967 | 1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b |
| notices/whisper-model-card.md | 3,196 | 21fd967098804f33fc84e803fb0e5ab7666d71801f4027cf28a65e7af09c1758 |
| notices/openai-whisper-MIT.txt | 1,063 | b5d65a59060e68c4ff940e1eddfa6f94b2d68fdf58ed7f4dd57721c997e35e9d |

終了操作後に最初のPID14123が消えたことを読み取り確認し、同一artifactを再起動。取得記録は「完了記録あり（実体未照合）」で始まり、明示的な実体照合で成功しコピー確認入口へfocus復帰した。再取得・管理コピー・CPU起動・使用設定への適用はQA内で行っていない。既存QAの設定は守る。実機選択画面画像 `/private/tmp/mon-asr-onboarding-native-catalog-20261011.png`、SHA `5f8ffb3bd8abfbe74232cb7cc7024f8cac6fa2c57659f5d02a5afe533a16e8a9`。

専用ignored試験 `real_whisper_hybrid_catalog_copy_cpu_and_test_owned_apply_with_reused_assets` を明示実行し、1成功／0失敗、libtest20.72秒。QAで取得した実素材を読み取り再利用し、テスト自身が作ったTempDir・Storageでstage3.403秒→管理コピー5.051秒→合成10秒無音のCPU試行8.945秒→標準プロファイル1構成の適用を確認した。高精度は不変、全3構成は未完で、会議0。原本を変更せず、一時領域は試験終了時に回収。追加モデル通信はしない。構築56.93秒／runner28.14秒、cache8355.0MiB。再生成可能なcache4128.4MiBのみ回収した。この1成功を通常130成功や専用ignored4件に加算せず、通常GUIでコピー・CPU・適用した証拠や会議精度・長時間の代用にしない。

専用ログ `/private/tmp/mon-asr-onboarding-real-cpu-20261011.log` SHA `e363b8656ae06422f0a46a8e28942ceb70d6dca13df34aed6613ccbf8a9f825f`。QA buildログ `/private/tmp/mon-asr-onboarding-qa-build-20261011.log` SHA `b169fbd210699fd9e0636481f0cd7f651a6e64b848e260f2494683f2abb4367b`。

残るモデル導入gateは通常GUIの全3種類コピー・CPU確認・明示適用、LLMの通常GUI実TLS取得、正式配布の内包CLI署名／公証・利用条件等。録音／4時間・実IME／VoiceOver・その他23項目の残件、正式評価／Windows保留は維持する。冷起動照合後にfocusしたコピー入口が画面下に隠れるUIも発見したため、次の補修・実機確認に進める。計画全体やM4-02を完了にしない。CI・コミット／Pushなし。

### 照合後の次操作を画面内へ戻す補修

共有モーダルの通常キャンセルではscrollを変えない契約を維持し、取得記録を利用者が明示的に照合して成功した場合だけ、復帰したコピー確認入口を`nearest`で表示する。閉じた後のmicrotaskで同じ入口にfocusがある／記録画面が閉じていることを再確認し、別画面や別操作のfocusを奪わない。コピー・実行・適用は開始しない。ソース契約の回帰1件を追加。

最終フロント819成功／0失敗／0skip、2.543322秒。型検査・Vite1.56秒、QAのVite1.50秒／Cargo1分08秒成功。Rustはこの補修で変更せず、上記130成功と専用1成功が現行Rustの証拠。最終QA `/private/tmp/mon-asr-onboarding-focus-qa.fJXQrW/MON QA.app`、39.21MiB、main SHA `f17d778d66a2e5138af478553d296b6825196bced708300427e7ac7b8192d33f`、deep strictアドホック確認成功。ソナー／chunk・既知の配布準備warningは残り、正式署名／公証を示さない。

前QAのPID14637終了を読み取り確認後に新QAを冷起動。既存取得記録は実体未照合から始まり、small選択→全ファイル照合→記録モーダル終了→コピー入口focus・画面内表示を確認。コピー確認はチェック0／実行disabled／キャンセル初期focus、Escapeで画面内の入口focusへ復帰。実コピー・CPU・適用をQAでは行わず、再取得もなし。確認画像 `/private/tmp/mon-asr-onboarding-native-action-visible-20261011.png` SHA `828f5ee9bb946b452248734b9c8666d38a8a55455b02c2e317e6c53a52b21141`。新QAも終了しPID16447の消失を確認した。

終了後の読み取りSQLの4組は開始前と一致。下書き／出力 `acce9d590c669a10c3638fcd404a906ce250389e1e44156f1af037102db71e95`、版／head `da1b16ab22ba1bc56612c314c75287f1c4edb315ef2c5e5bc1dae0962796dafe`、設定／jobs `2c7dd00ac1b7cf37e60596e4733c652179efad902e8b68d20b07b01eafdb6a20`、用語 `8805da5d5a3469e2afe5237bdf6bfc97f090a0cf156ff45727391eea7c873d5e`。bootstrap `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` と会議11件も不変。SQL書き込みなし。取得モデル原本・既存cacheを削除せず、CI・コミット／Pushなし。

| 補修の最終ログ（`/private/tmp/`） | SHA-256 |
|---|---|
| mon-asr-onboarding-focus-tests-20261011.log | f02a9fb8fb3aa08fdf6c5da961e04a260d23c1060d8094470a4f8835fe0bb1f2 |
| mon-asr-onboarding-focus-build-20261011.log | 544630267f65c7318003ce594019ea6ae88dabb693fa7296ca01482389c9754c |
| mon-asr-onboarding-focus-qa-build-20261011.log | 6b65354e51b54b6b13b9b8124d9f7a12717913c8065e2d91c7d772f45aad5280 |

今回の新規UI補修は限定順序だけで、全UIや全3種類の通常導入は未完。上の画面外focus残件は補修前の観測。モデル導入以外を含む完了監査の残件は縮めない。

## 3種類の通常GUIコピー・CPU確認・標準適用

既存の11会議と使用設定へ触れず導入を最後まで試すため、通常コードを別アプリID `jp.mon.desktop.qa.onboarding20261011`、productName `MON Onboarding QA` で構築した。追加configは `/private/tmp/mon-onboarding-qa-20261011.conf.json` のID・名前だけで、元のQA configと同じ画面／機能・IPC・固定定義を使う。会議／使用設定のSQL投入や試験専用コマンドは追加しない。初期native画面で会議なし・管理モデル未選択を確認。保存先はこのIDのapp-data/data、modelsも別領域。

第一artifact `/private/tmp/mon-full-onboarding-qa.r6lcV7/MON Onboarding QA.app` は39.21MiB、Vite1.47秒／Cargo1分07秒、main SHA `0d74de0409c9074c7456f8d44494cd72fdab927bc761c7efc06160a9b2aa6b85`、deep strictアドホック照合成功。ビルドログ `/private/tmp/mon-model-full-onboarding-qa-build-20261011.log` SHA `0b623ee54ee539311bb6ee436cd11f576cf7525605d618b057079077fa8a4233`。

WhisperとSherpaは前回の実HTTPS取得領域をAPFS copy-on-writeコピーで再利用した。両フォルダーの全ファイル内容一致とcanonical recipe／completion対応を確認し、native画面では未照合一覧から明示的な全実体照合を行った。新規ダウンロードしたことにはしない。再取得・既存モデル／QA設定変更を避けるためのテスト用fixtureであり、両素材の実HTTPS取得自体の証拠は前回QAである。

LLMだけ、この別QAの通常3選択肢から固定定義を選び、チェック前disabled→配布元／全構成の確認→明示取得を行った。実TLSで完了し、ID `51a81851-f01f-483c-aa47-b25bccfdcfb1`、recipe SHA `e4aaf7c71da23a1cadc141d92208d6eac4274eb12fa1901383f7753234f210ba`、review SHA `f096ae1b70729aa993d1ebaac566c7737c60516b6450026f90e9524063f7a2fb`、manifestに宣言したpayload13件（manifest自体は別）1,136,172,053bytes。ネット取得は1,129,407,950bytes（TAR12,075,871＋GGUF1,117,320,736＋Apache本文11,343）。SHAはnative検査に加え、全payloadの独立stream計算とTARの容量・全SHAでも一致した。TAR SHA `1bc6e1ed668d8199705468aeea13d57542fa8892f409d5a640d1f6f2701193ed`。全payload計算ログ `/private/tmp/mon-full-onboarding-llm-payload-check-20261011.log` SHA `de362428f7312b88d5c98fed97c9443f33e352c7c19edf289346dbd9ba936fdb`。

既知の完成ファイル取得合計は3,794,392,252bytes（約3.53GiB）。承認4GiB以内で、これはmetadata／HTTP overhead等も含めた全wire実測ではない。Whisper／Sherpa再取得なし。

各素材を通常GUIで新規の管理領域へ確認コピーし、CPU実行の確認モーダルで明示起動、成功後だけ標準への適用内容を確認・適用した。高精度への適用は行わず、数値条件は各エンジンの既定4スレッド・GPU無効等を維持した。会議・ジョブは0で、実会議やマイク録音は使わない。

| engine | 新しい配置ID | 新しいCPU確認ID | native報告elapsedMs |
|---|---|---|---:|
| llama.cpp | 107280ab-a410-4c90-b420-9e32fc157f5c | 59524cc7-5931-4cee-be47-9a035e3e7e83 | 19,850 |
| whisper.cpp | dff30dfb-9de4-4398-bd9d-53bd97f97d8b | a9155ad4-ae2c-42be-bd3e-daed1e0e199a | 8,243 |
| sherpa-onnx | b6a89105-e11a-4a65-b3e7-1866f5dfd7eb | 6f1efd6e-8c99-4f5b-9d1b-88e72a8a296e | 1,781 |

標準構成版は#0→#1（LLM）→#2（Whisper）→#3（Sherpa）。native画面で3件すべて保存設定／CPU成功記録一致・不足なし、高精度は構成版なし・管理モデル未選択を確認した。画像 `/private/tmp/mon-full-onboarding-profile-20261011.png` SHA `3400b5dee635c2f2a33a17ccb44fdcfb145f243a70da82d8777f5ae561fc3234`。外部SQL書き込みでこの状態を作っていない。

第一QAの終了とPID18786消失を確認後、test-owned DB＋WALを私的APFS snapshotへコピーし、SQL read-onlyで3チェックsucceeded・標準#3 ready・高精度head null・会議0／job0を独立確認した。native app-dataでの直接read-only SQLiteはSHM作成がsandboxに拒否されたため、実DBへ書込権限を拡大せず停止後snapshotだけを読み取った。途中のapp_settings列名`value`誤りは`value_json`へ修正して再確認した。状態hash（app_settings／profiles／components／checksのソートSELECT）は `930443a4df65c963311152d73dc4d33cdd78a9945c47d21263d446e1eb3854c4`。snapshotは `/private/tmp/mon-onboarding-db-audit.lXGcW3`、SQL記録 `/private/tmp/mon-full-onboarding-db-before-restart-20261011.log`。

画面の旧「取得定義からのみ」「実モデル未検証」「管理構成とjob参照の統合は開発中」という説明も現在の実装に合わせた。一般利用は3選択肢でJSON不要、コピー／CPU／適用は別確認、既存job条件を変えないこと、正式品質・Windows・内包CLI署名／公証の未検証を明記。UI説明の回帰を追加し、フロント820成功／0失敗／0skip、2.279394125秒、型検査・Vite1.58秒成功。後続の最新artifact冷起動で保持と新説明を確認する。Rust／Pythonはこの説明補修で変更していない。

この段階で3種類の通常GUIコピー・CPU・標準適用が通過。冷起動後の最新確認は以下に記録する。正式品質・標準／高精度の品質選定・全端末・GPU・録音／4時間・正式配布・全UI／IME／VoiceOver等の23項目残件は維持する。

## 冷起動保持と配置済み取得の案内

独立QAの冷起動で標準#3の全3構成とCPU成功記録の一致、高精度未選択を確認した。LLMの取得記録を明示選択し全13payloadの再照合も成功。第一QA終了後のsnapshotと冷起動終了後snapshot `/private/tmp/mon-onboarding-cold-db-audit.ACyxPS` で設定／profiles／components／checksの状態SHA `930443a4df65c963311152d73dc4d33cdd78a9945c47d21263d446e1eb3854c4` が一致した。既存取得原本2種類も比較で不変。

この確認時にコピー済み取得まで「未配置・未実行」と表示される通常読戻しの文言を発見し修正。取得ID・recipeHash・reviewHash・placementRecorded・由来downloadの一致を条件に、照合素材の表示とコピー記録の表示を分離する。一致する記録がある場合は再コピー確認ではなく「配置・構成記録を開く」へ案内し、現在の本体／CPU／適用はその画面で確認する。歴史的コピー記録だけを現在の本体存在や実行準備の保証にしない。コピー開始側にも同じguardを追加。応答未確認操作の停止／結果確認は元の操作IDを保つ従来経路であり、今回の実機確認の対象ではない。

最新artifact `/private/tmp/mon-full-onboarding-copy-record-qa.JM1nnf/MON Onboarding QA.app`、main SHA `c4587f400eba3f8fd32a6f4d292aa3d783a05480b999673f3c65bc0961fef810`、deep strictアドホック検査成功。QA構築Cargo1分13秒、構築ログSHA `5c8d9a66a4f24c4863a36d44226a8de7b7e4cc91cedf67a8a2b026140410240f`。cold launch→LLM取得全実体読戻し→画面内の記録ボタンfocus→クリックで通常構成記録へ遷移を確認し、新しい取得・コピー・CPU・適用を開始しなかった。標準#3の3エンジン一致・高精度未選択を再確認し、Escapeで配置・構成記録入口へ復帰。画像 `/private/tmp/mon-full-onboarding-copy-record-ui-20261011.png` SHA `2c4a0e294e06de7d368123ce972c6b296f3cb907f19b37386292e20f230ed37f`。

PID30978の通常終了・消失を確認後、DB＋WALだけを `/private/tmp/mon-onboarding-commit-db-audit.HoHJq5` へcloneしsnapshotをread-onlyで照合。実DBへのSQL書込なし。状態SHAは上記と一致、CPU成功3件、会議0／job0。集計時の存在しないengine列参照はstate集計へ修正して確認した。既存11会議QAも下書き／出力・版／head・設定／jobs・用語の4組SHAとbootstrapが以前の値に一致した。

コミット前の最新フロント821成功／0失敗／0skip、1.762131042秒（ログSHA `37928deefe9c5b9af1909cc5394ef32f0f58edfb6165862fbfac77bcc7e49353`）。Python152件中151成功・専用1skip、10.915秒。型検査／Vite2.15秒成功（ログSHA `981f5dcc91f28304a25fb613ec3ed4921db90ef1f91413501aa6d01190bdffb0`）。既存Sonner use-client／chunk・QA公証未設定warningは残る。Rustは最新関連130成功／専用4ignored・専用実素材1成功以降変更していない。ユーザーの指示でUI一区切りごとのコミット／Pushを再開し、CIは引き続き実行しない。一般配布・全品質・全UI・録音／4時間・元の全23タスクを完了扱いにしない。
