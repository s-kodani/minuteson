# macOS 二系統録音プローブ

M1-03向けに、マイクをAVAudioEngine、PCの再生音をScreenCaptureKitまたはCore Audio Tapsで別々のCAF（非圧縮PCM）へ保存する。ScreenCaptureKitは全体・単一アプリ、Core Audio Tapsは指定した複数のAudio Processを対象にする。会議サービス固有APIには依存しない。両方式を同じ条件で比べ、権限・同期・欠損と対象外音声の混入を確認する。

## ビルド

```sh
cd probes/macos-capture
swift build
./build-bundle.sh
```

Core Audio Tapsの権限説明には`NSAudioCaptureUsageDescription`が必要なため、実録音は`build-bundle.sh`が作る`.build/MONCaptureProbe.app/Contents/MacOS/mon-macos-capture`を使う。スクリプトは生成したアプリにアドホック署名を付ける。再ビルドで権限の再確認が必要になる場合がある。

## 実行

録音は周囲の音声とPCで再生中の音声をファイルへ保存する。評価参加者の同意を得て、機密情報や通知音が入らないことを確認してから実行する。
以下の30秒コマンドは動作確認用。必須32条件の証拠を作る際は、評価設定に合わせて`--duration 900`を指定する。

```sh
.build/MONCaptureProbe.app/Contents/MacOS/mon-macos-capture \
  --output /absolute/path/to/run-directory \
  --duration 30 \
  --mode system \
  --scenario macos-meet-chrome-system-headset
```

選択可能なアプリとPIDを表示し、アプリ録音を実行する:

```sh
.build/MONCaptureProbe.app/Contents/MacOS/mon-macos-capture --list-apps
.build/MONCaptureProbe.app/Contents/MacOS/mon-macos-capture \
  --output /absolute/path/to/another-empty-run-directory \
  --duration 30 --mode application --pid 1234 \
  --scenario macos-meet-chrome-application-headset
```

出力先は新規または空のディレクトリを指定する。既存の録音を上書きしない。アプリ選択はScreenCaptureKitの`SCRunningApplication`に一致する単一アプリであり、ブラウザの特定タブや子プロセスを漏れなく含む保証はない。Chrome・Edge・Zoom・Teamsの各構成で、意図した音が取得できるかを実測する。

Core Audio Taps方式では、現在音声を扱っているプロセスのPIDを列挙して必要なものを複数指定する。子プロセスが後から開始された場合は今回のタップに自動追加されない。ブラウザの実行構成ごとに対象PIDを確認する。

時間を指定した検証で明示停止も受け付ける場合は`--stop-on-file`を付ける。このモードでは`--duration`（省略時30秒）を上限として録音し、出力ディレクトリ内に`stop.request`が作成されると、通常の監視間隔約200msで停止・CAF確定を開始する。停止完了までの時間を保証する値ではない。ファイルを置くだけで録音成功にはならず、プロセス終了と`manifest.json`の`state`、両CAFの検証を待つ。既定動作は従来どおり指定時間で停止する。

Tauriからの会議録音は`--until-stopped --stop-on-file --parent-pid <親PID>`で起動する。4時間の期限では止めず、停止ファイルまたは親プロセスの消失で停止・確定を開始する。`--until-stopped`には両方の停止手段が必須で、`--duration`との併用は拒否する。時間上限を要求していないため、このモードのmanifestでは`requestedDurationSeconds`を省略する。正式評価の時間指定プローブを置き換えるものではない。異常終了後のファイルはアプリの回復処理で確認するまで保存済み扱いにしない。

既存の署名済みヘルパーを使う開発環境では、アプリとヘルパーを同じソースへ再ビルドする。古いヘルパーは新しい引数を拒否するため、以前のバイナリで制限撤廃を検証したことにはならない。

時間上限なしの会議録音は形式版2のmanifestを出力する。マイク・Core Audioのホスト取得時刻とScreenCaptureKitの同期時計から、共通原点・UTC原点・保存した全フレームの補正区間を生成する。失敗した書込み、取得時刻の欠落・逆行、時計対応の変化、補正容量の超過では保存完了を報告せず、元音声を残す。明示スリープ後や機器再接続後の復帰はまだ未対応で、同期精度と実録音の長時間安定性は実機確認が必要である。時間指定の正式評価プローブは形式版1を維持し、同期確認済みにはしない。

ScreenCaptureKitの保存用時刻は、受信したPCMを連続する物理フレームとして保存するよう置き換える。取得時刻の空白は補正区間に残し、ファイルへ正常な無音を追加しない。ヘルパーバンドルには取得時刻を扱う利用理由の `PrivacyInfo.xcprivacy` を同梱する。実装と検査の範囲は[取得時刻と保存フレームの接続](../../docs/verification/capture-clock-producer-2026-10-08.md)を参照。

```sh
.build/MONCaptureProbe.app/Contents/MacOS/mon-macos-capture --list-audio-processes
.build/MONCaptureProbe.app/Contents/MacOS/mon-macos-capture \
  --output /absolute/path/to/coreaudio-run-directory \
  --duration 30 --mode application --backend coreaudio \
  --pid 1234 --pid 5678 \
  --scenario macos-meet-chrome-application-headset
```

初回はmacOSの「画面とシステムオーディオ録音」とマイク権限が必要になる。拒否した場合は録音済みと扱わず、`manifest.json`の状態と標準エラーを確認する。
ScreenCaptureKitの停止でエラーが出た場合も、受信済みCAFの確定を試みた後に`state: failed`とする。ファイルが残っていても合格扱いせず、形式・長さと末尾欠損を実機で確認する。

出力:

- `microphone.caf`: マイク側
- `system.caf`: PC音声側
- `manifest.json`: OS、形式、開始・終了、フレーム数、ドロップ、エラー

4時間の録音ではWAVのサイズ上限を超える場合があるため、macOSプローブはCAFを使う。`tools/mon_audio.py`の二系統解析はCAFをそのまま受け付ける。

Core Audio Tapsのソースとビルドだけでは、音声を正しく取得した証明にならない。両方式で録音結果・音質・アプリ外音声の混入を実測し、別々に記録する。
