# 話者分離の末尾時刻による失敗の修正（2026-10-24）

## 再現と原因

最新録音の解析出力で `diarization_failed` と「話者区間の時刻・順序・人数が不正です」が発生した。
保存済みの入力WAVは16 kHz、592,718サンプル、37.044875秒。話者分離の標準出力は次の2区間だった。

```text
0.031 -- 19.336 speaker_00
20.062 -- 37.072 speaker_00
```

順序と話者ラベルには問題がなく、最後の終了時刻だけが音声末尾を27.125 ms超えていた。
旧パーサーは20 msを超える超過を拒否したため、エンジンが正常終了しても結果の検証で失敗した。

[sherpa-onnx v1.13.8のComputeResult](https://github.com/k2-fsa/sherpa-onnx/blob/v1.13.8/sherpa-onnx/csrc/offline-speaker-diarization-pyannote-impl.h#L666-L713)
は区間の時刻に受容野の半分を加える。導入済みモデルのメタデータは
`receptive_field_size=991`、`receptive_field_shift=270`、`sample_rate=16000`。
このオフセットだけで30.96875 msになるため、従来の20 msは不足していた。

## 修正

ネイティブ出力の終了時刻だけに、受容野のオフセット・長時間入力のfloat32誤差・ミリ秒表示の丸めを含む50 msの許容範囲を設けた。
許容範囲内の終了時刻は、従来どおり正確な入力音声の長さに切り詰める。
保存済み区間の検証は音声末尾を1 μsでも超えれば拒否する。開始時刻、逆順、重複、話者ラベル、区間数、人数の検証は維持した。

録音音声、会議DB、失敗ジョブ、文字起こしの保存データは変更していない。

## 検証

- 実出力と同じ時刻・ラベルを使う回帰テストを追加し、修正前の失敗、修正後の成功を確認。
- 切り詰め後の区間、固定参照を置き換えたログの再解析、保存時の厳密な境界検証を確認。
- 1秒と134,000秒の入力について、50 ms超過の受入れ・51 ms超過の拒否を確認。
- `cargo test --manifest-path app/src-tauri/Cargo.toml --locked --offline --lib diarization::sherpa::tests`：7成功、実モデル試験1件は通常どおりignore。
- 作成したテスト実行ファイルで`diarization --test-threads=4`：21成功、実モデル試験2件はignore。
- 実録音をアプリから再解析して結果を保存する確認は未実施。修正版への更新後、「文字起こしを再生成」から再実行できる。画面の確認にあるとおり、成功時には本文・話者の修正が置き換わる。

テストログは `.cache/diarization-terminal-tests.log`、`.cache/diarization-terminal-related-tests.log` に記録。
Windows QAのビルドログは `.cache/diarization-terminal-qa-build.log`。

## 更新用インストーラー

`powershell.exe -NoProfile -ExecutionPolicy Bypass -File app/build-windows.ps1 -Qa` は終了コード0。
2026-10-24 15:18 JSTに `app/src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/MON QA_0.1.0_x64-setup.exe`
を生成した（268,441,334 bytes）。SHA-256：

```text
551531cbc7c1fab68545289cb2ed87d8182cbdb21e49158f114a6037f9a2f986
```

QAビルドであり、インストールと起動は行っていない。アプリを終了して更新後、保存済み会議から再生成する。
生成物の記録は `.cache/diarization-terminal-qa-result.json`。
