# 設計判断

採用する方針と理由、保守時に守る境界。

- [ADR 0001 ローカル処理とOS別連携](0001-local-native.md) — 会議の解析をPC内で行い、音声取得と実行環境をOS別に実装する。
- [ADR 0002 共通Rust処理と生成IPC契約](0002-shared-core-ipc.md) — 共通判定・ネイティブ境界・構造型を分離し、IPCとウィンドウ権限を照合する。
- [ADR 0003 保存資産の所有と復旧](0003-storage-recovery.md) — SQLiteと管理音声を所有・内容・操作IDで照合し、不明な状態を保持する。
- [ADR 0004 全体モデル設定と実行条件固定](0004-global-models.md) — モデル選択を全体設定へ集約し、開始済み処理の資材と条件を固定する。
- [ADR 0005 解析結果の直接保存と編集保護](0005-direct-results-edit-protection.md) — 結果を発言一覧へ直接保存し、明示再生成時の競合と後続処理を管理する。
- [ADR 0006 Markdownを議事録の正本とする](0006-markdown-minutes.md) — 議事録をMarkdownプレビューとテキスト編集へ絞り、生成は明示操作にする。
- [ADR 0007 限定的な前進更新と追加復元](0007-forward-updates-backup.md) — 実装が明示的に対応する旧形式だけ更新・復元し、既存データを保持する。
- [ADR 0008 完成版の位置付けとOKF文書管理](0008-release-documentation.md) — 1.0.1を完成版として扱い、現行仕様・手順・判断理由をOKFへ集約する。

- [ADR 0010 PR検査と下書きからの自動公開](0010-devsecops-release.md) — 公開前の検査、Releases画面でのタグ管理、署名処理の分離。

[ドキュメント目次へ戻る](../index.md)
