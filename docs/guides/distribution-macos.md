---
type: "Guide"
title: "macOSの署名と公証とDMG"
description: "Developer ID署名、公証用認証の準備とDMGの生成・確認方法。"
tags: ["macos", "distribution", "signing"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/tools/mon_macos_release.py"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/docs/macos-distribution.md"
  - resource: "https://github.com/s-kodani/mon/blob/4f44b1ed39d81f0a27c88a084e7bfc2b9bb3a83b/app/src-tauri/tauri.release.conf.json"
---

# macOSの署名と公証とDMG

## 準備

[macOSビルド環境](build-macos.md)と、有効なDeveloper ID Application証明書・対応する秘密鍵、公証用のキーチェーンプロファイルを用意します。コマンド例の氏名・Team ID・メールアドレスは自分の値へ置き換えます。

1. XcodeのSettings → Accounts／Apple AccountsでTeamを選び、Manage CertificatesからDeveloper ID Applicationを作成します。既存の有効な証明書と秘密鍵があれば再利用できます。
2. 証明書と鍵を`security find-identity -v -p codesigning`で確認します。
3. Apple Accountで公証用のアプリ用パスワードを作成し、次のコマンドの対話入力で保存します。通常のApple Accountパスワードは使いません。

```sh
xcrun notarytool store-credentials minuteson-notary \
  --apple-id 'Apple Accountのメールアドレス' \
  --team-id ABCDE12345
xcrun notarytool history --keychain-profile minuteson-notary --output-format json
```

プロファイル名`minuteson-notary`は任意です。提出履歴が0件でも認証が成功していれば準備できます。登録だけではアプリのアップロード・公証は行いません。

秘密鍵・P12・パスワードをGitやコマンド引数へ残しません。環境固有の署名名やTeam IDのメモはGit管理外の`.local/`で扱います。

## 生成

リポジトリルートから実行します。このコマンドはアプリとDMGをAppleへ送信して公証します。

```sh
export APPLE_SIGNING_IDENTITY='Developer ID Application: Your Name (ABCDE12345)'
mon_release_parent=$(mktemp -d /private/tmp/minuteson-release.XXXXXX)
npm --prefix app run tauri:release:build -- \
  --team-id ABCDE12345 \
  --notary-profile minuteson-notary \
  --output-dir "$mon_release_parent/output"
```

`--output-dir`は未作成の絶対パスを指定し、リンクを含まない既存の親を使います。スクリプトは開始時に4 GiB以上の空きを確認しますが、ビルド中の使用量上限ではありません。共有Rustテストキャッシュとは分けた構築領域を使います。

署名だけを確認する場合は別の新規出力先を使い、`--notary-profile`の代わりに`--sign-only`を指定します。両方は併用できません。

```sh
mon_sign_parent=$(mktemp -d /private/tmp/minuteson-sign.XXXXXX)
npm --prefix app run tauri:release:build -- \
  --team-id ABCDE12345 --sign-only \
  --output-dir "$mon_sign_parent/output"
```

署名のみでもsecure timestamp取得などの通信は発生し得ます。`notarization: not_requested`の成果物を公証済みとして扱いません。

## 成果物と確認

| 成果物 | 内容 |
| --- | --- |
| `MinutesOn.app` | 通常版アプリ |
| `MinutesOn_<version>_aarch64.dmg` | アプリを収めたDMG |
| `build-result.json` | 版、Team、公証要求ID、容量・SHA、処理結果 |
| `preparation-build.log` | 準備ビルドのログ |

終了コード0、指定したTeam ID、`notarization: tickets_validated`、2件の公証要求IDを確認します。appとDMGそれぞれのAccepted応答、チケット添付・検証と最終署名が成功条件です。署名後にアプリ・DMGの内容を編集しません。出力が一時領域の場合は必要な成果物と記録を永続的な保管先へコピーします。

内包ASR CLI・ワーカーも実署名を検査し、署名で変わったバイトに合わせてランタイムのアーカイブ・indexを再構成します。外側appの公証だけでCLI単体の利用条件や実機動作を確認済みにしません。

`runtimeQA`・`gatekeeper`・`licenseReview`・`manualUpdate`は別途確認します。ビルド成功だけでは`distributionReady`は`false`のままです。ダウンロードしたDMGからの起動、権限、モデル導入・推論、通常版の更新とデータ保持を確認して配布判断を行います。

## 失敗時

証明書名・Teamの一致、鍵と有効期限、キーチェーンプロファイルを確認します。公証失敗は提出履歴から状態を調べ、署名できたことだけで成功にしません。失敗・中断時は出力とログを保ち、所有するビルドプロセスの終了を確認してから再実行します。以前の出力先を上書きしません。

現在の配布実績は[製品概要](../spec/overview.md#配布と評価の確認状況)、アプリ置換は[更新手順](updating.md)を参照してください。
