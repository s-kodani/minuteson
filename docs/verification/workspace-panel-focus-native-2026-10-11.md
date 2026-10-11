# 目次と固定根拠の実操作入口への焦点復帰

2026-10-11、基準 `c59e843ebb3aaeb7ada8dfcae8ec50b2e56781d1`。M3-08／UX-11とM3-04の限定補修。正式評価／Windows・実録音／署名等の保留を維持。CI、録音、音声再生、追加取得、実AI推論は実施しない。

## 再現と修正

現行Mac QAで目次入口をクリックし、開いた直後と×で閉じた後のAX焦点が会議workspaceに残ることを再現した。Safari系のクリックによるfocusを暗黙に前提として `document.activeElement` を保存していたため、実際に押した入口を記録できなかった。

WorkspacePanelの入口で、実buttonへ明示focusしてevent.currentTargetをrevealへ渡す。wideの開く遷移では、その入口から来て焦点が入口／bodyに残る場合だけ×へ移す。既に選んだ別操作、別modal、根拠からのrevealでは奪わず、narrowのmodal初期focusは既存共通処理に委ねる。閉じた後は既存の可視／接続／enabled検査と明示参照先の優先を保って入口へ戻す。パネルやeditorを二重生成せず、resizeのportal再配置・ページ／入力／scroll保持を変えない。

最初の修正版で目次入口は改善したが、Macの本文根拠時刻クリック→根拠一覧を閉じるでは本文regionに戻ることも再現した。readerはevent.currentTarget、rich fieldは実chipをhandleへ渡すよう追加修正。未知根拠／locked／full／inactiveのguard後、可視・接続・enabledのchipへ明示focusして正確なopenerを記録する。根拠本文／引用／選択・保存処理は変更せず、buttonとrich field内のspan双方を扱う。

## 合成画面と回帰の範囲

独立した実React＋合成IPC画面だけをCUAで操作。wide目次を開くと×focus、Tabは目次summary、逆Tabは×、Enterで閉じて入口、再Enterで×、narrow960pxではmodal内focus・Escapeで入口。根拠の閲覧button／編集spanそれぞれで、wide開く／閉じるのchip focus、narrow960pxのmodal内focus／Escape後chip focusを確認。最後の合成整合表示は168ブロック・3項目・25根拠、初期下書きJSON一致・生成原文保持・Undo要求0・保存要求0。

source契約3件を追加：native clickの明示入口、wideの条件付き移動、reader／rich chipの伝達。修正前は入口2件が失敗（全体0.12秒）、根拠追加1件も修正前失敗を確認。既存browser driverへEnter／Tab／逆Tab・入口往復の検査を加え、構文／契約検査も補完したが、その全driverは今回再実行していない。CUA操作・source契約・全driver再実行を混同しない。別focus／別modalを奪わない条件はsource契約であって実raceの証明ではない。

最終フロント848成功／0失敗／0skip、runner2.287639833秒、全体2.66秒。最初の2修正時は847成功（初回全体2.50秒／driver補完後2.25秒）。型検査＋Vite初回6.44秒（Vite1.51秒）、根拠追加後6.70秒（Vite1.50秒）。最初のQA全体83.99秒・Cargo1分13秒、ad-hoc deep／strict成功。初回修正版Macで目次開く→×・Tab→目次・逆Tab→×・Enterで閉じて入口・Enterで再度×・クリックで閉じて入口を確認した。

## 最終Macの実機証拠と保存データ

最終QA `/private/tmp/mon-panel-focus-final-qa.YgRJef/MON Onboarding QA.app`、main SHA `36d5cdb1c37d99c5ac42dba61cb201aab0cfc9c287b2f537885c48e5dd7d72f9`。全体81.78秒、Cargo1分12秒、Vite1.51秒、39.22MiB。ad-hoc deep／strict照合成功で、Developer ID署名・公証ではない。既存のbundleサイズ／Swift user cache／未承認notice／公証未実施の警告を残す。

保存先 `jp.mon.desktop.qa.onboarding20261011` の自作会議 `18de4837-b6e1-466b-86a6-ba145b3caed4` のみを操作。最終版で閲覧時刻クリック→固定引用／3発言の文脈表示→一覧を閉じて同じ時刻focus、編集時刻クリック→同じspan focus→閉じてspan、Enterで再表示→閉じて同じspanの2反復を確認した。内容は入力せず、根拠検証checkも変更しない。無変更の編集終了後、目次入口→×・Tab→目次・逆Tab→×・Enter→入口を再確認した。

最後の画像 `/private/tmp/mon-panel-focus-final-native-20261011.png`、SHA `bd8be5454196588dbf348b2837a6478d556097d28d4abd912705a350935a6fed`。AXで入口focusを確認し保存したもので、全VoiceOver検証ではない。

通常終了・対象process不在後、DB＋WALを `/private/tmp/mon-panel-focus-db.E3MUjq/meetings.sqlite3` へcloneしread-only照合。全dump SHA `dd191a4b5afc39cb7677f318b7890f56c1d1dc0e672aab7bb5ee15cc4f68ddf1` は前回停止後と一致、integrity `ok`・5会議・FK違反0。全本文／下書き／版／出力／設定は不変。本番DBは対象外。合成tabを閉じ、viewportもresetした。

## 工程時間

実装／回帰補完は約3分、画面調査／確認・照合は約5分の区間（概算・文書作成除外）。ビルドと画面確認は並行。型検査／Vite2回とQA2回の全実行時間は合計178.91秒（約3分）、最終テストは2.66秒、成功3回の合計7.41秒。修正前入口の失敗0.12秒、根拠の修正前失敗は全体個別未計測。工程合計を実際の経過時間とは扱わない。通常Rust／Python再実行なし。Rust test cache8271MiB（今回末の表示丸め・上限10240MiB）。追加取得0bytes。

ログ：`/private/tmp/mon-panel-focus-before-20261011.log`、`/private/tmp/mon-evidence-focus-before-20261011.log`、`/private/tmp/mon-panel-focus-tests-20261011.log`、`/private/tmp/mon-panel-focus-build-20261011.log`、`/private/tmp/mon-panel-focus-build-final-20261011.log`、`/private/tmp/mon-panel-focus-qa-build-20261011.log`、`/private/tmp/mon-panel-focus-qa-final-20261011.log`。最終testログは最終848件の結果であり、先行847件は実行時に確認した値。

## 留保と残件

Macで全画面幅・全テーマ・全focus・実IME／VoiceOver・全異常順序を確認したとはしない。[全23タスク監査](implementation-completion-audit-2026-10-11.md)を維持。通常Rust再実行／Rustソース変更なし。正式評価／Windowsは保留、実録音は今は実施しない、Developer ID／公証等は外部準備待ち。

残工数は暫定12〜29時間以上：非録音の未網羅UI／編集／異常経路4〜8時間、実録音／耐久5〜8時間（今は実施しない）、配布／更新／試用3〜13時間。署名等の外部準備待ちは別。限定修正で目標全体を完了にしない。
