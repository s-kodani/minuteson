# 配布前の依存通知を現在の収集元全体へ照合

2026-10-11、検査修正前の基準 `8674c7ba865f5c0dba5700649d23aa2c20950534`。M4-02の通知検査を限定補強。ライセンス承認・不足通知の解消・正式署名／公証・アップロード・公開ではない。録音・CI・追加モデル取得・通常Rustテストは実施しない。

## 再現と変更

旧検査は通知本文のSHA／容量、lockと補完indexのSHA、未承認状態等は確認するが、inventory内の不足一覧とpackage情報の真正性は確認しなかった。本文を変えず、`missingNoticeTexts`を空配列、または`packages`を無関係な非空一覧に変更すると通る。新規回帰の修正前実行で2 subtestとも「ValueError not raised」を再現した（1 test、runner0.011秒、全体0.15秒）。これは配布許可を既に得たという意味ではない。

`verify_notices`は既存の境界検査に加え、現在のCargo lock／オフラインmetadata（locked・arm64 Mac filter）とnpm lock／インストール済みproduction依存、固定補完indexから既存collectorを再実行する。通知本文はbyte単位、inventoryは順序を正規化したJSON全体で照合する。不足一覧やpackage／出典等の変更も拒否し、収集エラーは署名・送信前に停止する。再収集はメモリ上で行い、通知fileを書き換えず、通信／build／license承認を行わない。配布のコピー後・最終検査の両方で同じ照合を行う。

## 検査の種別と結果

3回帰を追加：2種類のinventory変更拒否（1 testの2 subtest）、offline／locked／arm64コマンドと実collectorの不正metadata拒否、pipelineで不一致を署名／notarytool前に拒否し成功resultを残さないこと。pipelineはsynthetic bundle／collector境界のunit検査であり、実署名・実送信の結果ではない。別の実装moduleを使うcollector境界testもCargo応答はmockし、実lockの読取りと不正graphの拒否を確認する。

関連25 tests成功（runner0.346秒、全体0.44秒）。Python全体155 tests中154成功・既存の専用環境1 skip（runner12.367秒、全体12.51秒）。ログ `/private/tmp/mon-notice-origin-before-20261011.log`、`/private/tmp/mon-notice-origin-after-20261011.log`、`/private/tmp/mon-notice-source-python-20261011.log`。意図したbuild failure／interrupt検査の診断文は期待されたunit結果であり、正式build失敗と混同しない。

mockなしの実offline収集・照合も実施：前回QA `/private/tmp/mon-minutes-retry-focus-qa.LdNss3/MON Onboarding QA.app`（全体0.81秒）と今回UIの最終QA `/private/tmp/mon-minutes-paper-qa.DVtnBr/MON Onboarding QA.app`（0.50秒）。いずれも現在の通知本文・全inventoryと一致し、278 packages、状態 `collected_not_reviewed`、不足12件を保持した。実コマンドはCargo metadataだけでbuild／通信ではない。

- notices SHA `f2e06524432cfeaa8811079adff7113d7fe3c728c5a90c6733af08c0658a83ef`。
- inventory SHA `28df49828f758079c625b4281941234e312f171f2b14a562fe5fa98df6454341`。
- supplement index SHA `d28f031194354a42b4a97e2e6cf883ff225114abb23f0c6c8bd0d938c31d8ba7`。

## 時間・留保

調査／再現／実装／回帰追加は約5分（概算・UI／文書作成除外）。本補強だけのbuildは不要・未実行。UI commitのQA buildを再利用して収集を実照合したため、そのbuild時間を重複計上しない。テストは上記の実測時間。Rust test cache8357MiBを保持し、追加取得0bytes。

不足12件・SDK由来条件・別導入モデル／内包CLIの配布条件・Developer ID／公証／Gatekeeper／実更新／試用を残す。通知検査の一致は配布条件の適合証明ではない。[版固定補完の留保](macos-notice-supplements-2026-10-10.md)と[全23タスク監査](implementation-completion-audit-2026-10-11.md)を維持。暫定残工数12〜29時間以上（非録音4〜8、現在実施しない実録音／耐久5〜8、配布／更新／試用3〜13）、外部署名準備待ちは別。正式評価／Windowsは保留で目標完了にしない。
