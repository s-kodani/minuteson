# 入力デバイスを意識させない録音の調査（2026-10-14）

## 結論と製品方針

通常は「自動」を初期値とし、MON単独録音ではOSの既定入力を追従する。会議アプリを対象にした録音では、そのプロセスが使用中の入力デバイスをCore Audio HALから一意に確認できた場合のみ、そのデバイスを優先する。プロセスがまだ入力していない、ヘルパープロセスが別PIDである、候補が複数ある、または情報取得が失敗した場合はOSの既定入力へ戻し、切替理由を重複しない通知で知らせる。ユーザーの回答による決定であり、推測だけで別のマイクへ切り替えない。

「固定」は明示したUIDのみを使用する。録音中の自動／固定切替と固定機器の変更は別の操作として提供する。機器の切断・権限変更・切替中の時刻と欠損を明示し、切替の前後で取得元を混同しない。新しい機器のPCM形式を保存形式へ変換できない場合は、無音を捏造せず入力を中断して通知する。

## macOSで確認した公開API

- Xcodeに同梱の `CoreAudio.framework/Headers/AudioHardware.h` は `kAudioHardwarePropertyTranslatePIDToProcessObject` でPIDからAudioObjectIDを取得でき、存在しないPIDは `kAudioObjectUnknown` を返すと明記する。
- 同ヘッダーは `kAudioProcessPropertyDevices` の **input scope** がプロセスの入力デバイス一覧を返し、output scopeは出力デバイス一覧を返すと明記する。`kAudioProcessPropertyIsRunningInput` で入力IOの活動も確認できる。これらは音声サンプルを取得せずに問い合わせられる。
- AppleのSwiftラッパー [`AudioHardwareProcess.devices`](https://developer.apple.com/documentation/coreaudio/audiohardwareprocess/devices) は出力デバイスだけを説明しているため、この用途ではscopeを明示するHAL C APIを使う。対象アプリが別の音声ヘルパープロセスを使う場合などはPIDとの対応を実機で検証する必要がある。
- [Apple Developer Forumsの報告](https://developer.apple.com/forums/thread/748257) では、FaceTimeで通話中にマイクを切り替えた後、`kAudioProcessPropertyDevices` のinput scopeが出力機器IDを返した例がある。Appleによる確認・修正の発表ではなく単一の利用者報告だが、HALの結果を確定事実とは扱わず、入力能力の照合と実機切替試験が必要。入力がない機器IDや曖昧な結果はOS既定へフォールバックする。
- OS既定入力は [`AudioHardwareSystem.defaultInputDevice`](https://developer.apple.com/documentation/coreaudio/audiohardwaresystem) または既存の `kAudioHardwarePropertyDefaultInputDevice` から取得できる。

## Windowsの調査（実装・実機検証は保留）

- [`IMMNotificationClient::OnDefaultDeviceChanged`](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immnotificationclient-ondefaultdevicechanged) は `eCapture` とロールごとの既定入力変更・新しいendpoint IDを通知する。OS既定追従は実装可能。
- capture endpointごとに [`IAudioSessionManager2::GetSessionEnumerator`](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessionmanager2-getsessionenumerator) と新規セッション通知を併用し、[`IAudioSessionControl::GetState`](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol-getstate) のactive状態と [`IAudioSessionControl2::GetProcessId`](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol2-getprocessid) のPIDを照合する方法が候補。ただし共有セッションの `AUDCLNT_S_NO_SINGLE_PROCESS` ではPIDが一意でなく、endpointが無効化される場合もある。Windows実機で会議アプリ・通信ロール・ヘルパープロセスを検証するまで確定仕様にしない。

## 全マイクの音声を常時監視しない理由

48kHz・mono・Float32なら生PCMは1台につき約192kB/s、8台でも約1.54MB/sで、転送量だけが決定的な問題ではない。少数の機器で各ストリームを開いて音量だけを見ること自体は性能的に検討可能だが、実際のCPU・電力・安定性はデバイス数とドライバで測定する必要がある。一方、全機器のストリーム起動はUSB/Bluetooth機器の電力、権限、他アプリとの競合、異なるクロック・サンプルレート、切断時処理を増やす。さらに音声レベルは会議アプリがその機器を選んだ証明にならない（無音の通話、同じ部屋の複数マイク、他アプリ利用）。HALの入力デバイス情報を先に使い、全機器録音による推定は採用しない。もし補助信号にするなら、利用者に取得範囲を明示して承諾を得たうえで、候補数・監視時間を制限し、無音や重複音声から自動的に機器を断定しない。

## 実装前に守る既存境界

現行の `MicrophoneRecorder` と `MicrophoneRecovery` は開始時のUIDを固定し、`microphone.caf` と生セグメントは最初のPCM形式・クロックを前提にする。UIDだけを変更する改修は不正なPCMを保存し得る。入力決定、切替要求の所有権・停止競合、形式変換または区間別保存、クロックと欠損表示、重複しない通知を一体で実装・合成テストする。現時点では実録音を行わず、実機器切替の成功と会議アプリPIDの照合は未検証とする。

## 実装中の接続と確認

`InputDevices.activeInputObjects(for:)` でPIDをCore Audioのprocess objectへ変換し、入力IOが稼働中の場合だけinput scopeのデバイスIDを読み取る。`automaticChoice` はそのIDが一覧内の1台と一意に一致するときだけアプリ側を選び、それ以外はOS既定を選んで理由を返す。どちらも録音ストリームを開かない。合成判定と、このMacのテストプロセスでの読取りが成功。

録音開始画面の初期値は「自動」、明示的な機器選択は「固定」とした。通常録音の自動ではヘルパーに固定UIDを渡さない。録音中の変更はセッションIDと単調増加する要求版を持つ所有ファイルで受け渡し、ヘルパーが要求を採用した版と実際の選択先を別の状態ファイルに公開する。UIは要求受付と反映を混同しない。会議アプリの入力が不明・複数の場合にOS既定を選んだことを、同じ状態の繰り返しでは重複しない通知として表示する。入力接続・PCM受信・保存成功の証明は既存の別状態で扱う。

機器切替前に旧バインディングを破棄し、遅延コールバックを拒否する。新機器のレート／チャンネル数が異なる場合は既存CAF形式へ変換し、生セグメントも同じ保存形式で継続する。形式変換に失敗した場合は無音を補わず取得失敗として扱う。固定機器への切替要求を受理しても実機器がまだ変わっていない間は、旧機器を新しい固定機器として表示せず「切替待ち」とする。5秒テストは現行仕様ではOS既定入力だけを使うため、自動モード中でも会議アプリ固有のマイク検証にはならないことを画面に明示した。

自動→固定→自動とレート変更を含む合成テスト、要求所有・単調版、状態形式のテストを追加した。変更前の境界ではSwift全250テスト（3件スキップ）、Rust通常1,757件成功／0失敗／76件ignored（ネットワーク依存の49件はスキップ指定）、フロント全792件が通過した。その後の「切替待ち」表示修正と5秒テスト説明を含むフロント全793件・ビルド、Swift全250件（3件スキップ）、Rustの選択状態2件・入力一覧6件、およびSwift生成JSONをRustで読む1件が通過した。Rust用テストキャッシュは後処理で約9.06GiB、設定上限10GiB以内。実マイクや会議アプリの入力を使う試験は、静かな時間について別途合意するまで実施しない。

QA専用識別子 `jp.mon.desktop.qa` の更新版を `/private/tmp/mon-auto-microphone-qa-artifact.mBxnDj/MON QA.app` に保持した。構築元との全ファイル一致、アプリのdeep・strict署名、録音ヘルパーの同梱と録音しない `--help` 成功を確認。本体SHA-256は `4305c80cf2ea956393a11ad592f80a65a885ae4454cd0f667614efe04798b43d`、録音ヘルパーは `7616982cf1441ddd27b8a0d31c9db142ce0706310345bd69e9b64050983d4d70`。約39MiB。署名はアドホックでありDeveloper ID署名・公証済み配布物ではない。構築専用の約1.2GiB一時キャッシュだけ削除し、QAアプリは保持した。実録音・実会議アプリとの機器切替・4時間安定性は未検証。
