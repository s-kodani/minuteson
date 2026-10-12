# 議事録の保存応答喪失と明示再試行

2026-10-12。基準 `0fd833ab835f524420e0b688e0ddb51139d03fcd`。M3-04／M3-08、仕様R-17／UX-10の限定修正。実録音の再試験はユーザーの指示で保留し、5秒テスト・4時間録音・OS許可変更・CIは行わない。

## 修正した保存処理

DB保存が完了しても画面へ成功応答が届かなければ、従来のキューは古い版番号を保持した。明示再試行はその番号で再送して競合し、応答喪失後に入力を前の内容へ戻すと未保存判定まで消える問題があった。

失敗した保存の基準版と送信本文を、最新のローカル入力とは別に保持する。失敗後の自動再送・自動読戻しは行わない。明示再試行だけが現在の有効な下書きを読み、会議・基底版・下書きIDを照合する。版番号が同じで本文も基準版と一致する場合は未完了の保存として最新入力を保存する。直後の1版だけ進み、全文が送信本文と一致する場合は保存済みとして照合し、その後の追記だけを新しい版番号で保存する。

それ以外の変更・下書き消失・読取り失敗では入力と保存要確認を保持し、上書きしない。既存のRust側の版照合・本文検証・固定根拠検証は変更していない。照合後と再保存の間に別の保存が入っても、Rustの版照合で拒否される。過去版の下書きを読んで現在版と誤認しないため、取得時の議事録IDはnullを指定する。これは保存操作の受付記録を新設する方式ではなく、現在の版と本文の厳密な一致を確認する方式である。

## 自動検査と画面確認

新規16件を含むフロント894件成功、失敗・skipなし。最終runner2.1905595秒、起動込み2.37秒。追加検査は保存完了後の応答喪失、追加入力、入力を元へ戻す場合、重複書込み防止、会議・基底版・ID・本文・版番号不一致、消失、退役した基底版、照合読取り失敗、照合中の入力と再試行競合、照合後の他画面更新を含む。固定根拠と生成原文、編集履歴を保護する。

独立したブラウザーセッションで製品React・製品キューを使い、合成IPCで実際に模擬保存を完了してから1回だけ応答失敗を返した。画面確認4条件は追記の保存、元の入力への戻し、追記なしの照合だけ、別画面の更新拒否。保存要求は計6件で、最後の競合後の再試行は書込みなし。入力欄を交換せず、明示再試行の成功後に編集終了入口へ焦点が戻ることも確認した。検証セッションは終了し、ユーザーの既存タブは変更していない。

最終画面確認2.11秒。これは合成状態での試験であり、Mac実DBの保存成功後の実通信切断ではない。

| 画像 | SHA-256 |
|---|---|
| `output/playwright/minutes-response-20261012/minutes-response-retry-success.png` | `18c340c04bd691522dd4a58bd6acad871d139ccdfa366c2ec3f7582b970d69a9` |
| `output/playwright/minutes-response-20261012/minutes-response-conflict-preserved.png` | `58b18f67c24524d4ef4cfc2feebd05b718a07d8a1dcbd0a471825a5fd4fd534c` |

## 工程時間と再試行

型検査とViteの最終ビルド7.97秒（Vite1.74秒）。先行ビルド6.05秒。初回の関連テストは拡張子なしのimportで起動に失敗、次はVM由来配列のprototype差で1件失敗し、importとテスト側の配列生成を修正した。最初の画面確認でstructuredCloneをキューのmethodとして呼び出すとブラウザーがIllegal invocationを返したため、元と同じ関数wrapperへ修正した。Node単体だけでは捉えなかった実行環境差である。

画面driverの起動は最初に関数形式を誤り、続いてCLI内のimport／requireが利用できず失敗した。CLIの対応するfilename指定へ変更して成功し、最終ソースでも4条件を再実行した。画面の最初のfavicon404とReact開発案内は製品処理の失敗とは分ける。

QAは先行120.50秒、現在版の照合を含む最終ソース119.74秒、計240.24秒。最終Vite1.52秒／Swift0.14秒／Rust1分49秒、40.06MiB。途中で現在版の照合を追加したため先行QAを実機確認には使わず再構築した。既存のQAビルド領域を再利用し、新しいRust targetは作成していない。

Rust側の保存・版競合・履歴・固定根拠の関連8件が成功。最終はキャッシュを再利用して構築0.59秒、実行0.62秒（テスト本文0.60秒）、起動込み1.48秒。初回はfilter名を誤って0件になったため合格証拠から除外する。この初回に構築68.08秒・実行8.11秒・全体76.94秒を要した。Rust製品ソースは変更していないが、フロントの組込み資産変更でテスト用buildも更新された。再構築可能な旧cache4119.3MiBを容量制限付きrunnerが回収し、終了時cache8646.1MiB、上限10240MiBを維持した。構築中の一時使用量は終了時容量とは別である。QA targetは2224MiB、ネットワーク追加取得0bytes。

実装・回帰追加は約6分、ブラウザーの画面操作と起動方法の修正は約3分、Mac通常経路約1分、保存照合約1分、文書整理約3分の概算で一部並行。build・testのrunner時間とは単純合算しない。Sonnerのdirective無視／bundleサイズ、Rustテストlinkerのunwind警告、notice本文不足12件／未承認、公証未実行は残る。

## Mac通常経路とデータ保持

最終QA `/private/tmp/mon-minutes-response-qa.3y8i4k/MON QA.app` のdeep／strict署名照合を通過後、既存の自作会議だけを開いた。主実行ファイルSHA-256 `2ad3de7c387cb99534caebf642f48daaad0ef6283d0e5ae9911cb34d2def3dd9`。アドホックQAであり、正式署名・公証済み配布ではない。

製品の議事録表示→編集開始で見出し・長文・表・太字／斜体・固定根拠を確認し、本文を変更せず編集終了。入口の焦点復帰・保存済みを確認してCmd＋Qで正常終了した。これはMac上の通常経路の確認で、実通信応答喪失の再現ではない。録音・音声再生・日本語入力ソース変更・VoiceOverは行っていない。

QA本体・録音ヘルパーの停止を確認してDBと存在するWAL／SHMを別ディレクトリへ複製し、その複製だけをreadonlyで照合した。開始前 `/private/tmp/mon-minutes-response-before.sz9K6r/meetings.sqlite3` と終了後 `/private/tmp/mon-minutes-response-after.pCbXzu/meetings.sqlite3` の全論理dumpが一致。SHA-256 `c9d1dc7767bea9ad9f279ebfa7de42cb264ae8fb3b838da270909da7392a5af1`、integrity ok、FK違反0、14会議46表。既存13会議を含む全行を保持し、元DBへのSQL書込みはない。bootstrap hash `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` と前回失敗録音manifest hash `6ad5ea9f3d52b5639dd43f4951b0830823f2a028df7b263d57b3992ddba66220` も不変。既存録音とモデルは削除していない。

通常経路画像 `/private/tmp/mon-minutes-response-native-20261012.png`、SHA-256 `8a8b752be3952eacb351a04ca8f6f4c2c34d576d7919d31860a7c0107f37968a`。

| 最終ログ | SHA-256 |
|---|---|
| `/private/tmp/mon-minutes-response-tests-complete-20261012.log` | `b2ecb180407f9d50386afc799e3f0a3769b84de6398eae5c5a278840d584b5cd` |
| `/private/tmp/mon-minutes-response-build-final-20261012.log` | `5d924db6e804428264e9a9c795a9ae274da39efa81a31fb92f3d293dacca14ce` |
| `/private/tmp/mon-minutes-response-browser-final-20261012.log` | `9031db93c75b3f8f824bca708608ceaddf6aa2be3e94478495d92e2cf5dd97d2` |
| `/private/tmp/mon-minutes-response-qa-build-final-20261012.log` | `165b562bc9580b104c0a00be4791828ffb1bffc3fba1f301f6fffb07da766c8d` |
| `/private/tmp/mon-minutes-response-rust-final-20261012.log` | `358fee209481c6a8ed2156419475313f041a4daf6a1e63574e0ca0ebe9220d5b` |

## 残る検証

実Macの通信応答喪失、容量不足、複数プロセスの同時保存、全編集形態の異常順序は残る。今回の合成試験をM3-04／M3-08の全条件や[全23タスクの完了監査](implementation-completion-audit-2026-10-11.md)の完了に拡大しない。

残工数の暫定目安は非録音のUI・編集・異常経路6〜12時間、許可後の短時間録音30〜60分、PC音声の基本確認1〜3時間、4時間録音と異常経路5〜8時間、署名資料準備後の配布4〜8時間、先行試用の準備1〜3時間。録音・正式配布・協力者調整の待機時間は別。正式評価・Windowsは保留、VoiceOver・旧形式互換は対象外を維持する。
