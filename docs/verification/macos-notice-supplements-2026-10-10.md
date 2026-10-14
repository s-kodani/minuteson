# macOS依存通知文の版固定補完とソース参照

## 2026-10-15 現行依存グラフの再照合

現行checkoutで `python3 tools/mon_third_party_notices.py --output-dir /private/tmp/mon-notice-audit-20261015` をオフライン実行した。依存一覧は379件、`missingNoticeTexts` と `referenceOnlyPackages` はともに12件で、対象は `block2 0.6.2`、`dispatch2 0.3.1`、`objc2 0.6.4`、`objc2-app-kit`・`objc2-core-foundation`・`objc2-core-graphics`・`objc2-foundation`・`objc2-io-surface`・`objc2-quartz-core`・`objc2-web-kit` の各0.3.2、`objc2-encode 4.1.0`、`objc2-exception-helper 0.1.1`。以下の旧時点の275件はその時点の実測であり、現在の依存件数として使わない。再生成された一覧の `licenseReview` は `not_verified`、`distributionReady` は `false` のまま。

補完indexは各crateの版・checksum・VCS commitに対応する上流 `LICENSE.md` とSPDXのMIT雛形を参照している。[固定commitの上流説明](https://github.com/madsmtm/objc2/blob/7b1abfd750a2cacaea71d6a56ecfb83cb7de560b/LICENSE.md)は、crate群のライセンス選択肢を示す一方、Apple SDK由来の生成物の配布条件に疑問を明記する。これは個別著作権表示の収集や配布条件の確認に代わらない。雛形の年・名義を推定して `license_text` に格上げしたり、不足12件を解消済みと扱ったりしない。配布前には各版に対応する通知・選択するライセンス文面とApple SDK由来の条件を確認する必要がある。MON本体・別導入モデル／実行ファイル、Developer ID署名・公証等の別条件も残る。今回の照合ではアプリの実行、録音、署名、公証、モデル取得、データ変更を行っていない。

## 2026-10-11 MIT参照本文のオフライン同梱

objc2の上流4commitの説明はMITを選択肢として示すが、12件の個別著作権通知の不足は解消していない。[SPDX v3.26.0のMIT template](https://github.com/spdx/license-list-data/blob/v3.26.0/text/MIT.txt)を参照用に追加した。HTTPS取得した原文と保存資料は全バイト一致、1087bytes／SHA256 `6d7c9e9d7d12c8d3753bdb24417b84819ba4d6c2f025c37948113d86dc81eef9`。著作権の年・名義placeholderを推測して置換しない。上流説明とApple SDKに関する注意も保持する。

`license_template`は`license_text`とは別の資料種別とし、生成通知に「個別著作権通知ではない」と明示する。既存のcrate版・checksum・VCS・path・原文SHA・UTF-8・容量・非リンク検査を維持し、templateはmissingNoticeTextsを解消しない。説明とtemplateを両方持つ場合の不足維持・改変拒否、およびtemplate単独でも未承認状態を保持する回帰2件を追加した。275依存（Cargo271／npm4）、template添付12、missing／referenceOnlyも12、警告12。`licenseReview: not_verified`／`distributionReady: false`を維持する。必要な個別通知・SDK由来条件・他の除外対象の条件確認は引き続き未完了。

Python全127件成功（runner11.265秒）、フロント727件成功（runner1.754秒）。QA構築はCargo表示1分18秒、Vite1.42秒、Swift0.14秒（内数、単純合算しない）。QAのコピー全一致・strict deepアドホック署名・identifier `jp.mon.desktop.qa`・main arm64、独立生成物との通知文／一覧byte一致、製品配布ツールのnotice検査成功を確認。更新前QAは新indexとの差異により拒否することも実確認した。公証・Developer ID署名の証拠ではない。

保持QA `/private/tmp/mon-notice-template-qa.Dp8N6r/MON QA.app` は57.13MiB、本体SHA256 `58b8149c75a9e400e306915cd7f6e8651366f64ad4a54fe31581ac4faa38a72e`。通知文2,527,554bytes／SHA256 `42aab1b9ea2d5f40ff2d49950acc2acdeaabe0cbaa9162661022726a48a31916`、一覧 `fdc0135499bf342184c11fa7547e4711871db5b62f7fe2886e76cb6163d5f9b2`、index `d28f031194354a42b4a97e2e6cf883ff225114abb23f0c6c8bd0d938c31d8ba7`。既存の専用非増分構築target約1.38GiBを再利用し、別の大容量targetは作らない。新QAは起動せず、UI・録音・モデル・会議データを変更しない。通常Rust／Swift全試験・GUIの再検証・CI・コミット／Pushなし。HTTPS取得のsandbox DNS拒否は承認された読取り実行で再試行し成功。資料原文の末尾空白は改変せず保持し、それ以外の今回変更の差分空白検査は成功。以下は先行検証の履歴。

- 日付: 2026-10-10
- 基準: M4-02、仕様11章。前回の`fb0bb41`を継続し、画面機能・録音・AIの動作は変更しない。
- 状態: 収集判定の修正、版固定の補完、実取得比較・QA同梱を確認。ライセンス全体の承認・配布完了ではない。

## 修正と補完

前回のファイル名判定は`LICENSES/MIT.txt`等を見落とし、`src/copying.rs`を通知文候補として採用していた。新規回帰を修正前に実行し、未検出と誤採用を再現した。通知文用ディレクトリ内を扱い、ソース拡張子を除外する。`LICENSE.MIT`・`LICENSE.BSD-3-Clause`等の本文を拡張子の限定で見落とさない。明示license-file、UTF-8・容量・リンク／外部参照の制限は維持する。

途中の新規試験の挿入位置により容量試験の`path`が別の試験へ混入し、NameErrorとなった。試験を正しい関数へ戻し、製品の制限を緩めず再検査した。拡張子の限定案でbrotliの正当な`LICENSE.MIT`等が除外された中間観測もあり、最終はソース拡張子の除外に修正して回帰に含めた。

`third-party/notices/index.json`は21件の対象版と9資料を紐付ける。各crateの申告ライセンス・Cargo.lockの配布checksum・`.cargo_vcs_info.json`のcommitとpathが一致する場合だけ使う。VCS metadataも非リンク・最大64KiBを要求する。資料の容量・SHA・UTF-8・非リンク・HTTPS出典も検査し、相違・欠落・改変を拒否する。通常ビルドは通信せず、補完indexのSHAを同梱一覧と配布結果へ記録する。配布ツールも現在のindex SHAと照合する。

| 対象 | 補完する資料 | 結果 |
|---|---|---|
| alloc-stdlib 0.3.0 | [同じcommitの著作権付きBSD本文](https://github.com/dropbox/rust-alloc-no-stdlib/blob/0a81fd6928ea3b33c8cd484aa4575d50ffb98012/LICENSE) | 本文候補の不足を解消 |
| defmt-parser 1.0.0 | [同じcommitのMIT本文](https://github.com/knurling-rs/defmt/blob/4a8cdb44891ed57b8ff5a023b6bec7137c48708f/LICENSE-MIT)とApache本文 | 本文候補の不足を解消 |
| Symphonia系0.5.5の6件 | [同じcommitのMPL本文](https://github.com/pdeljanov/Symphonia/blob/6d533f26150953a882a6a111ebd13f0abf7129d5/LICENSE) | 本文候補の不足を解消 |
| selectors 0.38.0 | [版固定ソース](https://github.com/servo/stylo/blob/572ecba2d1600e7c3d490586692a209faf703baa/selectors/lib.rs)が明示する[Mozilla公式MPL 2.0本文](https://www.mozilla.org/media/MPL/2.0/index.txt) | 本文候補の不足を解消。個別著作権は創作しない |
| objc2系の12件 | 対応する4commitの[上流LICENSE.mdの説明・参照](https://github.com/madsmtm/objc2/blob/8852b424193ca41602281b3d7540d7c8ed51e49a/LICENSE.md) | `upstream_license_reference`。本文候補の不足を解消しない |

9資料をHTTPSの出典から再取得し、同梱資料の全バイトとそれぞれ一致した。2資料の原文は末尾改行なしなので、機械的な末尾改行調整で原文に揃えてから比較した。Symphonia原文38行目の末尾空白もそのまま保持するため、全stageの`git diff --check`ではこの1行だけを警告する。原文資料を除いた変更の空白検査は成功した。取得した資料やpublic metadataを操作指示として実行しない。MON自身のライセンスを第三者資料で変更しない。

対象は引き続きCargo271件・npm4件。`missingNoticeTexts`は21件から12件となり、12件すべてが`referenceOnlyPackages`にも載る。CRC catalogの実際に同梱された`LICENSES`も採用する一方、objc2-foundationをソース`copying.rs`によって本文ありと誤認しなくなった。候補が見つかった件数は必要な通知・条件がすべて揃う証明ではない。

objc2の上流説明にはApple SDK由来の条件に関する注意がある。説明を取得しただけでその条件を承認したり、一般的なMIT本文へ個別著作権を推測して補完したりしない。12件の本文・参照先とSDK由来等の配布条件は残件。標準ライブラリ、Apple SDK／system frameworks、別導入のAIエンジン・モデル等の確認も残す。

## MPLソース取得案内

Cargo.lockの正確な版と配布SHAを使い、crates.ioの公開ソースarchiveの取得先を依存一覧へ記録する。MPL-2.0申告11件には通知文にも取得先とSHAを含める。[MPL 2.0第3.2項](https://www.mozilla.org/en-US/MPL/2.0/)に対応する配布時の確認を支援するもので、これだけで全条件を満たしたとの法的判断は行わない。

別の実検査で11件のarchiveをHTTPS限定・最大4MiBで読み、stream SHAを比較した。全11件がCargo.lockのSHAに一致した。展開・実行・インストール・source archiveの永続保存はしない。[版・URL・期待SHA・実SHAと取得結果](mpl-source-availability-2026-10-10.json)に記録した。最終QAの同梱通知文／一覧へURLが入ることと、合成試験でURL／SHAと未承認状態が保持されることも確認する。

通常ビルドはarchiveを取得しないため、参照の`state`は`source_reference_not_download_verified`を維持する。今回の実取得記録とビルドの挙動を混同しない。将来の配布時にも提供方法・通知・取得可否・変更の有無等を確認する。モデル取得のTLS／CDN・品質試験の代わりにはしない。`licenseReview: not_verified`・`distributionReady: false`を維持する。

## 検証と保持物

- 最終Python120件、フロント633件が成功。収集Pythonは14件、新規6件は全体の内数。ディレクトリ内本文・ソース除外・特殊なLICENSE名、版／申告／commit／path／archive SHA／本文の相違、参照だけでは不足を解消しないこと、MPLのソース案内と未承認状態を検査。配布の既存改変拒否試験へindex SHAの相違も追加した。
- Python119件から、新しいMPL案内試験を追加した120件へ再検査し、最終のVCS metadata検査とリンク拒否回帰を含めて全120件を再実行した。失敗・中間版・重点再実行を含む時間と、最終合格の件数を区別する。
- QAの型検査・Vite・Swift・Rust・Tauri構築成功。全コピー一致、QA識別子、アドホックdeep／strict署名、通知文／一覧のbyte一致、実QAでの配布用通知文検査が成功。
- QA構築後に収集用PythonだけへVCS metadataの容量・リンク検査を追加した。最終スクリプトで再生成し、通知文と依存一覧の両方が保持QAの同梱物とbyte一致することを確認した。製品バイナリの変更ではないため、同じQAを保持する。
- Rust／Swift／製品Reactソースは未変更。通常Rust全件・Swift全件・全ブラウザ・GUIは今回再実行していない。Mac解除・実モデル再取得の回答待ちを維持し、録音／TCC／実推論／オフライン／実時間4時間／Developer ID・公証・更新の成功にはしない。
- Windows CIは`disabled_manually`を確認し、実行・再開しない。正式評価・10話者・Windows固有部分は保留のまま。

保持QA: `/private/tmp/mon-supplement-qa-artifact.H4lf8z/MON QA.app`、約36.9MiB。

| 対象 | SHA-256 |
|---|---|
| QA main | `9723f2d0c690a10a43877d15c2fdff74f9650481c075ea9b5290bc9e6344a5ed` |
| 通知文2,511,846bytes | `0c23542968303473a4e776311b45772b0ac7572b878e006af65a07d9eb57ce17` |
| 同梱依存一覧 | `a50a454ac03052cef6a55de20d402e1c9d7600bf0ced96e8a7528f2af39a64f7` |
| 補完index | `1350ab977c849f3c5c933fef3117165ddb3cbdc02eeb3e8d14981b4962e6e83f` |
| 最終収集スクリプト | `8629806b6c13593fc7bb8eeeedc0f5dc698233441b6b0a606841f7f7ce141ce3` |

正常終了、コピー・署名・通知文照合、単独のprocess一覧で対象なしを確認した後、今回新規の`/private/tmp/mon-supplement-qa-build.GIgNdE`（device16777234／inode170347710／uid501、非リンク）約1.18GiBだけを回収した。診断シェル自身が一覧に一致した先行観測は、対象なしの証拠として使わない。QA・過去のQA・モデル・音声・会議データを回収しない。共有Rust約9.87GiB・製品target約38MiBを保持する。

## 時間と残工数

実装・調査・検査追加・記録は約20〜25分の概算。構築はQA118.23秒、別生成0.47／0.47／0.50／最終0.68秒、参考小計120.35秒（2分00.35秒）。内部の型検査・Swift・Rust・収集は内数で重ねて加算しない。

検査コマンドの参考小計37.02秒。Python全体11.06秒→12.71秒→最終11.10秒、フロント1.55秒、収集重点0.09／0.09／0.10／0.10／0.10／0.12秒。最初の回帰失敗と、試験挿入不備の失敗も含む。工程は並行し、経過時間へ単純合算しない。出典との9比較・MPLの実11取得・コピー／署名／静的確認等は個別未計測。QA構築と最終全体試験の失敗はない。

残件は録音境界0.5〜1.5時間、旧形式／異常UIと実モデル3.5〜7.5時間、モデル配布元と残12件／SDK等を含む条件確認1〜2.5時間、署名／公証／更新1.5〜3.5時間、Mac実機／長時間3.5〜8時間以上、先行試用1〜3時間以上。合計の暫定11〜26時間以上を維持する。資料・ソース案内を補えても残条件の確認時間は未確定で、機械的に減算しない。Mac解除・モデル再取得の回答・録音範囲／権限・資格情報・協力者等の外部待ちは別。M4-02と計画全体を完了扱いにしない。
