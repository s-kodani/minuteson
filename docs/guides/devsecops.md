---
type: "Guide"
title: "DevSecOpsの検査と運用"
description: "セキュリティ検査、例外、GitHub保護設定、署名とStore認証の初期設定。"
tags: ["security", "ci", "maintenance"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/.github/workflows/security.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/6512783dfb31263f73e7abad572d736f972811b2/Cargo.lock"
  - resource: "https://github.com/s-kodani/minuteson/blob/6512783dfb31263f73e7abad572d736f972811b2/app/src-tauri/Cargo.toml"
  - resource: "https://github.com/s-kodani/minuteson/blob/130c3902eea205bc0ca599c9da45635bb8974ede/tools/mon_security.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/130c3902eea205bc0ca599c9da45635bb8974ede/tools/mon_macos_ci_sign.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/tools/mon_store_submission.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/.github/workflows/release-build.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/95ec6d6e28e6cccbba792a534da44341157008a1/tools/mon_macos_release.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/tools/mon_store_provenance.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/744afeca545faf1ef75f1f34feffa3228f76d274/.github/security-exceptions.json"
  - resource: "https://github.com/s-kodani/minuteson/blob/744afeca545faf1ef75f1f34feffa3228f76d274/tools/mon_windows_runtime.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/744afeca545faf1ef75f1f34feffa3228f76d274/tools/mon_llama_runtime_check.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/bddf5c66505d0d2c097eb55ae05806b9389f306b/.github/workflows/desktop-platform-build.yml"
  - resource: "https://github.com/s-kodani/minuteson/blob/cd647f3d8c5917619f266eba8d2112d909182862/.github/dependabot.yml"
---

# DevSecOpsの検査と運用

## 検査と判定

セキュリティ検査はPRの変更範囲、mainの全件検証、毎日09:30（日本時間）に実行します。Release公開時の再検査は行いません。文書と秘密情報・ワークフロー方針の検査は常時維持し、PRのCodeQLは該当言語、依存検査は該当変更時に実行します。日次は全言語・全依存を検査します。`Security gate`は予定した省略だけを許可し、範囲判定の失敗では停止します。ネイティブ・配布物検証の条件は[開発フロー](development-workflow.md#pr日次リリース前の検証)を参照してください。

| 検査 | 対象と停止条件 |
| --- | --- |
| Gitleaks | 初期・通常ブランチの全履歴、PRの追加コミット。秘密情報を検出すると停止し、値はログで伏せます |
| OSV-Scanner | Cargo・npmの固定依存と、ビルド定義・モデルカタログから抽出したネイティブ依存の版・コミット。CVSS 7.0以上と重大度不明を停止します |
| CodeQL | Actions、JS/TS、Python、Rust、C/C++、Swift。SARIF内のHigh以上のセキュリティ指摘と重大度不明を停止します。Swiftは実ビルドで解析します |
| 文書検査 | OKFメタデータ、内部リンク・アンカー、コード参照、目次到達性 |

低・中程度の指摘と保守終了の通知は診断成果物で確認します。別名IDは同一の指摘としてまとめ、既知の重大度を共有します。検査サービスへの通信失敗、壊れた出力、必要なジョブの取消・未実施は成功にしません。CodeQLが非公開設定により省略された場合は、その未実施を明示します。解析未実施の状態をSAST合格と扱いません。

OSVのネイティブ依存の収録範囲は完全ではありません。SDK、CRT、OSライブラリ、モデルデータはこの脆弱性検査の対象外です。取得時の既存ハッシュ照合は維持し、毎週の保守時に固定資材と上流の更新・通知も確認します。SBOMにはCargoの通常・ビルド依存、npmの製品依存、ネイティブ取得定義を記録し、任意ダウンロード資材が含まれる範囲も明示します。

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

公式カタログとWindows生成資材の議事録エンジンは、RPCサーバーの[CVE-2026-118375](https://github.com/ggml-org/llama.cpp/pull/30399)・[CVE-2026-119052](https://github.com/ggml-org/llama.cpp/pull/31204)の修正を含む`llama.cpp b11960`へ統一しています。旧`b11533`の取得定義と2件の期限付き例外は除去しています。Windowsは固定版を取得し、許可したCLI・ライブラリ・通知だけを新しいパッケージ領域へ配置します。復元キャッシュの旧DLLを成果物へ混ぜません。

議事録処理は固定引数で起動し、継承環境を消去する境界を維持します。今後RPCを導入する場合は到達性を再評価します。更新時は取得元・実バイトの容量とSHA、レシピ、モデルデータの再利用、開始済みジョブの条件固定、両OSの互換性を確認します。代表モデルの実行検査は[テスト運用](testing.md#議事録ランタイムの実資材検証)を参照してください。

### GLibの対象プラットフォーム

`glib 0.18.5`の[RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html)は、`VariantStrIter`のC関数への出力ポインターの渡し方に関する未定義動作です。修正版は`0.20.0`以降ですが、現在のTauriのLinux向けGTK／WebKit2GTK依存は`glib 0.18`を要求するため、GLibだけを更新することはできません。

2026-11-04に、通常ビルドと`tauri/custom-protocol`を有効にしたビルドの依存グラフを確認しました。対応対象のWindows x64とmacOSでは`glib`は通常・ビルド依存に選択されず、Linux x64では選択されます。GitHubの該当アラートを個別判断する場合は、対応プラットフォームで未使用であることを根拠にします。上流の脆弱性修正完了や、Linux向けビルドの安全性を示す判断ではありません。OSVの診断からは除外しません。

```sh
cargo tree --frozen --target x86_64-pc-windows-msvc --edges normal,build --invert glib@0.18.5
cargo tree --frozen --target aarch64-apple-darwin --edges normal,build --invert glib@0.18.5
cargo tree --frozen --target x86_64-pc-windows-msvc --features tauri/custom-protocol --edges normal,build --invert glib@0.18.5
cargo tree --frozen --target aarch64-apple-darwin --features tauri/custom-protocol --edges normal,build --invert glib@0.18.5
cargo tree --frozen --target x86_64-unknown-linux-gnu --edges normal,build --invert glib@0.18.5
```

対象OS、Tauriの機能指定、GTK／WebKit2GTK依存が変わる場合は、この判断を再検証します。lockfile全体を対象とする検査と、実際にビルドへ選択される依存の確認を区別し、包括的なアラート除外や重大度ゲートの緩和で回避しません。

## GitHubの初期設定

1. 全履歴の秘密情報検査と依存検査を実施し、指摘を修正または個別判断します。Actions履歴・ログと保存済み成果物の公開範囲も確認します。
2. main上のワークフローを有効にし、標準ランナーの利用上限・権限・資材取得を確認します。Desktop platform validationの`full`とSecurity validationをmainで実行し、`CI gate`・`Security gate`とCodeQL各言語の結果を確認します。
3. CodeQLはこのリポジトリのadvanced setupを使い、default setupとの重複実行を避けます。Dependency graph・Dependabot alerts/security updatesとSecret scanning・Push protectionを有効化します。Private vulnerability reportingを有効化し、[非公開報告のリンク](../../SECURITY.md#非公開の報告)が利用できることを確認します。
4. mainのrulesetでPR、`CI gate`と`Security gate`の成功、force push・削除の禁止を設定します。チェック名は初回実行結果から選択します。承認数は0とし、本人が自分のPRを確認してマージできるようにします。通常のバイパスは設けません。
5. `v*`タグの更新・削除を禁止します。新規作成はReleases画面での手動Publishに許可します。成果物の添付処理はタグを作成しません。Actionsの既定権限はread-only、forkワークフローの実行承認を有効化します。各ジョブで必要な権限だけを宣言します。

設定や検査が未完了の状態を導入済みの保護と扱いません。

Dependabotは[定義](../../.github/dependabot.yml)に従い、npm・Cargo・GitHub Actions・検査用Python依存の4対象を毎週月曜10:00（日本時間）に確認します。バージョン更新PRの上限は対象ごとに5件です。npm/Cargoのminor・patch更新はグループ化し、major更新は別PRで確認します。自動マージは行わず、通常のPRと同じ必須検査と互換性の確認を通して取り込みます。

バージョン更新はActionsの有効・無効と別に動作します。[GitHubの設定仕様](https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-options-reference#open-pull-requests-limit)に従い、セキュリティ更新はバージョン更新PRの上限とは別に、リポジトリ設定で管理します。スキャナーのバイナリ定義と手動固定する資材は保守担当者が確認・更新します。

## 自動配布の初期設定

配布条件と確認状況は[製品概要](../spec/overview.md#配布と評価の確認状況)を参照してください。`RELEASE_AUTOMATION_ENABLED=true`でmainの正式配布物保存と下書きへの添付を有効化します。公開は担当者のPublish releaseです。旧`RELEASE_PUBLISH_ENABLED`と`release-publish` Environmentは新フローで参照せず、自動Publishは行いません。

Environmentの構成は次のとおりです。いずれもdeployment branchesを**mainのみ**に限定します。

| Environment | 用途 | Required reviewers |
| --- | --- | --- |
| `release-macos` | main最終アプリの署名・公証 | なし。公開前のローカル確認に使う最終版をmainで作る |
| `release-store` | Storeへの作成・更新・アップロード・commit | 保守担当者を必須に設定。1名運用ではPrevent self-reviewを無効にし、管理者の承認バイパスも無効 |
| `release-store-status` | 既存の提出IDの状態照会 | なし。コードはStoreへのGET statusだけを許可 |

YAMLの`environment: release-store`だけでは承認待ちになりません。Settings → Environments → release-store → **Required reviewers**を設定してください。新規作成したEnvironmentの既定では承認者は未設定です。

`release-macos`のSecrets:

- `MACOS_CERTIFICATE_P12_BASE64`: Developer ID Application証明書と秘密鍵を含むP12のBase64
- `MACOS_CERTIFICATE_PASSWORD`: P12のパスワード
- `APPLE_NOTARY_KEY_BASE64`: 公証に利用できるチームAPIキー（P8）のBase64

同環境のVariables:

- `APPLE_SIGNING_IDENTITY`: Teamの完全なDeveloper ID Application署名名
- `APPLE_TEAM_ID`: Team ID
- `APPLE_NOTARY_KEY_ID`、`APPLE_NOTARY_ISSUER_ID`: 公証APIキーの識別子

資格情報をGitやPR本文へ記載しません。ビルドjobには渡さず、準備済みアプリを検証する署名jobだけで一時キーチェーン等へ導入して終了時に削除します。署名jobは再ビルドしません。mainの組立jobはOIDCで来歴証明を作り、候補を30日保存します。公開用PATは不要です。

有効化後は**Main build**の成功とサマリーの候補を確認します。ローカル確認、下書き作成、添付完了、手動Publishの手順は[開発フロー](development-workflow.md#releases画面でバージョンタグを管理する)に従います。模擬検査を実署名・公証・Store認定の成功とは扱いません。

## Store更新の有効化と再開

初回公開はPartner Centerで行い、年齢区分・掲載情報・プライバシー情報を整えます。初回公開後、Partner Centerへ関連付けたEntraアプリを用意します。[Store配布](distribution-store.md)も参照してください。

`release-store`と`release-store-status`のそれぞれにVariables `STORE_TENANT_ID`、`STORE_APPLICATION_ID`（Store ID）、`STORE_CLIENT_ID`とSecret `STORE_CLIENT_SECRET`を登録します。照会用資格情報は承認不要の別Environmentで管理し、Store送信jobの承認は維持します。既存SecretはGitHubから平文で読み出せないため、照会用Environmentへは所有者が再登録します。照会クライアントは認証後のStore APIを`GET .../submissions/{id}/status`に制限し、作成・PUT・upload・commitを拒否します。

初回公開・資格情報・Required reviewers・main限定を確認して、Repository variable **`STORE_PUBLISH_ENABLED=true`**を設定します。

1. 担当者がGitHub ReleaseをPublishすると、releaseイベントからmainのStore送信workflowをdispatchします。
2. **Review deployments → release-store → Approve and deploy**で承認するまで、Storeの作成・更新・アップロード・commitは実行しません。
3. 承認後、公開済みmanifestの来歴、タグSHA、保存MSIXのサイズとSHAを確認し、価格・掲載情報を保持してパッケージだけを差し替えます。
4. 提出IDと状態をReleaseの状態コメントへ記録します。毎時07分・37分の照会は既存IDの状態だけを読み、承認を再要求しません。提出・認定・Store一般公開を区別します。

`PendingCommit`でも照会から送信は再開しません。失敗・アップロード中断を再開する場合は、mainの**Microsoft Store submission → Run workflow**で`operation=submit`と対象`release_id`を指定し、改めてその送信を承認します。`operation=poll`は状態だけを確認します。

PUT後に中断した場合も、同じ提出ID・MSIXのIDとアップロード状態を保持し、同名パッケージを重複作成しません。別のpending submissionは流用・削除せず停止します。ID保存に失敗した場合も既存提出を確認するまで新規作成しません。Partner Centerからの同時編集は避けます。旧フローで進行中の提出IDも照会を継続できます。

出自はRelease本文の`sourceSha`・`buildRunId`ではなく、manifestの実バイトに対するGitHub attestationで判断します。現在の数値リポジトリID、mainの正式workflow、GitHubの署名発行元、証明書が示すrun・attemptと完了ステップを照合します。新形式はmainのビルド対象SHAと証明書のsource SHAも一致させます。旧schema v1は旧公開ステップの成功を要求し、既存の公開済みRelease・提出との互換性を維持します。

GitHub CLIの署名者指定はmain workflowと完全一致する`--cert-identity`だけを使い、排他的な`--signer-workflow`等を併用しません。タグの解決先・main包含・Store receipt・MSIXの容量とSHAの既存照合も維持します。

不足・不一致・期限切れではStoreを書き換えず停止します。main候補が実行中でも別runや最新mainへ切り替えず、対象が完了してから明示再開します。MSIXは30日保持し、期限切れを別ソースの再ビルドで補いません。送信が進んでいる既存IDは原本の再取得なしに状況照会できます。

## 資格情報の更新と停止

証明書・APIキー・クライアントシークレットの期限を所有者が保守時に確認し、期限前に更新します。疑わしい資格情報はサービス側で失効し、両EnvironmentのSecretを更新します。

下書きへの添付・mainの正式配布物保存は`RELEASE_AUTOMATION_ENABLED=false`、Store送信・照会は`STORE_PUBLISH_ENABLED=false`で停止します。既に承認待ち・実行中の処理はActionsでも取消します。公開済みバイナリは上書きせず、新しい版で修正します。
