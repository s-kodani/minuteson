# MinutesOnの評価キット

同意を得た音声と正解データを使い、品質・性能を再現可能な条件で測定する任意の評価ツールです。設定の正本は`config.toml`です。現行Markdown議事録と旧来の構造化採点入力の違いは[テストと品質評価](../docs/guides/testing.md#任意の品質と性能評価)を参照してください。

## 準備

音声・正解は`evaluation/data/<dataset-id>/`へ配置し、同意・話者数・長さ・入力ハッシュを実データに合わせて設定します。`needed`の項目は準備できてから更新します。音声をGitやチャットへ添付せず、端末条件・モデル・試行IDと共に評価します。

1. `python3 tools/mon_eval.py doctor`で必須16GB端末それぞれの`cpu_model`と`power_source`を確認し、`evaluation/config.toml` の `needed` 項目を実機・評価音声の情報で更新する。`cpu`は観測文字列に一致させ、`power_mode`には電源プラン・低電力モード等の設定条件を記録する。
2. 同意を得た匿名化済み音声と正解データを `evaluation/data/<dataset-id>/` に置く。
3. `python3 tools/mon_eval.py validate` を実行し、エラーが0件になることを確認する。
4. `python3 tools/mon_eval.py new-run --name <run-name>` で固定した条件の実行記録を作る。

必須16GB端末の試行は、実行時にmacOS 15以降またはWindows 11 24H2以降（クライアントOS、ビルド26100以上）を確認し、各工程の終了時にもOS版が変わっていないことを検査する。OS版を取得できない試行は比較対象にしない。

必須録音マトリクスは `python3 tools/mon_eval.py list-scenarios` で展開できる。両OS × 4クライアント × 全体/アプリ選択 × ヘッドセット/スピーカーの32条件を生成する。

録音したマイクとPC音声の形式、時間差、冒頭と末尾の音量・類似度を調べる。両トラックに同じ校正音を入れた試験でだけ `--calibration-signal-present` を指定する。
macOSプローブの出力は`microphone.caf`と`system.caf`、Windowsプローブの出力は分割WAVである。どちらも`ffprobe`/`ffmpeg`で解析できる。

```sh
python3 tools/mon_audio.py \
  --macos-manifest /path/to/macos-recording/manifest.json \
  --calibration-signal-present \
  --output /path/to/audio-analysis.json
```

Windowsの分割WAVでは保存済みの`manifest.json`を指定すると、記載順の全セグメントをつなぎ、両トラックの最初のWASAPI QPC時刻で共通時間帯を揃えて冒頭と末尾を解析する。セグメント境界をまたぐ窓も評価でき、4時間音声全体をメモリへ読み込まない。タイムスタンプエラーまたはデータ不連続がある録音では、連続した時間軸としての同期解析を拒否する。

解析JSONには各音声セグメントと録音マニフェストのSHA-256を保存する。レポートは合否集計時に再照合し、同じ長さ・形式でも内容が差し替わった録音を無効とする。録音条件ID・全体/アプリ選択モード・要求時間もマニフェストと照合し、共通録音時間を実音声の長さと開始時刻から再計算する。
短時間で停止した録音は末尾の評価窓が作れない場合がある。その場合でも、実音声と冒頭の解析結果を添えた人手レビューは不合格として記録できる。合格には末尾窓と所定の録音時間が必要。

```sh
python3 tools/mon_audio.py \
  --windows-manifest /path/to/windows-recording/manifest.json \
  --calibration-signal-present \
  --output /path/to/windows-recording/audio-analysis.json
```

探索用には`--microphone`と`--system`をセグメント順に繰り返せる。ファイルの並びは自動推測しない。この指定では両ファイルの先頭が同時刻と仮定され、録音条件との結び付きもないため、32条件の合否集計には使えない。分割間の実時間の欠損はWAVの長さだけでは判定できないため、Windows録音マニフェストの不連続数・QPC時刻と照合する。

正の `lag_ms` はPC音声に対してマイク側が遅れていることを示す。十分な相関がない場合は遅延を `null` にする。近似ゼロ率や相関だけで権限拒否、会議中の無音、エコー除去の合否を断定しない。実際の重複発話が消されていないかは正解発言と人手で照合する。

`evaluation/data/` と実行結果は `.gitignore` で除外される。共有時は、音声を含めてよいかを別途確認する。

## 正解データ形式

### transcript.json

```json
{
  "schema_version": 1,
  "text": "正規化前の正解文字起こし",
  "utterances": [
    {"id": "u1", "start_us": 0, "end_us": 1500000, "speaker": "s1", "text": "おはようございます"}
  ]
}
```

### diarization.json

```json
{
  "schema_version": 1,
  "segments": [
    {"start_us": 0, "end_us": 1500000, "speaker": "s1"}
  ]
}
```

重複発話は時間が重なる複数セグメントで表す。区間は開始を含み終了を含まない。
必須データセットで`contains_overlap=true`とした場合、異なる話者の区間が実際に重なる必要がある。発言の話者IDと時刻も、その話者の区間との重なりを検査する。同一話者の重複区間を二人の同時発話とは数えない。

### minutes-truth.json

```json
{
  "schema_version": 1,
  "items": [
    {
      "id": "d1",
      "kind": "decision",
      "body": "採用を決定した事項",
      "assignee": null,
      "due": null,
      "evidence_utterance_ids": ["u1"]
    }
  ]
}
```

`kind` は `decision`、`todo`、`open_issue`、`topic`、`summary` のいずれか。担当者・期限が不明なら `null` とする。
必須の10話者評価データでは、決定事項、担当者・期限がともに明示されたToDo、いずれかが未確認のToDoを正解に含める。`validate`はこの構成と根拠IDの参照を確認する。

### terms.json

```json
{"schema_version": 1, "terms": ["製品名", "専門用語"]}
```

評価に使う登録用語・固有名詞の固定リスト。`config.toml`の`terms_path`で指定し、試行作成時にSHA-256を保存する。正解文字起こしに少なくとも1回出現する用語を含める。用語採点には正解・予測の両方に明示的な`text`が必要。同じリストを用語ヒントなし／ありの両方に適用する。

## 採点

```sh
python3 tools/mon_eval.py score-transcript \
  --reference evaluation/data/sample/transcript.json \
  --hypothesis /path/to/hypothesis.json \
  --terms evaluation/data/sample/terms.json \
  --run evaluation/runs/<run-id> --trial-id <trial-id>

python3 tools/mon_eval.py score-diarization \
  --reference evaluation/data/sample/diarization.json \
  --hypothesis /path/to/hypothesis.json \
  --run evaluation/runs/<run-id> --trial-id <trial-id>

python3 tools/mon_eval.py score-minutes \
  --reference evaluation/data/sample/minutes-truth.json \
  --hypothesis /path/to/minutes.json \
  --matches /path/to/reviewed-matches.json --reviewer <reviewer-id> \
  --run evaluation/runs/<run-id> --trial-id <trial-id>
```

whisper.cppの`--output-json`で得たセグメントJSONは、ミリ秒の`offsets`を評価用のマイクロ秒区間へ変換できる。単語・トークンの時刻は使わない。

```sh
python3 tools/mon_eval.py convert-whisper \
  --input /path/to/whisper-output.json \
  --output /path/to/hypothesis.json \
  --run evaluation/runs/<run-id> --trial-id <trial-id>
```

生成議事録の項目IDが正解データと異なる場合は、担当者が本文と根拠を見て意味が対応する項目を1対1で記録する。`reviewed-matches.json` は `{"schema_version":1,"matches":[{"reference_id":"d1","hypothesis_id":"generated-7"}]}` という形式。対応しない項目は追加漏れ・根拠のない追加として数える。意味が対応しても`decision`と`todo`を取り違えた項目は`misclassified_pairs`に残し、種類別・全体の適合率と再現率では正解に数えず、`missing_ids`と`unsupported_ids`の両方へ入れる。ToDoの担当者と期限は個別の一致率も出す。単独採点ではIDが一致する場合に`--matches`を省略できるが、実行記録へ添付する議事録スコアには、同じIDを含めて人手確認した`--matches`と`--reviewer`が必須。

`--run`と`--trial-id`を使う採点は、正解データの固定ハッシュと、同じ試行のエンジン出力（または検証済み変換結果）のハッシュが一致した場合だけ`run.json`に追記する。単独の手動採点には両オプションを省略できる。`--output`で指定する採点結果や変換結果は新しいパスへ保存し、既存ファイルを上書きしない。

ローカルエンジンは実行ディレクトリへ計測結果を追記する。`--`より後はそのまま子プロセスへ渡す。

```sh
python3 tools/mon_eval.py benchmark \
  --run evaluation/runs/<run-id> \
  --stage transcription --engine whisper.cpp --profile standard \
  --trial-id candidate-a-cold --run-kind cold --variant baseline \
  --device-id <16gb-device-id> --dataset-id <dataset-id> \
  --model-file /absolute/path/to/model.bin \
  --engine-settings-file /absolute/path/to/transcription-settings.json \
  --output-file /absolute/path/to/whisper-output.json \
  --input evaluation/data/<dataset-id>/source.flac --input-duration 3600 \
  -- /absolute/path/to/whisper-cli -m /absolute/path/to/model.bin -f evaluation/data/<dataset-id>/source.flac
```

必須16GB端末の各工程には、比較条件として固定するエンジン設定JSONが必要。例：`{"schema_version":1,"engine":"whisper.cpp","settings":{"threads":8,"language":"ja","beam_size":5}}`。入力・出力パスや用語あり／なしの差分ではなく、共通に維持する推論設定を記録する。レポートはこのファイルのハッシュを再照合して構成指紋へ含める。JSONの申告内容と実行コマンドの対応は人手レビューで確認する。

同じ一連の解析に属する`audio-preparation`、`transcription`、`diarization`、`integration`、`minutes`には同じ`--trial-id`を指定する。60分の一連試行では、音声準備の`--input`を固定データセットの音声とし、その出力を文字起こし・話者分離の`--input`に使う。統合工程は文字起こし出力を`--input`、話者分離出力を`--dependency-file`として指定し、議事録工程は統合出力を`--input`に使う。各工程の成果物を`--output-file`で記録する。レポートはファイルハッシュでこのつながりと実行バイナリ・モデル・ログの現存内容を検査し、欠けている試行や改変された試行を時間目標達成に数えない。用語登録あり/なしや初回/ウォーム実行は別の試行IDにする。`--variant terminology`の文字起こし工程には固定した`terms.json`を`--dependency-file`で指定し、`baseline`には指定しない。これは用語ファイルの同一性の証拠であり、エンジンが実際にヒントとして使ったかはコマンドと結果を人手で確認する。モデルが複数ファイルなら`--model-file`を繰り返す。文字起こし・話者分離・議事録には少なくとも1件のモデルファイルが必要。端末とデータセットは実行記録の設定スナップショットで`available`になっている必要がある。

標準出力・標準エラーは実行ディレクトリ内の一意名で保存され、起動失敗後の再試行や同時実行でも既存ログを上書きしない。`run.json`にはコマンド、実行バイナリ、端末・データセット・試行ID、開始・終了時の実機CPU型番と給電状態、主入力・追加依存ファイル・モデル・出力・ログのSHA-256、実時間、入力長に対する処理比率、最大RSS、終了コードが記録される。レポートはバイナリとログを含む証拠ファイルの変更を再検査し、バイナリハッシュをモデル構成比較にも含める。実行コマンドがスクリプトや設定ファイルを参照する場合は、それらを`--dependency-file`にも指定する。追記は短い排他区間で最新記録を読み直し、置換保存する。異常終了で`.run.json.lock`が残った場合は、実行中の書き込みプロセスがないことを確認してから空のロックディレクトリを除去する。必須16GB端末ではCPU型番・給電状態が固定設定と一致しない試行、または開始・終了で変わった試行を比較対象から外す。二点の観測で途中の一時的な切替まで否定はできない。macOSでは終了した直接の子プロセスの最大RSS、WindowsではプロセスのPeakWorkingSetSizeを取得する。採点を試行へ添付する工程では、まだ存在しない出力パスを`--output-file`で指定する。既存ファイルの使い回しを防ぐため、出力パスが存在すると実行前に拒否する。入力・依存ファイル・モデル・設定が測定中に変わった場合や、指定出力が欠けた場合は整合性エラーとして記録する。モデル取得時間やハッシュ計算時間は処理時間に含めない。RSSは孫プロセスを含まないため、対象エンジンを直接起動する。

## M1集約レポート

実測と人手評価が揃った後の構成決定には、[評価結果に残す情報](../docs/guides/testing.md#任意の品質と性能評価)を使う。標準／高精度のモデル構成、数値合格値、録音方式、未達と代替案を試行ID・ハッシュに結び付けて残す。

4つの安定性条件を両OSで試した後、実行ディレクトリに`stability.json`を置く。8件それぞれに判定、担当者、観測記録ファイルとSHA-256、レビュー理由を記載する。4時間録音には`observed_duration_seconds`、強制終了回復には`tail_loss_seconds`を追加する。合格時は録音時間と末尾損失を設定の目標値と照合する。証拠がない試行は未検証扱いとする。

```json
{
  "results": [
    {
      "scenario_id": "four-hour-recording",
      "platform": "macos",
      "verdict": "pass",
      "reviewer": "reviewer-id",
      "notes": "停止後の両トラックを確認",
      "evidence_file": "/absolute/path/to/observation.json",
      "evidence_sha256": "観測記録ファイルの64桁のSHA-256",
      "observed_duration_seconds": 14400
    }
  ]
}
```

`run.json`の`human_review`は、品質と重要な漏れの人手確認後に`status: "completed"`、担当者、空でない`notes`を記録する。レポートの`m1_06_ready_for_decision`は、前提条件、32録音条件、8安定性条件、同一データセットでの両OS×標準／高精度×初回／ウォーム×用語なし／ありの16試行（各5工程と文字起こし・話者分離・議事録の採点）、人手レビューが揃ったかだけを示す。各OS・プロファイル内の4条件は、エンジン名とモデルSHA-256から作る構成指紋も一致しなければ比較候補にならない。`true`でも構成を自動採用しない。`warm`のラベルだけでモデルが実際に読み込み済みだった証明にはならないため、比較条件はレビュー記録へ記す。

構成指紋にはエンジン名・実行バイナリ・モデル・固定エンジン設定の各ハッシュを含む。コマンド行や`warm`ラベルが正しい実行条件を示すかは機械的に証明できないため、決定記録で確認する。

```sh
python3 tools/mon_report.py --run evaluation/runs/<run-id> \
  --output evaluation/runs/<run-id>/report.md
```

`--run`を省略すると現行設定の前提条件と録音マトリクスの不足を表示する。`--format json`で機械可読な結果も出せる。実行記録を指定した場合は、そのディレクトリに保存された`config.toml`と`run.json`のハッシュ一致を確認する。

録音結果の人手レビュー後、実行ディレクトリに`recordings.json`を置く。各条件は1件ずつ記録し、`analysis_file`は同じ実行ディレクトリ内の`mon_audio.py`出力JSONを指す。録音元ファイルは評価データ領域に保持し、レポートにはコピーしない。

```json
{
  "results": [
    {
      "scenario_id": "macos-meet-chrome-system-headset",
      "verdict": "pass",
      "reviewer": "reviewer-id",
      "analysis_file": "macos-meet-chrome-system-headset-analysis.json",
      "notes": "漏れ・重複発話・エコーを音声と照合した記録"
    }
  ]
}
```

録音プローブが失敗して音声解析ファイルを作れなかった条件は、`verdict: "fail"`とレビュー担当者・理由に加え、`capture_manifest`（プローブが出力した`state: failed`のJSON）と`capture_manifest_sha256`を記録する。この場合は`analysis_file`を指定しない。レポートはシナリオID・録音モード・要求時間・対象PID・失敗理由・ハッシュを検査して不合格件数に数える。保存済み音声を聴いて品質不合格とした場合は、通常どおり`analysis_file`とレビュー理由を記録する。マニフェストを残さず異常終了した試行は、未検証のまま別途調査する。

正しい`scenario_id`は`list-scenarios`で取得する。判定・担当者・レビュー理由・対応する証拠が欠けた結果は未検証扱いになる。合格判定には録音時間を要し、レポートは音声ファイルの実際の形式・長さを解析JSONと照合する。Windowsでは元の録音マニフェストとQPC開始差も照合する。レポートは録音条件の進捗、前提不足、実行失敗を列挙し、16GB端末における60分・標準プロファイルの全5工程が揃った試行について、解析30分・議事録5分の時間目標を別々に比較する。高精度プロファイルは実測値を表示するが、標準プロファイルの時間目標を流用しない。添付済み品質スコアは元ファイルと正解データを再検証・再計算して表示する。時間目標の達成もファイル整合も品質合格ではない。人手の意味評価を代行せず、構成決定を自動で合格にしない。

文字評価は空白・句読点・英字大小を正規化したCERと、正解の明示的な`text`に空白による単語境界があり、予測にも明示的な`text`がある場合のWERを出す。発言配列の境界を単語境界とみなさず、どちらかの`text`がない場合はWERを`null`とする。固定用語リストを指定した場合は、NFKC・英字小文字化・句読点の空白化を施した明示的な全文内の各用語の出現回数から適合率・再現率を出す。英数字の用語は他の英数字語の内部一致を除くが、時間や意味までは照合しないため、用語の誤認識や文脈上の誤用は人手でも確認する。話者分離は重複時間が最大となる一対一の話者ラベル対応を求め、重複区間を含む時間加重誤り率を出す。2人以上が同時に発話する区間の適合率・再現率も時間長で採点する。境界許容幅は0 µsで、基準側または予測側に重複区間が存在しない場合、対応する再現率または適合率は`null`となる。議事録は照合済みの決定事項・ToDoの適合率・再現率、担当者・期限一致、対応済み項目内の根拠IDの適合率・再現率を確認する。対応しない追加項目は項目適合率で誤りとする。自由文の意味一致は人手評価欄に記録し、自動採点で推測しない。


## 会議をまたぐ声の照合の校正

`python3 tools/mon_voice_eval.py /absolute/path/voice-dataset.json` は、正解付きの特徴量を読み取り、校正用だけで閾値・次点との差を選び、独立した検証用で確認する。製品DBは変更せず、結果を標準出力へJSONで返す。モデルと抽出方式は全サンプルで一致させる。特徴量は製品の抽出ヘルパーで得たものを使い、人物の正解は人間が確認する。合成特徴量によるツールの単体検査を実音声の精度評価とは扱わない。

入力形式は以下。`personId: null` は未登録者を表す。`recordingHash` は登録元録音のSHA-256で、同じ録音を異なるsplitへ重複配置できない。同一人物を別録音から登録・校正・検証に含め、マイク差と似た別人も含める。

```json
{
  "schemaVersion": 1,
  "modelHash": "<embedding model SHA-256>",
  "extractorVersion": "sherpa-1.13.8-mono16k-normalized-v1",
  "samples": [
    {"split": "enrollment", "recordingHash": "<recording SHA-256>", "personId": "person-1", "vector": [0.6, 0.8]},
    {"split": "calibration", "recordingHash": "<different recording SHA-256>", "personId": "person-1", "vector": [0.6, 0.8]},
    {"split": "validation", "recordingHash": "<independent recording SHA-256>", "personId": null, "vector": [0.8, -0.6]}
  ]
}
```

各評価splitに登録済み人物100区間以上、未登録者20区間以上を要求し、自動割り当て100件以上かつ誤割り当て／自動割り当て1%以下を観測上の受入基準にする。校正では類似度0.50〜1.00、次点との差0.01〜0.50を0.01刻みで走査し、条件を満たす中で正しく自動適用できる件数を最大化する。検証側が未達・不足なら有効なpolicyを出力しない。

出力の `observed_target_met` は与えた正解データでの観測結果であり、別の会議での精度を保証しない。自動適用の有効化には実音声・正解・抽出条件と報告を確認し、対象モデルの `voice_policies` に閾値・差・評価ハッシュ・版IDを記録する必要がある。未評価の条件を一般ユーザー操作から有効化するAPIは提供しない。初期DBの判定条件は空で、すべて候補表示になる。
