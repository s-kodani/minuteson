# PC音声の旧出力解除を確認してから再取得する

- 対象: M3-01の入力再取得・形式変更・既存音声の保護
- 更新日: 2026-10-10
- 状態: 製品の解除失敗経路を修正。実SCStream・TCC・録音・長時間取得は未検証

## 不具合と修正

通常の再取得で使う`SystemAudioRecorder.retireStream`は、旧ストリームの`detach`が投げたエラーを`try?`で破棄し、所有を解除して次のfactoryへ進んでいた。旧ストリームの出力が解除されたという確認がないまま置換できた。時計復帰専用の解除処理では既にエラーを伝播していたが、通常の再取得・形式変更時のretirementにはこの不備が残っていた。

解除をthrowingのまま扱い、成功と受付済みcallbackのqueue排出を確認するまで旧ストリーム実体・tokenを保持する。失敗した通常再取得は既存の`restart_failed`・unavailableと同じ中断履歴へ接続し、新streamの作成・開始・connectedを許可しない。後続の所有付き再試行は保持している旧実体の解除から始める。同じ確認済み対象と新epochへの復帰、旧PCM／終了通知の拒否、既存CAFへの追記を維持する。

停止成功と出力解除成功は区別する。OS停止が正常終了した場合は、解除失敗でも停止済みの事実を保持して同じストリームの停止を重ねない。停止が投げた場合は停止済みにせず、解除／置換へ進まない。delegateが終了を確認済みのstreamにも再度停止を要求しない。形式変更では既存のformat blockを維持し、解除だけを再試行して新形式を取得しない。形式retirementの失敗直後に同じcleanupを重ねる経路も除いた。

原本を再作成・リサンプル・削除せず、保存済み音声や時計の拒否条件を緩めない。マイク、CoreAudio tap、IPC、React、DB形式、依存lock、Windowsは変更していない。実デバイスを使用せず、模擬入力を本番recorder／CAF writerへ渡して検査する。

## 再現と検査

修正前に新規2件を実行し、両方の失敗で不具合を再現した（7 assertion failures、unexpected failureなし）。解除エラー中でもfactoryが2回目に進み、次のsourceが開始してconnectedとなること、形式retirementの解除失敗が再試行されないことを確認した。

最初の製品修正後は通常の解除失敗の検査が成功したが、形式retirementがcatchのcleanupでもう一度解除を実行し、期待した1回の代わりに2回になる検査失敗が残った（2件中1件失敗、2 assertion failures）。検査条件を緩めず、形式retirement自身の重複cleanupを製品側から除いた。その後の重点2件は成功した。

さらに模擬sourceのasync `stop`にも実際のthrowを注入できるようにし、新規1件を追加した。最終Swift全体は162件中159件成功・既存専用fixture3件skip、別のSwift Testing1件成功。新規3件は全体の内数である。

| 新規検査 | 確認した範囲 |
| --- | --- |
| 旧出力解除失敗と同じ実体の再試行 | 解除失敗中はfactory／次source開始なし、unavailable／restart_failed、旧PCM拒否。解除成功後は新epoch3でconnected、旧終了通知も拒否。元の480framesと復帰後480framesの全左右PCMを実CAFから読戻し |
| 形式retirementの解除失敗 | 成功したOS停止を1回に維持、旧実体の解除だけを再試行、形式理由・format blockを維持。新sourceなし、CAF480frames保持、未解決のlegacy保存成功へ変換しない |
| 形式retirementの停止失敗 | 停止失敗を停止済みにせず、解除／factory／旧PCM受付をしない。録音終了／再試行でもclosureを作らず、CAF480frames保持 |

`make check-system-recovery check-capture-ending`は成功した。直接Swift生成1件と専用Rust3件で、通常の再取得・時計復帰・所有付き停止証拠の実CAF／時計／DB／再生の既存連携を確認した。Rustは既存バイナリを再利用し、今回の通常Rust全件は実行していない。フロント／全ブラウザも今回の変更には含まず再実行していない。

Swiftの共通cache readonly警告と、Rustの既存linker警告は残る。エラー終了した再現／途中検査と、最終の成功検査を分ける。検査失敗による原本削除・成功条件の緩和・実機動作の推測はしない。

## 構築・時間と保持物

最終製品ソースを含むQAを資格情報を除外した環境で構築し、所有する新規targetから`/private/tmp/mon-output-retirement-qa-artifact.0oNZjp/MON QA.app`へ全体をコピーした。全コピー一致、deep／strictアドホック署名、3実行ファイルのarm64、mainの`jp.mon.desktop.qa`・最低OS15.0、録音ヘルパー`app.mon.capture.probe`と音声取得の用途宣言を確認した。同梱録音CLIの`--help`は独立コマンドでも終了コード0。GUI・TCC・機器・録音へ進んでいない。

| 工程 | 累積参考時間 | 範囲 |
| --- | --- | --- |
| 実装・調査・検査追加／修正・記録 | 約5〜8分（概算） | 専用ストップウォッチではない。構築待ち全体を能動作業に加算しない |
| 直接Swift構築 | 11.02秒 | 再現2.65、途中2.96、最終重点2.89、全体2.31、連携生成0.21。QA／Rust内部は別加算しない |
| Rustラッパー構築 | 0.47秒 | 既存バイナリの再利用0.24＋0.23 |
| QA構築コマンド | 188.53秒（3分08.53秒） | 型検査・Vite・Swift releaseヘルパー・Rust release・バンドル・署名を内包。Swift内部10.64秒等を別加算しない |
| 構築参考小計 | 200.02秒（3分20.02秒） | 検査が失敗した先行回も含む。構築自体の失敗・中止はなし |
| 直接Swift検査本体 | 17.399秒 | 再現0.067、途中0.064、重点0.024、全体17.226、連携生成0.017、別枠Swift Testing1件0.001 |
| Rustラッパー実行 | 2.15秒 | 0.07＋2.08。一覧・起動・内部Swift fixture構築を含むので純本体ではない |
| 同梱helpと署名コマンド | 参考0.54秒 | help2回の表示0.52＋0.00、deep／strict署名0.02。0.00は計測表示の分解能以下であり厳密なゼロではない |
| 検査参考小計 | 約20.09秒 | 失敗・再試行込み。上記の本体とコマンド参考値を区別する |

並行実行・SwiftPM共通cache待ちと内部未分離を区別し、工程の和を実経過へ単純加算しない。コピー・plist・lipo・SHA・ディスク確認・書式・Gitは個別未計測。最終QAの長いRust構築では同じrustc PID21928が約398% CPUで実行中であることを読み取り専用で確認し、再起動／中止せず同じ実行の成功を確認した。今回のQA構築時間を通常Rustテストの再利用時間と混同しない。

| 同梱実行ファイル | SHA-256 |
| --- | --- |
| mon-desktop | `3004926ca2b10e69abd13addc2a96deb1e75e163de115682a0346daf19d5fd0c` |
| mon-macos-capture | `cf461e57b46cbdf06c526cf8a2a2ea310add1bcbaa58c27ccf04835d23c79920` |
| mon-speaker-embedding | `d9b217f5ebaced3957ebf77b3e35874fc21be06cf474eab0f550c313a21ba8a9` |

製品・検査ソース2ファイルのSHAは最終検査・QA構築後も一致した。アプリ35,000KiB（約34.18MiB）を保持し、構築プロセスの終了・コピー・署名照合後に、この実行だけの新規target`/private/tmp/mon-output-retirement-qa-build.6PKwxo`、1,236,792KiB（約1.18GiB）を回収した。非リンク・所有501・0700・実体IDを確認した、再生成可能な構築物である。録音・モデル・設定・既存QA／DMGを削除していない。共有Rust10,130.6MiB（約9.89GiB）、製品target約38MiB、取得Swift約342.63MiBを保持し、ユーザーのVite PID77750も存続を確認した。8〜10GiBは共有Rustの終了後目安で、専用QAや全cacheの合算・構築中のhard capではない。

このQAは実Developer ID署名・公証・Gatekeeper・実機の取得停止／解除／再接続・実モデルの成功を証明しない。以前の時計復帰QAも過去の検査物として保持し、今回の修正を含む最新物と区別する。ログは以下を保持する。

- `/private/tmp/mon-output-retirement-repro-v1.log`
- `/private/tmp/mon-output-retirement-focused-v1.log`
- `/private/tmp/mon-output-retirement-focused-v2.log`
- `/private/tmp/mon-output-retirement-swift-full-v1.log`
- `/private/tmp/mon-output-retirement-native-roundtrip-v1.log`
- `/private/tmp/mon-output-retirement-qa-build-v1.log`
- `/private/tmp/mon-output-retirement-qa-signature-v1.log`
- `/private/tmp/mon-output-retirement-bundled-help-v1.log`
- `/private/tmp/mon-output-retirement-bundled-help-v2-final.log`
- `/private/tmp/mon-output-retirement-source-v1.sha256`

## 残タスク

| 残対応 | 暫定能動工数 |
| --- | --- |
| 入力形式・容量不足等の残録音境界の突合 | 0.5〜1.5時間 |
| 旧形式／異常UI・保護の残確認と実モデル解析 | 3.5〜7.5時間 |
| 実配布元／HTTPS・CDN／モデル定義・ライセンス統合 | 0.5〜1.5時間 |
| 配布条件・実Developer ID署名／公証・実機手動更新 | 1.5〜3.5時間 |
| Mac製品の統合・実スリープ・長時間録音 | 4〜8時間以上 |
| 先行試用の準備・能動対応 | 1〜3時間以上 |
| 合計 | 11〜25時間以上 |

実機操作・モデル配置・署名資格情報・協力者等の準備待ちは別。正式評価・10話者性能・Windows固有部分とWindows CIは保留。今回の解除処理の検査だけでM3-01、Mac統合検証、計画全体を完了にはしない。
