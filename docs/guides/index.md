# 利用と保守

操作・構築・配布・更新・障害対応の手順。

- [GitHubでの開発とリリース](development-workflow.md) — ブランチ・PR・文書更新・必須検査とReleases画面からの公開。
- [DevSecOpsの検査と運用](devsecops.md) — セキュリティ検査、例外、GitHub保護設定、配布認証の初期設定。

- [MinutesOnの使い方](usage.md) — モデルの準備から録音・取り込み・修正・議事録出力までの操作。
- [macOSの開発とQAビルド](build-macos.md) — Apple Siliconでの前提環境、開発起動と通常版・QAアプリの構築。
- [Windowsの開発とQAビルド](build-windows.md) — Windows x64の開発ツール、依存準備とQAインストーラーの構築。
- [macOSの署名と公証とDMG](distribution-macos.md) — Developer ID署名、公証用認証の準備とDMGの生成・確認方法。
- [WindowsのEXE配布](distribution-windows.md) — 署名済み・未署名通常版とQA版の区別、生成物と実機照合。
- [Microsoft StoreのMSIX配布](distribution-store.md) — Store原本の生成・照合、同一内容のローカル検証と提出時の確認。
- [MSIXのローカル実機検証](testing-windows-msix.md) — 署名済み検証用パッケージの導入、同版ビルドの照合と録音・解析・復元の確認。
- [アプリ更新とデータの保持](updating.md) — 通常版・QA・Storeの更新と、保存形式の検査・データ保持の確認。
- [起動と録音と解析の障害対応](troubleshooting.md) — 起動診断、録音・モデル・解析失敗と保存結果不明時の確認方法。
- [テストと任意の品質評価](testing.md) — 共通・OS別の検査コマンド、Rustキャッシュと品質評価の使い分け。
- [OKFドキュメントの管理](documentation.md) — 文書の分類、OKFメタデータ、ADRと仕様を更新するためのルール。

[ドキュメント目次へ戻る](../index.md)
