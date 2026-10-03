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

アプリから明示停止する場合は`--stop-on-file`を付ける。このモードでは`--duration`を上限として録音し、出力ディレクトリ内に`stop.request`が作成されると約200ms以内に停止・CAF確定を開始する。ファイルを置くだけで録音成功にはならず、プロセス終了と`manifest.json`の`state`、両CAFの検証を待つ。既定動作は従来どおり指定時間で停止する。

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
