---
type: "Specification"
title: "MinutesOnの製品概要"
description: "機能範囲、対応環境と配布・評価の確認状況。"
tags: ["product", "release"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/d251874f4b7d4a65238a1bb4248c89525f4f7a01/app/src-tauri/src/startup_window.rs"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/src-tauri/tauri.conf.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/macos-distribution.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/verification/windows-store-dpi-process-audit-2026-11-01.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/docs/verification/m4-01-macos-native-qa-preparation-2026-10-07.md"
  - resource: "https://github.com/s-kodani/minuteson/blob/6d0dedff432144125524e2a18584fb4ffba0ba5d/LICENSE"
---

# MinutesOnの製品概要

MinutesOnは会議の録音・文字起こし・話者分離・議事録生成をローカルで行うデスクトップアプリです。1.0.9のリリース準備中で、公開版は[GitHub Releases](https://github.com/s-kodani/minuteson/releases)で確認できます。実行ファイル・保存形式などの内部識別子には略称monを使います。

## 対応環境

| OS | 対象 |
| --- | --- |
| Windows | Windows 11 24H2以降、x64 |
| macOS | macOS 15以降、Apple Silicon |

モデル導入後のAI処理はPC内で実行します。初回のモデル取得は外部配布元への通信を伴い、モデル本体と取得素材を置く空き容量が必要です。ブラウザーだけで画面を表示した場合はデスクトップ接続がなく、録音・AI処理は利用できません。

## 機能

- [録音・音声取り込み](recording.md)：マイク・PC音声、5秒テスト、ミニパネル、保存音声の取り込み。
- [文字起こし・話者・用語](analysis.md)：自動解析、本文・話者の修正、声の登録と照合、共通・会議別辞書。
- [モデル管理](models.md)：全体設定での選択、ダウンロード、切替、導入済みモデルの自動整理。
- [議事録](minutes.md)：ローカル生成、Markdownプレビュー・編集・保存・出力。
- [データ管理](storage.md)：手動バックアップ、追加復元、保存先変更、削除、異常終了からの回復。

表示はモノトーン／パープルとシステム／ライト／ダークを独立して設定します。初期設定はモノトーン・システムです。保存した設定は補助ウィンドウと通知にも反映します。

メインウィンドウは起動時に1280 × 820論理ピクセルを上限とし、OSの枠を含めて画面の作業領域の90%以内に収まるよう縮小して中央に表示します。通常の最小サイズは960 × 640ですが、小さい画面では必要な寸法だけ最小値を下げます。表示倍率と作業領域を取得して毎回プロセス起動時に計算し、常駐からの再表示では手動変更したサイズ・位置を維持します。前回プロセスのサイズ・位置は保存しません。通常版・QA版、macOS・Windowsで同じ方針を使います。

## 配布と評価の確認状況

### 1.0.9のリリース準備

1.0.8の公開コミット`79551bc335b31bf2dc8b47dcb4fcfa0c9236bd23`を基に、1.0.9を準備しています。起動時に見つかった異常の影響を対象の会議や操作へ絞り、読み取れる会議を新しい保存先へ復元します。録音の補助情報の不整合から検証済み音声を救出し、一部の旧FLACの読み込みにも対応します。会議状態と利用可能な操作の案内、起動中の処理表示、会議一覧と重複モデル整理の性能を改善し、llama.cppを脆弱性修正版へ更新します。DBスキーマ40とバックアップ形式8は維持します。

版番号を揃えたmainで全件CI・セキュリティ検査と正式配布物の生成・照合を行い、同じSHAのRelease下書きへ保存済み成果物を添付します。今回の範囲は下書き完成までです。過去のQA版の確認結果を今回の正式配布物の実機確認へ転用せず、両OSの実録音・更新後のデータ保持とライセンス審査は実施した範囲を個別に記録します。下書きの公開とStore送信は行いません。

### 1.0.8の準備時点の記録

以下は1.0.8の準備時点の記録です。1.0.8は[GitHub Releases](https://github.com/s-kodani/minuteson/releases/tag/v1.0.8)で公開済みです。

1.0.7の公開コミット`bf05ab78357aca5699700ffd3225fee11b75e3f8`を基に、1.0.8を準備しています。録音の補助ウィンドウを録音開始時に生成し、録音・保存の終了後に解放して待機中のメモリ使用量を抑えます。録音終了確認は通常の定期取得を省き、状態変更の通知を使います。内部ログでは日時整形の再利用と不要な一覧再描画の抑制により、画面移動時の負荷を減らします。

この性能改善による新しい通知トーストは追加しません。1.0.8には、通常録音を録音中から単一WAVへ合成し、取り込み・5秒テストもWAVへ統一する変更を加えます。録音のCAF・中間FLACと停止後の全体再ミックスを省き、取得・変換・合成と同期保存の順序を分離します。音量は区間ごとの取得入力数で平均し、旧未完了音声は専用処理で救出します。完成済みWAV・AIモデル形式は維持します。DBは取り込み確定条件の更新に限ってschema 39から40へ移行し、既存データを引き継ぎます。互換性の方向と旧版へ戻す条件は[保存仕様](storage.md#107との互換性)を参照してください。

性能改善とWAV化を含む最終main SHAで全件CI・正式配布物の生成・実機確認を個別に記録し、GitHub Releaseは検証済み候補の下書き添付完了までを今回の範囲とします。既存の旧1.0.8候補へ別SHAのバイナリを上書きせず、新しい候補に対応する下書きへ切り替えます。統合後の全件CI・正式配布物・両OSの実録音と更新確認を完了済みとは扱わず、1.0.8の公開・Store送信も行っていません。

WAV化を統合する前の性能改善では、同じMacで新規の空データ領域を使い、旧版・新版ともReleaseビルド（`custom-protocol`）のネイティブUIを起動して、モデル未ロードの待機時メモリを比較しました。各版の起動後1分以上待ち、`footprint -p PID -t`で本体と関連プロセスの合計を5秒間隔で3回取得しました。旧版は231／231／232 MB、新版は158／158／158 MBで、中央値は231 MBから158 MBへ約32%減少しました。関連するWebContentプロセス数は3個から1個になりました。

同じ統合前の性能改善に対して、macOSのWebKit 27.2（Playwright build 2370）、1200×900の画面と合成ログを使い、定期処理を止めた同一条件で各3回、内部ログから設定画面への移動を測定しました。クリック処理開始からログDOMの削除までの中央値は、200件で191 msから34 ms、1,000件で849 msから52 msへ短縮しました。これは開発画面と合成入力の比較であり、配布版や他端末の所要時間を保証する値ではありません。

### 1.0.7の準備時点の記録

以下は1.0.7の準備時点の記録です。1.0.7は[GitHub Releases](https://github.com/s-kodani/minuteson/releases/tag/v1.0.7)で公開済みです。

mainのコミット`fdb06eac4fa7ea7824c8933a9139a38dd85e73b7`を基に、1.0.7を準備しています。Windows・macOSで、正常停止・起動時の回復・再試行を録音ごとに1件の「録音の保存」ジョブと履歴へまとめ、既存の重複履歴を移行します。話者分離には、既存ERes2Netに加えてCPUで実行する「Pyannote int8＋TitaNet small」を選択できます。現在使用しているモデルと設定値は自動変更しません。

今回の対応は機能実装とその検証です。人手による音声品質・精度の評価は別途実施し、TitaNetによる過剰分離の改善や既定モデルへの採用を確認済みとは扱いません。過去の性能測定は対象モデル・入力・端末に限った結果で、今回の配布物や他端末の保証値へ転用しません。mainの全件CI・正式配布物の生成・実機確認と、1.0.7の公開・Store送信は、それぞれの完了記録で確認します。

### 1.0.6のリリース準備

以下は1.0.6の準備時点の記録です。以後の公開状況は[GitHub Releases](https://github.com/s-kodani/minuteson/releases)で確認してください。

mainのコミット`675b3e976bb9532a9e6983c6c91d258cedcc872f`を基に、1.0.6のリリースを準備しています。ReazonSpeechの準備時に付属文書の外部取得が失敗する問題を、同梱文書の再利用で解消します。Windows・macOSで、導入済みモデル・話者特徴量と、完了済み取得操作に残る検証済みアーカイブも再利用し、重複した外部取得を削減します。

版番号更新後のmainで全件CI・セキュリティ検査と正式配布物の生成・照合を行い、その成果物を確認してRelease下書きへ添付します。1.0.6の公開・Store送信は未実施です。過去版やQA版の確認結果を、今回の正式配布物の実機確認結果として扱いません。

### 1.0.5

2026-11-18（日本時間）に、コミット`d920b2e332ba2e0e7352b2f6589073bd387f3386`を対象とする[1.0.5を公開](https://github.com/s-kodani/minuteson/releases/tag/v1.0.5)しました。対象の[Main build](https://github.com/s-kodani/minuteson/actions/runs/44112327242)は成功しています。Windows・macOSでの音声機器切替時の再取得と音声保持、接続状態の表示、中断録音の自動保存・再試行と救出済み音声の解析結果保存の改善を含みます。

### 1.0.4

2026-11-16（日本時間）に、コミット`c22a928ac223950b6cae5f349c2f8bfef1d8266d`を対象とする[1.0.4を公開](https://github.com/s-kodani/minuteson/releases/tag/v1.0.4)しました。[Main build](https://github.com/s-kodani/minuteson/actions/runs/43733528587)の試行1で全件CI・実モデル・セキュリティ検査と、正式配布物の生成・照合が成功しました。同じ実行で生成したDeveloper ID署名・Apple公証済みDMG、Windows用未署名EXE、macOSアプリ内更新用の署名付きアーカイブ・署名・`latest.json`を、SBOM・チェックサム・ビルド来歴証明とともに再ビルドせず公開しています。添付された11ファイルのサイズ・ハッシュは、公開前の正式候補と公開後のRelease情報で一致しています。

macOS通常版の署名付きアプリ内自動更新、導入済みモデルの実行環境の更新案内と、不要になった重複モデルの自動整理を含みます。公開された更新情報の取得先も1.0.4を返すことを確認しています。更新機構のない旧macOS版からの初回移行にはDMGでの入れ替えが必要です。

Store用の未署名MSIX（`1.0.4.0`・x64）も同じmain実行で生成・照合しています。[承認後の送信](https://github.com/s-kodani/minuteson/actions/runs/43774713223)は成功し、提出ID`1152921505702411857`に対する送信確定（commit）の要求が受理されました。送信処理完了時の状態は`CommitStarted`で、1.0.4のStore審査・一般配信の完了は未確認です。

今回の自動検査と公開記録だけでは、利用端末での実機QA・ライセンス審査を確認済みとしません。配布記録の`runtimeQA`・`licenseReview`は`not_verified`、`distributionReady`は`false`を保持しています。アプリ内更新・更新後のデータ保持・Store経由更新の確認結果は、確認した版と環境に基づいて別途記録します。過去版の確認記録は1.0.4の実機確認結果として扱いません。

### 1.0.3

2026-11-15（日本時間）に、コミット`4ec0046e128df5864881d3e4357d0175aeac0607`を対象とする[1.0.3を公開](https://github.com/s-kodani/minuteson/releases/tag/v1.0.3)しました。[Main build](https://github.com/s-kodani/minuteson/actions/runs/43688157548)の試行2で全件CI・実モデル・セキュリティ検査と、正式配布物の生成・照合が成功しました。同じ実行で生成したDeveloper ID署名・Apple公証済みDMGとWindows用未署名EXEを、SBOM・チェックサム・ビルド来歴証明とともに再ビルドせず公開しています。添付された8ファイルのサイズ・ハッシュも正式候補と照合済みです。

Windows・macOSの録音中のディスク書き込み削減と「このアプリについて」画面の区切り線整理を含みます。CIの変更範囲判定、キャッシュ済み音声の変更拒否テストの時刻依存、検証後の後片付けルールも更新しています。mainの初回実行ではmacOSの画面検査で検証用サーバーへの接続リセットが1件発生しました。失敗ジョブを1回再実行して成功し、同じrunの成功済みネイティブ成果物を再利用しています。

Store用の未署名MSIX（`1.0.3.0`・x64）も同じmain実行で生成・照合しています。[承認後の送信](https://github.com/s-kodani/minuteson/actions/runs/43696252719)は成功し、提出ID`1152921505702408109`に対する送信確定（commit）の要求が受理されました。[状態照会](https://github.com/s-kodani/minuteson/actions/runs/43696822845)では`Certification`（審査中）で、審査・Store一般配信の完了は未確認です。

今回の自動検査と公開記録だけでは、実機QA・ライセンス審査を確認済みとしません。配布記録の`runtimeQA`・`licenseReview`は`not_verified`、`distributionReady`は`false`を保持しています。Gatekeeper・実機録音・更新後のデータ保持・Store経由更新の確認結果は、確認した版と環境に基づいて別途記録します。過去版の確認記録は1.0.3の実機確認結果として扱いません。

### 1.0.2

2026-11-15（日本時間）に、コミット`b0c65dd1d2f0c232d5b474f2e10b9c90b516ede1`を対象とする[1.0.2を公開](https://github.com/s-kodani/minuteson/releases/tag/v1.0.2)しました。[Main build](https://github.com/s-kodani/minuteson/actions/runs/43655189504)で全件CI・セキュリティ検査が成功しました。同じ実行で生成・照合したDeveloper ID署名・Apple公証済みのmacOS用DMGとWindows用未署名EXEを、SBOM・チェックサム・ビルド来歴証明とともに再ビルドせず公開しています。Store用未署名MSIXも同じ実行で生成・照合しています。

1.0.2では起動時のデータ復元・適用済みモデル構成の再登録、更新確認と案内、検証済み配布物の再利用を追加しています。

今回の公開作業では実機QA・ライセンス審査を新たに実施していません。配布記録の`runtimeQA`・`licenseReview`は`not_verified`、`distributionReady`は`false`を保持しています。macOSのGatekeeper・更新後データ保持、MSIXの導入確認・認定も、今回の自動検査で確認済みにはしていません。Store提出・認定・配信状況は別途確認します。

### 1.0.0の確認記録

2026-11-13（日本時間）に[1.0.0を公開](https://github.com/s-kodani/minuteson/releases/tag/v1.0.0)しました。Windowsストア公開時点のコミット`abaad2b56be302b22d31b2735d2499b3f27a9d29`を使い、下記の成功済みdry runの成果物を再ビルドせず公開しています。導入済みStore版のアプリファイル71件が当時のMSIX原本と一致し、マニフェストの差分はStoreによるPhoneIdentityの追加のみでした。その後の変更とPC音声取得の修正は1.0.1へ反映しました。以下の確認記録は1.0.0を対象とし、1.0.2の実機確認結果を示すものではありません。

2026-11-13（日本時間）に、コミット`abaad2b56be302b22d31b2735d2499b3f27a9d29`を対象とする1.0.0のdry runが成功しました。この検証時点ではインストーラーをGitHub Actionsの成果物として保存し、`v1.0.0`のGitHub Releaseは下書きでした。以下はこの検証の確認記録です。最新の公開状況とダウンロードは[GitHub Releases](https://github.com/s-kodani/minuteson/releases)で確認してください。

| 項目 | 確認状況 |
| --- | --- |
| 自動検査 | 対象コミットのmain CI・セキュリティ検査、dry run内の全件CI・セキュリティ検査が成功。Windows・macOSのブラウザー／ネイティブ回帰テストと、代表モデルを使ったランタイム検証を確認 |
| macOS | 通常版アプリとDMGのDeveloper ID署名、Apple公証、チケット添付・検証が成功（`notarization: tickets_validated`）。最終署名済みDMGのFinder表示・再マウント、ダウンロードしたDMGからのGatekeeper確認、実機での権限・録音・モデル導入、通常版の更新とデータ保持は未確認 |
| Windows | 未署名EXEとStore用未署名MSIXの生成・成果物収集、MSIXの内容照合、リンク済みEXEのDPI宣言検査が成功。対象クライアントOSでの今回の配布物の導入・実機操作・更新、Store提出・認定・一般公開は未確認。過去の検証用コピーによるWACKの総合結果はWARNINGで、DPIのInfrastructureエラーと任意検査の指摘は今回のCIでは解消確認していない |
| 配布証跡 | 全プラットフォームの成果物統合・ハッシュ照合、SBOM・チェックサム・GitHubのビルド来歴証明の生成が成功。GitHub Releaseの公開とStore提出は未実施 |
| 品質・性能 | 短い実モデル統合や合成入力の検査記録あり。16GB端末・10話者、60分処理、4時間録音、各機器・会議アプリでの網羅的な評価は未確認。CI上のランタイム検証は通常版の実機操作確認とは別に扱う |
| ライセンス | 自作コード・文書・自作画像はMIT。依存通知本文・SDK由来条件等の配布審査は未完了。依存ライブラリ・モデルの条件は個別に適用される |

確認根拠は[main CI](https://github.com/s-kodani/minuteson/actions/runs/43291696658)、[mainのセキュリティ検査](https://github.com/s-kodani/minuteson/actions/runs/43291696604)、[1.0.0のdry run](https://github.com/s-kodani/minuteson/actions/runs/43297943714)です。dry runでは[macOS署名・公証ジョブ](https://github.com/s-kodani/minuteson/actions/runs/43297943714/job/131519259835)と[成果物統合・証明生成ジョブ](https://github.com/s-kodani/minuteson/actions/runs/43297943714/job/131527178536)が成功し、後者の公開ステップは`dry_run=true`の条件でスキップされました。

添付予定の配布物は[verified-release-preview](https://github.com/s-kodani/minuteson/actions/runs/43297943714/artifacts/13989165779)、macOS単独のDMGは[release-macos](https://github.com/s-kodani/minuteson/actions/runs/43297943714/artifacts/13988415487)、Store提出用のMSIXは[release-store](https://github.com/s-kodani/minuteson/actions/runs/43297943714/artifacts/13988485309)に保存されています。これらは30日保持のActions成果物で、保持期限は2026-12-12（UTC）です。確認時点で`RELEASE_PUBLISH_ENABLED=false`、`STORE_PUBLISH_ENABLED=false`を維持しています。

今回の対象SHAには、`rustix`を1.1.6へ更新する[依存更新PR #67](https://github.com/s-kodani/minuteson/pull/67)、DMGのインストール案内・アイコン配置を改善する[macOS修正PR #68](https://github.com/s-kodani/minuteson/pull/68)、モデル取得のバックグラウンド継続と進捗表示を修正する[PR #70](https://github.com/s-kodani/minuteson/pull/70)が含まれます。通知操作と所有権検証のテストを安定化する[PR #71](https://github.com/s-kodani/minuteson/pull/71)、CI・リリース構築のRustを検証済みの1.99.1へ固定する[PR #72](https://github.com/s-kodani/minuteson/pull/72)も反映しています。対象SHAのmain検査とdry runはいずれも初回実行で成功しました。

DMG作成用の固定依存を認証情報の読み込み前に導入し、新しいDMG生成処理を含む署名・公証まで同じdry runで確認しました。取得したmacOS成果物の対象SHA、DMG・SBOMのハッシュも受領記録と照合済みです。Rust 1.100では既存のテスト補助処理が新しいビルド出力配置に対応していないため、更新時に対応と再検証が必要です。

署名・公証と自動検査の成功だけでは、実機確認・ライセンス審査を確認済みにしません。macOSの構築記録では`runtimeQA`・`gatekeeper`・`licenseReview`・`manualUpdate`が`not_verified`、MSIXの構築記録では`installedVerification`・`certification`が`pending`であり、いずれも`distributionReady`は`false`のままです。

ソースからのビルドは[macOS](../guides/build-macos.md)／[Windows](../guides/build-windows.md)、配布物の構築・検証は[macOS配布](../guides/distribution-macos.md)、[Windows EXE](../guides/distribution-windows.md)、[Microsoft Store](../guides/distribution-store.md)の手順を参照してください。性能や文字起こしの精度は端末・モデル・音声に依存し、上表の未測定条件について保証値を設けません。
