# 保存区間の話者特徴量抽出

保存済み16kHzモノラルPCM16 WAVの指定範囲だけを読み、sherpa-onnx 1.13.8のC APIで特徴量を抽出するmacOS専用ヘルパー。録音・ネットワーク・DB操作は行わない。ネイティブ推論をMONのUIプロセスから分離する。

製品側は[話者解析と編集](../../docs/spec/analysis.md)へ接続します。ジョブ単位の特徴量セッションを使う経路は[推論ワーカー](../inference-worker/README.md)を参照してください。ここでは単独ヘルパーの入出力と検査方法を説明します。

## ビルドと録音しない検査

リポジトリのルートから実行する。

```sh
sh probes/speaker-embedding/build.sh --out-dir /absolute/owned/output/directory
make check-speaker-embedding
```

ビルドは標準ライブラリだけのRust実行ファイルを作り、アドホック署名を検査する。`app/build-macos.sh` も `.build/mon-speaker-embedding` を作る。Developer ID署名・公証・配布バンドルへの組込みは別の配布工程となる。

`make check-speaker-embedding` は専用の一時ディレクトリへヘルパーと模擬C APIライブラリを作る。合成PCM、型・範囲・ABIの拒否、ネイティブ資源の解放、製品側の取消・制限時間・ログ上限・異常終了と子プロセス終了を検査し、生成物を回収する。実モデル検査は通常検査に含めない。通常の `make check` にはこの模擬連携が含まれる。

## 固定入力と出力

呼び出しは次の七組の名前付き引数だけを受け付ける。

```text
--protocol-version 1
--library /canonical/absolute/libsherpa-onnx-c-api.dylib
--model /canonical/absolute/embedding.onnx
--wave /canonical/absolute/run.wav
--first-frame 16000
--end-frame 64000
--threads 1
```

フレーム範囲は半開区間で、最大480,000フレーム（30秒）。標準出力はJSON一件と改行だけとし、エンジン・版・16kHz・範囲・次元・取得状態を返す。特徴量は小文字8桁のIEEE 754 binary32ビット列 `vectorBits` で保存し、丸めずに復元する。

取得できない場合は `status: "unavailable"` と理由 `insufficient_audio`、`non_finite_embedding`、`zero_embedding` のいずれかを返す。特徴量はnullのままにし、人物や信頼度を作らない。初期化失敗・版や必須シンボルの不一致・入力改変では終了コード1となり、標準出力に結果を出さない。

製品側はヘルパー・モデル・申告したランタイム・WAVのSHA-256を実行前後で照合し、ファイルの差替えを検査する。子プロセスの環境を空にし、取消・制限時間超過・ログ上限でkillとwaitを行う。ヘルパー自身のサイズ・更新時刻・実体識別子の検査だけではSHA-256照合の代わりにならない。

特徴量抽出の成功は、話者精度や認識品質、10話者・実録音・長時間性能の評価を意味しない。
