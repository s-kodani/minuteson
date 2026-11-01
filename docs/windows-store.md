# MinutesOnのMicrosoft Store向けMSIX

MinutesOn（略称 mon）のWindows 11 24H2以降・x64向け提出パッケージ。macOSは既存のDMG配布を継続する。

## 生成

[Windows開発環境](windows-setup.md)のNode.js 24、Python 3.12、Rust/MSVC、Visual Studio 2022、CMakeに加え、Windows SDKのx64 MakeAppxとMakePRIが必要。SDKとVisual Studio同梱CMakeは自動検出する。

```powershell
# 識別情報とSDKの確認。アプリや成果物を変更しない。
powershell -NoProfile -File app/build-windows-store.ps1 -ValidateOnly

# リポジトリルートで実行。
npm --prefix app run tauri:windows:store:build
```

`app/windows-store.json`にはPartner Centerの値を大文字小文字・句読点も含めてそのまま設定する。秘密情報は含まない。

| 項目 | 値 |
| --- | --- |
| Package/Identity/Name | s-kodani.MinutesOn |
| Package/Identity/Publisher | CN=C35CA2F5-F526-48AA-A0B4-26E70B666158 |
| Package/Properties/PublisherDisplayName | s-kodani |
| Store表示名 | MinutesOn |

アプリのバージョンは共通Tauri設定の`1.0.0`。MSIXは末尾0を付けた`1.0.0.0`とする。更新時は先頭3要素を増やす。各要素は0～65535、先頭は1以上、末尾はStore用に0を予約する。

生成処理は、ソース記録 → Tauri `--no-bundle`ビルド → 録音ヘルパーのCTest → 専用ステージへのコピー → MakePRIの索引生成・候補検証 → MakeAppxのスキーマ／意味検証 → パッケージ内全ファイルのSHA-256照合 → 配布用MSIXの置換、の順に行う。既存EXEビルドの署名処理は独立して維持する。Store用ビルドは`MON_WINDOWS_RELEASE_SIGNING=0`を子プロセスに設定し、自前証明書を必須にしない。

- 提出用：`output/windows-store/MinutesOn_1.0.0.0_x64.msix`
- 証跡・元パッケージ・ステージ：`output/windows-store-work/<build-id>/`
- 証跡：`source.json`と`build.json`。ソース・パッケージ・全同梱物のハッシュを含む。

```powershell
python tools/mon_windows_msix.py verify --manifest output/windows-store-work/<build-id>/build.json --package output/windows-store/MinutesOn_1.0.0.0_x64.msix --require-current-source
```

Store提出用MSIXは未署名。Microsoft Storeが審査後に署名する。生成成功はインストール・実録音・審査の成功を意味しない。証跡の`installedVerification`と`certification`は`pending`、`distributionReady`は`false`で出力する。

## タスクバーのアイコン

通常のタイル画像だけを同梱すると、Windowsがタスクバーのアイコンにアクセント色の背景を補うことがある。`Square44x44Logo`と同じ画像名の`targetsize`、`altform-unplated`、`altform-lightunplated`を14サイズずつ同梱し、ライト／ダーク両テーマで背景を付けずに表示できるようにする。既存のモノクロアイコンの形・色・透過部分は維持する。

素材は既存のSVGから`node app/scripts/generate-windows-unplated-icons.mjs`で再生成できる（事前に`npm --prefix app ci`）。生成済みPNGをソースに含めるため、通常のMSIXビルドで画像のレンダリングは行わない。

MakePRIはAssetsだけを持つ専用入力を使い、マニフェストに対応する`Files/Assets/Square44x44Logo.png`の候補を索引へ登録する。索引のダンプでStore識別子、論理パス、全サイズ・テーマを照合してから`resources.pri`をMSIXに含める。設定とダンプは`output/windows-store-work/<build-id>/pri/`に残し、パッケージへは含めない。証跡の`payloadVersion: 2`はこの構成を必須とし、従来の証跡も引き続き検証できる。

公開済み・インストール済みと同じバージョンで内容だけが異なるパッケージは、通常の更新として扱えない。既存データを残す場合は、同じName・Publisherのままバージョンを上げて更新する。同じ版数の提出用原本を確認する場合は別の検証環境を使用し、現在の登録やデータを削除して回避しない。Windowsが古いピン留めの画像だけを保持している場合は、修正版の起動後にMinutesOnのピン留め1件を登録し直す。

公式資料：[背景なしアイコンの仕様](https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-construction#app-icon)、[MSIXの資材とリソース索引の生成](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion)、[パッケージIDとバージョン](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/package-identity-overview)。

## 保存データと実行環境

Store版の内部識別子は`jp.mon.desktop.store`。直接配布版の`jp.mon.desktop`、QA版の`jp.mon.desktop.qa`と保存先・設定・単一起動を分離する。StoreパッケージのIdentityは上表の値を使う。EXE版から移る際は、旧版でバックアップを作り、Store版の既存の復元操作で取り込む。既存データの直接共有や自動移行は行わない。

録音ヘルパーと推論エンジンはパッケージ内に同梱する。モデル本体は設定から初回ダウンロードし、準備後の処理はローカルで実行する。WebView2はEvergreen Runtimeを利用する。Windows 11で削除・無効化されている場合は、MicrosoftのWebView2 Runtimeを修復してから起動する。

## インストール・認定の検証

Windows Server 2022のCIではパッケージの生成と内容を検証する。MinVersionが`10.0.26100.0`のため、インストール検査にはWindows 11 24H2以降の隔離した検証端末を使う。

提出用の原本を保持し、別フォルダーのコピーを、上表のPublisherとSubjectが完全一致する開発用Code Signing証明書で署名する。検証端末で公開証明書を信頼した上でインストールする。秘密鍵をリポジトリや成果物へ含めない。ビルドコマンド自体は証明書の作成・信頼登録・インストール・マイク録音を行わない。

```powershell
# 以下は隔離検証端末で実施。対象は提出用原本とは別のコピー。
signtool.exe sign /fd SHA256 /sha1 <テスト証明書の拇印> <検証用コピー.msix>
Add-AppxPackage -Path <検証用コピー.msix>
```

確認結果はビルドIDと共に別の検証記録に残す。

1. スタートメニューからの起動、MinutesOnの名称・アイコン、初期設定。
2. マイク許可・拒否、システム／会議アプリ音声の録音、取り込みと再生。
3. モデル取得、CPU／Vulkanでの文字起こし・話者分離・議事録生成。
4. トレイ常駐、再起動、二重起動制御、全プロセスの終了。
5. EXE／Store／QA各版のデータ分離、バックアップ・復元による移行。
6. 同一Identityで増分したバージョンへの更新とデータ保持、アンインストール。
7. Windows App Certification Kitのデスクトップアプリ検査。XML／HTMLレポートを保存し、必須検査の失敗を解消する。任意検査の指摘は検出根拠と実際の動作への影響を評価し、判断を記録してから提出する。[DPI・プロセス起動の検証記録](verification/windows-store-dpi-process-audit-2026-11-01.md)を参照。

Microsoftの[現在の説明](https://learn.microsoft.com/en-us/windows/msix/package/packaging-uwp-apps#validate-your-app-package-locally)ではWACKは非推奨・保守終了で、ローカルでの任意の事前検査という位置付け。正式な認証はPartner Centerへの提出時に行われる。検査ツールのInfrastructureエラーはアプリの不適合と区別し、埋め込みマニフェスト・実バイナリ等の独立した確認結果と併せて記録する。

## Partner Centerへの提出

パッケージ自動提出・一般公開はビルドに含めない。提出前に次を揃える。

- MSIXのIdentityと予約済み製品の一致、バージョン、x64、最低OS。
- 日本語の説明、実際の画面のスクリーンショット、サポートURL、プライバシーポリシーURL、対象年齢・カテゴリ・価格・公開地域。
- `runFullTrust`の理由：既存Win32/TauriアプリとしてWASAPI録音、ユーザーが選択したファイルの保存・取り込み、同梱したローカル推論エンジンの起動を行うため。管理者権限・ドライバーは要求しない。
- マイクと会議音声の用途、ローカルの会議・声の特徴量・内部ログの保存と削除、モデル配布元への通信を説明したプライバシーポリシー。未確認のURLや「外部通信なし」という説明を使わない。
- 審査メモ：対応OS、初回モデル導入に必要な通信・時間・容量、モデル準備後のオフライン操作、録音・生成・終了の再現手順、検証したハードウェアとWACK結果。

公式資料：[手動パッケージ作成](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion)、[MakeAppx](https://learn.microsoft.com/en-us/windows/msix/package/create-app-package-with-makeappx-tool)、[Store提出要件・署名・バージョン](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements)。
