---
type: "Guide"
title: "DevSecOpsの検査と運用"
description: "セキュリティ検査、例外、GitHub保護設定、署名とStore認証の初期設定。"
tags: ["security", "ci", "maintenance"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/130c3902eea205bc0ca599c9da45635bb8974ede/.github/workflows/security.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/130c3902eea205bc0ca599c9da45635bb8974ede/tools/mon_security.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/130c3902eea205bc0ca599c9da45635bb8974ede/tools/mon_macos_ci_sign.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/6d0dedff432144125524e2a18584fb4ffba0ba5d/tools/mon_store_submission.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/95ec6d6e28e6cccbba792a534da44341157008a1/.github/workflows/release-build.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/95ec6d6e28e6cccbba792a534da44341157008a1/tools/mon_macos_release.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/6d0dedff432144125524e2a18584fb4ffba0ba5d/tools/mon_store_provenance.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/6d0dedff432144125524e2a18584fb4ffba0ba5d/.github/security-exceptions.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/0648448fa12d6997d9611c34d4a65a1d6a53727f/.github/workflows/desktop-platform-build.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/94b513548614f37cb070273abe492975d062b29d/.github/dependabot.yml"
---

# DevSecOpsの検査と運用

## 検査と判定

セキュリティ検査はPR、main更新、毎日09:30（日本時間）、リリース前に実行します。この実行条件と`Security gate`は維持します。通常テストと重いネイティブ・配布物検証の分離は[GitHubでの開発とリリース](development-workflow.md#pr日次リリース前の検証)に集約します。全件の日次検証は別ワークフローで04:17（日本時間）に実行し、リリース前には対象SHAで全検証を再実行します。

| 検査 | 対象と停止条件 |
| --- | --- |
| Gitleaks | 初期・通常ブランチの全履歴、PRの追加コミット。秘密情報を検出すると停止し、値はログで伏せます |
| OSV-Scanner | Cargo・npmの固定依存と、ビルド定義・モデルカタログから抽出したネイティブ依存の版・コミット。CVSS 7.0以上と重大度不明を停止します |
| CodeQL | Actions、JS/TS、Python、Rust、C/C++、Swift。SARIF内のHigh以上のセキュリティ指摘と重大度不明を停止します。Swiftは実ビルドで解析します |
| 文書検査 | OKFメタデータ、内部リンク・アンカー、コード参照、目次到達性 |

低・中程度の指摘と保守終了の通知は診断成果物で確認します。別名IDは同一の指摘としてまとめ、既知の重大度を共有します。検査サービスへの通信失敗、壊れた出力、必要なジョブの取消・未実施は成功にしません。CodeQLが非公開設定により省略された場合は、その未実施を明示します。解析未実施の状態をSAST合格と扱いません。

OSVのネイティブ依存の収録範囲は完全ではありません。SDK、CRT、OSライブラリ、モデルの重みはこの脆弱性検査の対象外です。取得時の既存ハッシュ照合は維持し、毎週の保守時に固定資材と上流の更新・通知も確認します。SBOMにはCargoの通常・ビルド依存、npmの製品依存、ネイティブ取得定義を記録し、任意ダウンロード資材が含まれる範囲も明示します。

CodeQLの解析ジョブが成功しても安全とは断定せず、検出結果を別途評価します。独自IPC・OS権限・保存操作の安全性は既存テストと変更レビューでも確認します。

## ローカル検査

```sh
python3 -m venv .cache/security-venv
. .cache/security-venv/bin/activate
python -m pip install -r tools/requirements-security.txt
make check-docs
make check-security
```

`check-security`は固定版スキャナーを取得し、コミット済みのSHA-256と照合して利用します。ネットワーク接続が必要です。CodeQLの全言語解析はGitHub CIで実行し、ローカルの依存・秘密情報検査と区別します。ツール版とハッシュの正本は[検査ツール定義](../../tools/security-tools.json)です。

## 検出時の対応と例外

秘密情報を見つけた場合は、まず該当サービスで失効・再発行し、影響範囲と履歴の扱いを確認します。検出値をIssueやPR、ログへ転載しません。脆弱性は影響と修正版を確認して更新PRを作成し、再検査します。一般からの報告先は[セキュリティ方針](../../SECURITY.md)です。

例外は[例外定義](../../.github/security-exceptions.json)の`exceptions`へ追加します。`tool`は`osv`または`codeql`、`id`は指摘ID、`target`はレポートの完全一致対象、`reason`は影響しない根拠、`owner`は担当者です。`created`と`expires`はUTCの日付を`YYYY-MM-DD`で記載し、作成日から最長30日、期限日は無効とします。理由と担当者を含む全項目を必須にし、包括的な除外は認めません。重大度不明を無条件に低と見なさず、確認結果を残します。Gitleaksの一括除外は行いません。既知の非秘密値は、ファイルと検出値を完全に限定した設定で扱います。現在の除外は履歴内のSQLite設定キー`meeting_mix_v1_initialized`だけです。

2026-11-03の依存検査では、旧任意ランタイム`llama.cpp b11533`のRPCサーバーに対する[CVE-2026-118375](https://github.com/ggml-org/llama.cpp/pull/30399)と[CVE-2026-119052](https://github.com/ggml-org/llama.cpp/pull/31204)を個別判断しています。議事録処理は固定引数の`llama-completion`を起動し、`LLAMA_ARG_RPC`を含む継承環境を消去します。RPCサーバーを起動する呼出しはありません。macOSの旧任意パッケージにはRPCライブラリが含まれますが、Windows配布対象にはRPC実行物・DLLを含めません。この到達性の判断に限定し、2026-11-17 UTCまでの例外とします。上流バイナリ全体の安全性確認や修正完了を意味しません。RPCを導入する前、または期限までに再評価し、旧ランタイムを修正済み版へ更新する際はレシピ・ハッシュ・既存モデルの互換性を検証します。

## GitHubの初期設定

1. 全履歴の秘密情報検査と依存検査を実施し、指摘を修正または個別判断します。Actions履歴・ログと保存済み成果物の公開範囲も確認します。
2. main上のワークフローを有効にし、標準ランナーの利用上限・権限・資材取得を確認します。Desktop platform validationの`full`とSecurity validationをmainで実行し、`CI gate`・`Security gate`とCodeQL各言語の結果を確認します。
3. CodeQLはこのリポジトリのadvanced setupを使い、default setupとの重複実行を避けます。Dependency graph・Dependabot alerts/security updatesとSecret scanning・Push protectionを有効化します。Private vulnerability reportingを有効化し、[非公開報告のリンク](../../SECURITY.md#非公開の報告)が利用できることを確認します。
4. mainのrulesetでPR、`CI gate`と`Security gate`の成功、force push・削除の禁止を設定します。チェック名は初回実行結果から選択します。承認数は0とし、本人が自分のPRを確認してマージできるようにします。通常のバイパスは設けません。
5. `v*`タグの更新・削除を禁止します。新規作成はReleasesと公開用ワークフローに許可します。Actionsの既定権限はread-only、forkワークフローの実行承認を有効化します。各ジョブで必要な権限だけを宣言します。

設定や検査が未完了の状態を導入済みの保護と扱いません。Dependabotの通常更新はnpm/Cargo/Actionsと検査用Python依存が対象です。現在は[Dependabot定義](../../.github/dependabot.yml)の全4対象を`open-pull-requests-limit: 0`で停止しています。CI・セキュリティ検査と保護設定を確認してから、上限を`5`へ戻すPRを検査・マージして再開します。バージョン更新はActions停止と別に動作します。[GitHubの停止手順](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/secure-your-dependencies/configure-version-updates#disabling-dependabot-version-updates)に従い、上限`0`は通常のバージョン更新を止めるもので、セキュリティ更新は別のリポジトリ設定で管理します。スキャナーのバイナリ定義と手動固定する資材は保守担当者が確認・更新します。

## 自動配布の初期設定

配布条件と動作確認状況は[製品概要](../spec/overview.md#配布と評価の確認状況)で確認します。第三者通知・SDK・CRT等の条件と、対象経路の実機・署名・Store確認が完了するまで、`RELEASE_AUTOMATION_ENABLED`・`RELEASE_PUBLISH_ENABLED`・`STORE_PUBLISH_ENABLED`は未設定または`false`を維持します。通常CIのQAインストーラー・MSIX生成は検証用で、バイナリをActions成果物へアップロードしません。

環境`release-macos`、`release-publish`、`release-store`を作成し、deployment branchesを**mainのみ**に限定します。ワークフロー定義はmainで実行し、検査対象コードは固定SHAで取得します。自動公開のためrequired reviewersや待機時間は設定しません。

`release-macos`のSecrets:

- `MACOS_CERTIFICATE_P12_BASE64`: Developer ID Application証明書と秘密鍵を含むP12のBase64
- `MACOS_CERTIFICATE_PASSWORD`: P12のパスワード
- `APPLE_NOTARY_KEY_BASE64`: 公証に利用できるApp Store ConnectチームAPIキー（P8）のBase64

同環境のVariables:

- `APPLE_SIGNING_IDENTITY`: 対象Teamの完全なDeveloper ID Application署名名
- `APPLE_TEAM_ID`: Team ID
- `APPLE_NOTARY_KEY_ID`、`APPLE_NOTARY_ISSUER_ID`: 公証APIキーの識別子

資格情報をGitやPR本文へ記載しません。ビルドジョブには渡さず、準備済みアーカイブを検証した署名ジョブだけで一時ファイル・キーチェーンへ導入します。署名ジョブはビルド処理を実行せず、終了時に削除します。Secretsへ登録するための値をCIログへ出力しません。

`release-publish`には独自PATを登録せず、ジョブの`GITHUB_TOKEN`とOIDCで公開・証明を行います。下書き受付の`guard`も、GitHubの下書き参照に必要な`contents: write`を持ちます。この権限は受付ジョブに限定し、アプリのビルドやコード検査には書き込み権限を渡しません。macOS準備処理は`cargo fetch --locked`で依存を先に取得してから、オフラインのライセンス収集を含むビルドへ進みます。

所有者が公開設定、CI、資格情報を確認してから、Repository variable **`RELEASE_AUTOMATION_ENABLED=true`**を設定します。未設定では下書きの自動処理を行いません。**`RELEASE_PUBLISH_ENABLED`は未設定または`false`のまま**、公開開始指定のある下書きで試行します。この状態の検出処理は`dry_run=true`を指定し、公開せずに成果物を作成します。試行結果を確認してから**`RELEASE_PUBLISH_ENABLED=true`**にし、claim済み下書きはmainのRelease buildを`dry_run=false`で実行します。以降に作成する下書きは通常どおり自動公開します。実際のApple署名・公証は別途成功を確認し、模擬検査の結果から成功を推測しません。

## Store更新の有効化と再開

初回公開はPartner Centerで行い、年齢区分・掲載情報・プライバシー情報等を整えます。初回公開完了後、Partner Centerへ関連付けたEntraアプリの認証を用意します。[Store配布](distribution-store.md)も参照してください。

`release-store`のVariablesは`STORE_TENANT_ID`、`STORE_APPLICATION_ID`（Store ID）、`STORE_CLIENT_ID`です。Secretは`STORE_CLIENT_SECRET`です。価格・掲載情報は変更せず、前回公開の提出を基にMSIXだけを差し替えます。

設定と初回公開を確認してから、Repository variable **`STORE_PUBLISH_ENABLED=true`**を設定します。以降に公開するリリースが自動提出の対象です。Store処理は毎時07分・37分（UTC、日本時間も同じ分）に実行し、提出IDと状態をリリースの状態コメントへ記録します。提出済みとStore公開済みを区別し、認定失敗を公開成功にしません。

失敗時はPartner Centerと状態を確認し、main上の**Microsoft Store submission → Run workflow**に対象の`release_id`を指定して同じ提出を再開します。PUT後にアップロード等が中断した場合も、追加済みのMSIXのIDとアップロード状態を保って再利用し、同名の削除・追加レコードを重複作成しません。別のpending submissionがある場合は自動で流用・削除せず停止します。作成直後に状態保存が失敗した場合も重複提出を避けるため停止し、既存提出を手動で照合します。API作成した提出をPartner Centerから同時編集しません。

Store提出前に、Release assetのmanifestの実バイトへ付いたGitHub attestationを検証します。現在の数値リポジトリID、main上の正式なRelease build、GitHubの署名発行元、証明書が示すrun・attemptを照合し、実際の公開ステップが成功していることを要求します。dry runの証明だけでは提出しません。ワークフロー版のSHAとビルド対象のSHAは区別し、タグの解決先・main包含・署名済みStore receipt・取得MSIXの容量とSHAを確認します。Release本文の状態・sourceSha・buildRunIdは出自の証明ではありません。

証明不足・不一致・期限切れではStoreを変更せず停止します。公開処理が実行中なら次の定期確認を待ち、別runへの切替や再ビルドで補いません。提出済みでPendingCommitを超えた処理は成果物を再取得せず状態確認を継続します。PendingCommitからの再開では再度出自を確認します。

MSIXはActionsに30日保持します。保持期限を過ぎた提出は別ソースで勝手に再構築せず、原本の保管状況と提出を確認します。失敗状態は自動で繰り返し提出せず、修正後に明示再開します。

## 資格情報の更新と停止

証明書・APIキー・クライアントシークレットの期限を所有者が保守時に確認し、期限前に更新して試行します。疑わしい資格情報はサービス側で失効し、Secretsを更新します。

公開処理を止める場合は`RELEASE_AUTOMATION_ENABLED=false`と`RELEASE_PUBLISH_ENABLED=false`、Store提出は`STORE_PUBLISH_ENABLED=false`に設定します。既に実行中の公開処理はActionsでも取消します。検査の停止や例外の無期限化で回避せず、原因を修正して再実行します。既に公開したバイナリは上書きせず、新しい版で修正します。
