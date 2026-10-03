# Windows 二系統録音プローブ

M1-02向けのWASAPI検証用CLI。マイクは録音エンドポイント、PC音声は通常の出力エンドポイントのループバック、または指定PIDの子プロセスを含むApplication Loopbackで取得する。録音中は別々のWAVを512MiB以下のセグメントへ分割し、パケット数、無音フレーム数、時刻異常、データ不連続を`manifest.json`へ記録する。

このソースはWindows実機でのビルドと動作確認がまだできていない。GitHub ActionsのWindowsビルドジョブも、変更をpushするまで実行されない。対象OSはWindows 11 24H2以降、x64。Application LoopbackにはWindows 10 build 20348以降のAPIを使う。

## ビルド

Visual Studio 2022、Desktop development with C++、Windows 11 SDKを入れたx64 Native Tools Command Promptで実行する。

```bat
cmake -S probes\windows-capture -B probes\windows-capture\build -A x64
cmake --build probes\windows-capture\build --config Release
```

## 実行

以下の30秒コマンドは動作確認用。必須32条件の証拠を作る際は、評価設定に合わせて`--duration 900`を指定し、会議アプリ・出力機器・シナリオIDを実際の条件に合わせる。

全体録音:

```bat
probes\windows-capture\build\Release\mon-windows-capture.exe --output C:\mon-eval\run1 --duration 30 --mode system --scenario windows-meet-chrome-system-headset
```

アプリ選択録音では会議アプリまたは専用ブラウザのPIDを指定する。ブラウザの音声処理が別の親プロセスにある場合、実際の取得範囲を確認する。

```bat
probes\windows-capture\build\Release\mon-windows-capture.exe --output C:\mon-eval\run2 --duration 30 --mode application --pid 1234 --scenario windows-meet-chrome-application-headset
```

既定の入出力先と会議アプリの利用先が異なる場合は`--render-device-id`、`--microphone-device-id`でWindowsのエンドポイントIDを指定する。全体録音の出力先を確認せずに「PC全体を取得した」と判定しない。

録音したセグメントは`python tools/mon_audio.py --windows-manifest <run-directory>\manifest.json`で解析する。全セグメントを1つの時間軸として扱い、最初のWASAPI QPC時刻で両トラックの共通時間帯を揃えて境界をまたぐ冒頭・末尾の窓も調べられる。タイムスタンプエラーまたはデータ不連続がある録音は連続時間軸での解析を拒否するため、マニフェストの異常を先に調査する。

録音スレッドの失敗は指定時間を待たず検出し、停止時は両トラックの終了処理を試みる。受信フレームがゼロなら`state: failed`を記録する。失敗マニフェストにもシナリオ・モード・要求時間・対象PID・エラーを残すので、音声解析が作れない試行も評価記録の不合格証拠にできる。無音フレームを受信しただけでは、会議音声を取得できたとは判定しない。

Teamsデスクトップ版のApplication Loopbackが無音になる報告がMicrosoftのサンプルリポジトリにある。対象バージョン・出力先・PIDを記録し、標準モードとの違いを優先的に実機で調べる。無音の原因や回避策を実測前に断定しない。

参考: [Microsoft Application Loopback sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/)、[WASAPI loopback recording](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording)、[IAudioCaptureClient::GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)、[Teamsの無音報告](https://github.com/microsoft/Windows-classic-samples/issues/414)。
