# マイク再取得中の形式変更診断の保持

- 日付: 2026-10-10
- 対象: M3-01の入力形式変更、M3-08の原因表示。画面・設定項目を追加しない。
- 状態: 製品制御と合成検査を修正。実AVAudioEngine・機器変更・WebView・4時間録音は未検証。

## 原因と修正

通常録音のマイクは、異なる形式のPCMを保存せず、同じUIDと元の完全なprocessing formatだけで再取得する。ところが`maintain`はエンジンを解放する際、既存の`format_changed`を`engine_changed`・disconnectedへ置き換えていた。次の取得で形式不一致が再確認されるまで、原因とunavailableの表示が失われる。スリープ後の`clock_changed`は既に保持していた。

制御ループの再試行による中断を`MicrophoneRecovery.interruptForRetry`へ分離した。`format_changed`はunavailableのまま、`clock_changed`は元の理由のまま保持する。それ以外の不健康なエンジンは従来どおり`engine_changed`として無効化する。既存のlock内で入力unit・device・levelを無効化し、tap解除・engine停止を行う。再試行だけを新しい診断として扱わない。

新しい機器情報・権限・取得結果からの`interrupt`は引き続き理由を更新できる。形式変更を永久に固定せず、同じUID・元形式の再取得と新bindingのactivationで理由を解消する。未証明のbinding、旧callback、停止後の再開は認めない。別マイクへの切替、リサンプリング、CAFの再作成、録音許可、成功した取得閉鎖の生成は追加しない。PC音声の既存の形式blockは変更しない。

## 検査

旧`maintain`と同じ分岐をまず新しい政策入口へ移し、新規回帰を実行した。重点12件のうち新規2件と既存の形式保持試験で106アサーションが失敗した。106件のテストを追加した意味ではない。政策を修正後、全体を再検査した。

- 新規2件: 形式／時計中断中の100回の再試行で原因・状態・中断数・履歴を保持。新binding未確定中の再失敗も原因を失わず、古いactivationを拒否し、証明された新bindingだけで復帰する。停止後は変更せず、通常の不健康なエンジンの引退と新たな権限異常の診断更新も確認。
- 既存の実CAF／raw prefix試験を拡張: レート・チャンネル数・整数表現・interleavingの4種類を拒否し、再試行後も形式原因を保持。拒否したPCMで時計・frames・callbacks・appendFailures・原音声tip／entry／確定PCMを変更しない。同じ元形式の新bindingだけで再開し、旧callbackを拒否。最終42,000frames・5時計区間、実読戻しPCMと欠損を照合する。合成ハーネスであり実マイクではない。
- 最終Swift全体164件中161件成功・既存専用3件skip、別のSwift Testing1件成功。新規2件は全体の内数。既存のPC音声・時計復帰・容量／raw writer検査も含む。
- 既存`make check-microphone-recovery`はSwift合成CAF生成1件とRustのDB確定・PCM／欠損・他入力保持の専用1件が成功。合成fixtureは実デバイスを使わない。Rustの通常全件は再実行していない。
- 変更3ファイルのSwift書式検査と`git diff --check`が成功。製品Rust・React・Pythonは未変更で、全ブラウザ・GUI・正式評価を実行したとはしない。

## 固定したソース

| 対象 | SHA-256 |
|---|---|
| 製品App.swift | `6693b6610c4e8a90660df2eb7874cc4de08ab077a2f1946333663079dfc34c4a` |
| MicrophoneRecovery.swift | `ffe7a0a89d8c0fb4720bf31e606ddb437e5db037c3a0468220d4eb791e693a4e` |
| MicrophoneRecoveryTests.swift | `200bb9b27c31531f9fb8723e5e65a14d06be4d31c7b97ba904b27c109970fb5c` |

## 構築・時間・残対応

最終QAは`/private/tmp/mon-mic-retry-qa-artifact.Kngd86/MON QA.app`、37,776KiB（約36.9MiB）。全コピー一致、QA識別子`jp.mon.desktop.qa`、deep／strictアドホック署名、全実行ファイルarm64、同梱CLIのhelp、配布用通知文の照合が成功した。275依存・不足12件・`collected_not_reviewed`の状態は変えず、配布承認にはしない。

| 同梱実行ファイル | SHA-256 |
|---|---|
| main | `f19688e96ca9991518d034b95cd14faca5bc47168b2f8a5f5e8866ab4b35373c` |
| 録音ヘルパー | `710db4329492486d03431ca61fb9798626549ca3305b6a4abf294ea7c8c3a1e1` |
| 話者特徴量ヘルパー | `d9b217f5ebaced3957ebf77b3e35874fc21be06cf474eab0f550c313a21ba8a9` |

実装・調査・検査追加／拡張・記録は約5〜10分の概算。構築はQA134.49秒、Rustラッパー68.27秒、直接Swift構築4.83／1.10／2.01／0.20秒、参考小計210.90秒（3分30.90秒）。QA内Swift release12.00秒・Vite1.47秒と内側の型検査・Rustを再加算しない。Swift直接構築は各`Build complete`の時間で、コマンド起動・計画等は未分離。

検査参考小計42.60秒はSwift XCTest0.228／17.115／17.497／0.015秒、別Swift Testing最終2回の各0.001秒、Rustラッパー実行7.74秒。106アサーションの失敗・重点再実行を含む。通常Rust全件の実行時間ではない。Rust専用本体0.07秒は同ラッパー内数。Swiftの全体コマンドは6.43／18.98／20.67秒、`make check-microphone-recovery`全体は77.90秒で、構築と検査小計へ再加算しない。書式検査0.27／0.26秒は別記。help・コピー・署名・SHA・通知文・Git等は小計外。工程は並行し、小計の和を経過時間にしない。

構築・Rust検査の両sessionの正常終了、QAコピー／署名、専用targetのprocess一覧が空であること、実体ID16777234／170359446／uid501と非リンクを確認後、今回だけの`/private/tmp/mon-mic-retry-qa-build.NlTZLi`約1.18GiBを回収した。再構築可能。QA・過去QA・モデル・音声・会議データは保持する。共有Rustは古い再生成可能なキャッシュ4,096.4MiBをラッパーが回収し、終了後10,110.0MiB（約9.87GiB）、製品target38,668KiB（約38MiB）を保持する。

ログは`/private/tmp/mon-microphone-retry-red.log`、`mon-microphone-retry-swift-final.log`、`mon-microphone-retry-swift-expanded.log`、`mon-mic-retry-roundtrip.log`、`mon-mic-retry-qa-build.log`（すべて`/private/tmp/`配下）。最終の試験拡張は検査Swiftだけで、QAを構築した製品ソースは変更していない。実機の解除連絡・実モデル再取得の回答待ちは維持する。GUI・録音・TCC・実スリープ・長時間・公証の成功にはしない。Windows workflowは`disabled_manually`を確認し、実行・有効化していない。

入力形式の原因保持を修正しても、実入力変更・容量・取得／停止境界は残る。残工数は録音境界0.5〜1.5時間、旧形式／異常UIと実モデル3.5〜7.5時間、モデル配布と残12件／SDK等の条件確認1〜2.5時間、署名／公証／更新1.5〜3.5時間、Mac実機／長時間3.5〜8時間以上、先行試用1〜3時間以上、計11〜26時間以上の暫定概算を維持する。外部待ちは別、正式評価・10話者・Windows固有部分とCIは保留する。M3-01と計画全体を完了扱いにしない。
