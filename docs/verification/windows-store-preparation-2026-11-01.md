# MinutesOn 1.0.0の名称統一とStore用MSIX生成

Windows・macOSの正式名称をMinutesOnに統一した。monは内部識別子・実行ファイル・保存形式に使う略称とし、QA表示はMON QAを維持する。macOS録音ヘルパーの表示名はMinutesOn Capture Helper、内部名は従来どおり。

## 生成した成果物

2026-11-01、Windows build 10.0.26200、Windows SDK 10.0.26100.0で、本体・同梱ランタイムを再ビルドし、MakeAppxの検証を有効にしたままMSIXを作成した。

- 提出用原本：`output/windows-store/MinutesOn_1.0.0.0_x64.msix`
- 容量：202,247,688 bytes（約193 MiB）
- SHA-256：`6deb565166d94ba28a09575cc4e62d6b686209d5eded1871f273eb3085832d26`
- ビルドID：`20261101T035434Z-a1fc05affeba438e9fde336530e78465`
- 証跡：`output/windows-store-work/<build-id>/source.json`、`build.json`
- ソース一覧SHA-256：`9db3f771db99629bdf3bc81751c645c8d203041b6df76a23f9008bf284c07f86`
- 同梱113ファイルの容量・SHA、マニフェスト、SHA-256ブロックマップを照合した。公開用コピーへの独立した`verify --package ... --require-current-source`も成功し、`currentSourceMatches: true`を確認した。
- EXEのWindowsメタデータはProductName／FileDescriptionがMinutesOn、ProductVersion／FileVersionが1.0.0。

StoreのNameは`s-kodani.MinutesOn`、Publisherは`CN=C35CA2F5-F526-48AA-A0B4-26E70B666158`、PublisherDisplayNameは`s-kodani`。Store版の内部識別子は`jp.mon.desktop.store`とし、直接配布版・QA版と分離した。

## 検証

| 検査 | 結果 |
| --- | --- |
| フロント単体 | 906件中905件成功、実機fixture依存1件skip |
| TypeScript／Vite本番ビルド | 成功 |
| Chromiumの名称・幅・起動／復旧表示 | 2スイート成功、960pxスクリーンショットを目視確認 |
| Windowsファイル検証／モデル資材読み込み | 7件＋10件成功 |
| Windows録音ヘルパーCTest | 8件成功、14.10秒 |
| MSIX生成・検証ツール | 14件成功。実MakeAppxの小型パッケージ検査を含む |
| EXE配布回帰 | 7件成功。新公開名・既存QA保持・ジャンクション拒否を確認 |
| macOS配布スクリプト | Windows上で5件成功、POSIX限定33件skip |
| 設定・バージョン、ライセンス通知、既存ビルド証跡、署名前提 | 関連Python検査成功。Windowsシンボリックリンク権限依存はskip |
| 生成契約、モデルカタログ、Rust整形、差分空白 | 検査成功 |

Windowsのハードリンク・ジャンクション検査とMSBuild／ブラウザーの検証では、サンドボックスの制約があったため、対象を作業フォルダー内に限定して許可された通常実行環境を使用した。ファイル検証のシンボリックリンク固有の分岐は、OSの作成権限がなく未実施。

本番ビルドは`npm --prefix app run tauri:windows:store:build`で成功した。ログは`output/windows-store-build.log`。初回のMSBuild通信待機と、CTestのPATH検出失敗のログは別名で保持し、後者は検出した絶対パスでCTestを起動する実装と回帰検査で修正した。

## 公開前の残件

提出用MSIXは未署名で、Microsoft Storeによる署名を前提にする。証跡の`installedVerification`／`certification`は`pending`、`distributionReady`は`false`。

実機インストール、録音許可・拒否、モデル導入・CPU／Vulkan推論、常駐終了、Store更新、版ごとのデータ分離とバックアップ復元、アンインストール、WACKは未実施。macOSの実ビルド・署名・公証・旧アプリからの更新も、このWindows環境では未実施。CI追加分は未実行で、Storeへの提出・一般公開も行っていない。

その後のWACK準備では、`output/windows-wack/20261101T043833Z/`に署名済み検証用コピーと管理者実行スクリプトを作成し、同梱113ファイルと提出用原本の不変を確認した。今回専用の一時秘密鍵は署名後に削除した。標準ユーザー環境の管理者認証はキャンセルされ、WACK本体・インストール・証明書の信頼登録は未実施のまま、ユーザーの指示により後日実施とした。ローカルの検証用コピー、公開証明書、再開用スクリプトは保持し、Gitの対象には含めない。

この成果物の証跡はコミット前の作業状態を記録している。コミット後は同じファイル内容でもコミット・差分IDが変わるため、`--require-current-source`は不一致となる。元のMSIXの内容とハッシュは記録済み証跡で照合でき、コミット単位の新しい証跡はCIまたは再ビルドで生成する。

再生成・隔離環境での検査・申請項目は[Store向け手順](../windows-store.md)、macOSの名称変更と更新は[手動更新手順](../manual-update-macos.md)を参照。
