# macOS依存通知文の収集・同梱

- 日付: 2026-10-10
- 対象: M4-02、仕様11章の配布条件。画面機能は追加しない。
- 状態: 収集・同梱を実装して実アドホックQAで確認。ライセンス全体の承認・一般配布可の証明ではない。

## 変更と範囲

`tools/mon_third_party_notices.py`は既に取得済みの依存をオフラインで読む。`cargo metadata --offline --locked --format-version 1 --filter-platform aarch64-apple-darwin`のresolveから、rootを除くnormal/build依存を保守的に辿り、devのみ・未到達の他OS依存を除く。packages配列全体をMacの使用依存と誤認しない。[Cargo公式の形式・platform filter](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)に基づく。

npm lock v3のproduction依存と実インストールの名前・版を照合する。ビルドツールは含めず、scoped／nestedの配置も扱う。依存が欠落・異なる場合は停止する。[npmのlockfile形式](https://docs.npmjs.com/cli/v11/configuring-npm/package-lock-json/)を参照。

パッケージ内のLICENSE／LICENCE／COPYING／NOTICE／COPYRIGHT候補と明示license-fileを読み、見つかったUTF-8文を改変せず結合する。リンク・外部を指す明示license-file・不正文字・単体1MiB／本文合計16MiB超を拒否する。汎用ライセンス文への置換やSPDX選択による自動承認は行わない。通知文候補が存在しても、すべての必要な文が揃うことや条件を満たすことの証明ではない。

生成物は`.cache/third-party-notices`の2ファイルのみで、通常のGit対象から除外する。非リンクの既知出力を原子的に置換し、受領者が読める0644で保持する。Mac共通のbeforeBuildCommandで生成し、QAと通常版のResourcesへ`THIRD_PARTY_NOTICES.txt`・`third-party-inventory.json`を同梱する。

一覧には版・申告ライセンス・公開repository・原文相対パス／容量／SHA、Cargo/npm lockのSHA、通知文全体のSHAと候補が見つからない依存を含む。ローカルのホームパスは出力しない。MON自身のコード、Rust標準ライブラリ／toolchain、Apple SDK／system frameworks、別取得のAIエンジン・モデル・runtimeは対象外として明記する。これらやベンダー内部の追加条件も別途確認する必要がある。

配布ツールはコピー後・最終署名検査後に同梱ファイル、lock、容量・SHA、未承認フラグを照合し、欠落・旧版・改変・自己承認を拒否する。結果の`dependencyNotices.state`は`collected_not_reviewed`。`licenseReview: not_verified`・`distributionReady: false`を維持する。Developer ID／公証の実行は行っていない。

## 実収集と残る条件

現在のMac resolveはCargo271件とnpm4件の計275件。通知文2,355,821bytes（約2.25MiB）、一覧を含む生成ディレクトリは約2.4MiB。通知文候補が一つも見つからない21件を一覧に記録した。objc2系、Symphonia系、selectors等を一般的なMIT／MPLの文だけで補完して確認済みとはしない。正確な版の配布元文・著作権・NOTICE等を別途取得して照合する必要がある。

MPL-2.0申告の依存は11件。依存をコンパイルした実行物の組織外配布には、対応ソースの取得案内等も検討する必要があり、通知文コピーだけで完了しない。[Mozilla公式FAQのQ8・Q10](https://www.mozilla.org/en-US/MPL/2.0/FAQ/)に沿って実配布前の確認を残す。本収集は法的判断や採用ライセンスの変更を行うものではない。

以前のAI統合試験素材も公式配布情報を読み取り直した。モデル本体はまだ再取得していない。QAの起動・録音なし操作の許可と、再取得の容量確認を区別し、標準／高精度への採用や正式品質評価は行わない。

| 以前の素材 | 今回の公式メタデータ確認 | 未検証の範囲 |
|---|---|---|
| whisper.cppのggml-tiny.bin | [公式モデル配置先](https://huggingface.co/ggerganov/whisper.cpp/blob/main/ggml-tiny.bin)、77,691,713bytes、公開LFS SHAは以前の`be07e048…`と一致 | 本体の再取得・ライセンス資料一式・現QAでの互換性／実解析 |
| sherpa-onnx v1.13.8 osx-arm64-shared-no-tts | [公式release](https://github.com/k2-fsa/sherpa-onnx/releases/tag/v1.13.8)、archive18,252,168bytes、APIのdigest `sha256:91b96512c4fa1960f8a9ed5360a6c8dda53a4b5015d0590244f14086a234557a` | 個別バイナリ・runtimeの取得／照合、第三者通知文、現QAでの実解析 |
| sherpa配布のPyannote segmentation 3.0 | [公式release](https://github.com/k2-fsa/sherpa-onnx/releases/tag/speaker-segmentation-models)、archive6,958,444bytes、API digestなし | 元モデルは[MIT表示とアクセス条件](https://huggingface.co/pyannote/segmentation-3.0)がある。変換済み配布物との由来・条件を別途確認し、元モデルのアクセス条件を代理承諾しない |
| 3D-Speaker ERes2Net baseのONNX | [公式release](https://github.com/k2-fsa/sherpa-onnx/releases/tag/speaker-recongition-models)、39,593,761bytes、API digestなし | コードの[Apache-2.0 LICENSE](https://github.com/modelscope/3D-Speaker/blob/main/LICENSE)だけを変換モデル・runtime全体の確認とみなさない |
| Qwen2.5-0.5B-Instruct Q4_K_M GGUF | [公式モデルファイル](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/blob/main/qwen2.5-0.5b-instruct-q4_k_m.gguf)、公開SHAは以前の`74a4da8c…`と一致、[モデルLICENSE](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/blob/main/LICENSE)はApache-2.0 | 実ファイル／ライセンス資料の取得・llama-completion固定版・現QA統合・会議品質 |

現行のアプリ内取得は個別ファイルのみでarchiveの展開を提供しない。上記のarchive URLをそのまま実行ファイルの取得定義として案内してはならない。公式の個別配布URL、または条件確認済みのパッケージと手動導入の経路が必要。公式releaseから通信できたというメタデータ読み取りだけで、製品の実TLS／CDN取得成功とは扱わない。

## 検証

- 新規収集Python8件、配布Pythonの新規2件、設定フロントの新規1件を追加。既存込みの最終Python114件、フロント633件が成功。normal/build／dev／platform graph、原文保持・nested通知、欠落、name/version差、リンク・外部参照・容量・文字、生成物の読取り、旧lock／改変／自己承認拒否と署名前拒否を含む。
- Pythonの署名・公証pipelineは合成試験で、実証明書／Appleサービスではない。テスト全体内の件数と重点再実行を重複加算しない。
- 実QAを専用非増分targetで構築。型検査／Vite／Swift／Rust／Tauri同梱が成功し、コピー全一致、アドホックdeep／strict署名、QA識別子、通知文・一覧のbyte一致、製品配布用の通知文検査が成功。
- 今回はGUIを再操作していない。前回のMacロック解除待ち、移動先での冷起動・修正文言・実モデル・TCC／録音・実時間4時間は未確認。Rust／Swift製品ソースは未変更で、通常Rust全件・Swift全件・全ブラウザは今回再実行していない。

保持QA: `/private/tmp/mon-notices-qa-artifact.WaV1fC/MON QA.app`、約36.6MiB。

全ビルド終了・コピー／署名／通知文照合と、sandbox外のprocess一覧で対象なしを確認した後、今回新規の`/private/tmp/mon-notices-qa-build.7qgma4`（device16777234／inode170334569／uid501、非リンク）の約1.18GiBだけを回収した。共有Rustは約9.87GiB、製品targetは約38MiB、生成通知文は約2.4MiB。QA成果物、過去のQA、モデル／検証音声・会議・保存先変更のデータを回収対象にしない。専用targetは再構築で再生成可能。

| 対象 | SHA-256 |
|---|---|
| QA main | `ae0991284f27db3b3521dee29f97d1f8157195706a36b9a27bd61be6f745aaa0` |
| 通知文 | `44bcba555cf1f10bb292f43edde365eec5bb55dfa4f41f6cb753b83abebbfaa2` |
| 一覧 | `dc6ab4de2c92747a2ece5f709224a857d41541e33708b5370f72122990f0290e` |
| 最終収集スクリプト | `db9a3233e61b0f929c765458f47cb62a634114ebdb4f9ef625a5fb39d0476262` |

構築中に収集スクリプトへ名前照合・scoped/nestedテストを追加した。最終版で再生成し、構築時の同梱2ファイルとbyte一致を確認した。アプリ内の製品コードをビルド後に変更したものではない。

## 時間と残工数

実装・調査・検査追加・記録は約15〜20分の概算。QA構築115.93秒と別の生成0.48／0.58秒を分け、構築参考小計は116.99秒（1分56.99秒）。構築内の型検査・Swift・Rust・内蔵収集は内数で、重ねて加算しない。

検査コマンドの計測参考小計24.80秒。Python全体113件11.39秒→最終114件10.86秒、収集重点0.16／0.19／0.09秒、配布重点0.50秒、フロント全体1.61秒。工程は並行し、全体経過と単純加算しない。コピー・署名・メタデータ読み取り等は個別未計測。今回の自動試験と構築に失敗はない。sandbox内のprocess一覧読取りだけは拒否され、cache回収前の安全確認には使っていない。

残件は録音境界0.5〜1.5時間、旧形式／異常UIと実モデル3.5〜7.5時間、モデル配布元と欠落通知文／対応ソースを含む条件確認1〜2.5時間、署名／公証／更新1.5〜3.5時間、Mac実機／長時間3.5〜8時間以上、先行試用1〜3時間以上。合計は暫定11〜26時間以上で、Mac解除・モデル再取得の回答・録音範囲／権限・配布資格情報・協力者の外部待ちは別。新たに具体化した通知文欠落とソース取得案内の確認を見積りに含めた。M4-02と計画全体は進行中、正式評価・10話者・Windows固有部分とCIの保留を維持する。
