# macOS配布の明示ビルド・ヘルパー別署名とDMG準備

仕様11章・M4-02の配布経路を追加した。[手順](../macos-distribution.md)から、明示証明書・期待Team・新規出力と、署名のみ／明示profile公証の排他的指定を行う。通常の開発／QAとWindows CIは変えない。前のgoalターンは、初回取得の画面・IPC接続をcommit・pushした進捗であり、今回も実装・実構築・DMG内の物理照合によって進めた。

## 実装と署名の境界

既存Macビルドは両ヘルパーが常にアドホック署名だった。準備はTauriでappだけを生成し、Appleの署名／公証環境変数を除外する。Developer ID署名は新規出力内のコピーだけへ内側から行う。録音のaudio-inputと、外部ライブラリを読む特徴量ヘルパーのdisable-library-validationを別のpolicyへ限定し、親は例外なし。実際の署名をstrict／deep、Apple anchor・Developer ID OID・期待Team・secure timestamp・runtimeビット・完全一致entitlementsで検証する。表示文字列の一致だけでは合格にしない。[Appleの公証条件](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)と[コード要件](https://developer.apple.com/library/archive/technotes/tn2206/_index.html)を参照した。

通常版の識別子・short/build版・最低OS15・実際の実行名`mon-desktop`・全3実行ファイルのarm64／実行mode・録音用途の文字列・両ヘルパーを検査する。release専用のInfo.plistを追加し、現在の開発／QAへ混ぜない。録音等の権限利用・GUI操作をビルドによって実行しない。

公証は既存profileの明示指定だけ。app ZIP提出→Accepted／正準要求ID→app ticket検査→DMG生成／署名→DMG提出／ticket検査の順序で、受理されたDMGを作り直さない。失敗・中断時は途中物を保全し、成功記録・自動再送・既存出力の上書きをしない。公開、アプリ置換、Gatekeeper／TCC／隔離属性の変更は含めない。

## 検査と修正

Pythonの最終全体は104件成功、新規18件は内数。フロントは627件成功、新規設定1件は内数。Pythonには明示モード・Teamと証明書・資格情報の子環境除外・事前拒否・新規出力・署名順序・要求式・最低権限・公証2段階・途中保全・正常終了後の新規キャッシュ回収・所有コマンドの取消／時間上限を含む。公証・Developer ID成功経路は合成コマンドであり、実証明書・サービスの合格ではない。

実構築前の静的確認で、メインの実行名はproduct名の`mon`ではなくCargo名の`mon-desktop`と既存バンドルから確認し、検査を実際の構成へ合わせた。通常版のrelease準備バンドルを、Appleの資格情報を除外してローカル構築した。113.32秒で成功し、`jp.mon.desktop`・0.1.0・macOS15・arm64・録音用途の日本語説明と両ヘルパーを確認した。内部の型／フロント／Rust／Swift／特徴量構築はこの時間の内数である。

コピー全体の一致を確認した後、今回専用コピーへアドホック・timestampなしのruntime署名を内側から付けた。最初の実メタデータ検査は、runtimeを`flags=0x10000`の表記に固定したため0.11秒で失敗した。実物は`0x10002(adhoc,runtime)`で、runtimeビットを確認する検査へ修正した。製品側はもともとビット判定であり、署名条件を緩めていない。

次のメタデータ検査は0.23秒で終了したが、直接のネイティブ要件検査で、inline要件をファイル名として扱う指定不備が判明した。この段階のエラーをDeveloper IDの正当な拒否証拠にしない。先頭`=`を付ける製品修正と回帰assertを追加した。固定ソースでは実際のcodesignが`test-requirement: code failed to satisfy specified code requirement(s)`・exit3でアドホックを拒否する。拒否基準を削除したり、文字列確認だけへ置き換えたりしない。

アドホックの検証用UDZO DMGは6.06秒で生成した。外側のアドホック署名・strict検査、コンテナのチェックサムVALIDを確認。readonly／nobrowseで今回専用の一時マウントを行い、内部appと元の全コピーが一致した。内部のレイアウト・実行mode・全3arm64・strict署名・runtimeビット・正確なpolicyとApplicationsリンクも照合した（0.27秒）。正しいinline要件による拒否を含む。マウントを解除し、同じfilesystemへ戻った空ディレクトリを確認した。最後の固定ソースで素材appも0.24秒で再検査した。

`hdiutil create/detach`には将来のdiskutil移行を促すdeprecated警告がある。現行環境で成功したが、警告を消すために最低macOS15での未検証コマンドへ置き換えない。codesignの古い`--entitlements :-`の警告は、実機で成功した`--entitlements - --xml`へ変更した。GUI・TCC・Gatekeeperやアプリ起動を伴う検査はしていない。

## 保存物と容量

`/private/tmp/mon-distribution-artifact.J6DQ62/mon.app`と`MON-ad-hoc-preparation.dmg`を保持した。合計46,748KiB（約45.65MiB）。通常版IDを持つ準備素材であり、配布物やGUI用QAとして起動しない。アプリのDeveloper ID署名・secure timestamp・公証は未実施。

- DMG SHA: `37727ecf2142a50adc5d976b0bd8337372ef409900a3ec144aed06cd1e90af1e`
- 本体SHA: `0186d6382cb58f345cd360fd1c5ef299421e88d21ede1126956d8a1d4fe60e94`
- 録音ヘルパーSHA: `1688969f23897151b40c7d2c382d34cc1cc65189f37d8618bc143ba33d3fad79`
- 特徴量ヘルパーSHA: `0c80cfdfd4dd7fe52b3256aab4e6c4d0220b4675549190e645405e749aa660bd`

終了済みの今回専用ビルド`/private/tmp/mon-distribution-preparation.7Cun5D`（1,237,360KiB）とDMG素材の重複コピー`/private/tmp/mon-distribution-image-source.brAkEX`（34,908KiB）だけを回収した。計約1.21GiBの再生成可能な状態であり、QA・音声・モデル・設定・ログは保持する。解除済みの空マウント先も削除した。共有Rust約9.90GiB・製品target38,664KiBを維持した。

最終ログは`/private/tmp/mon-distribution-python-tests-v6-final.log`、`mon-distribution-frontend-tests-v1.log`、`mon-distribution-preparation-build.log`、`mon-distribution-dmg-build.log`、`mon-distribution-dmg-verify.log`、`mon-distribution-mounted-check-final.log`、`mon-distribution-artifact-check-v3-final.log`（いずれも/private/tmp）。追加ソース9ファイルの固定SHAは`/private/tmp/mon-distribution-source.sha256`。前commitの製品Rust／Swift／Reactコードは変更せず、前回の固定14ファイルも一致した。Rust通常全件・Swift全件・ブラウザ全件は今回は再実行せず、実バンドル構築・Python全体・設定回帰とアドホックDMG内の実体を対象とした。

## commit単位の時間

実装・調査・修正・記録は約15〜25分の概算。能動作業の個別ストップウォッチ計測ではない。失敗・再試行を含む計測済み小計は次のとおり。

| 工程 | 参考小計 |
|---|---:|
| ビルド | 119.38秒（1分59.38秒）：準備app113.32秒＋DMG6.06秒 |
| テスト実行 | 68.44秒（1分08.44秒）：Python重点／全体64.405秒＋フロント本体1.269402125秒＋実物検査／マウント等2.77秒 |

Pythonの重点14／18件・全体100／104件の先行反復を新規件数へ重複加算しない。最後の全体104件の本体10.764秒（コマンド10.86秒）。フロント本体1.269402125秒（コマンド1.45秒）。内部構築は別加算せず、アドホック署名・単独の要件コマンド・コピー照合・SHA・容量・書式・Git操作等は個別未計測。並行工程と待機を含むため、工程の和を経過時間や性能値にしない。

## 残タスクと暫定工数

| 残タスク | 想定能動作業 |
|---|---:|
| 録音の形式・時計・回復境界 | 0.5〜1.5時間 |
| 旧形式／異常UI・保護と実モデル解析 | 3.5〜7.5時間 |
| 実配布元・HTTPS／CDN・定義／ライセンスの統合 | 0.5〜1.5時間 |
| 配布条件・実Developer ID／公証・手動更新 | 1.5〜3.5時間 |
| Mac実機の通常／異常統合・長時間 | 4〜8時間以上 |
| 少人数先行試用の準備・能動対応 | 1〜3時間以上 |
| 合計 | 11〜25時間以上 |

今回の配布工程実装を見積もりへ反映した。署名／公証成功を模擬応答やアドホック検査で完了にしない。profile資格情報・実モデル配置先・GUI操作許可・協力者等の外部準備待ちは別。正式評価・10話者性能・Windows固有部分／CIは保留。M4-02と計画全体は進行中とする。
