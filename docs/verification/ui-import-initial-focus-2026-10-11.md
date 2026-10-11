# 取り込み読込後の安全な初期フォーカス

2026-10-11。直前の共通境界修正 `974c096cb6645f8c006c53505b0fc84707847418` に残っていたM3-08の限定修正。正式評価・Windows・実録音・正式配布の保留は維持する。

## 再現と修正

現ブラウザーの通常取り込みで、readiness後もactiveElementがdialogに残ることを再現。共通境界には入力保持のため非表示の兄弟viewがmountされ、そこに有効な取消ボタンが存在する。旧初期queryはdisabledだけを除外していたため、読込中に非表示の取消を選ぶとfocusできず、初期focusの待機フラグだけを消していた。

`visibleModalTarget`で接続済み・有効・hidden／inert祖先なし・描画ありの要素だけを表示順に選ぶ。CommonModalの初期cancel／headingとreadiness後のcancelに利用。読込後はdialog／body／表示headingにfocusが残る場合のみ安全な取消へ移し、既に選んだ入力には移さない。別native境界・所有確認・replacementに割り込まない。対象がまだなければ待機を保持する。入力／保存・取消controller、token guard、ネイティブ境界や保存コマンドは変更しない。他のreplacement／fallbackの全focus経路を今回一律に変更したものではない。

## 限定実機検証

最終QA `/private/tmp/mon-import-focus-qa.5cGf8d/MON Onboarding QA.app`、main SHA `e5b36d63ecc06eba54c8e87bf0a8b1449ddda54b35316db85c0b74d6113e4990`。通常コード・専用ID `jp.mon.desktop.qa.onboarding20261011`、deep strictアドホック照合成功。正式署名／公証ではない。

- Mac通常ファイル選択で自作29.610375秒WAVを選ぶ→readiness後×「確認をキャンセル」focusを取得。再起動せず二回目も同じfocusを取得。
- 初期Enterは取り込みを開始せず破棄確認になる。Escapeでその確認を断ると×へ復帰。Tabで会議名選択、Shift＋Tabで×復帰を確認。
- Escapeによる破棄確認の明示確定で未保存のテスト選択だけ取り消し、元の「音声を取り込む」入口へfocus復帰を2回確認。元ファイル・保存済み会議は残る。新会議保存／解析job開始なし。
- 合成ブラウザーでもreadiness後×focusを2回、Enter／確認取消・同一境界を確認。通常1280px幅で開いたdialogは1個、横溢れ0。全browser driver一括・狭幅・両テーマ・実IME／VoiceOverの合格ではない。

native画像 `/private/tmp/mon-import-focus-native-20261011.png` SHA `9c223c937b51f519ccfff62c810d7618003957bc1faad4b75932259b5129688a`。合成画像 `/private/tmp/mon-import-focus-browser-20261011.png` SHA `102599633c3bdaaf8d0631193fcffe0818245a95372d842b5e61b276ac2cfe33`。専用タブとQAは通常終了。通常終了後pgrep終了1でQA不在を確認。

## 保存保護と検査時間

停止中DB＋WALの起動前clone `/private/tmp/mon-import-focus-before.lFFmtu` と終了後clone `/private/tmp/mon-import-focus-after.e1jlcg` をread-onlyで照合。全DBのsqlite `.dump` SHAは両方 `793471dc1b6387dade57e8457bb006d2dd976d9759c35322e33725fca7b8c61e`、integrity_check ok／foreign_key_check 0行、5会議を保持。元WAV SHA `ccc3316f158ff9691267f176c32039f51eb49d5faf79b21908f43e6f328bfa93`、既存Markdown SHA `ea4eff608194af1b983fcda8beceaf30659bc374e2f4dadaea2e36ca348c1c0a` も不変。外部SQL書込みなし。既存11会議QAは起動せず、下書き／出力SHA `acce9d590c669a10c3638fcd404a906ce250389e1e44156f1af037102db71e95` とbootstrap SHA `aa26004e6897602aa0d6d2dc07d2c10ef7f18db59eaf97d07ede6b8aab845020` の一致も確認。今回旧QAの他全列を再集計したとの主張ではない。

フロント836成功／0失敗／0skip、1.806819083秒。新4件は非表示兄弟取消・inert／未描画／未接続除外・表示順／対象なし・初期待機とowner guardの契約。browser driverにも初期focus assertionを追加したが一括実行はしていない。型検査／Vite1.50秒、QA Vite1.51秒／Cargo1分13秒で終了0。Rust製品ソース・controller未変更で通常Rust／Pythonを再実行しない。CI／録音／モデル追加取得なし、Rust cache8357MiBの丸め値を保持し10240MiB上限内。

実装時間は調査込み概算約3分、時計を取得したコード変更区間UTC06:41:10〜06:41:33は23秒（全実装・実機・記録時間ではない）。Mac操作・データ照合・文書時間は別で未分離。構築はQA Cargo1分13秒／前記Viteを区別し、実行検査1.806819083秒を壁時計総時間に合算しない。既存Sonner／chunk／Swift readonly cache／公証未設定warningは維持。

| ログ（`/private/tmp/`） | SHA-256 |
|---|---|
| mon-import-focus-tests-20261011.log | fd55593d4bacb7de0b546fa4813dd4d98f06dec5cc57b4a7ce00262d94d46630 |
| mon-import-focus-build-20261011.log | 2307afa4eb258188b5bb79ec5811b3c9063c324c7eeb98cd231b4460ac80104f |
| mon-import-focus-qa-build-20261011.log | 750ba6d5f28d9dbceb3687e29e05d6c36eb2ce2e6e4cb3f1b29ce93da7279f6b |

全23タスクの[完了監査](implementation-completion-audit-2026-10-11.md)は継続。他モーダル順序・主要操作のキーボード／IME／VoiceOver・両テーマ／長文、編集／異常経路は未網羅。暫定残工数12〜29時間以上（非録音UI／編集／異常経路4〜8時間、保留中の実録音・耐久5〜8時間、配布／更新／試用3〜13時間）。正式評価／Windowsと録音許可・Developer ID／公証・協力者の外部待ちは別。
