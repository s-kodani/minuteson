---
type: "Guide"
title: "GitHubでの開発とリリース"
description: "ブランチ・PR・文書更新・必須検査と、mainの最終ビルドを再利用する手動公開と、承認後のStore送信。"
tags: ["development", "github", "release"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/.github/workflows/desktop-platform-build.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/.github/workflows/desktop-daily.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/cd647f3d8c5917619f266eba8d2112d909182862/.github/dependabot.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/.github/workflows/release-controller.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/.github/workflows/release-build.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/tools/mon_release.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/16fc32b82fe1d521acbc691951e4f47a9d1445fd/tools/mon_ci_scope.py"
  - resource: "https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/configuring-issue-templates-for-your-repository"
  - resource: "https://docs.github.com/en/pull-requests/concepts/helping-others-review-your-changes"
---

# GitHubでの開発とリリース

## 開発の流れ

最新の`main`から作業ブランチを作り、コード・テスト・関連文書を同じPRで更新します。作業中の変更がある場合は専用worktreeで分け、他の変更を混ぜません。

1. 目的に対応するブランチを作成します。例は`fix/audio-import`や`perf/ci-artifact-promotion`です。
2. 実装・必要なテストと、変更した仕様・操作・制約の文書を更新します。
3. ブランチをpushし、`main`向けPRを[PRテンプレート](../../.github/pull_request_template.md)に沿って作成します。変更後の動作、検査結果、未実施の検証、文書更新先を記載します。
4. `CI gate`・`Security gate`の成功と差分を確認してマージします。保守担当者1名の運用ではPR承認数を0にし、本人が確認してマージします。
5. mainの正式ビルドを手元で検証してからReleaseを下書きします。mainへのマージでは公開しません。

Dependabotは毎週月曜10:00（日本時間）に更新PRを作成します。[DevSecOps運用](devsecops.md#githubの初期設定)の条件と通常のPR検査を通し、担当者が取り込みます。

```mermaid
flowchart TD
    pr[PRの差分に応じた検証] --> main[mainへマージ]
    main --> full[全件検証・セキュリティ検査]
    full --> build[正式EXE・MSIX・署名公証済みDMGを保存]
    build --> local[同じ成果物で最終ローカル検証]
    local --> draft[固定SHAのRelease下書きを保存]
    draft --> attach[保存済み成果物を自動添付]
    attach --> publish[担当者がPublish release]
    publish --> tag[新規タグ作成・添付済み成果物を公開]
    tag --> approval[ActionsでStore送信を承認]
    approval --> store[同じMSIXをStoreへ送信]
    store --> poll[提出IDでGET状況照会]
```

## Rustの版の固定

使うRustの版は[`rust-toolchain.toml`](../../rust-toolchain.toml)で固定します。リポジトリ内で`cargo`・`rustc`を実行すると、rustupがこの版を選びます。未導入の場合は取得するため通信が必要です。環境変数`RUSTUP_TOOLCHAIN`と`cargo +<版>`の指定は固定より優先されるため、通常の開発では使いません。

指定のない`stable`を使うと、手元だけが新しい版になり、CIと違う結果になります。固定版より新しい1.100系には次の非互換があります。

| 対象 | 内容 | 確認状況 |
| --- | --- | --- |
| Windowsの録音ヘルパー | Rust標準ライブラリが`synchronization.lib`を要求するようになりました。[CMake定義](../../probes/windows-capture/CMakeLists.txt)がRustの静的ライブラリ向けに指定するOSライブラリに含まれないため、`WaitOnAddress`などが未解決でリンクに失敗します。 | 1.100.0でのQAビルドで再現 |
| テスト補助処理 | Cargoのビルド出力の配置が変わり、`target/debug/deps`が作られません。[macOSの所有エンジン検査](../../app/src-tauri/src/local_engine/guard_tests.rs)はテスト実行ファイルと同じ場所の`libc`ライブラリを探し、[Rustテストのキャッシュ整理](../../tools/mon_rust_tests.py)は`debug/deps`を対象にします。 | 配置の変化はWindowsの1.100.0で確認。2つの処理への影響はコードの確認で、macOSでは未再現 |

版を上げる場合は、上の非互換への対応と次の更新を同じPRで行い、全件CIで確認します。

1. `rust-toolchain.toml`の`channel`を更新します。
2. `.github/workflows/`で`dtolnay/rust-toolchain`を使う箇所の`toolchain`と固定コミットを同じ版へ揃えます。`rust-toolchain.toml`はワークフローの指定より優先されるため、片方だけを変えると指定と実際の版が食い違います。
3. この節の非互換の記述を更新します。

## Issue・PRの形式

本文は日本語を既定とし、1つの目的に絞ります。大きな変更は先にIssueで課題・目的・範囲を共有し、PRには関連Issueを記載します。

### Issue

GitHubのIssue作成画面で、次のフォームを選択します。重複するIssueを検索してから作成し、既存Issueで同じ問題を扱っている場合は、そのIssueへの追記を検討します。

| 種類 | 記載する情報 | 正本 |
| --- | --- | --- |
| 不具合報告 | 概要、再現手順、期待する動作、実際の動作、環境・バージョン、補足資料 | [不具合フォーム](../../.github/ISSUE_TEMPLATE/01-bug-report.yml) |
| 機能・改善提案 | 解決したい課題、利用場面、提案内容、期待効果・受け入れ条件、代替案、関連Issue・補足資料 | [提案フォーム](../../.github/ISSUE_TEMPLATE/02-feature-request.yml) |
| 開発・保守作業 | 目的、対象範囲、実施内容、完了条件、検証方法、関連Issue | [作業フォーム](../../.github/ISSUE_TEMPLATE/03-task.yml) |

重要項目はフォームで必須とし、補足資料や代替案等は任意です。不明な条件は不明と記載し、推測で補いません。本文・ログ・画像・音声には秘密情報・個人情報・会議データを含めず、再現には合成データを使用します。脆弱性の疑いは[セキュリティ方針](../../SECURITY.md)に従って非公開で報告します。[選択画面の設定](../../.github/ISSUE_TEMPLATE/config.yml)で通常の空白Issueを無効にし、非公開報告窓口を案内します。

### PR

[PRテンプレート](../../.github/pull_request_template.md)に沿って、目的・概要・関連Issue、ふるまい・I/F・例外それぞれの追加／変更／削除、性能・セキュリティへの考慮、検証、文書更新、初期設定・運用への影響を記載します。変更は変更前と変更後が分かるように説明します。

I/FはAPI・IPC・公開型・CLI・保存形式などの関係する契約を含みます。例外は例外型・エラー応答だけでなく、異常条件・通知・復旧・再試行の変更も含みます。「該当なし」には理由を書き、未確認の内容と区別します。検査結果には対象環境を添え、未実施の検証・性能測定には理由を記載します。互換性・データ移行・設定・運用手順に影響する場合も明示します。

解決して閉じるIssueには`Closes #番号`を使い、関連するだけなら`Refs #番号`を使います。追加コミットや範囲変更の後は、タイトルと本文を最終差分に合わせて更新します。PR本文の形式を理由に失敗させるCIゲートは設けず、テンプレートとスキルで統一します。

### Codexで作成・整備する

[github-issue-workflow](../../.agents/skills/github-issue-workflow/SKILL.md)と[github-pr-workflow](../../.agents/skills/github-pr-workflow/SKILL.md)は、それぞれ形式の導入・更新と本文の作成・編集に使用します。たとえば「`$github-issue-workflow`でこの不具合のIssue本文を下書きして」「`$github-pr-workflow`で現在の差分からPR本文を作成して」と依頼できます。

スキルは既存規約とテンプレートを確認し、GitHubツール経由の本文も同じ項目に揃えます。形式導入や下書きだけの依頼では投稿せず、実際の作成・更新は依頼範囲に従います。フォーム・テンプレートはデフォルトブランチへの反映後にGitHubの作成画面へ適用され、既存Issue・PRの本文は一括変更しません。

## PR・日次・リリース前の検証

PRは変更した経路の検査だけを実行します。最終ゲートは常に完了し、実行予定の失敗・取消・省略、変更範囲の判定失敗を拒否します。

| PRの変更 | 実行する主な検査 |
| --- | --- |
| 文書・エージェント向け説明と表示設定のみ | 文書・秘密情報・ワークフロー方針・最終ゲート |
| フロントエンド | フロントエンド／両OSのブラウザー・JS/TS CodeQL |
| Rust・共通コア | 通常ネイティブ・該当言語CodeQL |
| IPC契約・生成契約・プラットフォーム設定 | フロントエンドとネイティブ・該当言語CodeQL |
| Pythonツール・テスト | Python・Python CodeQL。native CIやruntimeの準備ツールでは対応するネイティブ検査も実行 |
| 依存・ワークフロー・未分類ファイル | 関係する検査。未分類・ワークフローは全経路 |

正本は[変更範囲判定](../../tools/mon_ci_scope.py)です。削除を含め、renameは削除＋追加として両方を判定します。アプリ同梱Markdown、第三者通知は文書だけの変更に分類しません。

文書扱いには、`README.md`・`SECURITY.md`・`AGENTS.md`、`docs/`・`assets/readme/`、`evaluation/`のMarkdown、PRテンプレート、Issueフォーム3種類と選択画面の設定を含めます。Issue用ディレクトリ全体を除外せず、登録した4ファイルだけを対象にします。`.agents/skills/`配下のMarkdownと各スキルの`agents/openai.yaml`も、アプリの検査・ビルドを必要としない作業説明・表示設定として扱います。

音声取り込みfixtureと第三者資料の3ディレクトリ（`model-licenses`・`notices`・`runtime-notices`）の案内用`README.md`、`evaluation/runs/.gitkeep`、worktreeの準備設定（ルートの`orca.yaml`・`.worktreeinclude`）も軽い検査だけを行います。ライセンス本文・通知index・モデル定義・実行スクリプトまで一括除外せず、未知のファイルは引き続き全経路を検査します。文書とコードが混在する変更はコード側の検査を維持し、手動fullは文書だけでも全件を実行します。

通常Rustはライブラリのテストを最大2プロセス×4スレッドで実行します。0件のバイナリテスト・doctestのための追加コンパイルを省き、実行ファイルのリンクは正式パッケージ生成で確認します。全件検証のスレッド設定は維持します。

通常ネイティブ検証でも、macOSのSwift検査とWindowsの録音ヘルパーのCMake・CTestを実行します。WindowsのBluetooth再接続・停止待ち・単一WAVの回帰検査はPR段階で必須とし、推論資材やインストーラーを準備する全件検証まで先送りしません。

mainのコード変更は[Main build](../../.github/workflows/release-build.yml)に集約し、全件Rust・Swift／Windows連携・実モデル・フロントエンド・セキュリティと最終パッケージ生成を一度実行します。EXEとMSIXは、資材を準備した同じWindows jobで順に作ります。macOSは検証jobで最終アプリを準備し、資格情報を持つ別jobで署名・公証します。文書だけの変更では軽い検査だけを行い、配布物を生成しません。

同じSHAに対する日次ネイティブ全件実行は廃止し、[Desktop full validation](../../.github/workflows/desktop-daily.yml)の手動実行を残します。[Security validation](../../.github/workflows/security.yml)の日次検査は毎日09:30（日本時間）に継続します。公開時にテストやビルドを再実行しません。

Actionsの**Desktop platform validation → Run workflow**では`quick`／`full`を選びます。既定は`full`で、QAインストーラー・MSIXの内容検証を含みます。この診断用実行はバイナリを配布せず、JSON証跡と診断を残します。ローカルのコマンド・分類は[テストガイド](testing.md)を参照してください。

## 文書を同じPRで更新する

開発フローの正本はこの文書、資格情報とGitHub設定の正本は[DevSecOps運用](devsecops.md)です。OS固有の配布処理は各配布ガイドを参照します。PRに更新先または不要の理由を書きます。CIはOKF形式、リンク・アンカー、参照先、目次到達性を検査し、実装との意味の一致は担当者が確認します。

## Releases画面でバージョンタグを管理する

[リリースノートのskill](../../.agents/skills/github-release-notes/SKILL.md)に従い、主な変更内容、ユーザー目線で変わる部分、互換ブレーク内容、対応PRを簡潔にまとめます。検証の詳細は対象のActionsや配布マニフェストへリンクし、更新判断に必要な注意事項は本文に残します。

[初期設定](devsecops.md#自動配布の初期設定)後、次の手順で公開します。

1. `app/package.json`、同ロックファイルのルート版、Tauri設定、Rustの`mon-desktop`の版を揃えたPRをmainへマージします。例は`1.2.3`です。
2. そのコミットの**Main build**が成功するまで待ちます。全件検証済みの正式DMG・EXE・MSIXを作成し、manifest・SBOM・SHA256SUMS・来歴証明とともにActionsに30日保存します。MSIX原本はStore送信用の`release-store`で保持し、Releasesの一般公開には含めません。
3. 実行サマリーの「最終ローカル検証用の配布物」からダウンロードして手元で確認します。署名・公証済みDMGと正式EXEをそのまま使います。MSIXの手元での確認は[同一原本からのローカル検証](distribution-store.md)に従い、送信には未署名原本を使います。
4. 同サマリーの「このコミットのRelease下書きを作成」を開きます。`v1.2.3`と確認した完全SHAが入力されています。ノートを編集して**Save draft**します。開始チェックボックスは不要です。
5. 約5分間隔のcontrollerが保存済み成果物を下書きへ添付します。GitHubのスケジュールは混雑で遅れる場合があります。急ぐ場合は**Release draft controller → Run workflow**をmainで実行します。再ビルド・再署名は行いません。
6. 添付完了と対象SHAを確認して**Publish release**を押します。新規タグはこの公開時に作成され、添付済みのバイナリが公開されます。ローカルでのタグpushを通常手順に含めません。タグをpushするだけではReleaseを公開しません。
7. **Microsoft Store submission**がmainで承認待ちになります。Actions → 対象実行 → **Review deployments** → `release-store` → **Approve and deploy**で承認すると、同じmain実行のMSIXを送信します。却下した場合は送信しません。

Targetを`main`にして手動作成した下書きは、最初の検出時のSHAへ固定します。手元で確認したコミットからmainが進んでいる可能性があるため、通常はサマリーのリンクを使ってください。既存タグを選んだ場合は実際のタグの解決先を採用し、そのSHAの保存済み正式ビルドを要求します。

状態コメント`mon-release`には添付したrun・attempt・artifact IDを保持します。来歴の証明にはGitHub attestationを使います。ノートの編集は可能ですが、対象SHAやタグ、添付したバイナリは途中で差し替えません。Windows EXEの未署名や、実機確認・ライセンス審査等の未確認項目を、CI成功だけで確認済みとしません。

## 失敗・試行・再実行

mainビルドに失敗した場合は原因を確認し、**Re-run failed jobs**で該当検査・生成を再開します。成功済みの同一SHAの候補は通常の再実行で上書きしません。コード修正は新しいmainコミットで作成し、手元の確認もその成果物で行います。

添付の通信失敗はcontrollerの次回実行で再開します。同名・同サイズ・同SHAの添付物は省略し、欠けた物だけを追加します。異なる添付物や予期しない添付物がある場合は停止し、自動削除・上書きはしません。対象が公開済みになった場合も操作を停止します。

成果物の不足・期限切れ・SHA不一致では下書きの添付を停止します。最新mainや別runへの自動切替、リリース時の再ビルドは行いません。新しい候補でやり直す場合は、明示的に新しいmainビルドと手元の確認から進めます。公開済みタグ・バイナリの修正は新しい版にします。

Store送信の中断は[Store更新の再開](devsecops.md#store更新の有効化と再開)に従います。定期処理は既存の提出IDの状態だけを照会し、`PendingCommit`でも送信を再開しません。再送信には対象IDを指定した手動実行と承認が必要です。

## 削減効果の見積もり

2026-11-13の調査値を基準にした計画上の予測です。下記は同じrunner・依存キャッシュ条件でのCI比較を完了した実測値ではありません。コンパイル時間とテスト実行時間を分けて評価します。

| 対象 | 変更前の調査値 | 変更後の目安 | 短縮の目安 |
| --- | ---: | ---: | ---: |
| 通常macOSのnativeステップ | 25.2分 | 20〜23分 | 2.2〜5.2分、9〜21% |
| 通常Windowsのnativeステップ | 14.8分 | 10〜12分 | 2.8〜4.8分、19〜32% |
| 全件macOSのnativeステップ | 36.7分 | 30〜35分 | 1.7〜6.7分、5〜18% |
| 全件Windowsのnativeステップ | 58.8分 | 50〜56分 | 2.8〜8.8分、5〜15% |
| Release下書きへの添付 | 約91分の検査・生成を含む旧フロー | 添付1〜5分＋検出待ち | 検査・生成はmainへ移動 |

テスト実行部分だけでは15〜30%削減を予測します。0件doctestの追加コンパイルは調査時に各OS約136秒でした。バックアップの入力生成共有、重複する成功復元と障害注入行列の削減の根拠は[残す保証の対応表](testing.md#重複削減と残す保証)に記載します。

新しいmain全体は全件検証と正式パッケージ生成を含むため、約60〜80分を粗い目安とします。旧mainの通常検証と単純比較しません。旧リリースの約91分が総CI時間からそのまま消えるわけではありません。日次の重複停止・公開時の重複検査停止・PRの範囲別実行で、不要な実行回数も減らします。

導入後は各OSで通常／全件を同じrunner・キャッシュ条件で各1回比較し、計測ノイズが大きい場合だけ追加します。`rust-tests-*`のコンパイルと実行の内訳、Windows準備・パッケージ生成の内訳、公開までの待ち時間を確認します。

この変更のローカル確認では、macOSでバックアップ162件（実行約63.5秒、コンパイル約419秒）と再確認・障害回復87件（実行約94.6秒、コンパイル約0.5秒）が成功しました。ビルドキャッシュ条件が異なるため、これらから上表の削減率を実測済みとは扱いません。Windows・署名公証・Actions上の全件所要時間は導入後の確認です。
