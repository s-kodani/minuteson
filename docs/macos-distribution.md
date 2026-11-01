# macOS本番ビルド手順（署名・公証・DMG）

対象はmacOS 15以降・Apple SiliconのMinutesOn（略称 mon）。これは配布工程の実装であり、Developer ID署名・公証済み配布物や実機更新を検証済みにするものではない。通常の開発ビルドとMinutesOn QAは既存のアドホック署名のまま。Windowsの配布は[Store向けMSIXビルド](windows-store.md)と[EXEビルド](windows-setup.md)を参照する。

正式名はWindows・macOSともMinutesOn。通常版の出力名は`MinutesOn.app`、`MinutesOn_1.0.1_aarch64.dmg`、DMGのボリューム名は`MinutesOn`となる。これらの名称と版はTauriの基本設定・macOS設定・配布設定を統合した値から生成する。内部の`mon-desktop`、録音ヘルパー名、通常版の識別子`jp.mon.desktop`は維持する。

## 今回の設定と確認結果（2026-11-01）

このMacでは、XcodeでDeveloper ID Application証明書を作成し、公証用の認証情報をキーチェーンへ登録した。2026-11-01に以下を確認済み。

| 項目 | 公開用の設定例・確認結果 |
|---|---|
| 署名証明書名 | `Developer ID Application: Your Name (ABCDE12345)` |
| Team ID | `ABCDE12345` |
| 公証用プロファイル名 | `minuteson-notary` |
| 証明書・秘密鍵 | `security find-identity -v -p codesigning`で有効な組み合わせを1件確認 |
| 公証サービスへの認証 | `notarytool history`で接続成功、確認時点の提出履歴は0件 |
| 本番ビルド・公証の完了 | まだ成功記録を確認していない。以下は実行手順であり、配布物の完成記録ではない |

この文書の証明書名`Your Name`とTeam ID`ABCDE12345`は公開用の例であり、自分の環境の値へ置き換える。`minuteson-notary`は任意の保存名で、認証情報そのものではない。実際の証明書名・Team ID・保存名をメモする場合は、Git管理外の`.local/`（例：`.local/macos-release.txt`）へ保存する。

Apple Accountのパスワード、アプリ用パスワード、秘密鍵、秘密鍵を含む`.p12`はリポジトリへ保存しない。`.gitignore`は`.local/`、認証設定用の`.env`、署名鍵・キーチェーン書き出し・CSRファイルを除外する。公開可能な設定例は`.env.example`・`.env.sample`・`.env.template`として置ける。除外設定は、すでにGitで追跡されているファイルや過去の履歴を削除するものではない。

## 本番ビルドの実行（設定済みのMac）

初回設定と接続確認が済んだMacで、リポジトリのルートから次のコマンドを実行する。例の証明書名・Team IDは実際の値へ置き換え、新しいターミナルでは`export`も実行する。

```sh
cd app

export APPLE_SIGNING_IDENTITY='Developer ID Application: Your Name (ABCDE12345)'

mon_release_parent=$(mktemp -d /private/tmp/minuteson-release.XXXXXX)

npm run tauri:release:build -- \
  --team-id ABCDE12345 \
  --notary-profile minuteson-notary \
  --output-dir "$mon_release_parent/output"
```

**このコマンドはアプリとDMGをAppleへ送信して公証する。** 署名・構築・公証の待機には時間がかかる。キーチェーンから署名用秘密鍵へのアクセス確認が表示された場合は、実行した署名処理であることを確認して対応する。

| 指定 | 意味 |
|---|---|
| `APPLE_SIGNING_IDENTITY` | このターミナルで使う署名証明書の名前 |
| `--team-id` | 証明書と同じApple開発チームの10文字のID |
| `--notary-profile` | キーチェーンへ登録した公証用認証情報の保存名 |
| `--output-dir` | 成果物を保存する、新規の絶対パス |
| `mktemp -d` | 毎回別の一時フォルダーを作り、既存の成果物を上書きしない |

`--output-dir`で指定する`output`自体は事前に作成しない。親はリンクを含まない既存ディレクトリとし、スクリプトは4GiB以上の空きを確認する。これはビルド中の容量上限ではない。上記の`/private/tmp`を使う形は、これらのパス条件を満たすためのもの。

実行したターミナルで次を入力すると、出力先を再表示できる。

```sh
echo "$mon_release_parent/output"
```

### 成果物と成功の確認

コマンドが終了コード0で完了し、出力先に以下が生成されていることを確認する。現在のアプリバージョンは`1.0.1`。

| ファイル | 内容 |
|---|---|
| `MinutesOn.app` | Developer ID署名・公証チケットを持つ本番アプリ |
| `MinutesOn_1.0.1_aarch64.dmg` | 本番アプリを収めた署名・公証済みDMG |
| `build-result.json` | Team ID、公証要求ID、DMGの容量・SHA-256、処理結果 |
| `preparation-build.log` | Tauri準備ビルドのログ |

`build-result.json`では`teamId`が指定したTeam IDと一致し、`notarization`が`tickets_validated`、`notaryRequestIds`が2件であることを確認する。スクリプトはアプリとDMGそれぞれについて、AppleのAccepted応答、チケットの添付・検証、最終署名検証を行ってから成功記録を作る。

`runtimeQA`・`gatekeeper`・`licenseReview`・`manualUpdate`は別途確認する項目であり、ビルドに成功しても`distributionReady`は`false`のまま。ダウンロードしたDMGからの起動、録音権限、モデル利用、旧版からの更新とデータ保持を配布前に確認する。

上記の出力先は一時領域なので、必要なアプリ・DMG・結果JSON・ログは永続的な保管先へコピーする。署名・公証済みアプリやDMGの内容は変更しない。

### 署名だけを確認する場合

Appleへの公証提出を行わずに署名工程を確認するには、別の新規出力先と`--sign-only`を使う。次もリポジトリのルートから実行し、証明書名とTeam IDは実際の値へ置き換える。

```sh
cd app
export APPLE_SIGNING_IDENTITY='Developer ID Application: Your Name (ABCDE12345)'
mon_sign_parent=$(mktemp -d /private/tmp/minuteson-sign.XXXXXX)

npm run tauri:release:build -- \
  --team-id ABCDE12345 \
  --sign-only \
  --output-dir "$mon_sign_parent/output"
```

`--sign-only`と`--notary-profile`はどちらか一方だけを指定する。署名のみの結果は`notarization: not_requested`となる。署名のsecure timestamp取得等にはAppleへの通信が発生し得る。署名のみのDMGは公証済みの配布物として扱わない。

## 初回設定・別Macでの準備

Node.js・Rust・Swift・Xcodeとアプリの依存パッケージが必要。今回のMacはMinutesOn QAの再ビルドまで確認済み。別Macでは、この環境の準備に加え、以下の証明書・公証設定を行う。

### 1. Xcodeで署名証明書を作成する

1. Xcodeを起動し、**Xcode → Settings…**を開く。MinutesOnのプロジェクトを開く必要はない。
2. **Apple Accounts／Accounts**でApple Accountを追加してサインインする。
3. Apple Developer Programに加入しているTeamを選び、**Manage Certificates…**を開く。
4. **「＋」→「Developer ID Application」**を選び、作成完了後にDoneを押す。
5. ターミナルで次を実行し、有効なDeveloper ID Application証明書名が表示されることを確認する。

```sh
security find-identity -v -p codesigning
```

Xcodeが秘密鍵・CSR（証明書署名要求）の作成から証明書の取得・キーチェーン登録まで行うため、CSRを手動で作成する必要はない。すでに同じTeamの有効な証明書と対応する秘密鍵が揃っている場合は、作り直す必要もない。証明書ファイルだけが存在しても、対応する秘密鍵がなければ署名には使えない。

Developer ID Applicationの発行にはAccount Holder権限が必要。選択肢が表示されない場合は、選択中のTeam・加入状況・権限を確認する。[AppleのXcode案内](https://help.apple.com/xcode/mac/current/en.lproj/dev88332a81e.html)、[証明書と必要権限](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/)。

### 2. 公証用のアプリ用パスワードを作成する

[Apple Account](https://account.apple.com/)の**サインインとセキュリティ → アプリ用パスワード**から、公証用のパスワードを生成する。Apple Accountの2ファクタ認証が必要。[Appleの作成手順](https://support.apple.com/ja-jp/102654)。

### 3. 公証の認証情報をキーチェーンへ保存する

メールアドレスを実際のApple Accountへ置き換えて実行する。別Teamで行う場合はTeam IDも置き換える。

```sh
xcrun notarytool store-credentials "minuteson-notary" \
  --apple-id "Apple Accountのメールアドレス" \
  --team-id ABCDE12345
```

実行後の入力待ちで、手順2の**アプリ用パスワード**を入力する。通常のApple Accountパスワードは使わない。入力した文字が画面に表示されなくても正常。パスワードをコマンド引数へ書かないため、コマンド履歴にもその値を残さない。

このコマンドはAppleへ接続して認証情報を検証し、キーチェーンへ保存する。`minuteson-notary`は任意の保存名で、アプリ名・証明書名・Team IDとは別のもの。保存したMacで後から`--notary-profile minuteson-notary`として参照する。この登録操作でアプリのアップロードや公証申請は行わない。

### 4. 保存した認証情報で接続を確認する

```sh
xcrun notarytool history \
  --keychain-profile minuteson-notary \
  --output-format json
```

認証に成功して履歴を取得できれば準備完了。初回の`No submission history.`も正常で、まだ公証申請がないことを表す。今回のMacではこの応答を確認済み。以降は「本番ビルドの実行」のコマンドを使い、毎回の認証情報登録は不要。

## 途中で失敗した場合

| 状況 | 確認すること |
|---|---|
| Developer ID証明書が見つからない | `security find-identity -v -p codesigning`で証明書と秘密鍵、有効期限を確認する |
| 証明書名・Team IDの指定で停止する | `APPLE_SIGNING_IDENTITY`の名前を正確に指定し、末尾のTeam IDと`--team-id`を一致させる |
| 公証用プロファイルが見つからない／認証が失敗する | 同じMacで`store-credentials`を行ったか、保存名が`minuteson-notary`かを確認し、`notarytool history`で再確認する |
| 出力先の指定で停止する | 存在しない絶対パスを指定する。以前の出力フォルダーを再利用しない |
| 公証がAcceptedにならない | `notarytool history`で提出状態を確認する。署名できたことだけで公証成功と判断しない |

共有Rustテストキャッシュは使わず、出力先の中に専用・非増分の構築領域を作る。正常完了後はその新規キャッシュだけを回収する。失敗・中断時は出力とキャッシュを保全し、成功記録を作らない。中断／タイムアウトは所有するコマンドのプロセスグループへ停止を送り、終了を待つ。失敗時は保存された出力とターミナルのエラーを確認してから再実行を判断する。

## 署名を分ける

Tauriの準備用プロファイルはアドホックのappだけを生成し、署名／公証用のApple環境変数を子へ渡さない。生成物を新規出力へコピーし、正式表示名・通常版`jp.mon.desktop`・版・最低OS・実行ファイル`mon-desktop`・arm64・録音用途の説明・両ヘルパーを確認する。QAを通常版の成果物として採用しない。

| 対象 | Hardened Runtimeのentitlements |
|---|---|
| 親アプリ、同梱Whisper CLI、Whisper／llamaワーカー | `main.plist`：`com.apple.security.device.audio-input` |
| 録音ヘルパー | `capture.plist`：`com.apple.security.device.audio-input` |
| 特徴量ヘルパー、Sherpa／transducerワーカー | `embedding.plist`：`com.apple.security.cs.disable-library-validation` |

特徴量ヘルパーとSherpa／transducerワーカーには、推論ライブラリの読込みに使う`embedding.plist`を適用する。同一Team署名だけに限るライブラリ検査の例外を、親アプリ・録音側へ広げない。これは隔離実行ではなく、既存のパス・容量・SHA・役割の照合と、CPU実行の別同意を維持する。[Appleのライブラリ検証の説明](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.disable-library-validation)を参照。録音用entitlementは[Audio Input](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.audio-input)に対応する。

コピーしたapp内の文字起こしTARを容量／SHAと固定3ファイルで照合し、専用の一時領域へ展開する。CLIのarm64・macOS 15・システム依存のみ・署名整合を検査し、Developer ID・timestamp・Hardened Runtimeと`main.plist`の権限で署名する。その実体を圧縮し直し、署名後の容量／SHAと期待Teamをindexへ記録する。noticeと固定build情報は変更を拒否する。元の開発用キャッシュはアドホックのまま保持する。署名／検査失敗は親の署名・公証へ進めず、成功記録を作らない。

CLIの再封入後、ヘルパーを先に署名し、親を最後に封印する。この順序は[Appleの内包コード署名の説明](https://developer.apple.com/library/archive/technotes/tn2206/_index.html#//apple_ref/doc/uid/DTS40007919-CH1-TNTAG201)に従う。署名には`--deep`を使わず、検証にはdeep／strict・Apple anchor・Developer IDの証明書OID・期待Teamを要求する。表示されたTeam文字列だけでは合格にしない。secure timestamp・runtimeビット・実際に封入されたentitlementsも照合する。debug権限や未選択の例外を追加したpolicyは構築前に拒否する。

最終検査でも実TARを再展開し、CLI自体の署名とTeam／権限を検査する。JSONの`runtimeSigning: developer_id_signed`と`signingTeamId`は配置情報であって証明書の検証結果ではない。ネイティブ導入は旧アドホック形式とこの新形式の両方を読めるが、固定出典・容量・SHA・取得内容の再照合・コピー・CPU確認・明示適用を省略しない。公開元の真正性・ライセンス承認や準備完了状態も昇格しない。

## 公証は別の明示モード

既存のnotarytool keychain profileを用意し、アップロードを承認した場合だけ、`--sign-only`の代わりに`--notary-profile <既存のprofile名>`を指定する。両モードの同時指定・未指定は拒否する。このモードは**2回のアップロード**を行うため、通常の署名のみの検証には指定しない。

1. 署名済みappのZIPを提出し、正準UUIDの要求IDとAcceptedを確認する。appへticketをstapleしvalidateする。
2. ticketを持つappとApplicationsへのリンクからUDZO DMGを作り、コンテナ検査とDeveloper ID署名を行う。
3. 最終DMGを提出し、Accepted・staple・validateを確認する。提出後にDMGを作り直さない。
4. 全署名・権限を再検査し、最終DMGの容量とSHA-256を記録する。

Invalid・処理中・不正応答・署名／ticket検証失敗は成功にしない。notarytoolは明示profileだけを使い、環境のメール／パスワードやAPI秘密鍵をログ・結果へ書かない。[Appleの公証条件](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)を参照。

## 結果と未検証ゲート

Macの準備ビルドはオフラインで依存通知文を収集し、`Contents/Resources/THIRD_PARTY_NOTICES.txt`と`third-party-inventory.json`を同梱する。現在のCargo normal/build依存・npm production依存を保守的に対象とし、正確な使用版・申告ライセンス・収集文の容量／SHA・lock SHA・候補が見つからない依存を記録する。MinutesOn自身、標準ライブラリ／Apple SDK、別導入のAIエンジン・モデル等は別確認。全収集物の存在は条件充足の証明ではなく、欠落文・追加NOTICE・対応ソースの取得案内等の確認を残す。[同梱範囲と実検証](verification/macos-dependency-notices-2026-10-10.md)を参照。

コピー後と最終検査後に通知文の容量／SHA・現在のlock・未承認状態を照合し、欠落・旧版・改変・自己承認を拒否する。さらに現在のlocked・offline Cargo metadataとnpm lock／配置済みproduction依存、固定補完indexからメモリ上で再収集し、通知本文の全bytesと全inventoryを照合する。本文を保ったまま不足一覧やpackage情報を変えても拒否する。再収集で通信／build／file書換えは行わず、不一致は署名・送信前に停止する。[回帰と実offline照合](verification/macos-notice-source-verification-2026-10-11.md)。build-resultの`dependencyNotices`は`collected_not_reviewed`であり、ライセンス全体の確認には昇格しない。

パッケージに本文がない場合の補完資料はcrateの正確な版・申告・配布checksum・公開VCS commit／pathに固定し、出典／容量／SHAを保持する。`upstream_license_reference`は説明・参照だけで、不足本文を解消しない。配布時の照合は補完index SHAにも及ぶ。MPL対象の正確な版の公開ソースarchive URLとSHAは通知文にも含める。通常ビルドでは通信せず、ソースの実取得確認や最終的な配布条件確認は別。現在の不足12件、SDK由来等の残条件、実11件のarchive照合については[版固定補完の記録](verification/macos-notice-supplements-2026-10-10.md)を参照。

正常なビルド結果は`build-result.json`に新規保存する。製品名・版・Team・DMG名・容量・SHA、公証要求ID／ticketの状態、工程時間と回収済みキャッシュの場所を含む。手動ビルドの結果であり、取得モデルの互換性や会議品質の証拠ではない。

`asrRuntime.developerIdSigning: verified`は、明示Teamに対する内包CLIの実署名検査が成功した場合だけ記録する。appのticketをCLI単体の公証・Gatekeeper・導入確認の代用にせず、`standaloneNotarization`・`nativeOnboarding`・`licenseReview`は`not_verified`を保持する。2026-11-01に証明書・秘密鍵と公証用プロファイルの認証確認は完了したが、Developer ID署名・公証を含む本番ビルドの成功記録はまだ確認していない。

署名だけなら`notarization: not_requested`。ticketの検査まで成功しても、`runtimeQA`・`gatekeeper`・`licenseReview`・`manualUpdate`は`not_verified`、`distributionReady`はfalseのまま。配布条件の確認、ダウンロード由来のGatekeeper、分離した実機でのGUI・録音・外部推論ライブラリ、通常版の置換と設定・会議・モデルの保持を別に検証する。Gatekeeper・隔離属性・TCCを無効化して合格にしない。

今回のアドホック準備アプリは通常版の識別子なので、実機GUI検証用MinutesOn QAの代わりに起動しない。実施範囲・保持物・時間と残件は[配布準備の検証記録](verification/macos-distribution-preparation-2026-10-10.md)を参照。
