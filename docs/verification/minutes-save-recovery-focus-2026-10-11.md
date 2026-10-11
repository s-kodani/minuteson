# 議事録保存失敗の保護と明示再試行focus

2026-10-11、基準 `087e10042cea10669affeae67e430d552919b3d4`。UX-10／UX-11、M2-06／M3-04／M3-08の限定追確認。正式評価・Windows保留、録音・音声再生・追加モデル取得・実AI推論・CIは実施しない。

## 発見と修正

production Reactを使う独立した合成IPC画面で、保存失敗後の明示再試行に成功すると再試行buttonが消え、焦点がAXWebArea／bodyへ落ちることを再現した。入力・固定根拠は保持されていたが、続けて操作する入口を失うため補修した。

本文側の実再試行buttonをクリック開始時に明示focus。既存の保存キューを明示retryし、処理後の描画で、再失敗なら現在の再試行button、成功なら編集操作へ戻す。mount／可視／enabled／modal不在を照合し、待ち時間中に選ばれた別focusは奪わない。通知内の再試行は既存Toastの焦点復帰を使い、同じ入力確定guardとretry処理を共有する。再試行で編集を終了・再読込・本文書換えしない。確認中は本文側の再試行も無効。前回の終了処理は字下げだけ整理した。

## 画面から確認した範囲

`http://localhost:1420/tests/detail-mock-browser.html?view=minutes&checks=1` のエージェント作成tabだけを使用。保存失敗・遅延は画面内の明示合成操作で設定し、実ファイル／本番DBを変更していない。

- 保存失敗で「編集を終了」を押しても編集を終了せず、本文「QA保存失敗でも保持」と保存要確認・再試行・要対応を保持した。通知を閉じても要対応を保持し、再度終了を押しても保存要求は1のままで再送しなかった。
- 失敗後の追加入力を保持し、文字起こしへの切替を拒否した。復帰操作後の明示再試行で最新追記を保存したが、その成功時にfocus欠落を発見した（修正前）。
- 修正後の初回失敗は保存要求1。通知を閉じた後の明示再試行も失敗し、要求2・入力保持・現在の再試行button focusを確認した。
- 「QA再試行成功の最新追記」へ変更し、復帰後に明示再試行。要求3、保存済み・入力保持・編集終了button focusへ復帰した。根拠2・項目2・生成原文保持を合成の整合表示で確認した。
- 続く失敗は要求4。3秒遅延で明示再試行し要求5／保存中を観測、外側の「議事録保存を復帰」を選んだ。完了後もそのfocusを保持し、最新追記が保存済み。その後の編集終了は要求5のまま、編集buttonへfocus復帰した。
- 最後の別入力でも失敗・再失敗時の保護を確認して画像を保存し、検証tabを終了した。

画像 `/private/tmp/mon-minutes-retry-failure-browser-20261011.png`、SHA-256 `73480139078d09b75e0f8644c2f8b9d8cbc998c8c265cb205a992a93d64335c9`。保存要確認・明示再試行・保持した本文と固定根拠を表示している。合成画面であってMacの実ディスク不足の画像ではない。

## 自動検査とMacの通常経路

新規5件：2件はsource契約（入口／共有retry／復帰guard）、2件は実DraftSaveQueue＋実リッチ編集＋合成IPCの挙動、1件はbrowser driverの構文／検査条件。キューの検査では、失敗中の追加編集・無断再送拒否・元生成版／固定根拠保持、最初の保存だけ成功した後の次の失敗で、acknowledged版と最新未保存本文を分けて保持し明示retryすることを確認した。

既存 `detail-minutes-browser-check.js` に失敗終了→再失敗→最新追記の成功→clean終了の検査を追加。新しいdriver全体は今回実行しておらず、構文／契約検査と上記CUAの画面操作を混同しない。既存長文／両テーマ／生成終了のdriver全ケースの再合格とも扱わない。

フロント843成功／0失敗／0skip、runner3.158387375秒。型検査／Vite2.22秒成功。最終QAのVite1.56秒、Cargo1分17秒、39.22MiB。警告（Sonner directive／bundleサイズ／Swiftユーザーcache・未承認notice／公証未実行）は残す。

Mac通常QA `/private/tmp/mon-minutes-retry-focus-qa.LdNss3/MON Onboarding QA.app` をdeep／strict署名照合後に起動。アドホックQAで、正式署名・公証ではない。主実行ファイルSHA-256 `1e8a60c07d0e4895a33fad5aa11c630adf030fbfda0d183b0a42cba9e9d403a6`、製品editorソースSHA `dd30c658baf5a3b7c208cfc65830d6ea8ce8d201839ab38d71904f5615de3362`。

独立namespace `jp.mon.desktop.qa.onboarding20261011` の前回自作会議を開き、#1の書式／表・8根拠を表示。通常編集開始の本文focus→変更せず終了の編集button focus→TabでMarkdownbuttonを確認した。これは実Mac保存失敗／再試行の検証ではない。通常終了しprocess不在を確認、DB＋WALをAPFS clone `/private/tmp/mon-minutes-retry-focus-db.jO39Ku` へ移してreadonly照合。前回停止cloneとの全論理dump SHA `dd191a4b5afc39cb7677f318b7890f56c1d1dc0e672aab7bb5ee15cc4f68ddf1` 一致、integrity ok／FK違反0／5会議。元DBへSQL書込みやWALを無視したimmutable接続はない。旧11会議QA・本番は起動しない。

## ログ・時間・残件

| ログ | SHA-256 |
|---|---|
| `/private/tmp/mon-minutes-retry-focus-tests-20261011.log` | `0fc98023bef996597412550285e6c1b1295c082bef5ed120e87914c4d611ddec` |
| `/private/tmp/mon-minutes-retry-focus-build-20261011.log` | `df402206210281bd953cb9c03413406f05337e58c482257ab35ce2afc1d5b397` |
| `/private/tmp/mon-minutes-retry-focus-qa-build-20261011.log` | `e72f9741e388611e6d0f1c2314d61358aa7942e16272cb40324fbc2b5a4dc1e5` |

実装修正／回帰補完は約2分（概算・文書整理を含まない）。画面確認・照合は約6分の区間で、実装とビルドに並行した概算。buildは上記各runner、型検査／署名等を含む全build時間は分離計測していない。通常Rust／Python再実行なし、Rust製品ソース変更なし。Rust test cache8357MiB（表示丸め）、上限10240MiB。追加取得0bytes。

保存失敗は合成とキュー検査の限定証拠であり、実Macの容量不足／競合・応答喪失・再起動回復、実IME／VoiceOver・長文全ページ・全書式・全異常順序は残る。[全23タスク監査](implementation-completion-audit-2026-10-11.md)を維持し、正式評価／Windows保留以外も含めて目標を完了にしない。暫定残工数12〜29時間以上：非録音の未網羅UI／編集／異常経路4〜8時間、現在実施しない実録音／耐久5〜8時間、配布／更新／試用3〜13時間。署名等の外部準備待ちは別。
