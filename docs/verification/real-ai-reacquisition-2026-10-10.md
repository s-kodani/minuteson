# 実AI再取得とMac QAのローカル統合検証

日付: 2026-10-10。ユーザーが「公式配布元から合計4GiB以内で再取得し、QA内でローカル実行」を明示許可した後の記録。正式評価・10話者・Windows固有部分は保留のまま。CI・コミット・Pushなし。

UIの追確認では閉じた設定hostを開く順に揃え、文字起こし→議事録設定→新候補#3比較→採用前確認取消→設定取消→文字起こしへの往復2巡と、子編集管理の開閉でnative AX本文・操作・入口焦点復帰を確認した。先行のAX欠落はこの経路では再現しないが、全VoiceOver／全順序合格ではない。再生成・採用は繰り返さず、終了後のDB整合性・保護対象4組SHA一致を確認。[最新UIの検証範囲](ui-modal-and-transcript-spacing-2026-10-10.md)を参照。以下のAX残課題は追確認前の履歴。

## 2026-10-11 完全一致反復の新規生成条件v4（検証中）

最終限定Rustは終了0、86成功／0失敗／3ignored（89件）。構築387.92秒／実行583.30秒、再生成可能cache4438.1MiBを回収し8342.5MiBを保持。長いadaptive checkpoint3件も終了成功を確認した。これはgeneration限定であり、最新v4の全Rustを実行済みとはしない。

最新QAの通常GUIから新規ジョブ `1ee239c3-2821-4ce0-9aeb-6e57ee5bd97c` が成功し、generation.schemaVersion4／候補#3／2項目を保存。DB開始・終了差42.509秒はRust検査と並行した値で性能評価ではない。原文1422bytes／SHA256 `8a67f3fa447187dde09835240c4150882d43f8474f981a29644e5349cf3d6a15` は完全JSON＋`[end of text]`。原文4項目／全7フィールドが異なる項目2件、normalized2項目は最初のraw値・metadata・根拠順と一致した（初回のJSON文字列比較はキー順差でfalseになり、明示的な7フィールド値比較で訂正）。未知根拠0、非ToDoの担当・期限null。prompt834bytes／system1465bytes。旧候補#2／6項目、現在版#1／保存済み編集は保持し、候補を採用しない。正式品質評価は保留のまま。

新候補比較→採用前確認→取消→設定の正常系はnative AXで列挙できたが、その後の文字起こし編集オプション開閉→議事録タブ→設定再表示で、画像には設定があるのにnative AXの本文・操作が欠落する別経路が残った。Tab後も欠落、Escで背景と設定入口の焦点が戻ることを確認し終了。対象QA本体不在後のreadonly DBは整合性ok／外部キー0行／会議8／ジョブ成功10・失敗1。下書き＋出力履歴、確定解析＋確定議事録、会議タイトルと版参照、bootstrapの4組SHA256は下記通常v3後の値と全て一致。AX問題の全体解消とはしない。以下の最終限定検査・通常v4生成「実行中」はこの終了確認前の履歴。

通常v3の原文6項目／完全一致3項目という実測に対し、新規生成だけgeneration.schemaVersion4へ分け、厳密な解析・根拠検証後に完全一致項目を整理する処理を追加した。body・kind・担当者名／ID・期限date／text・根拠ID集合の全てが同じ場合だけ最初の項目を残す。最初の本文・metadata・根拠順は変更せず、異なる根拠・担当者・期限・種類・本文は残す。意味の近さを理由に削除しない。既存の分割・semantic merge・全source ID検証・容量上限は維持する。v4の抽出指示と分割はv3と同じbyte列で、result schema／文書schema／標準テンプレートは1のまま。旧生成1〜3の保存項目・再開・再試行は元versionの処理を維持し、未対応versionを拒否する。

生成単位の保存、再開時の原文とnormalized照合、adaptive merge完了、追加復元の再対応付けに同じversion付き処理を接続。復元の抽出原文はnormalized subsetから作らず、厳密に検証した全raw項目の参照だけを再対応付けする。生成元の出力原文を削除・書換えず、旧版の6項目を勝手に3項目へ変更しない。新しいraw6／normalized3の合成回帰では、再open後の推論再実行なし・元下書き保持・候補未採用、追加復元でのraw6保持／normalized3と改変拒否を検査する。

最初の限定検査は87件で終了0、84成功／0失敗／3ignored、構築384.98秒／実行542.76秒、再構築可能cache4855.9MiBを回収し8347.3MiBを保持。追加のstorage／restore回帰2件を含む最新ソースの最終限定検査はCargo表示6分27秒で構築し89件を実行中。新しい再open後のraw6保持／version別候補保存と、復元後のraw6保持／version別normalized照合の両テストも成功表示を確認したが、長いadaptive merge検査の終了はまだ待つ。ログは `/private/tmp/mon-generation-v4-tests-20261011.log` と `/private/tmp/mon-generation-v4-final-tests-20261011.log`。終了前の件数を成功総数へ置き換えない。fmt・git diff --check成功。最新v4の全Rustは未実施。最新725件UIとv4をMac QAへ反映し、単一境界の候補比較でnative AXの内容・操作が列挙されることを確認。既存モデルで通常の新規v4生成を開始し、実行結果と保存データの再照合は追検証中。コミット／Push／CIなし、追加取得なし。

既存719件UI QAの冷起動で、最初に開いた議事録設定はnative AXへ内容が列挙された。その状態から候補比較を開くと候補本文と実操作が列挙されなくなり、Escで比較を閉じると設定内容と候補確認入口の焦点が戻った。さらにEscで現在版へ戻ることを確認して終了、対象本体不在を確認した後のreadonly DB整合性ok・外部キー0行、下書き＋出力履歴／確定解析＋確定議事録／会議タイトルと版参照／bootstrapのSHAは下記通常v3後と全て一致した。AXの原因をOS固有と断定せず、複数モーダルから候補比較に進む操作を次の修正・実機比較対象とする。今回のnative操作では生成・採用・保存・再生は行っていない。

## 2026-10-11 通常v3生成と全Rust検査の終了確認

最新QA `/private/tmp/mon-minutes-v3-qa-artifact.J8oT7d/MON QA.app`（構築194.50秒、本体SHA256 `ede674e01b6e99f5a308acbcca955d9859c8b35e6f20b30d5f1964c0e07a9daa`、コピー一致・strict deepアドホック署名・arm64を確認）で、通常GUIの新規「議事録を再生成」を1回実行。管理済みQwen1.5B／llama-completion、threads4／context8192／predict2048／GPU0／seed1／promptBudget5400を保持した。旧失敗ジョブの再試行ではなく、固定生成条件version3の新規ジョブ `f8b99bc4-b562-4718-a03a-aec06f62a42f` が成功し、候補#2／6項目を保存。DBの開始・終了差47.541秒はRust検査と並行した記録値で、性能評価ではない。

原文1646bytes／SHA256 `ee400f37c1607b52351f2ca023b2b1ba47e9e2771fee329e38bcea05c91a4614` は完全JSON＋正規終端marker。製品system1465bytesはソースのv3指示と完全一致、prompt834bytes／2発言だが、過去診断のpromptとの完全byte一致ではない。原文6項目・異なる完全項目3件で、各項目が2回ずつ反復している。normalized itemsとDBも6項目、全summary、未知根拠0、非ToDoの担当・期限null。parserを緩めず候補保存まで到達した機能証拠だが、反復解消や品質合格ではない。候補を採用せず、旧失敗version2も変更しない。

候補は画面上で比較できた一方、native AXには背景設定の閉じる操作だけが残り、候補本文・実際の閉じる操作が列挙されない問題を観測。Escで比較を閉じて現在版へ戻り、管理モーダルのTab／Escは正常。冷起動後も8会議・現在議事録#1の手動編集マーカー・候補#2／6項目・完了ジョブを確認して終了した。このAX問題は未修正。

QA終了・対象プロセス不在後、readonly DBのintegrity_check=ok／外部キー0行／会議8／ジョブ成功9・失敗1を確認。下書き＋出力履歴SHA256 `c73af0f0de54e66787ecaf57b96fd6ff37fad5f125b1073ff15d192be2bd309c`、会議タイトルと現在／直前版参照SHA256 `6a04f298df7bfa0f91ff28c1f097085bd72acb8b871fff48b5548c4d83b36d78`、bootstrap SHA256 `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` は再生成前と一致。検査側の最初のジョブ集計は存在しないjobs表を参照して失敗し、実スキーマprocessing_jobsで訂正した。

最新ソース（v3・test-only ready公開修正を含む）の全Rustは終了0。1825成功・失敗0・79ignored、doc test0件成功。build354.66秒／execution777.90秒、終了後4274.9MiBの再生成可能cacheを回収し8730.5MiBを保持。原SIGKILL回復・ready公開回帰・長いadaptive checkpointケースを含む。ログ `/private/tmp/mon-minutes-prompt-diagnostic.4VM4P7/rust-full.log`。先行85件限定検査は終了結果を保持できず、全件成功の証拠へ数えない。以下のRust実行中・通常Mac生成未検証記載は、この終了確認前の履歴。

確定解析＋確定議事録の全行JSON SHA256も `8a1d80c720c6f32f27282231a38f21dd95c744d29213eae49d14a34754cdb620` で再生成前と一致。後続の719件UI反映QAでは生成を繰り返さず、終了後にこの値を含む4組のSHA一致を再確認した。

## 2026-10-11 抽出終了指示の比較と固定条件の互換性

通常生成失敗3fd185e4の保存原文を本文非表示で検査。入力2発言・prompt834bytes・system1054bytesに対し、未完JSON内に完全なbody15個／異なるbody1個／kindは全てsummary、終端markerなし。以前の成功7133f555も同じ入力容量で6項目／異なるbody2個だった。打切りのtoken数・原因は直接ログにないため上限到達の推定は維持するが、反復の存在は保存原文から直接確認した。

専用 `/private/tmp/mon-minutes-prompt-diagnostic.4VM4P7` で、失敗時のprompt／schemaを読み取り、取得済み公式Qwen1.5B／同じllama-completionをローカル実行。既存の全指示を保持したうえで、同じ事実・要約の反復禁止、異なる内容を省略せず一度ずつ抽出、完了後にitems配列とJSONを閉じる、存在しない種類を埋めるための項目を作らない、という指示だけを追加した。threads4／context8192／predict2048／GPU0／seed1／temp0・offline・reasoning offなど製品の引数を維持。モデル・出力上限・JSON schema・入力本文は変更していない。私的本文や出力はQA内だけに置きGitに入れない。

比較1回はexit0、19.238秒、1347bytes、JSONオブジェクト＋正規終端marker、schemaVersion1、4項目／完全一致重複0、全kind summary。根拠IDは既知発言のみ、項目の必須7フィールドと非todoの担当／期限nullを確認。これは診断スクリプトの構造検査であり、製品parser・通常worker・候補保存・Mac UIの合格証拠でも正式品質評価でもない。実行はRust全体テストと一部並行しており速度評価にしない。診断systemには末尾改行があり、製品の指示文字列の完全byte一致試験とは区別する。

新規生成の内部固定条件schemaVersionを3にし、従来1／2は元の1054bytesの指示と元の分割計画を維持する実装を追加。3は指示1465bytesを入力予算へ含めてUTF-8分割し、従来2と同じbounded adaptive mergeを使う。結果schemaVersion1、文書template standard-v1、出力上限、根拠・担当／期限照合、破損JSON拒否は変更なし。読み込み／再試行／復元は保存された条件のversionに従い、既存ジョブの書換え・自動再試行・不完全結果の修復や採用は行わない。旧指示の固定SHA256、全versionのUTF-8無損失、旧2の冷起動後再試行、旧2／新3の復元・指示差替え拒否の回帰を追加。現在は新実装の限定Rust検査中で、全体成功・Mac再生成成功にはまだ数えない。

先に開始したDB修正後のRust全体はbuild54.31秒／execution471.62秒でexit101。localhost HTTPの前回bind拒否は同じfixtureを許可する実行条件で回避したが、読取りコピー強制終了テストで、readyファイル作成直後・本文書込み前に親が存在だけを観測し空のPathBuf（parent=None）を読む競合を確認。失敗はstartup_admission.rs:935、検査側の通知レースでありDB耐久条件を緩めない。途中失敗のため全件成功・全件実行とはせずdoc testも未実行。この実行の単体バイナリは抽出version3変更前に構築されている。4498.9MiBの再生成可能cacheを回収して8280.0MiBを保持した。

test-onlyの準備通知を一時ファイルへの書込み・sync完了後の非上書き公開へ変更し、「公開前にreadyが存在しない／公開時に完全なパス／既存素材は上書きしない」を検査する回帰を追加。製品の読取りコピー、SIGKILL時点、元DB・WAL・保持SHM照合、bootstrap再起動検査は維持。修正後の全体実行はまだ残る。新規モデル取得・録音・本番設定変更・外部送信・CI・コミット／Pushなし。

比較出力のbody文字列も4種すべて異なることを追加確認（長さ10／8／43／58文字、根拠各1発言）。原文SHA256は `a8d139732c09923dbe010de8987a2e23e4bdd54bc2e3fa7d3e759988d8f7dd00`。意味の重複や正式品質の合格はこの構造計数からは判断しない。関連UI source検査40件成功（runner0.126秒）、容量管理runner26件成功（0.154秒）、Python全125件成功（13.816秒）。cargo fmt／git diff --check成功。議事録関連のRust限定検査は共通cacheのlock待ち後に構築を開始しており、準備通知のtest-only修正はその構築開始後のため、修正後全体検査を別途実行する。

限定再構築はCargo表示9分50秒で終了し、85件の選択された検査を実行中。新旧指示のbyte照合、全versionのUTF-8分割、指示差替え拒否、旧2の冷起動後再試行、旧2／新3のバックアップ復元、完了／途中のadaptive復元は成功表示を確認。長い分割生成・中断回復の3ケースはなお実行中で、限定検査全体の終了・成功件数は未確定。古い実行を止めたり重複起動したりしていない。構築中cache約11GiB、空き404GiBを読み取り確認し、実行中は回収しない。終了後10GiBソフト予算の回収を維持。最新QAへのこの生成指示の反映・通常GUI再生成はまだ行っていない。

## 範囲と保存先

- Apple Silicon M1 Max／32GiB、CPUで実行。新規録音、音声／文字起こしの外部アップロード、TCC変更、本番アプリの設定変更なし。
- 取得・構築・確認記録: `/private/tmp/mon-real-ai-qa.T3hkeF`。専用一時directory、uid501／mode700。
- QA: `/private/tmp/mon-button-modal-qa.X1luwR/MON QA.app`、identifier `jp.mon.desktop.qa`。main SHA-256 `be755dfaf4de14373a956b7156c345b322675bd3d49bd90ea11e68e09f587d30`。製品ソースは以前の690件確認時点と同じで、今回再構築なし。
- QA DB: `/private/tmp/mon-native-ui-smoke.SOSoB9/MON-meetings-f352507d-7b63-4335-b953-6b063348c5fe/meetings.sqlite3`。操作は実WebViewの通常画面から行い、直接DB変更／IPC注入なし。DB検査はreadonly。
- 提供FLACの5:00〜5:30だけを16kHz／mono／PCM16のCAF・WAVへコピー。元ファイル452,246,304bytesとmtimeを保持。全編再解析ではない。実際の話者人数は3人以上との申告だけで確定しない。
- CAF SHA-256 `1a7914e6651557229cc1090af7221770a114a73492fcc7aebd22d51434d3dae7`、WAV `c9e75747cbcda21d566da73298b605892f037af2a1f88809dce1a1498710705c`。音声・生成本文・画像はGitへ追加しない。

## 配布元、固定版、照合

whisperのGGMLはOpenAI直配布ではなく、[whisper.cpp公式取得スクリプト](https://github.com/ggml-org/whisper.cpp/blob/v1.9.4/models/download-ggml-model.sh)が参照するメンテナーの配布。Qwenは[Qwen公式0.5B GGUF](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF)と[同1.5B GGUF](https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF)、話者分離は[sherpa公式モデル案内](https://k2-fsa.github.io/sherpa/onnx/speaker-diarization/models.html)とk2-fsaのGitHub releaseを使用した。

| 用途 | 固定版・ファイル | SHA-256 |
| --- | --- | --- |
| ASRモデル | ggerganov/whisper.cpp revision `5359861c739e955e79d9a303bcbc70fb988958b1`、`ggml-tiny.bin`、77,691,713bytes | `be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21` |
| LLM小型比較 | Qwen revision `9217f5db79a29953eb74d5343926648285ec7e67`、0.5B Q4_K_M、491,400,032bytes | `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db` |
| LLM最終試験 | Qwen revision `91cad51170dc346986eccefdc2dd33a9da36ead9`、1.5B Q4_K_M、1,117,320,736bytes | `6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e` |
| 話者区間 | `sherpa-onnx-pyannote-segmentation-3-0/model.onnx` | `220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079` |
| 話者埋め込み | `3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx`、39,593,761bytes、保存名 `embedding-base.onnx` | `1a331345f04805badbb495c775a6ddffcdd1a732567d5ec8b3d5749e3c7a5e4b` |
| sherpa CLI | v1.13.8 osx-arm64 shared-no-tts、`sherpa-onnx-offline-speaker-diarization` | `e1170a93308867d8e343ac22a00b46b1d8e786c763c32a17caff07cf934ff66f` |
| ONNX runtime | 同release `libonnxruntime.dylib` | `3567d114f7299d559993e536d605a6f46d7bc9d2542004accc80ee9bf5457f0b` |
| sherpa C API | 同release `libsherpa-onnx-c-api.dylib` | `ae729669633008692c670d4e4c942db7ea5bdc38abc2f479516062d48849e9d1` |

取得はHTTPS限定・サイズ上限・ハッシュ照合・固定revisionで実施。アーカイブ展開は絶対パス／親directory逸脱／特殊ファイル／危険なリンク／上書きを拒否。認証情報や非公開本文を取得URLへ入れない。配布元digestがないsource archive／segmentation archiveのローカル計算値を、配布者の署名やdigestとは呼ばない。segmentationモデル本体は以前の検証hashと一致。ライセンス／製品同梱条件の最終判定は別の残件で、今回の取得を製品モデルの正式採用にはしない。

whisper.cpp v1.9.4 commit `927cfce34f31707e17f2bff35c349632fb9e2c3a`、llama.cpp commit `edd6e2bbdad5930899a93db8fa73c3b61c7b9bcc` を公式ソースからRelease static CPU＋Accelerateで構築。Metal／OpenMPを無効化し、必要なCLIだけを対象にした。CMakeが端末にないため、Kitware公式4.1.2 macOS universal（SHA `3be85f5b999e327b1ac7d804cbc9acd767059e9f603c42ec2765f6ab68fbd367`）をQA directory内だけで利用し、グローバル導入なし。

- `whisper-build/bin/whisper-cli`: SHA `9f631df18105fb3246bfb66fb4c349372c9c338c0c34ed40271a682eab61b7c0`。
- `llama-build/bin/llama-completion`: SHA `688807fcb3c9da85e417c967f1228a23a3ae3fc290c2fc3bf536e23193f6cc9b`。
- 詳細URL・bytes・hashは専用directoryの `acquisition.json`／`extra-llm-acquisition.json`、構築条件はconfigure／buildログを保持。

初回、過去のbaseモデルhashに別の220,623,485bytesのembeddingモデルを対応させてしまい、hash不一致を検出して実行しなかった。公式ファイル名を照合してbaseを取り直した。未使用の `embedding.onnx` はowned directory／uid501／inode170955708／mode100600、実SHA `2b9c4219b25326473524f006f1a09050ac28ccaf58c1f7dbc53e7631fa2fb1df` を再確認後に削除。ユーザーデータではなく、公式配布元から再取得可能な検証用ファイルだけを回収した。

完成した取得ファイルの累計は、未使用モデル・ソース・CMake・比較LLMを含め2,099,788,805bytes（約1.96GiB）。部分転送／プロトコルの通信量そのものの測定ではない。削除前の構築物込み実使用は約2.59GiB、削除後は2,489,824KiB（約2.37GiB）、通常ファイルのlogical合計2,473,223,956bytes。取得と展開・追加LLMの上限確認で4GiB以内。使用中のCLI／モデル／runtime・source／license・記録・音声コピーは保持。

## 実エンジン試験と試験準備の修正

既存のignored integration試験だけを `tools/mon_rust_tests.py` で実行。共有キャッシュのlockで直列化し、独立native CLI構築とGUI確認を並行した。Rust／Swiftの全試験やCIの再実行ではない。

| 実行 | 結果 | Rust build | runner execution | wall（待ち／回収込み） |
| --- | --- | ---: | ---: | ---: |
| whisper初回 | 1成功・1失敗 | 63.99秒 | 12.23秒 | 77.01秒 |
| llama 0.5B | 2成功・1失敗 | 0.48秒 | 28.26秒 | 29.08秒 |
| sherpa | 2成功・0失敗 | 0.26秒 | 11.91秒 | 22.54秒 |
| whisper準備修正後 | 2成功・0失敗 | 390.23秒 | 14.76秒 | 405.66秒 |
| llama 1.5B | 3成功・0失敗 | 0.59秒 | 63.32秒 | 289.67秒 |

llama最終wallにはRust共有lock待ちが含まれ、モデル推論に289秒かかったという意味ではない。runner executionは回収処理も含む値で、libtest自身の実行はwhisper最終6.80秒／sherpa11.88秒／llama1.5B63.30秒。外部CLI構築はwhisper26.22秒／llama56.18秒（configureは別）。実装・調査・GUI操作の個別wall時間は未計測で、並行小計を全工程時間としない。

whisper初回の失敗は、既存試験が別ジョブのraw specをコピーしてoperation_idだけを変え、新規操作の用語固定スナップショットの由来検証に反したため。`storage/transcription_tests.rs` を通常の `transcription::enqueue` 経路で新規ジョブを作るよう修正し、再試行には元のretry経路を維持。製品の由来検証や安全条件を緩めていない。最終試験は候補の手動採用、編集保持、取消後の結果資産再利用まで通過。

0.5Bの統合試験は重複 `sourceItemIds` を出力し拒否された。JSON検査／根拠検査を弱めず、4GiB内でQwen公式1.5Bを追加して比較。1.5Bはschema制約、複数windowのbounded merge、候補保存・pause／resume・取消再試行で結果再利用の3試験が通った。合成fixtureの保存経路試験であり、要約品質の合格ではない。

sherpaは30秒の実音声で自動推定と明示3指定のCLI動作、および保存workerを確認。自動17区間／明示3指定12区間を保存できた。これは正しい話者数／話者正解を証明しない。workerの文字起こし部分は合成fixtureで、実WebViewからの結合は次節に分ける。

共有Rustキャッシュは一時的に約12GiBになった観測があり、10GiBはピーク保証ではなくpost-run目安。runnerが初回4,087.2MiB／最終4,210.6MiBの再生成可能なキャッシュを回収し、最後8,239.4MiB（約8.05GiB）を保持。再compile390秒は隠さず記録し、通常の長時間Rust全試験を追加しない。`cargo fmt --check`／`git diff --check` 成功。テストの最終後に行った変更は同じenqueue呼出しのrustfmt形式への改行だけで、改行後にintegration binaryを再構築していない。既知のlinker unwind警告は残る。

## Mac実WebViewの通常操作

新しいQA会議 `QA Real AI 30s 2026-10-10` をGUIから作成し、30秒WAVを取り込み。既存3会議を保持した。取得モデルのパスはQA画面の各エンジン設定へ入力し、autosaveとreadonly `app_settings` の一致を確認。管理モデルパッケージ取込の試験ではなく、導入済みファイルの直接パス設定。

1. whisperのパス入力初回は末尾cliが欠けて保存され、開始時にos error 2で拒否。画面とreadonly値を照合して正しい絶対パスへ修正し、jobが開始・成功。入力自動化のタイミングと製品入力処理の根本原因は未判定で、初回を成功扱いにしない。
2. 文字起こしは候補#1／2発言を保存し、自動採用なし。候補確認→採用確認→明示採用で現在版#1にした。
3. 先頭発言にQA専用の保存確認文字を追加し、Cmd＋Enterで確定、保存済み表示を確認。Cmd-Q後既知main不在を確認して再起動し、同じ会議の本文に編集が保持されることを画面で確認。recognized_textに編集markerはなく、working_draftsにのみ存在。
4. llama-completion＋Qwen1.5BをQAの標準設定に指定し、CPU4／GPU0／context8192／output2048の既存設定で画面から生成。job成功／候補#1／6項目を保存し、自動採用なし。候補確認→明示採用で現在の議事録#1へ。
5. 生成文書の見出しにQA議事録保存markerを追加し、編集を終了。保存済み表示、readonly draftへの保存、generated_document_jsonに追加markerがないことを確認。
6. native Save Panelで専用directory内の `qa-real-ai-minutes-export.md` へ書き出し。成功通知・5,159bytes・mode600・SHA `fa23384fefc6070c491a2d21b737f8fa7e681414f49e4e1a4b5643ecf97bf034` がexport_recordsの成功・サイズ・hashと一致。source_draft_idがあるので保存編集を含む出力。本文はGitへ追加しない。
7. 再度Cmd-Q→再起動し、現在版#1／保存済み編集を含む表示と見出しmarkerを確認。画像 `native-minutes-restart.png` をprivate QA directoryにmode600で保存。
8. sherpa CLI／segmentation／base embedding／共有runtime3件をQAのモーダルに設定し、自動推定で画面から実行。job成功、話者分離候補#4を保存。現在の文字起こし#1と編集を勝手に置き換えず、候補#4は未採用のまま保持。

readonly jobsのstarted_at〜finished_at差: transcription2.063秒／minutes_generation29.510秒／diarization4.668秒。画面操作を含む全工程や正式性能値ではない。終了時は会議4、解析版4（#1 committed／#2・#3 internal_snapshot／#4 candidate）、議事録版1 committed、integrity_check=ok、foreign_key_check=0行。最後にQAをCmd-Qで終了し、既知QA／実AIの正確な実行パスに絞ったプロセス一覧に該当なし。

## まだ合格にしない事項

- 接続・保存・手動採用・基本編集・再起動・Markdown出力の実機証拠は得たが、文字起こし精度、話者正解、要約品質は未評価。今回tiny ASRの誤認識や議事録の重複要約も見られる。QA markerが要約対象になった検証入力であり、本番品質・モデル選定の材料と混同しない。
- 一部モーダル／候補確認でnative AXが空になり、画像と座標で操作した。別の設定モーダルではAX操作できたため、全モーダルのアクセシビリティ合格を主張しない。実VoiceOver／IME、全通知／異常系、UI全面完了は残る。
- embedding helperによる実特徴抽出の今回再実行、話者候補の採用／名前編集／復元の全UI、実根拠再生とMarkdown取消／失敗の実機網羅は未実施。
- 全編60分以上・10話者・16GB／4時間・録音／マイク／PC／TCC・Meet／Zoom／Teams・Windows、Developer ID／公証／更新、製品同梱ライセンス判定、先行試用は未達／保留の区別を維持。
- プロダクトの新機能・安全条件の緩和なし。今回のrepo変更はignored試験の準備修正と検証記録のみ。UI修正が全面的に完了するまでコミット／Pushしないユーザー指定を保持。

## 2026-10-11 実whisperの管理取り込み・CPU確認・標準適用

前節の直接パス設定とは別に、通常GUIの手動取り込みから管理パッケージへの経路を確認した。新規ネットワーク取得・録音・TCC変更・外部送信なし。対象はwhisper.cpp一構成だけで、sherpa／llamaの管理導入や初回HTTPS取得画面の合格に拡張しない。

最新QA `/private/tmp/mon-spacing-complete-qa.v3a7Na/MON QA.app`、identifier `jp.mon.desktop.qa`、main SHA `46a59e398e4a65243e396eba088eef6612dff61cf288f68b8a539750b0461590`。前ターンの701件成功時点のUIを反映。専用targetでCARGO_INCREMENTAL=0、release opt-level1／debug0／LTO off／codegen-units256のQA構築。Rust表示1分25秒、全構築の完了観測約118秒（観測待ち込みで正確な全工程wall測定ではない）。ソースの製品profileは変更しない。helperと同梱noticeを再構築し、生成元との全コピー一致・strict deepアドホック署名・arm64 mainを確認。公証・Developer ID配布検証ではない。既存275件通知の未承認12件とchunk警告を保持。

Mac実WebViewで録音セッションの見出し／info中央、会議作成入力／ボタン、文字起こしエンジンの入力／select、項目間余白を目視確認。数値40px／13pxは前ターンのブラウザDOM計測と区別する。設定Escで親のエンジン設定入口、親Escで会議操作入口へ復帰。画像 `/private/tmp/mon-spacing-heading-native-final.png` と `/private/tmp/mon-spacing-engine-native-final.png`。この時点では設定を変更しなかった。

取得済み素材を再hashしたうえで、`/private/tmp/mon-managed-whisper-qa.oyBDB9` に実コピー2件とmanifestを準備。上表のtiny／公式fixed commitから構築済みwhisper-cliを使用。実行依存はlibSystem／Accelerate／libc++だけで、外部runtime_library指定なし。2件合計80,193,385bytes、期待SHAと容量をmanifestに指定。manifestのengineVersionはソース版の申告で実行版検証ではなく、modelVersionはモデルカードの固定revision。ライセンス参照は固定版[whisper.cpp LICENSE](https://github.com/ggml-org/whisper.cpp/blob/927cfce34f31707e17f2bff35c349632fb9e2c3a/LICENSE)と[モデルカード](https://huggingface.co/ggerganov/whisper.cpp/blob/5359861c739e955e79d9a303bcbc70fb988958b1/README.md)。製品再配布許諾を確定する操作ではない。

1. 設定→ローカルから取り込む→絶対パス入力→事前検査。未導入／互換性未検証、2件80,193,385bytesを表示。事前検査の成功通知はモーダル背後にあり、開いているフォーム内に結果／次工程が表示されないため、Escで閉じ「モデルの配置・構成記録」を手動で開いた。操作完遂はできたが、導線改善の残件として扱う。
2. コピー確認→明示コピー。配置ID `3ca2d231-55e6-4190-9efc-5b62b0bace04`、reviewHash `d08535219e91c1954d4b8c343ac9b2ae7cd1c1ad0aaf06d8eddb1ce0137bd581`。QA app-data/modelsに専用フォルダー0700、実行ファイル0700、モデル／manifest0600で配置。元とコピーの2実ファイルSHAは上表と一致、manifest SHAは双方 `2e0e5dec772a00db90533473510709031270c177d3c073582fc1cf8174016c54`。model_filesはready、model_packageはunavailableのままでAI準備完了を主張しない。
3. この時点の互換性記録0・プロファイル0、全既存設定hash／lock不変、適用操作は無効で、コピーによる自動実行／設定変更なし。
4. CPU確認の説明を読み、明示「実行ファイルを起動して確認する」。確認ID `362acfa7-98d3-41b8-88e4-ee5195e27740`、succeeded、合成10秒16kHz mono PCM16無音・CPU1／GPUなし・日本語／beam5。record.elapsedMs=3405。これはファイル照合を含むCPU試行経過で、60分性能・精度の評価ではない。成功だけでは設定とプロファイルを変更しなかった。
5. report JSON SHA `41926f8b4c919fba06b09a7d319b50bde362ee35babb6ff197f1244c968086d8` を再計算してDBと一致。model-checks内input.wav320044bytes／result.json870／stderr.log3128／stdout.log80の実サイズ・全SHAがreportの4証拠と一致。会議音声は使っていない。
6. 標準適用の確認画面で旧外部パス→管理コピー、threads4・GPUなしの維持を確認して一度取消。profile0とwhisper_profiles lock2／hashを保持、他3設定も不変。取消後は標準への適用入口に焦点復帰。
7. 新しい確認→明示適用。標準profile `bdedb652-9cf2-4bd3-a4cd-35119b4f4329` 構成版1、asr componentだけを登録、whisper_profiles lock2→3／hash `5e38943af28693f6f9ce73c56a641dd32e6946d00d7cc6e8d735b4965c014195`。標準の実行／モデルパスのみ変更、highAccuracyの全値・標準数値threads4／useGpu=false／VAD=nullを維持。diarization_profiles lock4／llama_profiles lock2／ui_preferences lock4は元のhashと一致。管理headを新規追加。設定と最新CPU記録の一致を表示するが不足sherpa／llamaを明記。全AI準備完了にはしない。
8. Cmd-Q、関連main／whisper／buildプロセス不在確認後に同じQAを冷起動。8会議、標準構成版1／保存設定一致／最新CPU一致、高精度未選択を保持。互換性試行は1件のまま、既存成功ジョブ6件で自動解析開始なし。記録画像 `/private/tmp/mon-managed-whisper-applied-native.png` と `/private/tmp/mon-managed-whisper-restart-native.png`。終了時は再びCmd-Qと関連プロセス不在確認。

読み取りDB検査はintegrity_check=ok／foreign_key_check0行。meetings8／model_packages11（旧未配置10＋今回1）／profile1／CPU記録1。操作前後・再起動後のmeetings／working_drafts／processing_jobs／export_records全行をid順JSONにした連結SHAは `e6a9150d99eac460a7653a9ab9f6127975d4fb0c01ab9d1d499f8e9888cebd5f` で同一。bootstrap SHA `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` も不変。初回の件数SQLは存在しないjobs表名で拒否されたため、sqlite_schemaのprocessing_jobsへ修正して全検査を行った。profile component名もschemaから確認した。

取得素材2,464,896KiB＋今回元パッケージ78,320＋管理コピー78,320＋CPU証拠328=2,621,864KiB（約2.50GiB）、追加ダウンロード0。QA58,488KiBは別計上。終了・生成元とのコピー一致・署名・非リンクdir／uid501／inode171044404を照合し、専用target `/private/tmp/mon-spacing-complete-build.QkOdMs` 1,392,412KiB（約1.33GiB）だけを回収。再生成可能なビルドだけで、保持QA・公式素材・取り込み元・管理モデル・試行証拠・会議を削除しない。回収後のQA署名とdiff空白検査成功。今回全自動テスト再実行／CI／コミット／Pushなし。管理コピーからの実会議新規解析、他2エンジンの管理導入、TLS/CDN初回取得、全異常系・UI導線・配布条件と計画全体の残件は維持する。

## 2026-10-11 話者分離・議事録モデルの管理導入と3エンジン保持

前節の他2エンジンの管理導入残件へ追加検証。製品ソースの変更・再構築なし。最新QA `/private/tmp/mon-model-guide-qa.zFYwZC/MON QA.app`（main SHA `a22b8107f8115180b2ec155c063c51ce5b4df61c32ea48cee1fddbd85a808364`）のstrict deepアドホック署名検査を確認して利用。最初の読み取りプロセス照合でPID66782とこの正確なパスを確認した。Developer ID署名／公証済み配布物ではない。

取得済み公式ファイルを再hashし、上表・取得記録との一致を確認。llamaは固定commit `edd6e2bbdad5930899a93db8fa73c3b61c7b9bcc` から構築済みのstatic CPU＋Accelerate CLI9,658,448bytesとQwen公式固定revisionの1.5B Q4_K_M1,117,320,736bytesを使用。配布情報は[Qwenの固定版モデルカード](https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/blob/91cad51170dc346986eccefdc2dd33a9da36ead9/README.md)と[取得済みソースのLICENSE](https://github.com/ggml-org/llama.cpp/blob/edd6e2bbdad5930899a93db8fa73c3b61c7b9bcc/LICENSE)をmanifestへ参照。GitHubのllama LICENSEページ読取はwebでエラーだったため、それを今回のオンライン本文確認成功とはしない。

sherpaはv1.13.8のCLI405,440bytes、segmentation5,992,913bytes、base embedding39,593,761bytes、ONNX runtime28,775,120bytes。otoolでCLIの非システム依存は `@rpath/libonnxruntime.dylib`、探索先に `@loader_path` があり、このruntimeの他依存はOSライブラリのみと確認した。管理パッケージにはruntimeをCLIと同じ階層へ含め、直接パス設定時のC／C++ APIライブラリ2件はこのCLI試行に不要なため追加しなかった。参考は[公式モデル案内](https://k2-fsa.github.io/sherpa/onnx/speaker-diarization/models.html)・[固定版engine LICENSE](https://github.com/k2-fsa/sherpa-onnx/blob/v1.13.8/LICENSE)。参照記載を製品同梱条件の最終判定や法的許諾の取得にはしない。

一時取込素材は `cp -c` のAPFS省容量コピーで準備し、リンクではない通常ファイルとして全SHAを再照合。sherpa元 `/private/tmp/mon-managed-sherpa-qa.oOWb2E` のmanifest SHA `acdec8a78fdfa9fbb94209ab5d4217eb8f4a2de07478b9178269b7f851a27952`、llama元 `/private/tmp/mon-managed-llama-qa.vaPoQK` は `7ca1ea437f8e5f09d69d720b652b473f3b8f58a6b075d0daa1a1fe5ca7abf807`。申告engine／model版・期待容量／SHA・macos_aarch64と必要roleを明記し、実行ファイル0700、モデル／runtime／manifest0600、専用directory0700。モデルの新しい取得は0で、取得上限4GiBは変えない。

通常GUIで各エンジンについて入力→事前検査→個別コピー確認→明示コピー→「配置・構成記録を開く」→別CPU確認→標準適用確認の取消→新しい確認→明示適用を実施。コピーはsherpa4件74,767,234bytes、llama2件1,126,979,184bytes。新しい次画面導線が実WebViewで動き、移動だけでは実行・適用しないことを確認した。

| 項目 | sherpa | llama |
| --- | --- | --- |
| 配置ID | `0394ea94-4723-43b4-8f67-778a584deaa7` | `12673a1d-0013-40e1-8fe6-7adc8810c67d` |
| reviewHash | `d52331b693a1897a1e308a541d9e905a4d4f083627fa59e3a24432aef5b06ce9` | `d35cfd53f284d4b07038e16190074fafbdca45e60592671a3e93a8dbae621046` |
| CPU確認ID | `101df8c9-a88a-420a-b059-e0f21399ddcc` | `638abb46-fed9-4864-bc6d-da35937aba26` |
| CPU試行経過 | 2,169ms | 13,773ms |
| report SHA | `bed16773dd8a2847828585c4f09280f78d256ca51f8314d47fc0aff0ac72f462` | `9f14f94fa75fef4f277eb5f9a25994af96a31573fecbe3030daebb74fe17cc0e` |
| 合成証拠 | 10秒無音WAV・stderr・stdoutの3件 | prompt・result・schema・stderr・systemの5件 |

管理モデルの全6実ファイル・両manifestのSHAは元と一致。DB report_jsonの再hashとreport内の全artifactの実サイズ／SHAを再計算して一致した。既存whisperの4証拠もこの検査で再照合した。CPUは1スレッド／GPUなし、llamaはcontext2048／出力64／seed42の固定JSON確認。会議品質・60分速度や通常設定の互換性合格へ拡張しない。

コピー・CPU成功後もプロファイルと設定は自動変更なし。sherpa適用取消後の全app_settings JSON SHAは元の `52bb954acfe2c0309e5132fc0c30045772f093f949b42c8163b8a2b4eb63408f`、構成版#1のまま。明示適用で標準#2 `668c64bb-0948-4008-b579-20c9135682b6`、diarization_profiles lock4→5／hash `24f48070c7a555e828edc1a2f1ef57e9e5a0dde6d8e499793d5ef4b6e1ee88ff`。標準の実行・モデル・runtimeパスだけを変更し、threads4／閾値500／窓移動100／最小発話0／結合ギャップ0を維持。適用確認UIと保存bindingを照合した。

llamaコピー・CPU確認・適用取消前後の全app_settings JSON SHAは `5fe81b0efe601523174a43f6a79ea93a5be5fb3b339d1d5cec94139570b55855`。取消後は元の適用入口にフォーカス復帰。明示適用で標準#3 `438c64cc-3993-4649-a0fb-da7ca7a2360c`、llama_profiles lock2→3／hash `0e730994e51de80fb174213c5ed54bc9ca39c8c51fff869bfbac84412689c583`。標準の2パスだけを変更し、threads4／context8192／output2048／GPU0／seed1／promptBudget5400を維持。ASR設定lock3／hash `5e38943af28693f6f9ce73c56a641dd32e6946d00d7cc6e8d735b4965c014195` と表示設定lock4／hashは操作前後不変。managed headは版#3を指すlock3／hash `25df5e5888c50dc4bcc98a6b373de3615fe5eed9d940c460a549ff603695d077`。高精度の全3設定JSONの連結SHA `fce8b91bca8149dd3c71c83edcfe48a29c3e7b1e77942cf847112be4ad28f779` はllama適用前後・冷起動後で一致（sherpa適用前の同集計は未取得）。

Cmd-Q→正確なQA／今回managed CLIのプロセス不在→同じQAを冷起動。会議8件、標準#3の全3エンジンの保存設定／最新CPU記録一致、DB上の不足なし、高精度未選択を通常画面で確認した。再起動でもCPU試行3件・既存成功ジョブ6件のまま、自動解析なし。profile_componentsはasr／llm／segmentation／speaker_embeddingが同じ構成版#3に結び付き、先行#1／#2も保持。試行成功と保存構成の一致を「全AI実行準備完了」とは表示していない。画像 `/private/tmp/mon-managed-three-engines-native.png` と `/private/tmp/mon-managed-three-engines-restart-native.png`。取消直後のCPU画面からのnative焦点はcontainerで、全フォーカス／VoiceOverの合格とはしない。

モデル導入完了時点のreadonly DBはintegrity_check=ok／foreign_key_check0行、meetings8／model_packages13／profile版3／CPU試行3／成功ジョブ6。会議・回復下書き・既存ジョブ・出力履歴の全行JSON連結SHAは開始前と同じ `e6a9150d99eac460a7653a9ab9f6127975d4fb0c01ab9d1d499f8e9888cebd5f`、bootstrapも既存SHAと一致。初回のanalysis_flows集計は存在しないstate列で失敗したため、その出力を証拠にせず後の正しいschemaの集計と区別する。

使用終了後、一時取込用の6重複実ファイルだけを通常ファイル／uid501／inode171096479〜171096484・managedコピーとSHA一致で照合して削除。公式取得原本・管理モデル・manifest・試行証拠・会議・画像は保持した。これらの重複ファイルは原本から再作成可能。取込用directoryにはmanifestのみ残るため、元パッケージをそのまま再検査できる状態とはしない。準備2directoryのdu値は1,173,604→8KiBとなったが、APFS共有ブロックを含むため約1.12GiBの物理空き増加を保証する値ではない。

回収後のモデル関連du参考合計は3,801,108KiB（約3.63GiB）：公式取得・構築保持2,464,104、whisper元78,320、今回元manifest8、managedモデル1,258,004、CPU証拠672。取得量の累計と保持量／共有物理使用量を混同しない。QA58,488KiBは別計上。ソース変更・Rust／フロント再試験・新QA構築・追加ダウンロード・録音・TCC変更・CI・コミット／Pushなし。今回進んだのは通常GUIの管理導入と固定CPU試行・設定適用・冷起動保持で、正式評価／Windows保留やTLS初回取得・署名／公証・ライセンス・長時間等の残件は維持する。

## 管理モデルからの実会議候補生成と二つの失敗

上記の導入検証に続き、同じQAの標準構成版#3から既存30秒QA会議の解析を明示開始した。音声の追加取得／録音なし。meeting `b95b2543-14a6-4e49-ac70-03418516aa56` のASR job `40f9c6ce-063e-43b7-b2e8-f4a791b07d53` はsucceeded・started〜finished差3,086ms、解析候補#5 `80bcfcc2-4c09-4767-a2d1-9df6fa0de0de`。続く話者分離 job `56b17700-1937-4d14-877e-c7ba2a339708` はsucceeded・5,507ms、候補#7 `e288e407-668f-4dbc-af06-d4af6a7ce36b`。両jobの固定構成版は#3、管理コピーのモデルを使用し、候補は自動採用しない。

議事録の通常画面から明示再生成した job `3fd185e4-da3d-44e1-805c-0d73540d13ab` はfailed・`minutes_generation_failed`・54,477ms、output_minutes_idなし。同じ構成版#3と管理llamaを使用したが、入力は採用済み文字起こしであり新しい未採用候補ではない。CLIのresult.jsonは4,731bytes、文字列の途中で終わる不完全JSONで、厳格parserが拒否した。出力上限2,048tokensによる打切りは推定であり、token終端原因の直接ログ証拠はない。破損JSONの補修／抜き出し採用、根拠照合の緩和、繰り返し再実行は行わない。失敗unitのprompt／result／schema／stderrはQA内に保持し、私的会議本文をGitへ入れない。この実行は議事録生成成功や全3段統合成功とは扱わない。

別にQA本体PID67364が02:25:45JSTに異常終了した。`/Users/skodani/Library/Logs/DiagnosticReports/mon-desktop-2026-10-11-022547.ips` はSIGBUS・FS pagein EINVAL、32KiB mapped-file領域での`walFindFrame`、minutes::read_content→minutes_drafts::load→get_minutes_versionsを記録。生成JSONの拒否とは異なるDB読み込みの失敗。再起動後の整合性はok・外部キー0行、会議8／ジョブ9（成功8・失敗1）／議事録2 committedを保持。working_drafts＋export_records全行SHA `c73af0f0de54e66787ecaf57b96fd6ff37fad5f125b1073ff15d192be2bd309c`、committed解析／議事録全行SHA `8a1d80c720c6f32f27282231a38f21dd95c744d29213eae49d14a34754cdb620`、会議のid／title／現在・直前head連結SHA `6a04f298df7bfa0f91ff28c1f097085bd72acb8b871fff48b5548c4d83b36d78` は新規解析開始前と一致。全meetings／jobsテーブル不変とは主張しない。bootstrap SHAも既存値を保持した。

検査中に使用したSQLite `-readonly` 接続は業務SQLの更新をしないが、WALのSHM作成／初期化は可能であり、ファイルシステムへの完全な無書込み操作ではない。[SQLite WALのread-only条件](https://sqlite.org/wal.html#read_only_databases)と[同じファイルのcloseでPOSIX lockが解除される制約](https://sqlite.org/howtocorrupt.html#posix_advisory_locks_canceled_by_a_separate_thread_doing_close_)を踏まえ、今後の外部SQL検査はQA停止確認後に限る。クラッシュ原因の特定・修正／再検証は次工程で、元DB／WAL／SHMは手動削除・縮小しない。

## WAL起動順序と制御ファイルのロック保持修正

同梱SQLiteはlibsqlite3-sys 0.38.2の3.53.2。クラッシュ記録と起動経路を点検すると、既存WALへアクセスするjournal_mode設定の後にEXCLUSIVEへ切り替えていた。[公式WAL手順](https://sqlite.org/wal.html#use_of_wal_without_shared_memory)に合わせ、最初のDBアクセスより前にEXCLUSIVEを設定する。WAL／foreign_keys／synchronous=FULLは維持し、mmap_size変更や同期緩和、元DB／WAL／SHMの削除は行わない。同梱walIndexPageReallocも、HEAPMEMORY_MODEだけがヒープ上のwal-indexを使うことを確認した。

別にRootAnchor::verifyがseal済み制御ファイルを再openしてcloseしていた。同じinodeの別descriptorをcloseするとプロセスのPOSIX lockを解除するため、SQLiteのSHM dead-man-switchを壊し得る。保持descriptorのmetadataとパスの実体照合だけへ変更し、制御ファイルの再openを除去。inode・親・共有リンク・既知／未知ファイルの有無の拒否は残す。公式資料の制約と同梱unixLockSharedMemoryのDMS検査・無ロック時のSHM初期化を照合した。ただし観測済みSIGBUSとの正確な因果関係や実機での解消は、ここでは確定しない。

合成テストを追加：大きな本文（overflow pages）の3回再起動保持・SHM未作成・FULL維持・別プロセスの読み取り拒否、保持descriptorのPOSIX lockを別プロセスで確認して3回実体検査後も維持、制御ファイルの置換／symlink／hardlink／未知journalの拒否。別プロセスの補助テストは通常ignoredで、親からの実行成功に加え「1 passed; 0 failed」を要求し、名前違いによる0件成功を除く。実音声・私的本文はfixtureへ入れていない。

修正前の再起動回帰は1件失敗し、初回はSHMなし・既存WAL再open時はSHMありを再現した（意図的なSIGBUSは起こしていない）。この旧ソースの構築369.70秒、libtest本体0.14秒、runner実行7.76秒（libtest本体とは別の工程範囲）。安全なキャッシュ回収4,212.0MiB、保持8,239.0MiB。その後の全体構築は追加ロック検査のSEEK_SET型（Macのi16）に2件エラー、34.69秒・テスト未実行で終了。テスト側をc_shortへ明示変換して同じ共有キャッシュで再実行中。これらを修正後の合格に含めない。Mac QAの保持バンドルはこのRust修正をまだ含まない。CI・コミット／Pushなし。

型修正後の全対象ビルドは終了し、Cargo表示11分17秒。binは0件成功、libの通常全体（専用補助のignoredも含む列挙1,898件）は実行中。通常テストはまだ終端未確認、最終runnerの正確な工程時間・終了後キャッシュ予算・doc testは結果待ち。検証子の実稼働を確認し、重いQAビルドを重ねていない。外部SQLのQA検査や実モデル再実行も行っていない。ソース点検では通常run_unitの出力制約をそのまま維持し、固定CPU試行やmaxItems=1の合成プロトコル試験から、通常の議事録生成の成功を推測しない。

全体実行中に、同じ構築済み実行ファイルから追加3件を直接exact／1threadで先行実行（追加ビルド／別cache／cache回収なし）。2件成功、再起動1件はSHM未作成の検査を通過後、locking_modeの観測に失敗した。`PRAGMA locking_mode`は接続のdefaultを返し、`PRAGMA main.locking_mode`の設定値とは異なることを同梱SQLiteのPragTyp_LOCKING_MODEと[公式PRAGMA説明](https://sqlite.org/pragma.html#pragma_locking_mode)で確認し、検査をmain指定へ修正。成功条件を除去せず主DBのEXCLUSIVE確認を維持した。先行3件は本体0.18秒、全体の新規件数には加算しない。進行中の全体実行は修正前の観測SQLを含む構築物であり、最新テストの成功には扱わない。製品コードは先行検査から変更なし、外部読み取り拒否まで含む訂正後の再検証は結果待ち。

全体検査ではモデル取得の合成HTTP8件も失敗。既存Server::newの127.0.0.1:0 bindがsandboxでEPERMとなり、同じ構築物のredirect1件を単独実行してPermissionDenied（tests.rs:790）を確認した。製品のHTTPS制約やテストを緩めず、許可された一時ループバック通信として同じexact8件だけを実行し、8成功・失敗0（libtest1.53秒）。新規取得・私的データ通信・モデル実行・CIなし。これは全件成功ではなく環境差の分離証拠、単独先行の重複件数を全体へ加算しない。

訂正後の再起動回帰は同じ共有ランナーへ待機させ、全体実行のcache lock解放後にだけ構築・実行する。主DBのmode／FULL確認を残し、外部読み取り拒否後のSHM未作成とRootAnchor再照合も追加。先行実行の2成功／1失敗やHTTP8成功を、訂正後の再起動テストやMac実機の合格とはしない。

全体ランナーは終了コード101で終端確認。bin0件成功、libは1,810成功／9失敗／79ignored・libtest804.47秒。9失敗の全stderrは上記HTTP bind EPERM8件と訂正前のmode観測1件で、他の失敗はない。新規のPOSIX lock保持／置換・リンク等拒否2件も全体の中で成功。build677.87秒、runner実行812.32秒（libtest本体と重複する工程で別加算しない）。失敗によりdoc testは未実行、通常全体成功とはしない。これは製品修正後・観測SQL訂正前のテスト構築物の結果である。

主DB mode参照を訂正し、外部reader拒否後のSHM不在とanchor保持を追加した同じ重点ランナーは終了0。1成功・0失敗・0ignored・1,897filtered、libtest0.09秒。build354.50秒、runner実行7.75秒、再生成可能cache4,215.3MiB回収・保持8,270.1MiB。主DBのEXCLUSIVE／FULL、3回の再起動で大きな合成行の保持、外部reader拒否後もSHMを作らない検査が成功した。最新全体・doc test・Mac実機でのSIGBUS解消は引き続き未確認で、全体の9失敗を単一のgreen runへ置き換えない。

全体終了時に所有付きcache lock下で5,129.4MiBの古い再生成可能なcacheを回収し、8,296.8MiB（約8.10GiB）を保持。元音声・モデル・会議・保持QA・製品targetは回収対象ではない。待機中の訂正後再起動検査はその後にlockを取得し、同じcacheで再構築へ移った。新しい合格／最終容量／実機解消は引き続き結果待ち。重いQA構築・録音・TCC変更・新規取得・CI・コミット／Pushなし。
