# M2-06 llama.cppバッチ推論と結果読出し

議事録の固定入力・文書・候補保存に続き、ローカルllama.cppを1処理単位ずつ起動し、構造化生成結果を文書と固定根拠へ変換する内部アダプターを追加した。分割・統合・チェックポイントワーカー・生成RPC・画面はまだ未接続で、M2-06の完了ではない。

## 実装

- 対話CLIのログを本文と混同しないよう、[公式のバッチ入口](https://github.com/ggml-org/llama.cpp/blob/edd6e2bbdad5930899a93db8fa73c3b61c7b9bcc/tools/completion/completion.cpp)の`llama-completion`を使用する。ローカルGGUF・実行ファイルのパス、容量、SHA-256、スレッド数、コンテキスト、出力トークン数、GPU層数、seedを固定する。
- シェルを介さず、system/prompt/schemaを新規ファイルとして保存し、JSON Schema制約・単一ターン・温度0・コンテキストシフト無効・オフラインで実行する。環境変数をクリアし、RPC・モデルURL・認証情報等を継承しない。入力がコンテキスト上限を超える場合はCLIが失敗する。バイト数を正確なトークン数とは扱わない。
- 標準出力とエラーログを別々に保存し、サイズ・時間上限を検査する。中断・失敗時は子プロセスを終了・回収し、不完全な処理単位を結果やチェックポイントとして返さない。既存ファイルを上書きせず、実行中のモデル・CLI・入力の変更を拒否する。
- 出力は完全なJSONオブジェクト1個だけを許容する。公式バッチ入口の終了マーカーだけは末尾に許可するが、対話ログ、コードフェンス、思考タグ、余分なオブジェクト、途中のJSON、重複フィールドは拒否する。見つかったJSON部分だけを抜き取る救済は行わない。
- 議事録用のschemaVersion 1結果形式を追加し、項目種別・本文・明示的なnullを含む担当者/期限・処理単位内の根拠発言IDを要求する。生成側のschema制約だけを信用せず、必須フィールド・件数・余分なフィールド・固定入力と処理単位のID・引用中の担当者/期限を再検査する。
- モデルにアプリUUID・根拠の解析版・引用文・時刻を生成させない。検証した意味項目から既存の文書処理でIDを作り、固定入力の引用と時刻を対応付ける。モデルの出力は依然として利用者の内容確認を必要とする。
- whisper.cppとllama.cppでファイル指紋・実行中の変更検出・子プロセス回収を共有化し、文字起こしの呼出しAPIは維持する。

## ローカル実エンジン

接続試験用に一時領域で公式ソースをビルドした。配布用エンジンや正式採用モデルではない。

- ソース: llama.cpp `edd6e2bbdad5930899a93db8fa73c3b61c7b9bcc`（バイナリ表示0.5.0-dev）
- CPU＋Accelerate、Metal無効、静的リンク。UI・server・統合appのビルドは無効。
- バイナリ: `/tmp/mon-llama-integration.dq3nxo/build/bin/llama-completion`
- CLI SHA-256: `f3dfac79d358d49d7594fb88d3025e87256b98cb0b9f60c30ca9c8818fc59c28`
- モデル: [提供元のQwen2.5-0.5B-Instruct-GGUF](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF)、Q4_K_M、Apache-2.0表記、491400032 bytes。
- モデル: `/tmp/mon-llama-integration.dq3nxo/qwen2.5-0.5b-instruct-q4_k_m.gguf`
- モデル SHA-256: `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db`

実モデル試験は短い合成日本語発言から1項目のJSONを生成し、議事録の項目形式・固定根拠・Markdownへ変換する経路を確認する。試験時だけ項目数を1個に制約し、抽出の網羅性・正しさは採点しない。会議音声・本文の外部送信は行っていない。モデルはローカルに保持し、提供済みの私的会議音声はこのLLM試験に使用していない。

```sh
env MON_LLAMA_COMPLETION=/tmp/mon-llama-integration.dq3nxo/build/bin/llama-completion \
  MON_LLAMA_MODEL=/tmp/mon-llama-integration.dq3nxo/qwen2.5-0.5b-instruct-q4_k_m.gguf \
  cargo test --manifest-path app/src-tauri/Cargo.toml --locked --offline real_llama -- --ignored --nocapture
```

## 検証と残件

新規の通常Rustテスト6件で、厳密なJSONと重複フィールド拒否、固定引数と入力保護、失敗・中断・時間超過・過大ログ時の結果拒否と子回収、モデル/入力改変、構造化項目からの文書変換、必須/未知フィールドと処理単位外の根拠ID拒否を確認した。最終の実llama.cpp接続試験1件と、共有化後の実whisper.cpp回帰試験2件も成功した。

最終の`make check`が成功。Python 65件、フロントエンド15件、Rust通常90件、AVFoundation製CAF読出し1件。正式評価の前提不足7警告は未解消として保持する。開発用`.app`のビルドと`codesign --verify --deep --strict`も成功（14.35 MiB、アドホック署名・公証なし）。ネイティブ生成画面の実操作は未検証。

長時間入力の分割抽出と統合、処理結果ファイルの管理登録、チェックポイント再開、録音優先のスケジューラー接続、生成操作・候補確認・編集・出力画面が残る。親アプリの強制終了時に解析子プロセスを停止する対策も未実装。今回のアダプターは呼出し元が生存する間の中断・失敗に対応するもので、長時間安定性・議事録品質・5分目標・ネイティブ画面の統合を証明しない。正式評価・10話者・16GB端末・Windows固有部分は保留を維持する。
