# ReazonSpeech / ASR adapters — Windows追試（2026-10-29）

対象ブランチは `codex/asr-adapters-reazonspeech`、取得したコミットは `333c40d`。以下の修正を含む作業ツリーをWindows x64で検証した。

## 修正

- Transducerワーカーの `std::min` / `std::max` とWindowsヘッダーのマクロ、`std::ofstream::close` と共有ワーカーヘッダーのPOSIX互換マクロが衝突するMSVCコンパイルエラーを解消。
- ASR設定をバックアップ用の可搬表現から検証するときの仮パスをWindowsでも絶対パスにした。ホストのパスを可搬JSONに保存しないことと、非対応GPU設定を拒否することを回帰テストで確認。
- 追加したIPCコマンドを登録順に合わせ、登録一覧・ACLマニフェストの契約テストを修正。
- 実モデル統合テストのワーカー名・DLL参照と、設定検証テストの絶対パスをWindowsに対応。
- WindowsカタログのRust/TypeScript検証をReazonSpeech 2種類を含む16モデル・4エンジンへ更新。
- ReazonSpeechからWhisperへ戻す際、切り替え先のVAD標準値をフロントの既存設定保護が拒否する問題を修正。バックエンドIDが変わる場合だけ標準値の変更を許可し、同じバックエンドのモデル変更ではカスタムVAD設定を保持する。往復切り替えと同一バックエンド内の設定保持を回帰テストで確認。
- ブランチ内のRustコードを `cargo fmt` に合わせた。

## 検証

| 検証 | 結果 | 記録 |
| --- | --- | --- |
| Windows Rust全体 `cargo test --locked --lib` | 914成功、44 ignored、失敗なし | `output/asr-rust-tests.log` |
| 最終Windowsカタログ（更新した16モデルのアサーション） | 1成功 | `output/asr-model-catalog-test.log` |
| 実IPCカタログを渡したフロント全体 | 838成功、スキップなし | `output/asr-frontend-final-tests.log` |
| TypeScript / Viteビルド | 成功 | `output/asr-frontend-build.log` |
| Python全体 | 104成功、59スキップ、失敗なし | `output/asr-python-tests.log` |
| 録音ヘルパーのRelease CTest | 6成功 | `output/asr-capture-tests.log` |
| Windows CPU / Vulkan推論ランタイム | ビルド成功、Transducerを含むカタログ生成成功 | `output/asr-windows-runtime-fixed.log` |
| ReazonSpeech実モデル統合（INT8/FP32とFP32） | 成功 | `output/asr-real-model-tests.log` |

実モデル統合では固定SHA-256のモデルを `.cache/asr-test-assets` に取得した。以前のWindows検証で用意した日本語合成音声（約22.34秒）をmono PCM16 / 16 kHzへ変換し、共通RustアダプターとWindowsの常駐ワーカーから各モデルを2回実行した。空でない結果、反復結果の一致、各セッションのモデル読み込みが1回であることを確認した。一般の会議音声の精度・性能比較を示す検証ではない。

## インストーラー

`powershell -NoProfile -ExecutionPolicy Bypass -File app/build-windows.ps1 -Qa` で未署名のQA版を生成する。生成ログは `output/asr-qa-build.log`、配布先は `output/windows-qa/MON-QA-setup.exe`。

成功したビルドのソース差分・本体・ヘルパー・インストーラーの対応は `output/windows-qa-work/*-source.json` と `*-build.json` に記録される。インストール先のバイナリ照合とインストール後の画面操作は今回の対象外。
