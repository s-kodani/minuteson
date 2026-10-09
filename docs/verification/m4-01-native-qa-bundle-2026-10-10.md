# MON QAのネイティブバンドル検証

- 対象: M4-01のMac実機検証準備、M4-02の同梱構成
- 更新日: 2026-10-10
- 状態: ローカル構築・静的バンドル検査成功。起動・実モデル・録音・公証は未検証。計画全体は進行中

## 修正

MacのbeforeBuildは話者特徴量ヘルパーを生成していたが、`tauri.macos.conf.json`のresourcesには録音ヘルパーだけが指定されていた。実行時は`Contents/Resources/mon-speaker-embedding`を優先し、開発リポジトリへのfallbackもあるため、開発環境だけで検査すると同梱漏れを見落とす可能性があった。生成するヘルパーをその実行時パスへ同梱する指定を追加した。本番とQA共通のMac設定に適用する。

設定テストを1件追加し、生成先・同梱先・既存の実行時探索パスを照合する。QA識別子・ウィンドウ・保存先・権限・署名の上書き禁止など既存4件は維持する。Rust・Swiftの製品コード、依存lock、モデル配置・保存先設定は変更していない。

## 構築と静的検査

macOS 27.0（26A428）arm64、Node v22.23.2、rustc 1.98.1、Swift 6.3.3、既存Xcode環境で実行した。必須の16GB評価端末での性能測定ではない。

```sh
cd /Users/skodani/Codebase/mon/app
CARGO_TARGET_DIR=/private/tmp/mon-qa-build.4wSF3z CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 \
SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache \
CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache npm run tauri:qa:build
```

専用targetは今回mktempで作成した。上記は実行記録であり、削除済みのtarget名を再利用する手順ではない。初回Rust release構築、型検査・Vite、両ヘルパーの生成、Tauriバンドル・署名まで終了コード0。録音ヘルパーは既存スクリプトのSwift debug構成を使用し、アプリ本体はRust release、特徴量ヘルパーは既存のopt-level=2構成である。全バイナリがrelease構成だとは扱わない。

| 対象 | 確認結果 |
| --- | --- |
| メインInfo.plist | `CFBundleIdentifier=jp.mon.desktop.qa`、名前・表示名MON QA、version 0.1.0、minimum macOS 15.0 |
| メインバイナリ | Mach-O arm64。署名はadhoc・hardened runtime、TeamIdentifierなし |
| アプリ署名 | `codesign --verify --deep --strict`成功 |
| 話者特徴量 | `Contents/Resources/mon-speaker-embedding`に実ファイル・実行属性あり。個別strict署名検査成功、生成元とSHA-256一致 |
| 録音ヘルパー | `Contents/Resources/MONCaptureProbe.app`に同梱。個別deep・strict署名検査成功、実行ファイルのSHA-256が生成元と一致 |
| 録音の宣言 | 同梱ヘルパーInfo.plistのマイク・音声取得説明、PrivacyInfo.xcprivacyの存在・plutil lint成功 |

SHA-256はメイン`064e475bf7b03d126ba32865627d501205a1742c786eec114a1445b6e86b8483`、特徴量`d9b217f5ebaced3957ebf77b3e35874fc21be06cf474eab0f550c313a21ba8a9`、録音`dd2c1ffc6b2aeddf9b78554a0ed8e1bc7817a7c8441580c3601e3a67776e5920`。設定・テストと生成スクリプト・既存探索コードのSHAを検査後に確認した。

SwiftPMの共通キャッシュreadonly警告、Viteの大きなchunk警告は残る。配布用資格情報を与えておらず、公証はスキップされた。署名検査成功をDeveloper ID署名・公証・Gatekeeper通過の証明にはしない。保存先分離・TCC権限・ヘルパー実行・推論・WebView操作の成功も静的検査から推測しない。

## 検査と所要時間

| 工程 | 時間 | 区分と範囲 |
| --- | --- | --- |
| 実装・調査・記録 | 約10〜15分 | 概算。実装待ち・検査待ちを純作業として加算しない |
| ネイティブ構築全体 | 177.60秒（2分57.60秒） | 実測。型検査・Vite・ヘルパー生成・初回Rust release・バンドル・署名込み |
| 構築内部 | Rust 2分49秒、Swift 0.20秒、Vite 1.40秒 | 上記177.60秒の内数。別加算しない。他の内部工程は個別未計測 |
| 設定テスト | 本体0.07497775秒、コマンド0.12秒 | 5件成功、失敗・skipなし |
| フロントエンド全体 | 本体1.216837167秒、コマンド1.40秒 | 573件成功、失敗・skipなし。設定5件を含む |
| 署名・plist・architecture・SHA・コピー照合 | 個別未計測 | すべて成功。アプリ起動を含まない |

計測済みテスト本体参考小計1.29秒、コマンド参考小計1.52秒。先行設定5件を全体573件に新規件数として二重加算しない。工程時間を単純に経過時間へ加算しない。構築・テストの失敗・中止・再構築はない。未存在パスの読取りを修正した確認コマンドと、sandboxで拒否されたps読取りの権限付き再確認は構築・テスト失敗に含めず、個別未計測として記録する。

変更範囲に応じて実バンドル構築と全フロントエンドを確認し、今回のRust・Swift全体検査は再実行していない。変更のない製品コードの直前の全体`make check`は[tip故障検査](capture-tip-failure-2026-10-10.md)で成功しているが、今回の検査件数・時間へ混ぜない。

## 保持と回収

生成アプリを`/private/tmp/mon-native-qa-artifact.BeeD8B/MON QA.app`へdittoで保持した。元との全ファイル差分なしと、コピー後のdeep・strict署名を確認した。35,212KiB（約34.39MiB）。一時領域のため永続的な配布物ではない。

構築コマンド終了とコンパイラ・バンドラーの不在を確認後、今回だけの`/private/tmp/mon-qa-build.4wSF3z`（1,231,524KiB、約1.17GiB）を削除した。再生成可能な構築物で、必要時は新しいtargetへ再構築する。保持アプリ・ログは削除していない。共通Rustテストキャッシュ10,356,744KiB（約9.88GiB）、製品target38,664KiB（約38MiB）、録音・モデル・ユーザーのVite・既存releaseは保持した。

ログ: `/private/tmp/mon-native-qa-build-v1.log`、`/private/tmp/mon-native-qa-packaging-config-v1.log`、`/private/tmp/mon-native-qa-packaging-frontend-v1.log`。

## 残タスク

取得形式変更・残境界確認1〜2時間、実モデル・残UI・配布6〜12時間、Mac実機・実時間長時間4〜8時間以上、合計11〜22時間以上の暫定残工数を維持する。今回の同梱修正だけで実機統合完了とはしない。起動・保存先確認・許可済み音声コピーの取り込み／再生／編集／保存／再起動は操作許可後、録音・権限承認は別途確認後に進める。署名・公証・先行試用の外部準備待ちは別。正式評価・Windows固有部分とWindows CIは保留のまま。GitHubの読み取り専用APIでWindows capture buildの`disabled_manually`を確認し、有効化・手動実行していない。
