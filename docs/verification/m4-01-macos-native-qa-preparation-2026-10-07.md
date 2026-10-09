# macOS実機検証用MON QAの準備

MON QAは、通常利用のMONとは別のアプリ識別子とウィンドウ名で実機検証するためのビルド設定である。メイン・録音ミニパネル・録音確認・録音説明ウィンドウの操作経路と権限制限は本番と同じにする。起動や録音は設定追加だけでは行わず、操作権限と検証範囲の許可後に進める。M4-01と計画全体は進行中である。

## 本番との分離

| 対象 | 通常利用 | 検証用 |
|---|---|---|
| アプリ名 | mon | MON QA |
| 識別子 | jp.mon.desktop | jp.mon.desktop.qa |
| ウィンドウ名 | mon | MON QAで始まる名前 |
| ウィンドウの役割 | main、recorder、recording-confirmation、recorder-info | 同じ役割とURL |

`app/src-tauri/tauri.qa.conf.json`は識別子・アプリ名・ウィンドウ名だけを変更する。Tauriの既存のmacOS設定は自動的に読み込まれるため、録音・復号・再生ヘルパーと最低OS条件を維持する。通常の設定ファイルとnpmの`tauri`コマンドは変更しない。

起動処理はTauriの`app_config_dir`と`app_data_dir`を使い、インストール済みTauri 2.12.1では既定の領域にアプリ識別子を付ける。QA設定には領域の上書きを置かない。ただし、既存QAの起動設定で保存先が変更されている場合はその保存先が使われる。起動前にQAの設定・DBと本番との分離を確認し、本番の保存先をQAへ指定したり、本番のbootstrap設定をコピーしたりしない。

通常利用のMONを起動すると、未完了の音声取り込み・録音回復や解析待ち行列が動くことがある。検証目的で本番アプリを起動せず、QAのウィンドウ名・バンドル識別子と保存先を確認する。

## ビルドと起動前確認

対象はmacOS 15以降のApple Siliconである。空き容量とNode.js・Rust・Swift・Xcodeの既存環境を確認する。必要容量を確保できない場合はビルドを始めない。リポジトリのtargetへ大容量キャッシュを戻さず、今回専用の一時領域を指定する。

```sh
cd /Users/skodani/Codebase/mon/app
qa_target=$(mktemp -d /private/tmp/mon-qa-build.XXXXXX)
CARGO_TARGET_DIR="$qa_target" CARGO_INCREMENTAL=0 npm run tauri:qa:build
```

このコマンドはローカルビルドだけを行い、アプリやCIを起動しない。生成されたアプリの`Contents/Info.plist`で`CFBundleIdentifier`が`jp.mon.desktop.qa`、アプリ名がMON QAであることを確認する。出力場所は実際のビルド結果で確定する。署名検査も行うが、既存設定のアドホック署名は配布用署名・公証の完了ではない。

アプリ操作の許可後にQAだけを起動する。最初は録音を行わず、利用を許可された音声のコピーで取り込み・再生・編集・保存・再起動を検証する。初回QAに本番の会議や設定が表示された場合は操作を止め、領域の分離を再確認する。録音の検証は、対象音声と時間・取得範囲を確認し、マイク・システム音声の権限承認後に別途行う。

ビルド・検証のプロセスがすべて終了してから、生成アプリと必要なログを保持し、今回作成した専用キャッシュだけを整理する。既存の会議・音声・モデル・本番設定は削除しない。

## 静的検査と実機の確認項目

2026-10-07にQA設定の追加4件を含むフロントエンド268件が成功し、失敗・skipは0件となった。通常のフロントエンドビルドも成功した。検査は識別子の分離、全ウィンドウの役割・URL・寸法・表示条件の一致、保存先・権限・署名・ヘルパー設定を上書きしないこと、起動を含まないビルドコマンドを対象とする。ログは`/private/tmp/mon-native-qa-config-units.log`と`/private/tmp/mon-native-qa-config-build.log`に保存した。

2026-10-07時点ではネイティブバンドルは作成・起動していなかった。2026-10-10に初回のMON QAバンドル構築と識別子・署名・リソースの確認を完了した。構築前に話者特徴量ヘルパーの同梱指定漏れを修正し、本番・QA共通のMac設定へ追加した。詳細と保持場所は[MON QAのネイティブバンドル検証](m4-01-native-qa-bundle-2026-10-10.md)を参照。アプリの起動は行っていない。

実際の設定・DB領域、WebViewとネイティブ操作、録音権限と取得条件、通常・異常時の統合は実機の確認項目として残る。通知実装の本番Reactと合成IPCによる全38組・450ケースは別の検査であり、ネイティブ統合の代わりにしない。正式評価とWindows固有部分は保留し、Windows CIは停止したままとする。

2026-10-10の直前のQAは[release録音ヘルパーとCLI入口の分離](m4-02-capture-release-bundle-2026-10-10.md)で再構築した。最新のPCM形式ガードとreleaseヘルパーを含み、同梱CLIの5形式取り込み・再起動再生は合成音声で確認済み。保持場所は`/private/tmp/mon-native-qa-release-artifact.bs0dqS/MON QA.app`。

直前のQAは[録音パネルの独立した説明画面](m3-08-recorder-info-window-2026-10-10.md)を含めて構築し、`/private/tmp/mon-recorder-info-qa-artifact.p91QMy/MON QA.app`に保持した。

最新QAは[録音パネルの操作通知と原因保持](m3-08-recorder-operation-notifications-2026-10-10.md)も含めて再構築し、`/private/tmp/mon-operation-notices-qa-artifact.oerdTa/MON QA.app`に保持した。署名・識別子・両ヘルパーの確認済みだが、GUI起動・保存先分離・実録音・公証は未検証のままである。
