# macOSと直接配布の更新境界

Mac App Store配布とDeveloper ID署名・公証した直接配布を区別する。Store向けの更新方式を直接配布版へ流用しない。

- 更新の案内だけなら、信頼する配布元の公開情報と現在版を比較し、対応CPU・OSの成果物があることを確認して配布ページへ案内できる。
- アプリ内で更新を適用する方式は、署名した更新用成果物、更新情報の配信、署名鍵の管理、インストール先、終了と再起動まで含む別の変更になる。通知の依頼だけで導入しない。
- 更新用の署名検証と、Developer ID署名・Apple公証の検証は目的が異なる。採用する更新機構が必要とする証拠をそれぞれ確認する。
- アプリの置換前に録音・解析・保存を終了する。プロセスの再起動は、同じデータや途中成果を安全に再度開けることと一緒に検証する。
- DMG上からの実行、Applicationsへの配置、読み取り専用領域、権限不足を採用方式に応じて確認する。
- 起動中に取得・適用・再起動するか、正常終了後や次回起動へ延期するかは製品の方針に合わせる。

公式資料:

- [Appleの配布方式](https://developer.apple.com/documentation/xcode/distributing-your-app-for-beta-testing-and-releases)
- [Tauriの更新機構](https://v2.tauri.app/plugin/updater/)
- [GitHub Releases API](https://docs.github.com/en/rest/releases/releases)
