# 議事録の小画面本文領域と上部操作の整理

2026-10-11。M3-08のモックに近い簡潔なUI・余白・整列と長文閲覧／編集を対象にした限定修正。正式評価・Windows固有部分の保留、実録音を今は行わない指示を維持。CI・コミット／Pushなし。

## 問題と変更

360×640の長文合成画面で、本文スクロール領域は70.3203125px。上部の書き出し文字ボタン・保存状態による折返し、長い版情報、2行のヘッダーが本文を圧迫していた。root横溢れ0でも、書き出しボタンが右端で見切れていたため、横溢れ値だけで快適さを合格にしない。

- Markdown書き出しを共通downloadアイコンへ。ホバー／読み上げ名と、応答不明時の「前回の書き出し結果を確認」、既存disabled・要求・focus復帰は維持。
- 保存状態を「保存済み／未保存／保存中／保存要確認」に簡潔化。名前付きstatusとして残し、保存エラー通知／再試行は削除しない。
- 現在版の「保存済み編集を含む」を「編集あり」へ。「議事録」の説明で意味を補完。「元データ更新あり」とその説明は保持。
- 書式詳細の三点入口を基本書式4アイコンと同じ列へ移動。見出しの操作列へ5つ目のボタンを足さず、設定モーダルと選択／focus guardは維持。
- 小画面のヘッダー・カード／版情報の余白を整理。録音状態は消さず、通常待機時にはヘッダーを1行へ。通知や録音時に必要な折返しは許可する。

## 合成ブラウザー実操作

専用 `/tests/detail-mock-browser.html?view=minutes&long=1&checks=1` を2tabで使用。保存応答やテーマは検証用UIから変更し、製品状態への直接注入はしない。検証パネルは確認後に閉じる。

| 確認 | 結果 |
| --- | --- |
| ライト360×640、編集 | 本文203.90625px、上部操作32px、版情報24px、root横溢れ0 |
| ライト360→960×640 | 本文220.3125px、編集状態／先頭「点検会議」を保持、root横溢れ0 |
| ダーク360×640、編集／閲覧 | いずれも本文203.90625px、上部操作32px、root横溢れ0 |
| 基本書式 | 太字・見出し2・箇条書き・表・三点すべて32px、360では上端308.09375pxで一致 |
| 書式詳細→Escape | 全操作を表示し、三点入口へfocus復帰。文書JSON・生成原文を保持 |
| 長文の次編集ページ | 2/6、先頭「長いリスト項目46」へfocus。本文高さ維持 |
| 保存不変 | 保存要求0、Undo0、初期文書JSON一致、生成原文保持 |

本文は同じ閲覧状態で約70→204pxとなり、編集状態でも約204px。Control+Endをスクロール領域で押しても今回のMac browserではスクロールしなかったので、そのキー操作の成功には数えない。実際の次ページボタンは通常clickで表示位置までスクロールされて動作した。360の書式toolbar／文書paddingはなお本文領域の一部を使い、全長文／全倍率／全端末の快適さを完了扱いにしない。

画像:

- `/private/tmp/mon-minutes-compact-360-20261011.png`
- `/private/tmp/mon-minutes-compact-dark-360-20261011.png`
- `/private/tmp/mon-minutes-compact-native-20261011.png`

一度、viewport変更直後のダーク画像が旧サイズの合成frameで小さく見えた。次のcaptureで正常な360×640を保存し、初回画像を配置証拠に使用していない。

## Mac QA

共有targetを再使用してbuild終了0、Vite1.52秒・Cargo1分15秒。

- retained app: `/private/tmp/mon-minutes-compact-qa.YHXPfg/MON QA.app`
- main executable SHA-256: `ebc889d120c366d973d703130775187dd740f31ad3b2a8d09a1d15ed4ec6368f`
- deep／strict ad-hoc signature検証終了0。正式署名／公証ではない。

自作会議 `QA合成会議・実AI編集 2026-10-11`（`6bbb0fc9-c5b6-4045-8853-f75ac2bd5aa3`）のみを開き、本文の入力や書式適用をしないで検査:

1. 議事録タブで短い保存状態／版情報・書き出しの読み上げ名を確認。
2. 編集へ移動して基本書式5アイコンと書式モーダル全入力をAXで確認。初期×focus→Tabで段落→Escapeで三点へ復帰。
3. 書き出しアイコンから通常Save panelを開きCancel。書き出し取消通知と、再有効化したMarkdownアイコンへのfocus復帰を確認。
4. QAを終了し、filtered processチェックで停止を確認してからreadonly DB照合。

会議11件、`PRAGMA integrity_check` はok。前後一致したsorted JSONのSHA-256:

- 下書き／出力: `54e5ff1ce509a1d11ce1db11fd58ae5788e6f40337c050c795a19862215e94f7`
- 解析／議事録版・会議参照: `6bdb8534aa2495a4ff042106f68c079adbafa448e8bb9201bff766fa0f542961`
- 設定／処理job: `213509aef1d22a5aa172f73463d19be7651103e056d52a6c876ca88651a26226`
- 用語／scope／snapshot: `e6882c0c2721d29b3babf5bcac02f17dc6c036d4dc7af77b90c9dfd0c5f34aee`
- QA bootstrap: `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020`

本番DB・既存10会議を編集しない。今回の照合を、ファイル全体／復元監査全履歴／実AI再検査まで拡大しない。

## 自動検査と残件

- フロント799成功・0失敗／skip、本体1.926秒。
  - `/private/tmp/mon-minutes-compact-tests-20261011.log`
  - SHA-256 `5a37c1da248dc9f425488dbf6e634cf5f15425720a8dae8451889e7c40ecdce9`
- 型検査＋frontend build終了0、Vite1.52秒。既存大chunk警告は残る。
  - `/private/tmp/mon-minutes-compact-build-20261011.log`
  - SHA-256 `540f1ead266f9ebcda0270e47ab373b6d99643484fc2856f27f5cb52b6ca95a5`
- QA buildログ: `/private/tmp/mon-minutes-compact-qa-build-20261011.log`
  - SHA-256 `bf0db4259b216ebd7b0969583b6549c4a242c63f461a1774c4ac3b28bf23f501`
- 保存要確認の短い表示へ旧driver3ファイルを追従。構文検査と差分検査のみで、CLI browser全suiteは未実行。

Rust製品ソースは今回変更していないため22分の通常Rust全suiteは再実行しない。新規の重いtarget cacheは作らない。検証用tabは閉じ、viewportもreset済み。実IME／VoiceOver・全UI／異常順序・実録音／長時間耐久・正式配布・正式品質等は残る。Sonnerは検討までで未導入。実装計画全体の完了宣言は行わない。

## 追確認：書式・構造変更後の本文への復帰

同日の後続検査。上のSonner未導入は先行時点の記録であり、その後Sonner2.0.8を導入済み。今回は議事録のリッチ編集を点検した。

### 再現と修正

合成会議で段落へ変換すると、構造変更によって選択状態が解除される一方、書式モーダルが残り、ほとんどの操作が無効になることを再現した。成功した構造操作は、残る本文／項目／セルの位置とページを決めてから書式モーダルを閉じるようにした。既存の次フレームでのfocus復帰、確認待ち／開いたdialog／有効な既存focusを優先するguardを維持した。失敗や日本語変換中の操作拒否ではモーダルを閉じない。太字・斜体など文字範囲への書式操作は、この自動終了の対象にしない。

### 合成ブラウザーの実操作

専用tabの `/tests/detail-mock-browser.html?view=minutes&checks=1` で、状態への直接注入をせずUI操作を実施。

- 段落・見出し3へ変換後、開いたdialogは0、元の「議事録ブロック1」へfocus復帰。再度開くと次の構造操作を実行できる。
- 表5の行追加後は「行2列1」へ、追加行の削除後は「行1列1」へfocus復帰。いずれもdialogは0。
- 箇条書きの項目追加後は新しい「議事録リスト3の項目2」へfocus復帰。
- 編集終了で保存要求5・保存済みを確認。再編集で見出し3／追加項目を保持。太字と斜体もDOMのstrong／emで確認し、編集終了時は保存要求7。
- 根拠チップ2件を保持し、変更後に固定発言「点検日を確認することに決まりました。」を表示。元発言読込1・生成原文保持・項目2／根拠2。
- 画像 `/private/tmp/mon-rich-format-browser-20261011.png`。検証tabは終了後に閉じ、ユーザーの既存tabは変更していない。

### Mac QAの保存・再起動・取り消し

- app `/private/tmp/mon-rich-format-qa.Vs43PV/MON QA.app`
- executable SHA-256 `099d1b887a3010cf437aaf09025163608f0e2d2a52cd49e7796d12dcd7756c21`
- deep／strict ad-hoc署名検証終了0。正式署名／公証ではない。
- 自作会議 `6bbb0fc9-c5b6-4045-8853-f75ac2bd5aa3` の議事録下書き `62503797-df6c-49bd-b031-b9bfb9015c2e` の先頭見出しだけを1→3へ変更。書式モーダルが閉じ、先頭本文にfocusと入力枠が戻ることをAX／画像で確認。
- 保存済み・編集終了後の見出しlevel3を確認。QA終了とprocess停止確認後のreadonly DBでもlevel3／lock3。再起動して同じ会議を開き、見出しlevel3と保存済みを確認。
- 編集管理から直前の編集を取り消し、編集終了後は見出しlevel1。QAを終了して照合した本文JSONは操作前と完全一致。lockは2→3→4で、保存／取り消しの履歴は正常に残す。DBへ直接書き戻していない。
- 自作下書きpayloadの前後SHA-256 `ceb88bf1f3fb1780a199aa3955a08e1a4204d16f1db85546380c5a401d7e8e43`、content_hash `8492f17dacf8ce5f67d9c332e3b32c6dfb0f85f0236631e37b3a419b2408df49`。
- 自作会議の議事録下書き以外の全下書き／全出力は前後SHA `e4ecc389ba204f396fc5988cb8a4106e4ef80b000bb7a54d38f21dda299043c4` 一致。全解析／議事録版・参照、設定／jobs、用語とbootstrapは上記3組＋bootstrapのSHAと一致。会議11件、integrity_checkはok。
- 画像 `/private/tmp/mon-rich-format-native-20261011.png` は変更後の本文focus状態。録音・再生・AI再実行・新しい出力は行っていない。

### 検査・時間・限界

- フロント805件成功・失敗／skip0、本体1.875秒。新規2件は成功時の終了順序／focus guardのsource契約検査であり、描画の証拠は上記GUI操作で分けている。
- test log `/private/tmp/mon-rich-format-tests-20261011.log` SHA-256 `bc5a0853c02faecebf433bb0aae20fa9ac447dacb2e1411f5eeb08cb6997965e`。
- 型検査／QA build終了0、Vite1.75秒・Cargo1分19秒。共有targetを再使用。build log `/private/tmp/mon-rich-format-qa-build-20261011.log` SHA-256 `259df54366b944eded18da107f5ad07e88d1a5d0ce7d5fcabc2f38f1c2487182`。
- Sonnerのuse client／既存大chunk・Swift readonly cache・正式公証未設定の警告は残る。通常Rust全suiteは製品Rustに変更がないため再実行していない。
- Macの表／リストを含む全書式、ブロック削除確認の全順序、全ページ／異常応答、実IME／VoiceOverはなお未完了。今回の限定検査をM2-06／M3-08全体完了にしない。CI・コミット／Pushなし、正式評価・Windows・実録音・正式配布の保留条件は維持。

### 次の継続検査：Macの空のToDo表

同じmain SHAのQAを再使用し、専用会議の編集→表13の見出し1選択→書式詳細→表の行追加を通常UIで実施。0行の表が1行になり、4セル（新しいToDo／未確認／未確認／空の根拠）をAXで確認。書式モーダルは閉じ、新しい行1列1へfocus復帰した。画像 `/private/tmp/mon-rich-table-native-20261011.png` を保存した。

モデル導入の現状についてユーザーから質問が届いたため、検査を増やさず、編集管理の直前Undo→親／元画面へEscape→編集終了で元の0行へ戻してQAを終了した。process停止確認後、readonly DBでlock4→5→6、本文JSONのSHAは `ceb88bf1f3fb1780a199aa3955a08e1a4204d16f1db85546380c5a401d7e8e43` と一致、content_hashも元の `8492f17dacf8ce5f67d9c332e3b32c6dfb0f85f0236631e37b3a419b2408df49`。他下書き／全出力の保護SHAは `e4ecc389ba204f396fc5988cb8a4106e4ef80b000bb7a54d38f21dda299043c4` 一致、integrity_checkはok。製品ソースの変更・再ビルド・新しい自動試験・録音・再生・AI実行・CI・コミット／Pushはなし。列操作・リスト・削除確認等の残検査は、この行追加1件を根拠に完了扱いにしない。
