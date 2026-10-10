# macOS配布用アプリとDMGの明示ビルド

対象はmacOS 15以降・Apple Silicon。これは配布工程の実装であり、Developer ID署名・公証済み配布物や実機更新を検証済みにするものではない。通常の開発ビルドとMON QAは既存のアドホック署名のまま。Windows配布・CIは保留する。

## 前提と明示指定

利用可能なDeveloper ID Application証明書、対応する10桁のTeam ID、Node.js・Rust・Swift・Xcodeが必要。証明書名は`APPLE_SIGNING_IDENTITY`へ明示する。ツールは鍵・証明書・公証資格情報を探索／登録しない。未指定・別Team・開発証明書の指定は出力作成前に拒否する。指定された証明書の実在・使用可否は実際の署名段階で確認する。

次は構造例であり、証明書名とTeam IDを実際の値に置き換える。資格情報・秘密鍵・パスワードをリポジトリへ保存しない。

```sh
cd /Users/skodani/Codebase/mon/app
mon_release_parent=$(mktemp -d /private/tmp/mon-release-parent.XXXXXX)
export APPLE_SIGNING_IDENTITY='Developer ID Application: 実際の証明書名 (ABCDE12345)'
npm run tauri:release:build -- --team-id ABCDE12345 --sign-only --output-dir "$mon_release_parent/output"
```

出力は**不存在の絶対パス**に限る。親はリンクを含まない既存ディレクトリで、4GiB以上の空きを検査する。空き容量の予約や強制的な容量上限ではない。共有Rustテストキャッシュは使わず、専用・非増分の構築領域を作る。正常完了後はこの新規キャッシュだけを回収する。失敗・中断時は出力とキャッシュを保全し、成功記録を作らず、別名での自動再試行をしない。中断／タイムアウトは所有するコマンドのプロセスグループへ停止を送り、終了を待つ。

`--sign-only`は公証への成果物アップロードをしない。Developer ID署名のsecure timestamp取得等にはAppleへの通信が発生し得る。署名だけのDMGを公証済み・一般配布可能として扱わない。

## 署名を分ける

Tauriの準備用プロファイルはアドホックのappだけを生成し、署名／公証用のApple環境変数を子へ渡さない。生成物を新規出力へコピーし、通常版`jp.mon.desktop`・版・最低OS・実行ファイル`mon-desktop`・arm64・録音用途の説明・両ヘルパーを確認する。QAを通常版の成果物として採用しない。

| 対象 | Hardened Runtimeのentitlements |
|---|---|
| 録音ヘルパー | `com.apple.security.device.audio-input`だけ |
| 特徴量ヘルパー | `com.apple.security.cs.disable-library-validation`だけ |
| 親アプリ | 追加例外なし |

特徴量ヘルパーは利用者が別途準備した推論ライブラリを読み込むため、同一Team署名だけに限る検査をこのヘルパーだけで解除する。親アプリ・録音側へ広げない。これは隔離実行ではなく、既存のパス・容量・SHA・役割の照合と、CPU実行の別同意を維持する。[Appleのライブラリ検証の説明](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.disable-library-validation)を参照。録音用entitlementは[Audio Input](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.audio-input)に対応する。

ヘルパーを先に署名し、親を最後に封印する。署名には`--deep`を使わず、検証にはdeep／strict・Apple anchor・Developer IDの証明書OID・期待Teamを要求する。表示されたTeam文字列だけでは合格にしない。secure timestamp・runtimeビット・実際に封入されたentitlementsも照合する。debug権限や未選択の例外を追加したpolicyは構築前に拒否する。

## 公証は別の明示モード

既存のnotarytool keychain profileを用意し、アップロードを承認した場合だけ、`--sign-only`の代わりに`--notary-profile <既存のprofile名>`を指定する。両モードの同時指定・未指定は拒否する。このモードは**2回のアップロード**を行うため、通常の署名のみの検証には指定しない。

1. 署名済みappのZIPを提出し、正準UUIDの要求IDとAcceptedを確認する。appへticketをstapleしvalidateする。
2. ticketを持つappとApplicationsへのリンクからUDZO DMGを作り、コンテナ検査とDeveloper ID署名を行う。
3. 最終DMGを提出し、Accepted・staple・validateを確認する。提出後にDMGを作り直さない。
4. 全署名・権限を再検査し、最終DMGの容量とSHA-256を記録する。

Invalid・処理中・不正応答・署名／ticket検証失敗は成功にしない。notarytoolは明示profileだけを使い、環境のメール／パスワードやAPI秘密鍵をログ・結果へ書かない。[Appleの公証条件](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)を参照。

## 結果と未検証ゲート

Macの準備ビルドはオフラインで依存通知文を収集し、`Contents/Resources/THIRD_PARTY_NOTICES.txt`と`third-party-inventory.json`を同梱する。現在のCargo normal/build依存・npm production依存を保守的に対象とし、正確な使用版・申告ライセンス・収集文の容量／SHA・lock SHA・候補が見つからない依存を記録する。MON自身、標準ライブラリ／Apple SDK、別導入のAIエンジン・モデル等は別確認。全収集物の存在は条件充足の証明ではなく、欠落文・追加NOTICE・対応ソースの取得案内等の確認を残す。[同梱範囲と実検証](verification/macos-dependency-notices-2026-10-10.md)を参照。

コピー後と最終検査後に通知文の容量／SHA・現在のlock・未承認状態を照合し、欠落・旧版・改変・自己承認を拒否する。build-resultの`dependencyNotices`は`collected_not_reviewed`であり、ライセンス全体の確認には昇格しない。

正常なビルド結果は`build-result.json`に新規保存する。版・Team・DMG名・容量・SHA、公証要求ID／ticketの状態、工程時間と回収済みキャッシュの場所を含む。手動ビルドの結果であり、取得モデルの互換性や会議品質の証拠ではない。

署名だけなら`notarization: not_requested`。ticketの検査まで成功しても、`runtimeQA`・`gatekeeper`・`licenseReview`・`manualUpdate`は`not_verified`、`distributionReady`はfalseのまま。配布条件の確認、ダウンロード由来のGatekeeper、分離した実機でのGUI・録音・外部推論ライブラリ、通常版の置換と設定・会議・モデルの保持を別に検証する。Gatekeeper・隔離属性・TCCを無効化して合格にしない。

今回のアドホック準備アプリは通常版の識別子なので、実機GUI検証用MON QAの代わりに起動しない。実施範囲・保持物・時間と残件は[配布準備の検証記録](verification/macos-distribution-preparation-2026-10-10.md)を参照。
