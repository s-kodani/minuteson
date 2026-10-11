# 議事録枠の幅と目次入口の右端

2026-10-11、基準 `8674c7ba865f5c0dba5700649d23aa2c20950534`。ユーザーの「白い議事録枠と入口の右端を揃える」「左右の余白を減らす」への限定対応。録音・追加取得・実AI推論・CI・正式署名／公証は行わない。

## 修正と合成画面

議事録閲覧枠・リッチ編集枠・基本書式バーの760px上限を外し、本文内の段組も利用可能な幅へ広げた。プレビュー等の他画面の段組は変えない。閉じた目次入口の行は本文と同じ左右paddingとstable scrollbar gutterを持たせる。入口行のoverflowはhiddenで、新しい操作用スクロール領域を作らない。本文の縦スクロールは保持し、パネル開閉・保存・編集処理は変更しない。

変更前1280pxで白い枠は760px（左359.5／右1119.5）、入口は右1248。変更後は971px（左254／右1225）、入口も右1225。

独立した合成IPC画面の実ReactをCUAのPlaywright操作で確認した。閲覧／編集×light／dark×1440・1280・960・600・360・320px（高さ720px）の24配置で、枠幅1131・971・807・479・239・199px、利用可能幅と一致。入口の右端差は全て0px、編集書式バーとの差も0px。documentと本文の横溢れはなし、入口行の縦溢れなし。検証用テーマ選択は合成画面内だけで操作した。

1280px編集で目次を開くと、本文667px、右パネル280px、間隔24px、document横溢れなし。960pxでは既存の目次modalを開閉し、閉じると入口focusを確認。viewportをresetし、専用tabを閉じた。既存browser driverにも実幅・入口差の検査を追加したが、その全driverは今回再実行していない。source契約／構文検査と上記画面操作を区別する。

## Mac実機と保存データ

最終QA `/private/tmp/mon-minutes-paper-qa.DVtnBr/MON Onboarding QA.app`、main SHA-256 `603cc883c8c4607554e5e477977d2c7ad1d9fcfe8465b67c63897092078664f5`。ad-hoc署名のdeep／strict照合成功であり、Developer ID／公証ではない。保存先は `jp.mon.desktop.qa.onboarding20261011` のみ。

自作テスト会議 `18de4837-b6e1-466b-86a6-ba145b3caed4` を開き、閲覧枠と入口の右端、目次開閉、編集枠／書式バーの幅、内容を変更しない編集終了を確認。Mac全画面幅・全テーマ・全focus／VoiceOverの証明にはしない。

- 閲覧画像 `/private/tmp/mon-minutes-paper-native-20261011.png`、SHA `9e5f832c308558515e04bba40ea95005c2f69996ed2d57b4eb43509bbfb7a6ac`。
- 編集画像 `/private/tmp/mon-minutes-paper-native-edit-20261011.png`、SHA `705abe6ff461582efe62ea218629a668fdc7eb66e0f046c7340c0d7b77bbd92d`。

通常終了・対象process不在後にDB＋WALをcloneし、read-onlyで全dump SHA `dd191a4b5afc39cb7677f318b7890f56c1d1dc0e672aab7bb5ee15cc4f68ddf1`。前回停止後と一致。integrity `ok`、5会議、FK違反0。本文・下書き・版／出力・設定の変更はしていない。本番DBは対象外。

## 時間と残件

実装修正／回帰追加は約3分（概算・文書作成除外）。画面確認は約4分、ビルドと並行した概算。型検査＋Vite全体7.37秒、Vite1.53秒。QA全体83.86秒、Cargo1分13秒、39.22MiB。既存のbundleサイズ／Swift user cache／未承認notice／公証未実施の警告は残る。通常Rustテストの再実行・Rustソース変更なし。Rust test cache8357MiB（上限10240MiB）。

source契約とdriver構文／契約の2回帰を追加。最終フロント845件成功／0失敗／0skip、runner2.147130292秒、全実行2.37秒。ログ `/private/tmp/mon-minutes-paper-tests-20261011.log`、build `/private/tmp/mon-minutes-paper-build-20261011.log`、QA build `/private/tmp/mon-minutes-paper-qa-build-20261011.log`。画面操作とテストrunner時間を混同せず、並行時間を単純加算しない。

全23タスクの[監査](implementation-completion-audit-2026-10-11.md)を維持。暫定残工数12〜29時間以上：非録音の未網羅UI／編集／異常経路4〜8時間、現在実施しない実録音／耐久5〜8時間、配布／更新／試用3〜13時間。署名等の外部準備待ちは別。正式評価・Windowsは保留で、目標全体の完了ではない。
