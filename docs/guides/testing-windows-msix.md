---
type: "Guide"
title: "MSIXのローカル実機検証"
description: "同じバージョンで構築したMSIXの照合、導入、機能確認と検証後の後片付け。"
tags: ["windows", "msix", "testing"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/a1dfe19d0312a35a302947f30d9dd7871bf75bf5/tools/prepare-windows-store-test.ps1"
  - resource: "https://github.com/s-kodani/minuteson/blob/a1dfe19d0312a35a302947f30d9dd7871bf75bf5/tools/mon_windows_msix_test.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/a1dfe19d0312a35a302947f30d9dd7871bf75bf5/tools/mon_windows_msix.py"
  - resource: "https://github.com/s-kodani/minuteson/blob/a1dfe19d0312a35a302947f30d9dd7871bf75bf5/docs/guides/usage.md"
  - resource: "https://learn.microsoft.com/en-us/powershell/module/appx/add-appxpackage"
  - resource: "https://learn.microsoft.com/en-us/windows/msix/app-installer/troubleshoot-appinstaller-issues"
  - resource: "https://learn.microsoft.com/en-us/windows/win32/appxpkg/troubleshooting"
  - resource: "https://learn.microsoft.com/en-us/powershell/module/appx/remove-appxpackage"
  - resource: "https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.security/about/about_certificate_provider?view=powershell-5.1"
---

# MSIXのローカル実機検証

Storeと同じ内容の署名済み検証用MSIXを導入し、Windows上の動作を確認します。構築と署名の準備は[Store配布](distribution-store.md)、通常の操作は[使い方](usage.md)を参照してください。

## 検証環境と成果物

- Windows 11 24H2以降・x64、WebView2 Evergreen Runtime、録音用マイクとPC音声を用意します。
- モデルを新規導入する場合は通信とモデル分の空き容量が必要です。
- 用意された検証フォルダーを使用します。署名済みの`*-local-test.msix`、`test-signing-public.cer`、`test-manifest.json`、`test-plan.json`、`検証手順.md`を同じフォルダーに置きます。
- 署名準備が成功すると、このガイドに今回のコミット・版・SHA256・証明書拇印・期限・パスを反映した`検証手順.md`が必ず生成されます。配布ZIPにもMarkdownの手順書を含めます。
- `output/windows-store/`の未署名原本はStore提出用です。手元への導入には署名済み検証用コピーを使用します。
- アプリを通常の終了操作で閉じ、録音・解析・取り込み・バックアップが終了していることを確認します。必要な実データはアプリの手動バックアップで保全します。

版数を上げずに構築できます。ただし同じIdentity・同じ版数で内容が異なるMSIXは通常の更新として扱えません。既存版がある場合は、検証専用のWindowsユーザーまたは別の検証PC／VMを使います。別ユーザーでも端末上の既存パッケージと競合するときは別PC／VMで確認します。`-ForceUpdateFromAnyVersion`は同版・別内容の置換を保証するものとして使わず、アンインストールやリセットによる既存データの消去で回避しません。

## 1. 署名済みファイルを照合する

以下のパスを今回の検証フォルダーへ置き換え、Windows PowerShellで実行します。別の検証端末へフォルダーごとコピーした場合も、コピー先を指定します。

```powershell
$ErrorActionPreference = 'Stop'
$testDirectory = 'C:\path\to\windows-store-test\今回の検証フォルダー'
$report = Get-Content -LiteralPath (Join-Path $testDirectory 'test-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$plan = Get-Content -LiteralPath (Join-Path $testDirectory 'test-plan.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$package = Join-Path $testDirectory (Split-Path -Leaf $report.signedTestPackage)
$certificate = Join-Path $testDirectory 'test-signing-public.cer'

if ($report.originalVersion -ne $report.testVersion -or -not $report.manifestVersionIdentical -or
    -not $report.originalUnchanged -or -not $report.signatureCryptoVerified -or
    @($report.payloadChanges).Count -ne 0 -or $report.payloadFiles -ne $report.unchangedPayloadFiles) {
    throw '原本と検証用コピーの版数・内容・署名の照合に失敗しました。'
}
if ((Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash -ine $report.signedTestSha256 -or
    (Get-FileHash -LiteralPath $certificate -Algorithm SHA256).Hash -ine $report.certificate.publicCertificateSha256) {
    throw '配布されたMSIXまたは公開証明書のSHA256が一致しません。'
}
$publicCertificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($certificate)
try {
    if ($publicCertificate.Thumbprint -ine $report.certificate.thumbprint -or
        $publicCertificate.Subject -cne $plan.identity.publisher -or
        $publicCertificate.NotAfter -le (Get-Date)) {
        throw '署名証明書の拇印・Publisher・有効期限が一致しません。'
    }
} finally { $publicCertificate.Dispose() }
$report | Select-Object buildCommit, originalVersion, testVersion, signedTestSha256, payloadFiles
$report.certificate | Select-Object thumbprint, subject, notAfter
```

ビルド元コミット、MSIXのSHA256、証明書拇印を今回案内された値と照合します。準備時の`windowsTrustVerified: false`と`packageInstalled: false`は、準備ツールが端末への信頼登録やインストールを実施していないことを示します。

## 2. 公開証明書を信頼登録する

検証端末で初回のみ必要です。管理者としてWindows PowerShellを開き、同じフォルダーと、前の手順で照合した証明書拇印を指定します。

```powershell
$ErrorActionPreference = 'Stop'
$testDirectory = 'C:\path\to\windows-store-test\今回の検証フォルダー'
$certificate = Join-Path $testDirectory 'test-signing-public.cer'
$expectedThumbprint = '今回案内された40文字の証明書拇印'
$publicCertificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($certificate)
try {
    if ($publicCertificate.Thumbprint -ine $expectedThumbprint) { throw '証明書の拇印が一致しません。' }
    if (-not (Test-Path -LiteralPath ('Cert:\LocalMachine\TrustedPeople\' + $expectedThumbprint))) {
        Import-Certificate -FilePath $certificate -CertStoreLocation Cert:\LocalMachine\TrustedPeople -ErrorAction Stop | Out-Null
    }
} finally { $publicCertificate.Dispose() }
```

登録先はLocalMachineのTrustedPeopleです。CurrentUserの証明書ストアへ登録するだけではApp Installerの信頼確認を満たしません。管理者として別アカウントを使った場合も、アプリの導入は次の手順で検証するユーザーへ戻って実行します。[Microsoftの証明書登録手順](https://learn.microsoft.com/en-us/windows/msix/app-installer/troubleshoot-appinstaller-issues)も参照してください。

## 3. 検証ユーザーへ導入し、今回の本体を確認する

この例は初回導入用です。導入先の検証ユーザーの通常権限のWindows PowerShellで、以下のブロック全体を実行します。証明書登録用に開いた別アカウントの管理者ウィンドウでは実行しません。別端末へコピーした場合は`$testDirectory`をコピー先へ変更してください。

```powershell
$ErrorActionPreference = 'Stop'
$testDirectory = 'C:\path\to\windows-store-test\今回の検証フォルダー'
$report = Get-Content -LiteralPath (Join-Path $testDirectory 'test-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$plan = Get-Content -LiteralPath (Join-Path $testDirectory 'test-plan.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if ([string]::IsNullOrWhiteSpace($plan.identity.identityName) -or [string]::IsNullOrWhiteSpace($report.testVersion)) {
    throw '検証記録のパッケージ名または期待する版数がありません。手順1から照合してください。'
}
$package = Join-Path $testDirectory (Split-Path -Leaf $report.signedTestPackage)
if ((Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash -ine $report.signedTestSha256) {
    throw '導入するMSIXのSHA256が検証記録と一致しません。'
}
$verificationUser = [Security.Principal.WindowsIdentity]::GetCurrent().Name
Write-Host ('導入先ユーザー: ' + $verificationUser)
$existing = @(Get-AppxPackage -Name $plan.identity.identityName -ErrorAction Stop)
if ($existing.Count -ne 0) {
    $existing | Select-Object Name, Version, PackageFullName
    throw '既存版があります。同版の検証は未導入の検証環境で実行してください。'
}
Add-AppxPackage -Path $package -ErrorAction Stop
$installedPackages = @(Get-AppxPackage -Name $plan.identity.identityName -ErrorAction Stop)
Write-Host ('現在のユーザーで見つかった登録数: ' + $installedPackages.Count)
if ($installedPackages.Count -eq 0) {
    throw ('ユーザー ' + $verificationUser + ' にMinutesOnの登録が見つかりません。Add-AppxPackageの実行結果と、導入先ユーザーを確認してください。')
}
if ($installedPackages.Count -ne 1) {
    $installedPackages | Format-List Name, Version, Status, PackageFullName
    throw '検証対象の登録を1件に限定できません。'
}
$installed = $installedPackages[0]
Write-Host ('期待する版数: ' + $report.testVersion + ' / 登録された版数: ' + $installed.Version + ' / 状態: ' + $installed.Status)
if ($installed.Version.ToString() -ne $report.testVersion) {
    throw '登録された版数が検証記録と一致しません。導入したMSIXを確認してください。'
}
if ($installed.Publisher -cne $plan.identity.publisher) {
    throw '登録されたPublisherが検証記録と一致しません。'
}
if ($installed.Status.ToString() -ne 'Ok') {
    throw ('登録されたパッケージの状態が正常ではありません: ' + $installed.Status)
}
foreach ($relative in @('mon-desktop.exe', 'mon-windows-capture.exe', 'windows-runtime/workers/mon-whisper-worker.exe')) {
    $expected = @($plan.files | Where-Object { $_.path -ceq $relative })
    if ($expected.Count -ne 1) { throw ('照合用のペイロード情報が不正です: ' + $relative) }
    $actual = Join-Path $installed.InstallLocation $relative
    if ((Get-FileHash -LiteralPath $actual -Algorithm SHA256).Hash -ine $expected[0].sha256) {
        throw ('インストール済みファイルが今回のビルドと一致しません: ' + $relative)
    }
}
$installed | Select-Object Name, Version, PackageFullName, InstallLocation, Status
Start-Process ('shell:AppsFolder\' + $installed.PackageFamilyName + '!App')
```

同じ版数では画面上のバージョン表示だけで新旧を区別できません。本体・録音ヘルパー・推論ワーカーのハッシュ一致を確認してから、スタートメニューのStore版MinutesOnを使って検証します。WindowsAppsのACL変更や実行ファイルの直接起動は行いません。

## 4. 画面と実音声を確認する

検証専用の短い音声と会議を使います。各項目を成功・失敗・未実施で記録し、モデルや機器がない項目は未実施として残します。

| 確認 | 操作と合格条件 |
| --- | --- |
| 初回起動 | 起動エラーがなく、会議一覧と設定を開ける。通常版・QA版の会議が意図せず混ざらない。 |
| 表示と操作 | ライト／ダーク、100%・150%・200%の表示倍率で、主画面・確認画面・録音パネルのボタンや文字が切れない。タスクバーとスタートメニューのアイコンを確認する。 |
| モデル導入 | 設定のローカルAIで文字起こし・話者分離・議事録のモデルをそれぞれ選び、取得・照合・動作確認・使用への切替が完了する。 |
| 録音前テスト | マイクとPC音声を用意し、「5秒録音して確認」で意図した入力が聞こえる。マイク権限を拒否した場合は理由が表示され、設定変更後に再試行できる。 |
| 通常録音 | 30秒〜1分録音し、メイン画面とミニパネルを切り替える。停止を確定して保存完了を待ち、会議の音声を再生できる。 |
| アプリ指定録音 | PC音声の対象アプリを指定し、対象アプリの音が録音される。対象外の別アプリの音が意図せず混ざらない。 |
| 音声取り込み | WAV・MP3・AAC/ALAC M4A・FLACをそれぞれ取り込んで再生する。選択した元ファイルの内容が変わらない。 |
| 文字起こしと話者 | 日本語音声の解析が完了し、発言から音声を再生できる。本文・話者名・割当ての修正が再起動後も残る。 |
| 議事録 | 生成、Markdownの編集・保存・出力ができる。再生成の確認でキャンセルした場合は現在の本文を保持する。 |
| CPU／Vulkan | 利用できる実行方式で処理が完了する。Vulkanが利用できない端末ではCPU経路と表示される理由を確認し、Vulkan成功とは記録しない。 |
| 終了と再起動 | 録音・解析・保存を完了させ、通常の終了操作後に本体・録音ヘルパー・常駐ワーカーが残らない。再起動すると保存済みの会議を開ける。 |

### 初回案内・設定・履歴・ボイス表示の確認

| 確認 | 操作と合格条件 |
| --- | --- |
| 初回モデル案内 | 未使用の検証ユーザーで初回起動すると、ローカルAIモデルの案内が表示される。「モデルの設定を開く」で設定へ進む。案内だけでダウンロードは始まらない。 |
| 案内の終了保存 | 別の未使用の検証環境で「あとで」または右上の×・Escapeで閉じ、正常終了・再起動後に案内が再表示されない。既存の会議DBからの起動では初回案内を追加しない。既存データを消して初回状態を作らない。 |
| 新規設定の初期値 | 設定未保存の環境で、自動話者統合が「まとめる」、登録した声の自動適用と未検出発言の話者補完の類似度下限が両方0.70である。値を変更して保存し、再起動しても保存した値が維持される。 |
| ジョブ履歴 | 短い検証用の取り込み・解析・バックアップを実行し、「ジョブ一覧」の「履歴」で種類・結果・対象・日時・詳細を確認する。アプリ再起動後も残り、種類と結果で絞り込める。十分な件数がある場合は「さらに読み込む」で重複なく追加される。 |
| モデル準備の履歴 | 取得・配置・動作確認・適用が一連のモデル準備として1件にまとまる。失敗後に明示的に再試行した場合は別の履歴になる。未確定の処理を完了と表示しない。 |
| 関連履歴の削除 | 今回作った検証用会議だけを削除し、関連する履歴が消えることを確認する。削除操作には会議名を残さず、件数・日時・結果を表示する。 |
| ボイスのモデル名 | 声を登録し、ボイス一覧の人物IDと登録モデル名、登録区間ごとのモデル名・短いモデルIDを確認する。登録時のモデル名が現在選択したモデルによって変わらない。 |
| モーダルの配置と焦点 | 初回案内・履歴詳細・設定・確認画面で、タイトルと右上の×が上下中央に揃う。長いタイトル、ウィンドウ幅の縮小、100%・150%・200%倍率、本文スクロール後にも、Tabで表示したフォーカス枠が切れない。Escapeで閉じ、元の操作へフォーカスが戻る。 |

90日間の保持境界、保存済み旧設定の維持、既知・未知のモデル名は自動テストでも確認します。実機で試せなかった項目は未実施として記録してください。版数を変えないため、既存の同版パッケージへの入替え確認と新規導入の確認を混同しません。

## 5. バックアップと追加復元を確認する

1. 検証用会議で、文字起こし本文・話者名・議事録を編集して保存します。用語を登録し、可能なら人物と声も登録します。最終的な本文を控えます。
2. 設定のデータ管理から手動バックアップを作成し、完了を待ちます。
3. 同じバックアップを追加復元します。確認画面の件数を確認して実行し、元の会議を保持したまま復元された会議が増えることを確認します。
4. 復元した会議で、音声再生、現在の文字起こし・話者名・議事録、用語、登録人物・声の保持を確認します。過去の編集履歴や処理ジョブの復活は期待しません。
5. アプリを通常終了して再起動し、元と復元先の両方で保存内容が変わっていないことを確認します。
6. 別の検証用会議の音声だけを削除します。会議の文字起こし・議事録が残り、取り込み元の外部ファイルは保持されることを確認します。その会議を含めてバックアップ・追加復元し、削除済み音声が復活せず、本文が保持されることを確認します。

## 6. 検証後の後片付け

必要な結果の退避、検証アプリのアンインストール、今回の証明書の信頼解除、必要なら署名鍵の削除、検証ファイルの整理の順です。以下は手動実行用の手順で、手順書の作成・更新では削除を実行しません。アプリやデータを引き続き使う場合は、該当する削除を省略します。

### 6-A. データと検証結果を退避する

必要な会議・編集結果はアプリの手動バックアップで退避し、Markdown、ログ、検証結果も保管します。保存先は今回の検証フォルダーと`%LOCALAPPDATA%\Packages\<PackageFamilyName>`の外にします。モデル本体はバックアップ対象外なので、再導入に必要なモデル名を控えます。

アンインストールではパッケージ管理下の会議・録音・設定なども削除対象になります。外部に指定した保存先や書き出しは残る場合があるため、別に確認します。`-PreserveApplicationData`は展開済みファイルから登録する開発アプリ向けのオプションであり、このMSIXのデータ保持には使いません。[Microsoftのアンインストール仕様](https://learn.microsoft.com/en-us/powershell/module/appx/remove-appxpackage)を参照してください。

### 6-B. 今回導入した検証アプリをアンインストールする

編集を保存し、すべての処理が終わってからアプリの「終了」を選びます。ウィンドウを閉じるだけでトレイに残っていないか確認します。今回のインストールに成功した検証ユーザーの、通常権限のWindows PowerShellで実行します。

同版の前回版を取り違えないよう、Name・Publisher・Versionと今回の本体ハッシュを照合してから削除します。インストールに失敗した環境や、既存のStore版を使い続ける環境では実行しません。

```powershell
$ErrorActionPreference = 'Stop'
$testDirectory = 'C:\path\to\windows-store-test\今回の検証フォルダー'
$report = Get-Content -LiteralPath (Join-Path $testDirectory 'test-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$plan = Get-Content -LiteralPath (Join-Path $testDirectory 'test-plan.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$testPackage = @(Get-AppxPackage -Name $plan.identity.identityName | Where-Object {
    $_.Publisher -ceq $plan.identity.publisher -and $_.Version.ToString() -eq $report.testVersion
})
if ($testPackage.Count -ne 1) { throw '今回の検証対象を1件に限定できません。削除を中止しました。' }
$expectedApp = @($plan.files | Where-Object { $_.path -ceq 'mon-desktop.exe' })
if ($expectedApp.Count -ne 1) { throw '今回の本体ハッシュを特定できません。削除を中止しました。' }
$installedApp = Join-Path $testPackage[0].InstallLocation 'mon-desktop.exe'
if ((Get-FileHash -LiteralPath $installedApp -Algorithm SHA256).Hash -ine $expectedApp[0].sha256) {
    throw '今回の本体と一致しません。前回版や別ビルドを削除しないよう中止しました。'
}
$testPackage | Select-Object Name, Version, PackageFullName, PackageFamilyName
Remove-AppxPackage -Package $testPackage[0].PackageFullName -Confirm -ErrorAction Stop
$remaining = @(Get-AppxPackage -Name $plan.identity.identityName)
if ($remaining.Count -ne 0) { throw '登録が残っています。削除をキャンセルした場合も、完了扱いで次へ進めないでください。' }
Write-Output 'このユーザーの検証アプリの登録解除を確認しました。'
```

対象を確認して削除に同意した場合だけ進みます。ほかのユーザーを一括削除するオプションは使用しません。失敗・キャンセル時は原因を確認し、完了扱いで次の段階へ進めません。

### 6-C. 今回の公開証明書の信頼を解除する

この証明書で署名した検証版を、ほかのユーザーも含めて使わなくなった場合に行います。管理者のWindows PowerShellで、今回の拇印だけを対象にします。

```powershell
$ErrorActionPreference = 'Stop'
$expectedThumbprint = '今回案内された40文字の証明書拇印'
if ($expectedThumbprint -notmatch '^[0-9A-Fa-f]{40}$') { throw '今回の証明書拇印を指定してください。' }
$testTrustPath = 'Cert:\LocalMachine\TrustedPeople\' + $expectedThumbprint
if (Test-Path -LiteralPath $testTrustPath) {
    Remove-Item -Path $testTrustPath -Confirm -ErrorAction Stop
}
Test-Path -LiteralPath $testTrustPath
```

最後の期待値は`False`です。信頼解除だけではアプリはアンインストールされません。前回のビルドや、ほかの証明書は対象にしません。

### 6-D. 今後使わない場合だけ署名用証明書と秘密鍵を削除する

次回も今回の鍵で署名する場合は、この段階を省略します。完全に片付ける場合だけ、証明書を作成した本人のWindows PowerShellで実行します。別ユーザーとして開いた管理者の画面は、作成者の`CurrentUser\My`を操作するものではありません。公開証明書だけをコピーした検証PCには、今回の秘密鍵はありません。

非エクスポート型の秘密鍵は削除すると復元できません。次回は新しい証明書の作成と信頼登録が必要です。

```powershell
$ErrorActionPreference = 'Stop'
$expectedThumbprint = '今回案内された40文字の証明書拇印'
if ($expectedThumbprint -notmatch '^[0-9A-Fa-f]{40}$') { throw '今回の証明書拇印を指定してください。' }
$testKeyPath = 'Cert:\CurrentUser\My\' + $expectedThumbprint
if (Test-Path -LiteralPath $testKeyPath) {
    $testSigner = Get-Item -LiteralPath $testKeyPath
    if ($testSigner.FriendlyName -cne 'MinutesOn MSIX local test (persistent)') {
        throw '検証用署名証明書を確認できません。削除を中止しました。'
    }
    Remove-Item -Path $testKeyPath -DeleteKey -Confirm -ErrorAction Stop
}
Test-Path -LiteralPath $testKeyPath
```

削除した場合の期待値は`False`です。`-DeleteKey`は証明書に関連する秘密鍵も削除します。`.cer`ファイルを消すだけでは秘密鍵は削除されません。[Microsoftの証明書プロバイダー仕様](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.security/about/about_certificate_provider?view=powershell-5.1)を参照してください。

### 6-E. 検証ファイルを整理する

結果を退避した後、不要ならエクスプローラーで今回の検証フォルダーと検証ZIPだけを削除します。対象は署名済みMSIX、公開証明書、検証記録、手順書です。過去の検証フォルダー、通常版EXE、利用中の保存先までまとめて削除しません。

Store提出や再照合に使う未署名原本とビルド証跡は保管します。今回の場所は`test-manifest.json`の`originalPackage`と`buildManifest`で確認できます。外部の録音・モデル保存先、Markdown、バックアップ、試験音声は別に確認し、不要な試験用ファイルだけを個別に整理します。WindowsAppsやAppRepositoryを手動削除する作業は含めません。

## 失敗時の記録

インストール失敗時はPowerShellのHRESULTとActivityIdを控えます。直近の導入ログは`Get-AppxLog`、指定した操作は`Get-AppxLog -ActivityID '<表示されたActivityId>'`で取得できます。証明書の信頼不足、実行中のパッケージ、同版・別内容の競合を切り分けます。[Microsoftの導入トラブルシューティング](https://learn.microsoft.com/en-us/windows/win32/appxpkg/troubleshooting)を参照してください。

結果にはWindowsビルド番号、表示倍率、MSIXのSHA256、`buildCommit`、モデル名、CPU／Vulkan、録音機器、操作、期待した結果と実際の結果を残します。アプリ内の内部ログを共有する場合は本文やパスを確認します。署名・構築時の照合成功、インストール成功、実機の機能成功、WACK認定、Store提出はそれぞれ別の結果として記録します。
