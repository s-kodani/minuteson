# Mac標準終了要求を保存待ちへ接続

2026-10-12。Dock等が使うAppKitの標準終了要求を、既存の保存・録音停止確認・処理終了待ちへ接続した。非録音の検証用QAでは、標準の `terminate:` メニューによる未確定発言の保存後終了、入力検証エラーでの終了拒否、修復後の正常終了を確認した。Dockのアイコン操作そのものとOSログアウト／再起動は未検証。実録音の再試験はユーザー指定で保留する。

## 終了経路と修正

前回の[Cmd＋Qと独自メニューの補修](macos-guarded-quit-2026-10-12.md)は、TauriのExitRequestedへ送る独自項目だけを対象にしていた。現在のtao 0.37.1のdelegateには `applicationWillTerminate:` があるが `applicationShouldTerminate:` はなく、NSApplicationの標準終了はその保存バリアを通らない。Appleも、アプリ全体の終了とウィンドウのcloseを別経路として説明している。[Appleの終了処理の説明](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/AppArchitecture/Tasks/GracefulAppTermination.html)。

`macos_menu.rs`のメインスレッドの初期化で、Taoが所有する同じdelegateインスタンスを、追加フィールドのない直系サブクラスへ切り替える。親クラス名・継承関係・インスタンスサイズを検査し、既存の終了方針があれば上書きせず起動エラーにする。起動・再表示・URL・解放の既存処理と標準メニューを継承し、NSApplicationやNSObjectのグローバルメソッドは変更しない。

新しい `applicationShouldTerminate:` は `NSTerminateCancel` を返し、Tauriの終了イベントを非同期に送る。同期コールバック内でロック・WebView・保存・ヘルパー停止を待たず、既存のExitGuardとfinish_main_closeだけが最終終了を許可する。録音開始停止・録音停止確認・保存トークン・起動拒否条件は変更していない。OSログアウト等の要求も一度取り消す方式なので、OS全体の終了継続や再要求の必要性は別の残件であり、今回のアプリ終了成功から保証しない。

## 自動検査とQA構築

Rustの追加3件は、テスト専用インスタンスのサイズ・フィールドと既存メソッドの保持、実Objective-Cメッセージによる取消応答、既存終了方針と重複クラスの拒否を検査して成功した。既存の終了条件2件も成功。最終フロント単体878件が成功、失敗・skip0、本体1.843684959秒。ソース契約は2件追加し、非同期接続・取消・レイアウト検査と既存終了バリアの保持を確認した。Rust整形・差分空白検査成功。通常Rust全体、実録音、CIは実行していない。

最終ソースのQA構築は124.63秒、Vite1.72秒、Swiftのキャッシュ確認0.24秒、Rust release1分52秒。成果物は `/private/tmp/mon-dock-guard-qa.pAgmGU/MON QA.app`、メインSHA-256は `0564c49d98b4fb47000f1e81f57d017813a615853515ba8f9b25b6e5c92c85ba`、40.06MiB。deep・strict署名検査成功。公開用Developer ID署名・公証ではなくadhoc QAである。

Dockへの画面操作が取得できなかったため、終了項目だけを一時的にPredefinedMenuItem::quitへ変えた検証用QAを別途構築し、AXでもメニューの `terminate:` を確認した。構築123.01秒、Vite1.65秒、Swift0.22秒、Rust release1分51秒。成果物は `/private/tmp/mon-native-quit-probe.S9LJSI/MON QA.app`、メインSHAは `3b45b0168496ec762ad170caed61d9f94430d773e53ebf1d0a0ed5193c858566`、40.02MiB、deep・strict署名検査成功。この一時的な項目変更は戻し、最終ソースには独自Cmd＋Qを保持する。終了経路を実際に呼ぶ検証と、Dock操作の検証を区別する。製品に検証用メニューや新しいボタンは追加しない。

## 限定実機結果とデータ保持

編集対象は追加検証会議 `61d49fa3-431f-411c-a036-1bdc5d3e784a` のみ。実録音・音声再生・OS許可変更・入力ソース切替・VoiceOverなし。

| 操作 | 結果 |
|---|---|
| 最終ソースQAの起動・会議／議事録表示・独自Cmd＋Q | 画面応答と通常終了。停止前後の全DB dump一致 |
| 検証用QAの発言00:00:04へ未確定本文を入力し、標準Cmd＋Q | 下書き版3→4、編集操作1件追加、回復入力0件。回復記録だけでなく本文を確定保存して終了 |
| 冷起動・発言を再編集 | 保存した本文を表示。通常操作で試験前の本文へ復元し確定 |
| 概要の検証用項目を全削除し、標準Cmd＋Qとterminateメニューを続けて実行 | 空本文の保存検証エラーで両方を拒否。空欄・フォーカスと画面応答を保持 |
| Cmd＋Zで本文・太字・改行・固定根拠を復元 | 保存済みへ復帰し、標準終了可能 |
| 最終ソースQAで冷起動・会議／議事録表示・独自Cmd＋Q | 復元内容と根拠を表示。冷起動前後の全DB dump一致 |

QAとヘルパーの停止を確認してからDBと存在するWAL／SHMをコピーし、コピーだけを読み取り専用で比較した。全コピーの整合性ok、外部キー違反0、14会議・46表を保持。開始前 `/private/tmp/mon-dock-before.Xi5VZv/meetings.sqlite3` と最終ソースの初回smoke `mon-dock-final-smoke.YVZNTa` は全dump SHA `e79f03b2ef0264497f8a01c0356d5ececc0f043384317b68198cb9865ac15181` で一致。標準終了保存後 `mon-native-quit-saved.YCfsVJ` は `7af251d54d1d64a1b61e82d1363e8317d2b93011becaf3ab079bbce6a51a1234`。復元後 `mon-native-quit-restored.x7pSVQ` と最終冷起動後 `mon-native-quit-final-cold.0peTBS` は `c9d1dc7767bea9ad9f279ebfa7de42cb264ae8fb3b838da270909da7392a5af1` で一致。全て `/private/tmp/` 配下の停止コピー。

試験前と文字起こし／議事録のpayload_jsonとcontent_hashが完全一致。議事録は版44のまま、文字起こしだけ版3→5、回復記録と検証用編集操作2件が差分。元の13会議の46表全行、設定、生成原文、固定根拠、元の不具合manifestを保持し、データを削除していない。

終了拒否画像 `/private/tmp/mon-native-quit-save-error-2026-10-12.png` のSHAは `e008fc6fb9a51f7deba82418804c8e17c43f60c642e60a438bc095410a945619`。最終冷起動表示 `/private/tmp/mon-native-quit-final-verified-2026-10-12.png` は `cc9f0266bedc15f581e06dedc4059f5448efc97b0684ab1b5e8f165c97811939`。構築ログは `mon-dock-qa-build-2026-10-12.log` と `mon-dock-native-probe-build-2026-10-12.log`、最終フロント単体は `mon-dock-units-final-2026-10-12.log`、Rustは `mon-dock-rust-final-2026-10-12.log` と `mon-dock-close-rust-2026-10-12.log`。ログも `/private/tmp/` 配下。

## 工程時間と残件

調査・実装・契約検査追加は作業時間約10分、QA構築2回合計247.64秒。Rust初回の型推論エラーは18.17秒で停止し、型指定を補修。再構築84.42秒と関連条件の構築61.26秒、テスト実行は起動を含め7.88秒と7.72秒。フロントは初回2.858199917秒と最終1.843684959秒。非録音実機と停止コピーの照合は約7分、記録更新は別途約3分。工程は並行するため単純合算しない。

Dock取得の初回応答待ちが約8時間6分の予期しない停止になった。これは実装・ビルド時間に含めず、以後の画面操作に10秒の制限時間を指定した。Dockの別指定も取得できず、QA自体は応答し、強制終了せず通常終了した。検証用QA構築のMenuItem未使用警告とRustリンクの大きなunwind警告は記録に残す。最終ソースに未使用importはない。

Rustキャッシュは既存の容量制限付き領域を再利用。2回の構築後に再生成可能な古い増分状態4141.5MiB／4120.2MiBを自動回収し、終了後8656.1MiB。新しいRust targetは作らず、QA targetは既存の2224MiBを再利用。実行中の一時的増加と終了後の容量は区別する。録音・モデル・QA証拠は回収していない。Whisper素材の追加通信0bytes、配布の12件のライセンス文不足と未承認状態は残す。

Dockの実アイコン操作、OS全体の終了継続、録音中の停止確認／保存／ヘルパー終了、容量不足・保存競合・応答喪失を残す。M3-02・M3-08と全23タスクを完了にはしない。非録音UI残件6〜12時間、承認後の短時間録音30〜60分とPC音声基礎統合1〜3時間、4時間耐久と異常検証5〜8時間、署名素材入手後の配布4〜8時間、協力者試用準備1〜3時間を目安とし、外部準備の待ち時間は別。正式品質評価・10話者・Windowsは保留、VoiceOverと旧形式バックアップは対象外。
