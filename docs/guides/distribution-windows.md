---
type: "Guide"
title: "WindowsのEXE配布"
description: "署名済み・未署名通常版とQA版の区別、生成物と実機照合。"
tags: ["windows", "distribution", "signing"]
status: "stable"
sources:
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/app/build-windows.ps1"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/tools/sign-windows.ps1"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/tools/publish-windows-installer.ps1"
  - resource: "https://github.com/s-kodani/minuteson/blob/2d4d96801cdb1b6f849f20d370aa295905409cec/tools/mon_windows_build_evidence.py"
---

# WindowsのEXE配布

公開の開始とバージョンタグ管理は[GitHubでの開発とリリース](development-workflow.md)に従います。以下はOS固有の処理・手元での確認方法です。CIの認証と再実行は[DevSecOps運用](devsecops.md)を参照してください。

## ビルドの選択

[Windows開発環境](build-windows.md)を準備し、リポジトリルートで実行します。

| 用途 | コマンド | 配布用コピー |
| --- | --- | --- |
| 署名済み通常版 | `npm.cmd --prefix app run tauri:windows:build` | `output/windows-release/MinutesOn-setup.exe` |
| 未署名通常版 | `npm.cmd --prefix app run tauri:windows:unsigned:build` | `output/windows-unsigned/MinutesOn-unsigned-setup.exe` |
| QA版 | `npm.cmd --prefix app run tauri:windows:qa:build` | `output/windows-qa/MON-QA-setup.exe` |

未署名通常版は通常版と同じ機能・識別子を使い、QAへ切り替わるものではありません。発行元は未確認として表示されます。`-Qa`と`-Unsigned`の併用は拒否します。MSIXは別経路の[Store手順](distribution-store.md)を使います。

## WebView2の導入と更新

通常版・QA版のEXEは、WebView2 Evergreen Runtimeが未導入の端末でのみ、インストール時にMicrosoftから取得します。未導入のオフライン端末では、別の端末でx64のEvergreen Standalone Installerを入手し、MinutesOnより先にインストールします。[MicrosoftのWebView2配布ページ](https://developer.microsoft.com/en-us/microsoft-edge/webview2/#download-section)

更新前に録音・解析・保存を終え、アプリを終了します。更新画面では旧版をアンインストールしてからインストールする経路を選びます。同じ版を入れ替える場合は、Windowsの設定から旧版をアンインストールして新しいEXEを実行します。アンインストール画面のアプリデータ削除欄を空欄にして、会議・設定・導入済みモデルを保持します。上書きだけの入れ替えでは、旧版の同梱ファイルが残る場合があります。

## 公開用の署名

`MON_WINDOWS_CERTIFICATE_THUMBPRINT`へCurrentUser\MyのCode Signing証明書の完全な拇印、`MON_WINDOWS_TIMESTAMP_URL`へ証明書提供元のHTTPS RFC3161サービスを指定します。

```powershell
$env:MON_WINDOWS_CERTIFICATE_THUMBPRINT = '<完全な証明書拇印>'
$env:MON_WINDOWS_TIMESTAMP_URL = '<HTTPSのRFC3161 URL>'
npm.cmd --prefix app run tauri:windows:build
```

証明書・秘密鍵、署名・タイムスタンプ・署名検証のどれかが欠ければ失敗します。未署名版は専用`target-unsigned`で構築し、自作バイナリのAuthenticodeとPE証明書テーブルが未署名であることを照合します。第三者DLLの署名を除去しません。

## ソースとインストール先の照合

各経路の`output/windows-<flavor>-work/`に、ソースと本体・ヘルパー・インストーラーのSHA-256を含む証跡を残します。CTestと配布直前の照合が成功した後だけ配布先を更新します。`installedVerification: pending`はインストール先が未照合であることを表します。

QAのインストール後の例です。パスは確認する実体へ置き換えます。

```powershell
python tools/mon_windows_build_evidence.py verify --manifest 'output/windows-qa-work/<build-id>-build.json' --installed-app '<インストール先のアプリ.exe>' --installed-helper '<インストール先のmon-windows-capture.exe>' --published-installer output/windows-qa/MON-QA-setup.exe --output 'output/windows-qa-work/<build-id>-installed.json'
```

`currentSourceMatches`と`installed`の本体・ヘルパーを確認します。配布時の検査は`--require-current-source`を付け、元の構築物と現在のソースの一致も必須にします。過去のビルドを照合した結果を現在版の実行成功に流用しません。

実機では起動、録音許可・拒否、音声取り込み・再生、モデル導入・CPU／Vulkan推論、トレイ・終了、二重起動、更新後の会議・設定保持を確認します。署名やハッシュ一致だけでこれらの実行結果を置き換えません。確認状況は[製品概要](../spec/overview.md#配布と評価の確認状況)で管理します。

WebView2の導入検証では、Windows 11 x64の検証環境で、導入済み端末へのオフラインインストール、未導入端末へのオンラインインストール、未導入で通信に失敗した場合の案内を確認します。
