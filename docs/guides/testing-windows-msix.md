---
type: "Guide"
title: "MSIXのローカル実機検証"
description: "同じバージョンで構築したMSIXの照合、証明書登録、導入と録音・解析・復元の確認。"
tags: ["windows", "msix", "testing"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/mon/blob/ab332513fd61f56dff369e5f1c87f837e8fb4865/tools/prepare-windows-store-test.ps1"
  - resource: "https://github.com/s-kodani/mon/blob/ab332513fd61f56dff369e5f1c87f837e8fb4865/tools/mon_windows_msix_test.py"
  - resource: "https://github.com/s-kodani/mon/blob/ab332513fd61f56dff369e5f1c87f837e8fb4865/tools/mon_windows_msix.py"
  - resource: "https://github.com/s-kodani/mon/blob/ab332513fd61f56dff369e5f1c87f837e8fb4865/docs/guides/usage.md"
  - resource: "https://learn.microsoft.com/en-us/powershell/module/appx/add-appxpackage"
  - resource: "https://learn.microsoft.com/en-us/windows/msix/app-installer/troubleshoot-appinstaller-issues"
  - resource: "https://learn.microsoft.com/en-us/windows/win32/appxpkg/troubleshooting"
---

# MSIXのローカル実機検証

Storeと同じ内容の署名済み検証用MSIXを導入し、Windows上の動作を確認します。構築と署名の準備は[Store配布](distribution-store.md)、通常の操作は[使い方](usage.md)を参照してください。

## 検証環境と成果物

- Windows 11 24H2以降・x64、WebView2 Evergreen Runtime、録音用マイクとPC音声を用意します。
- モデルを新規導入する場合は通信とモデル分の空き容量が必要です。
- 用意された検証フォルダーを使用します。署名済みの`*-local-test.msix`、`test-signing-public.cer`、`test-manifest.json`、`test-plan.json`を同じフォルダーに置きます。
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

この例は初回導入用です。通常権限のWindows PowerShellで、手順1の変数を設定してから実行します。

```powershell
$existing = Get-AppxPackage -Name $plan.identity.identityName
if ($existing) {
    $existing | Select-Object Name, Version, PackageFullName
    throw '既存版があります。同版の検証は未導入の検証環境で実行してください。'
}
Add-AppxPackage -Path $package -ErrorAction Stop
$installed = Get-AppxPackage -Name $plan.identity.identityName
if (-not $installed -or $installed.Version.ToString() -ne $report.testVersion -or $installed.Status.ToString() -ne 'Ok') {
    throw 'インストール済みパッケージの版数または状態が不正です。'
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

## 5. 今回のバックアップ修正を重点確認する

1. 検証用会議で、文字起こし本文・話者名・議事録を編集して保存します。用語を登録し、可能なら人物と声も登録します。最終的な本文を控えます。
2. 設定のデータ管理から手動バックアップを作成し、完了を待ちます。
3. 同じバックアップを追加復元します。確認画面の件数を確認して実行し、元の会議を保持したまま復元された会議が増えることを確認します。
4. 復元した会議で、音声再生、現在の文字起こし・話者名・議事録、用語、登録人物・声の保持を確認します。過去の編集履歴や処理ジョブの復活は期待しません。
5. アプリを通常終了して再起動し、元と復元先の両方で保存内容が変わっていないことを確認します。
6. 別の検証用会議の音声だけを削除します。会議の文字起こし・議事録が残り、取り込み元の外部ファイルは保持されることを確認します。その会議を含めてバックアップ・追加復元し、削除済み音声が復活せず、本文が保持されることを確認します。

## 失敗時の記録

インストール失敗時はPowerShellのHRESULTとActivityIdを控えます。直近の導入ログは`Get-AppxLog`、指定した操作は`Get-AppxLog -ActivityID '<表示されたActivityId>'`で取得できます。証明書の信頼不足、実行中のパッケージ、同版・別内容の競合を切り分けます。[Microsoftの導入トラブルシューティング](https://learn.microsoft.com/en-us/windows/win32/appxpkg/troubleshooting)を参照してください。

結果にはWindowsビルド番号、表示倍率、MSIXのSHA256、`buildCommit`、モデル名、CPU／Vulkan、録音機器、操作、期待した結果と実際の結果を残します。アプリ内の内部ログを共有する場合は本文やパスを確認します。署名・構築時の照合成功、インストール成功、実機の機能成功、WACK認定、Store提出はそれぞれ別の結果として記録します。
