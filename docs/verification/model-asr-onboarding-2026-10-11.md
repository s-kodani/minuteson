# 文字起こしの同梱実行ファイル＋HTTPSモデル準備

2026-10-11。M4-02の残る文字起こし導入を進める変更。正式評価／Windowsは保留、CIは実行しない。後続のユーザー指示でUI一区切りごとのコミット／Pushを再開。以下のコミット停止は先行時点の履歴。既存のQA会議・設定へSQLを書き込まない。

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

## ガイド導入モデルから編集・議事録・出力までの実機統合

前節の全変更は `ad391d887ccef742a00301b6ae8cef9a314589f9` としてmainへコミット／Push済み。今回、同じ独立QAに通常GUIからテスト会議1件を新規作成し、導入した標準3モデルの実利用を確認した。音声は以前作った単一話者TTSの読取りclone（29.610375秒、16kHz mono、473,766frames）で、マイク／PC録音はしない。素材SHA `ccc3316f158ff9691267f176c32039f51eb49d5faf79b21908f43e6f328bfa93`、会議ID `18de4837-b6e1-466b-86a6-ba145b3caed4`。追加ネット取得0、既知の完成ファイル累計約3.53GiBを維持した。

### 実ASRで発見した最終時刻の失敗と修正

最初のjob `c1a57a39-161d-4d08-9191-8553dcb8fb2f` はCLI終了0・8発言のJSONを得たが、製品parserが最終end 29,980,000usを実入力29,610,375us＋100msより先として拒否し、0/1・failedを保存した。CLI total4052.27msとjob8982msは別計測。原JSON SHA `61424439e48b48986626f17886e8fe39db80bc7e92af9f95fe68e981323c0499`、stderr SHA `c39d9b5f02422bab890b80eec79383c77e84df744156dbe31e468051035d607c`。失敗job・原JSON・PCM・ログは削除／上書きせず保持。

[固定whisper.cppソース](https://github.com/ggml-org/whisper.cpp/blob/927cfce34f31707e17f2bff35c349632fb9e2c3a/src/whisper.cpp)の7745–7838行とCLI JSON writerを読取り確認すると、decoded timestamp／seek_deltaを区間endへ出し、通常結果のseek_endへのclampがない。実結果が推論窓のpaddingへ出たという判断は、この実測とソースからの推論である。既存の固定ソースarchive SHA `41b664fee09e79176ac277b5237debec34f8d74af3c7d71f333f1ec67989ecde` を再照合して読んだだけで再取得しない。

`transcription/whisper.rs` は最後のsegmentだけ、startが実入力内・正の区間・decoded spanが1推論窓30秒以内の場合に実入力endとの交差を取る。前の区間への100ms許容は維持し、順序逆転／実入力外start／30秒超の窓／整数overflow等は拒否。開始時刻・認識本文・元JSONを変更せず、gapを埋める音声を作らない。実測8発言fixtureの最初の7区間一致・最終end clip・原JSON／再parse一致と、不正境界の通常回帰2件を追加した。復元の不正fixtureも、今は許容する最後の1秒paddingではなく30秒超のdecoded spanと実入力外startで検査する。

### 通常GUIの再試行から冷起動まで

修正後QA `/private/tmp/mon-onboarding-e2e-qa.e82opj/MON Onboarding QA.app`、main SHA `21ec4cd726f110b40268096944ace84894c8f58446f4fc8459de461879ce6d17`。Vite1.59秒／Cargo1分23秒、deep strictアドホック確認成功（正式署名／公証ではない）。失敗QAのPID33523消失後に起動し、GUIの「同じ条件で再試行」→確認から再実行した。

| 実job | 保存結果 | job elapsed |
|---|---|---:|
| ASR `52c40efd-9aab-4560-9d11-ac17e2180fb2` | succeeded 1/1・attempt2・元failed jobへretry由来、候補#1・8発言 | 14,564ms |
| Sherpa `22596325-46ef-4489-8714-845adac66a99` | succeeded 1/1・候補#3・8発言・推定1話者 | 6,763ms |
| LLM `858678dc-dff2-4b03-83b7-b9279908e950` | succeeded 1/1・議事録候補#1・8項目 | 61,949ms |

候補#3を内容確認後に明示採用。本文先頭へ`【QA編集確認】`、話者名へ`QA読み上げ`を通常GUIで保存し、議事録生成を開始した。LLMの固定入力解析版 `35d56feb-2515-4b6b-ada3-e6755cf9f10d` と根拠プレビューで編集本文・認識原文の区別・話者名・時刻を確認。議事録 `8054c0e5-cd4f-4869-93f5-a88b76c55602` を明示採用し、日付欄へ`（QA保存確認）`を追記して保存。OS保存画面から新規 `/private/tmp/mon-onboarding-e2e.4hj4gQ/onboarding-e2e-minutes.md` へ出力した。2,986bytes・SHA `ea4eff608194af1b983fcda8beceaf30659bc374e2f4dadaea2e36ca348c1c0a` がexport recordの成功／保存版／実ファイルと一致し、脚注8件と編集された固定引用を保持。

通常終了PID34960消失→同じartifactの冷起動→本文・話者名・議事録の修正と保存済み表示保持→通常終了PID36551消失を確認。停止後DB＋WALの私的clone `/private/tmp/mon-onboarding-e2e-final-db.hYK5xv` と `/private/tmp/mon-onboarding-e2e-restart-db.5dQ0na` のread-only照合で、会議／下書き／解析版／議事録版／jobs／exportsソート内容SHA `bb1b6883ecb5c4720ede189e011e68c6be4d6a822fd54e197d3b4e4ddb01228a` が一致。設定／model profiles／components／CPU checksは導入時SHA `930443a4df65c963311152d73dc4d33cdd78a9945c47d21263d446e1eb3854c4` のまま。integrity_checkはok、foreign_key_checkは0行。外部SQL書込み・試験専用IPC・強制終了なし。

既存11会議QAの保護対象4組とbootstrapも前節のSHAと不変。今回の変更は別namespaceのテスト会議だけ。再起動保持画像 `/private/tmp/mon-onboarding-e2e-restart-20261011.png` SHA `c03b0f1f18e9e9c2322df8794806d2b5f73566b7b39c534b52ed9a3369dcc389`、出力直後画像 `/private/tmp/mon-onboarding-e2e-minutes-20261011.png` SHA `bc1eaf76ea931e45e9f4e32a5f98244ad7028530b54540df7e395c79e348a465`。

限定範囲の実機統合成功であり、単一TTSは3／10話者・長時間・精度・録音の代替ではない。今回のLLM結果は全8項目を概要へ分類し、ToDo／決定／担当期限の抽出品質は合格としない。生成中も「編集の保存を確認中」と出る表示は処理中表示との区別が残る。初回取り込み直後に会議操作モーダルのAX本文が消える事象も観測し、Escapeで復帰、再起動後の再試行／候補／採用は列挙できたが、原因解決や全VoiceOver合格としない。全23タスク・配布・実録音の残件、正式評価／Windows保留を維持する。

### この修正の検証時間

実装・調査の記録区間UTC05:36:42〜05:41:19は4分37秒（復元fixture追修正・文書整理は別、累積実装工数ではない）。QA構築は上記1分23秒＋Vite1.59秒。初回関連Rust476件は461成功・1失敗・14専用ignored、libtest271.96秒／構築98.06秒／runner execution279.53秒。復元fixtureが許容paddingを不正と期待した失敗で、endを31秒へ変えた第一追修正でもfixtureのfrom2.5秒との差28.5秒が許容内となり、再実行も461成功・1失敗・14ignored（libtest276.18秒／構築382.04秒／runner284.02秒）。この追修正の誤りを保持し、最終fixtureは実from＋30,001msのspan超過と実入力外startを検査する。製品parserは最初の修正から変えていないため、最後はWhisperと音声復元の全境界を再確認。15成功・0失敗・専用1ignored、libtest14.20秒／構築381.55秒／runner22.03秒、終了0。先の成功461件と重複加算せず、この15件だけを通常全Rustの合格としない。画面側は今回も821成功・0失敗・0skip、1.916381417秒。format／diff check成功。CIは実行しない。

今回のRust構築3回の小計861.65秒、runner execution小計585.58秒（うちlibtest562.34秒）。実機確認と並行しており、これらの合計をターンの壁時計時間や実装工数としない。最後のrunnerは管理する再生成可能cache4342.1MiBだけを回収し、保持8356.4MiB・終了後上限10240MiB。ビルド中の一時領域まで10GiBに制限する設定ではない。会議・モデル・QA原本の削除なし。

| ログ（`/private/tmp/`） | SHA-256 |
|---|---|
| mon-onboarding-e2e-qa-build-20261011.log | fc3b023cf14214310f5cd48a5d38cea98297e02dce0185e5afac1184be034515 |
| mon-onboarding-e2e-rust-20261011.log | 13b3bef339b58200aefe0f6cec639b6b79500d1d9f2773a53dd41075e7fb890a |
| mon-onboarding-e2e-rust-retest-20261011.log | 75c1548fbe03802ea42ee9b57f64f167d5011f6ce7f8c572cd2225c18ecec7b2 |
| mon-onboarding-e2e-boundaries-final-20261011.log | 10889cc50edb8740e818e8fa5137617a186de5a489a2684f9f3dfd1154d7a245 |
| mon-onboarding-e2e-frontend-20261011.log | 451747c7734e0f8d0e0905057fd21187faef5d86c1abb7e39b16b22477d37b97 |

## 保存待ちと操作中の表示を分離

末尾時刻修正は `757d327cced167ace941d065504e12674f926c61` でmainへPush済み。今回UIだけを変更し、保存barrierが待機する間は「編集の保存を確認中」、後続処理の間は「操作を実行中」と表示する。`runAfterSaving`のfinallyでidleへ戻し、保存失敗時には操作を開始しない。既存の重複操作ref／busy／失敗時の編集保持guardを維持。待機2段階・保存失敗・操作失敗・同期遷移・画面接続の5回帰を追加した。

QA `/private/tmp/mon-saved-action-qa.RMzb5H/MON Onboarding QA.app`（main SHA `6985a6bc8dfd19df5aa89d1932a312fc74c9c80fddca94ae6f50479d77a39e5d`）を通常起動し、既存テスト会議の議事録再生成をGUIで開始。生成中に設定をEscapeで閉じ、「操作を実行中」と本文の「保存済み」、無効の編集／出力／タブ操作をnative AXで確認した。画像 `/private/tmp/mon-saved-action-running-20261011.png` SHA `917643d1164fda383bf1fe1fe48b13d40f525fe7653e0618a92638eb87b62013`。完了後は処理中表示が消え操作が有効に戻り、現在版#1／編集ありを保持。job `90cfb680-0e24-4ddf-9ab6-6bb232255f3f` はsucceeded 1/1、50,146ms、新候補#2 `689e7112-5087-4e78-ac6c-48260fc26cd6` のみを追加し、採用しない。正式品質の合格ではない。

続いて同じ単一TTS素材を通常ファイル選択で新規テスト会議 `5408c711-6cd0-42c2-b3ea-acca0c49bdda` へ取り込んだ。直後の会議操作モーダルは描画／×focusがある一方native AX本文が消え、Escape後の再表示でも欠落、再びEscapeで元画面へ復帰した。画像 `/private/tmp/mon-first-import-modal-ax-20261011.png` SHA `c461333d7a03ec14afe383a67b5a55231d14c1b1461157a584db8cf6fbf0ba53`。原因解決やVoiceOver合格にせず、再現順序を残課題として固定する。この会議の通常ASR `8cdcda26-39db-4f84-934c-ae6567a86627`（13,403ms）とSherpa `2e45fded-4b05-4d19-ae6c-f11c1c2140b4`（6,668ms）は各succeeded 1/1、候補は未採用。単一TTSを複数話者評価へ換算しない。

通常終了PID42845消失後、DB＋WALを私的clone `/private/tmp/mon-saved-action-db.slc2Tq` へコピーしread-onlyで照合。integrity_checkはok／foreign_key_checkは0行。先の冷起動cloneと、既存テスト会議のid/title/現・前head、全下書き／出力履歴、採用議事録#1の全列を同じ順序で照合しSHA `62c05dcf98eff04431d5724d2a6c5303f0bf18bd351499b57bc26af3b8fb14dd` が一致。設定／構成／CPU記録SHA `930443a4df65c963311152d73dc4d33cdd78a9945c47d21263d446e1eb3854c4`、実Markdown SHA、既存11会議の4組／bootstrapも不変。追加候補／job／新会議があるためDB全内容不変とは言わない。SQL書込み・本番操作・マイク／PC録音・追加取得・CIなし。

調査から最初の実装までの記録区間UTC06:09:06〜06:10:12は1分06秒（その後の実機確認／文書時間は別、総実装工数ではない）。画面826成功／0失敗／0skip、1.948249583秒、型検査／Vite1.51秒、QA Vite1.51秒／Cargo1分11秒、deep strictアドホック確認・diff check成功。Rust製品ソースは前commitから変更せず、UI修正のみを理由に長いRust構築／検査を再実行しない。既存Sonner／chunk／公証未設定warningは残る。暫定残工数12〜29時間以上と許可／素材／配布準備待ち、正式評価／Windows保留を維持し、全23タスクや全UIを完了扱いにしない。

| ログ（`/private/tmp/`） | SHA-256 |
|---|---|
| mon-saved-action-tests-20261011.log | 160249dca9458d08b33c688839a1e8054706f845fa2aac5112a5fef611a87203 |
| mon-saved-action-build-20261011.log | 91a1250d1a501ae2f430bbc7f92a7e552b46ea20101bb78ccdb1556a7545e5b1 |
| mon-saved-action-qa-build-20261011.log | 56a4640a9667a3a43b34a29cac4f29b1c791c1cb28f3a8089e053051c9ca0b31 |
