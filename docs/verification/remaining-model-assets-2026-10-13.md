# 残モデル候補の公式素材取得

2026年10月13日、ユーザーが公式配布元からの取得上限を合計8GiBへ拡張した。Whisper base・mediumとQwen 2.5 1.5B Q8_0の3モデルを取得し、全バイトの容量とSHA-256が現行カタログに一致した。追加完成ファイル量は3,576,246,652bytes、既知の過去累計と合わせて7,370,638,904bytes、約6.86GiB。上限8GiB以内である。

取得とハッシュ検査の完了であり、モデルのCPU互換性・製品ワーカー・Mac画面での選択や設定適用・認識や議事録の品質の合格ではない。正式評価・10話者・Windowsの保留は維持する。

## 配布情報と実体照合

| モデル | 完成ファイルbytes | SHA256 |
| --- | ---: | --- |
| Whisper base 多言語 | 147951465 | `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` |
| Whisper medium 多言語 | 1533763059 | `6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208` |
| Qwen 2.5 1.5B Instruct Q8_0 | 1894532128 | `d7efb072e7724d25048a4fda0a3e10b04bdef5d06b1403a1c93bd9f1240a63c8` |

Whisperは `whisper-variants.json` と同梱recipeの固定revision `5359861c739e955e79d9a303bcbc70fb988958b1` を使う。[base配布ファイル](https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.bin)と[medium配布ファイル](https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-medium.bin)は、アプリの同じモデル定義から決まるURLである。GGML版の配布元はwhisper.cppのメンテナーで、OpenAIの元の重みと同一形式という主張ではない。

Qwenは `macos.json` の `llama-b11533-qwen25-15b-q8` と公式Qwenの固定revision `91cad51170dc346986eccefdc2dd33a9da36ead9` の[配布ファイル](https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/91cad51170dc346986eccefdc2dd33a9da36ead9/qwen2.5-1.5b-instruct-q8_0.gguf)を使う。各モデルの期待容量・ハッシュを変更して取得結果に合わせることはしない。

取得先は私的な `/private/tmp/mon-remaining-official-models.BH2AGS`。固定HTTPS URLからHTTPSへのリダイレクトだけを許可し、HTTP失敗・最大ファイル容量・接続30秒・全体900秒の制限を使った。利用者のcurl設定は読み込まず、認証情報を渡さず、各取得は終了コード0。取得完了後にサイズと全ファイルの独立SHA-256を確認した。ハッシュ一致は固定カタログとの内容一致であり、配布元の署名や品質の保証ではない。

モデル原本と取得・照合ログはこの専用フォルダーへ保持する。現在のQAの会議・モデル保存先へ配置せず、CLIも起動していない。後続のCPU確認には同じ素材を再利用し、追加ダウンロードしない。実行ファイルとライセンスは一致する取得済みの公式素材を別途照合して使う。

## 時間と残る検証

3モデルは並行取得で、base 16.98秒、medium 150.35秒、Qwen Q8 188.58秒だった。3件合計355.91秒は工程時間の合計で、実際の経過時間ではない。全3ファイルの最終SHA-256計算は14.54秒。取得意図の通常アプリ登録・管理コピー・設定適用・実推論はこの取得操作に含めない。

既知の過去の完成ファイル取得累計は3,794,392,252bytes。今回の累計は完成ファイルのバイト量で、HTTPヘッダー・配布metadata・プロトコル等を含む全通信量の実測ではない。実行ファイルやライセンスを今回再取得していないので、その追加量は0bytes。モデル原本・会議・設定の削除はない。

残るモデル候補について、固定recipeからの管理配置・CPU確認・全体設定への適用と製品統合を進める。Whisper base・mediumとQwen Q8は未実行で、取得済みのint8話者分離も別途確認が必要。標準モデルと小型モデルの既存の短い確認だけで全9候補の統合完了としない。モデル検証の目安は2〜4時間で、正式品質・60分性能は保留の評価条件に従う。実録音の実施時間と署名・公証の準備条件は変更しない。
