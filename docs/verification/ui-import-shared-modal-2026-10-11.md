# 取り込み直後の会議操作と共通モーダル

2026-10-11。M3-05／M3-08の限定検証。正式評価・Windows・実録音の保留を維持し、全UI／VoiceOverの完了宣言ではない。直前の保存待ち表示修正は `ed6ea85d9271cd9b2e741dcbd13b24bd82a5d851` でmainへPush済み。

後続で取り込みreadiness後の初期focusも修正し、Mac通常選択2回で×focusと安全な確認／復帰を取得した。[現在の限定証拠](ui-import-initial-focus-2026-10-11.md)。以下のcontainer残留は本記録時点の履歴であり、全modal／VoiceOver・IMEの完了にはしない。

## 失敗比較と採用した変更

独立QA `jp.mon.desktop.qa.onboarding20261011` で、通常ファイル選択→保存→詳細の会議操作を開く順序のnative AX欠落が再現していた。描画と×focusはあってもHTML本文・ボタンがAXから消え、Escapeで元画面へ戻る。前の証拠は[実機記録](model-asr-onboarding-2026-10-11.md#保存待ちと操作中の表示を分離)。

最初に、取り込みdialogのnative closeイベントを受けてから詳細を公開する順序へ変えた。合成6回帰を含む832件は成功したが、比較QA `/private/tmp/mon-import-handoff-qa.sWgtXz/MON Onboarding QA.app`（main SHA `7a2ef1c1676c9f0f10a877ff2bc7043478bdeb4be68f2508d7e7f1a9a1e812a3`）で新会議 `9ace6e0c-a654-4919-b417-73fc900042ae` の初回会議操作AX欠落が再現した。通常終了PID47868消失後、試行コード・専用6テストを取り下げた。順序変更を解決策として採用していない。

採用した変更は、取り込みを`ScopedModal`で既存の設定共通native境界へ接続するもの。取り込み・用語・会議操作の間で別のnative dialogへ乗り換えない。入力はbody／replacement slotへ常時portalし、showModal時のDOM移動やタイマー待機、保存コマンドの移管はしない。scope外は従来CommonModalへfallbackする。取り込みのblocked／restoreReady／取消ラベル／aria-labelledby／見た目をdescriptorで保持し、閉じたownerの最新readinessも読めるようにした。contextはslotが変わらない限り同じ値を保持し、replacement descriptorの再描画ループを避ける。

取り込みのadmission、token／入力一致の取消確認、保存失敗／結果未確認、用語の上限・入力保持、元ファイルの扱いは従来controllerのまま。共通ownerを利用しても初期閉じるは「確認をキャンセル」、用語は「確認を閉じる」、通常設定は従来の「確認を閉じる」。UIの機能追加や強制的なfocus移動・全画面の一律再mountはしない。WebKit内部の正確な原因までは断定しない。

## 最終Macの限定確認

最終QA `/private/tmp/mon-import-shared-qa.rNRKwj/MON Onboarding QA.app`、main SHA `43c46ccc3fb97d2d4a25377a77126d31c2f77e7d23e351605144a9639c580753`。deep strictアドホック照合成功、正式署名／公証ではない。テスト音声は同じ自作単一TTS 29.610375秒、SHA `ccc3316f158ff9691267f176c32039f51eb49d5faf79b21908f43e6f328bfa93`。追加取得・マイク／PC録音なし。

1. 通常選択→標準設定で会議 `a3c5a503-0150-4d34-a45c-4e7a96552790` を保存→直後に会議操作。録音状態・改名入力・解析操作・候補欄をnative AXで取得し、×へfocus。Escapeで入口へ戻り再表示しても列挙を保持。
2. 再起動せず二回目の通常選択。会議名を変更→用語／読み「点検日／てんけんび」入力→Escapeで元の用語入口へ復帰。破棄確認をEscapeで断り、会議名と用語／読みの再表示保持を確認→会議 `185a3deb-c192-4a0e-83c6-bdf20ec3ef97` を通常保存→直後の会議操作でも本文・入力・操作のnative AXと×focusを取得。
3. 既存の編集済みテスト会議へ移動し、文字起こし編集オプションの本文／操作→Escapeの入口復帰、議事録の保存済み・現在版#1／編集あり→議事録設定の全入力／候補#2／出力履歴→Escapeの入口復帰を確認。既存本文の編集・採用・再生成はしない。

native画像 `/private/tmp/mon-import-shared-native-20261011.png` SHA `2a0a1a2489079f9ad3d23d5eb029bf274929311ccd287cd3fddf9e043c4e1a16`。この2取り込みと再表示の成功だけを全モーダル順序・実VoiceOver／IME・全形式・長時間の合格へ拡大しない。取り込み読込後の初期focusがcontainerに残ることも観測し、安全な操作への初期focusの追確認は残る。

## 合成描画と保存データの照合

通常ブラウザーの合成取り込みでも、会議名／用語の保持・破棄確認取消・用語入口のfocus復帰を確認。開いたnative dialogは1個、横溢れ0。可視の取り込み・選び直し・再確認・閉じる4ボタンは各40px／13px。隠れた他ownerの操作は高さ0で非表示。画像 `/private/tmp/mon-import-shared-browser-20261011.png` SHA `bc8ae001e4df054923e77c56cdac09bd95de2aab625507c581d68d5ae834043c`。全browser driverの一括実行・狭幅・両テーマの再検査ではない。専用タブは終了し、利用者タブは変更しない。

通常終了PID49636消失後、DB＋WALを私的clone `/private/tmp/mon-import-shared-db.Cm7B1e` へコピーしread-only照合。integrity_checkはok／foreign_key_checkは0行。元の編集済みテスト会議のid/title/現・前head、全下書き／出力履歴、採用議事録#1の全列SHA `62c05dcf98eff04431d5724d2a6c5303f0bf18bd351499b57bc26af3b8fb14dd`、設定／管理構成／CPU記録SHA `930443a4df65c963311152d73dc4d33cdd78a9945c47d21263d446e1eb3854c4` は直前と一致。実Markdown SHAも `ea4eff608194af1b983fcda8beceaf30659bc374e2f4dadaea2e36ca348c1c0a` を保持。二回目の用語はその新会議scopeに有効1件として保存され、共通用語は変更しない。QA内は比較用1件＋最終2件を追加して5会議となり、DB全内容不変とは言わない。

| 最終取り込みの実job | state | elapsed |
|---|---|---:|
| ASR `12d78ca9-982b-4898-8920-08e4ed11df5f` | succeeded 1/1 | 13,373ms |
| Sherpa `af4003f2-b90d-4429-beca-f2ed0bd2408a` | succeeded 1/1 | 6,795ms |
| ASR `c8e910c1-ec6b-4d0d-ac39-7bd1f05c9ab3` | succeeded 1/1 | 13,538ms |
| Sherpa `794261a6-40f1-424f-85ca-5d13303441ac` | succeeded 1/1 | 6,558ms |

最終2会議の候補は未採用でheadなし。比較用会議もASR／Sherpa各succeeded 1/1（13,293／6,545ms）で、失敗していたのはAX表示であり音声保存／jobではない。単一TTSの成功を実複数話者／品質採点へ換算しない。

既存11会議QAの下書き／出力、版／head、設定／jobs、用語4組とbootstrapは前記録のSHAと不変。外部SQL書込み・試験専用IPC・本番操作・素材削除・追加取得・録音・CIなし。

## 検査と工程時間

最終フロント832成功／0失敗／0skip、2.238214417秒。新規6件はdescriptorのbusy／readiness／取消ラベルと既存設定への復帰、閉じたownerのreadiness、共有境界／portal保持、context安定、既存admission／取消guard、隠れたviewを再表示しないCSSを検査。型検査／Vite1.52秒、最終QA Vite1.47秒／Cargo1分11秒、diff check成功。Rust製品ソースは変更せず通常Rust／Python検査を再実行しない。Rust cache8357MiB（丸め値、約8.16GiB）を保持し、上限10240MiB以内。既存Sonner／chunk／Swift readonly cache／公証未設定warningは残る。

調査・実装の時計区間UTC06:19:02〜06:20:50（取り下げた案1分48秒）と06:25:41〜06:28:16（採用案2分35秒）の小計4分23秒。例外整理・実機操作・文書時間は別で未分離。比較QAはCargo1分16秒＋最終比較1分18秒、採用QA1分11秒で構築小計3分45秒（各Viteは別）。取り下げ案のフロント2回は832成功、1.832283416秒／3.382546292秒、採用案2.238214417秒と合わせexecution小計7.453044125秒。重複する成功件数を2,496件とせず、並行工程の合計を壁時計経過や全実装工数にしない。

| ログ（`/private/tmp/`） | SHA-256 |
|---|---|
| mon-import-handoff-qa-build-20261011.log | c030a44ebc6c9746f6726fcb957764356b7934bcf5028dfecbca7eefe49c3177 |
| mon-import-handoff-qa-build-final-20261011.log | 797056100106d4a2d450c593cf39f852d266139ab7450d5190f4fde56907783e |
| mon-import-shared-tests-20261011.log | 57a86c7078e8565b81a987a4cd60586e64277132d1908850a42e732f2a65a95f |
| mon-import-shared-build-20261011.log | 5286817975e9b48f1534687c85e7d36235b6a4c8a1401f3238aeabe8312c0b98 |
| mon-import-shared-qa-build-20261011.log | 2bccfb1110c0b03e0f4516ee2a649afa731fb6cc0d6f51322f079de72a804859 |

計画全23タスクは[完了監査](implementation-completion-audit-2026-10-11.md)で維持。暫定残工数12〜29時間以上（非録音UI／編集／異常経路4〜8時間、保留中の実録音・耐久5〜8時間、配布・更新・試用3〜13時間）。正式評価／Windowsと、録音許可・Developer ID／公証・協力者の外部待ちは別。今回の限定成功を製品完成へ置き換えない。
