# アプリ内通知の全発火元と整理方針

2026-10-27。Windows／macOS共通の通知表示、OS固有の発火経路、画面内の警告説明を調査。通知部品の呼出し・直接発行・画面内alertの全箇所は notification-inventory-2026-10-27.json に、変更前の条件・文言のソース式とともに保存した。重複する呼出しと共通転送部品も含めている。動的な会議ID・入力レーン・ジョブ・操作sourceは実行時の値に展開される。

通知発行の参照 106 箇所、画面内alert 19 箇所。JSONの行番号は変更前HEADの位置。

## 判断基準

自動再確認だけの監視・診断は通知を発行しない。通知に再読込や会議を開くボタンが存在するだけでは維持理由にしない。入力・権限・対象・保存の修正、再試行、音声確認、再起動につながる通知は残す。成功・情報通知は維持。通知履歴はメモリ内でありDBの移行はない。

録音関連の解消表示は廃止。確定した失敗は確認証拠が届くまで保持し、確認後はforget(key)で表示・履歴のみを削除する。画面遷移・停止・タイムアウトだけで解消扱いにせず、表示済み・閉じたというネイティブ応答も発行しない。

## 発火元別一覧

| 発火元 | 通知ID／対象 | 判断と次の操作 |
| --- | --- | --- |
| analysis-publication.ts:50 | toasts.show | 維持：会議を開いて生成結果と保存状態を確認 |
| analysis.tsx:144 | "analysis-operation" | 変更：定期読取失敗は状態表示のみ。ユーザー操作・ネイティブ解析失敗は再生成・再開・設定修正のため維持 |
| archived-input-notices.tsx:24 | {input} | 維持：実際の入力中断・復帰の履歴：会議を開き保存音声と欠損を確認 |
| archived-input-notices.tsx:122 | "read" | 変更：自動読取り失敗は削除。自動再確認を停止する時計異常だけ再起動案内を維持 |
| archived-input-notices.tsx:123 | "listen" | 削除：購読・画面状態の自動再確認だけで利用者の対応不要 |
| archived-input-notices.tsx:125 | "diagnostic" | 変更：読取り不能・配信準備・確認待ちは削除。確定した保存失敗・不整合・記録欠落・保持損失は音声確認のため維持 |
| audio-deletion.tsx:150 | "deletion-result" | 維持：対象を再検査し削除を再確認・続行・中止 |
| audio-deletion.tsx:151 | "audio-deletion" | 維持：対象を再検査し削除を再確認・続行・中止 |
| audio-deletion.tsx:159 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| audio-deletion.tsx:160 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| audio-deletion.tsx:162 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| audio-deletion.tsx:164 | "AudioDeletionPanel-resume" | 維持：対象を再検査し削除を再確認・続行・中止 |
| audio-deletion.tsx:166 | "AudioDeletionPanel-paths" | 維持：対象を再検査し削除を再確認・続行・中止 |
| audio-import.tsx:137 | "result" | 維持：入力ファイルの選び直し・取り込み再試行・保存した会議を開く |
| audio-import.tsx:138 | "saved" | 維持：入力ファイルの選び直し・取り込み再試行・保存した会議を開く |
| audio-import.tsx:140 | "operation" | 維持：入力ファイルの選び直し・取り込み再試行・保存した会議を開く |
| capture-applications.tsx:48 | "application-ended" | 維持：取得対象を選び直し、OSの権限または出力機器を確認 |
| capture-applications.tsx:50 | "system-permission" | 維持：取得対象を選び直し、OSの権限または出力機器を確認 |
| capture-applications.tsx:51 | "CaptureApplicationSelection-operation" | 維持：取得対象を選び直し、OSの権限または出力機器を確認 |
| capture-inputs.tsx:51 | "missing-input" | 維持：OSのマイク権限・接続を変更し、機器選択または一覧を再確認 |
| capture-inputs.tsx:52 | "permission" | 維持：OSのマイク権限・接続を変更し、機器選択または一覧を再確認 |
| capture-inputs.tsx:54 | "CaptureInputSelection-operation" | 維持：OSのマイク権限・接続を変更し、機器選択または一覧を再確認 |
| capture-test-audio.tsx:81 | "CaptureTestAudio-operation" | 維持：テスト音声の再生を再試行 |
| capture-test.tsx:52 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| capture-test.tsx:59 | "cleanup" | 維持：テスト受付・結果を確認し、終了・一時音声の破棄を再確認 |
| capture-test.tsx:60 | "test-run" | 維持：テスト受付・結果を確認し、終了・一時音声の破棄を再確認 |
| capture-test.tsx:62 | "unknown-request" | 維持：テスト受付・結果を確認し、終了・一時音声の破棄を再確認 |
| capture-test.tsx:64 | "CaptureTestPanel-operation" | 維持：テスト受付・結果を確認し、終了・一時音声の破棄を再確認 |
| confirmation-body.tsx:11 | "ConfirmationBody-progress" | 維持：元の操作の状態確認・停止要求の明示的な再試行 |
| confirmation-body.tsx:12 | "ConfirmationBody-operation" | 維持：元の操作の状態確認・停止要求の明示的な再試行 |
| data-save.tsx:189 | "save-result" | 維持：空き容量・保存先・対象を確認し、再検証・続行・中止・再起動 |
| data-save.tsx:190 | "save-operation" | 維持：空き容量・保存先・対象を確認し、再検証・続行・中止・再起動 |
| data-save.tsx:198 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| data-save.tsx:199 | "storage-read" | 維持：空き容量・保存先・対象を確認し、再検証・続行・中止・再起動 |
| data-save.tsx:200 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| data-save.tsx:210 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| data-save.tsx:211 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| data-save.tsx:212 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| data-save.tsx:213 | "open-result" | 維持：空き容量・保存先・対象を確認し、再検証・続行・中止・再起動 |
| data-save.tsx:217 | "DataSavePanel-resume" | 維持：空き容量・保存先・対象を確認し、再検証・続行・中止・再起動 |
| data-save.tsx:217 | "move-capacity" | 維持：空き容量・保存先・対象を確認し、再検証・続行・中止・再起動 |
| data-save.tsx:219 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| data-save.tsx:219 | "restore-capacity" | 維持：空き容量・保存先・対象を確認し、再検証・続行・中止・再起動 |
| internal-logs.tsx:161 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| internal-logs.tsx:169 | "clipboard" | 維持：日時・絞り込みの修正、ログ表示の更新、コピーの再試行 |
| internal-logs.tsx:170 | "InternalLogs-operation" | 維持：日時・絞り込みの修正、ログ表示の更新、コピーの再試行 |
| internal-logs.tsx:171 | "InternalLogs-storage" | 通知削除：収集・保存の内部エラーはログ画面の状態として表示 |
| job-list.tsx:16 | "read" | 通知削除：自動読込失敗は一覧内の確認中表示へ変更。成功・空一覧を誤表示しない |
| job-notifications.tsx:184 | {item.meetingId} | 維持：該当会議を開き解析結果の確認・再生成、必要時は案内された再起動 |
| job-notifications.tsx:187 | {key} | 維持：該当会議を開き解析結果の確認・再生成、必要時は案内された再起動 |
| job-notifications.tsx:188 | "read" | 通知削除：自動読取・結果照合・購読失敗。定期読取と結果の成功確認は継続 |
| job-notifications.tsx:189 | "listen" | 通知削除：自動読取・結果照合・購読失敗。定期読取と結果の成功確認は継続 |
| job-notifications.tsx:190 | "worker" | 維持：該当会議を開き解析結果の確認・再生成、必要時は案内された再起動 |
| job-notifications.tsx:197 | {job.id} | 維持：該当会議を開き解析結果の確認・再生成、必要時は案内された再起動 |
| main.tsx:542 | toasts.show | 維持：会議・録音・保存の元の操作から再試行または対象を確認 |
| main.tsx:740 | "meeting-operation" | 維持：会議・録音・保存の元の操作から再試行または対象を確認 |
| main.tsx:741 | "recording-command" | 維持：会議・録音・保存の元の操作から再試行または対象を確認 |
| main.tsx:742 | "recording-confirmation" | 維持：会議・録音・保存の元の操作から再試行または対象を確認 |
| main.tsx:748 | "transcription-publication" | 維持：会議・録音・保存の元の操作から再試行または対象を確認 |
| main.tsx:749 | "recording-failure" | 変更：自動状態読取失敗は削除。ネイティブの録音・保存失敗は確認を案内 |
| main.tsx:753 | "meeting-operation" | 維持：会議・録音・保存の元の操作から再試行または対象を確認 |
| main.tsx:793 | "recording-command" | 維持：会議・録音・保存の元の操作から再試行または対象を確認 |
| main.tsx:793 | "recording-confirmation" | 維持：会議・録音・保存の元の操作から再試行または対象を確認 |
| main.tsx:793 | "meeting-operation" | 維持：会議・録音・保存の元の操作から再試行または対象を確認 |
| main.tsx:802 | "recording-recovered" | 維持：会議・録音・保存の元の操作から再試行または対象を確認 |
| main.tsx:803 | "meeting-operation" | 維持：会議・録音・保存の元の操作から再試行または対象を確認 |
| main.tsx:805 | "restore-origin" | 維持：会議・録音・保存の元の操作から再試行または対象を確認 |
| meeting-audio.tsx:67 | "meeting-audio-summary" | 維持：音声の再読込・選択・再生を再試行 |
| meeting-deletion.tsx:169 | "meeting-result" | 維持：対象を再検査し削除を再確認・続行・中止 |
| meeting-deletion.tsx:170 | "meeting-deletion" | 維持：対象を再検査し削除を再確認・続行・中止 |
| meeting-deletion.tsx:176 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| meeting-deletion.tsx:177 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| meeting-deletion.tsx:181 | "MeetingDeletionPanel-resume" | 維持：対象を再検査し削除を再確認・続行・中止 |
| meeting-deletion.tsx:182 | "MeetingDeletionPanel-paths" | 維持：対象を再検査し削除を再確認・続行・中止 |
| microphone-selection-status.tsx:25 | toasts.show | 維持：情報通知：自動選択されたマイクを確認 |
| minutes-markdown-editor.tsx:44 | "MinutesMarkdownEditor-operation" | 維持：保持された入力を修正・保存再試行 |
| minutes-markdown-editor.tsx:60 | "plain-minutes-save" | 維持：保持された入力を修正・保存再試行 |
| minutes-prompt-settings.tsx:83 | "preferences" | 維持：プロンプトを修正・再読込・保存再試行 |
| minutes.tsx:224 | "minutes-operation" | 維持：議事録の生成・書き出しを再試行、対象や保存先を確認 |
| minutes.tsx:225 | "plain-minutes-export-error" | 維持：議事録の生成・書き出しを再試行、対象や保存先を確認 |
| minutes.tsx:226 | "plain-minutes-export" | 維持：議事録の生成・書き出しを再試行、対象や保存先を確認 |
| model-deletion.tsx:167 | "result" | 維持：対象を再検査しモデル削除を再確認・続行・中止 |
| model-deletion.tsx:168 | "model-deletion" | 維持：対象を再検査しモデル削除を再確認・続行・中止 |
| model-deletion.tsx:186 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| model-deletion.tsx:188 | "ModelDeletionDialog-resume" | 維持：対象を再検査しモデル削除を再確認・続行・中止 |
| model-deletion.tsx:191 | "ModelDeletionDialog-paths" | 維持：対象を再検査しモデル削除を再確認・続行・中止 |
| model-inspection.tsx:252 | "モデルの準備" | 維持：モデル選択・準備・検証・設定の再試行 |
| model-issue.tsx:4 | "operation" | 共通部品：元の発火元に従う。通知内操作の失敗は元の操作を再確認 |
| recorder-info.tsx:56 | span | 変更：自動状態読取失敗は通知共有しない。ユーザーが開く・閉じる操作の失敗は再試行 |
| recorder-operation-notices.tsx:54 | p | 変更：theme_read/session_readは削除。ウィンドウ操作・停止確認・明示的な説明表示の失敗は元の操作を再試行 |
| recorder-operation-notices.tsx:67 | {e.source} | 変更：theme_read/session_readは削除。ウィンドウ操作・停止確認・明示的な説明表示の失敗は元の操作を再試行 |
| recorder-operation-notices.tsx:68 | "read" | 通知削除：通知自体の読込・削除受付の再照合は自動処理 |
| recording-input-status.tsx:79 | "journal-save" | 変更：待ち・読取り不能・更新遅延は削除。確定した保存失敗・不整合は音声確認を案内。確認済みなら履歴を無通知で削除 |
| recording-input-status.tsx:91 | {`${input}-status`} | 維持：機器・権限の確認、保存後の音声・欠損確認 |
| recording-input-status.tsx:91 | {`${input}-history-loss`} | 維持：機器・権限の確認、保存後の音声・欠損確認 |
| recording-recovery.tsx:11 | "read" | 変更：自動ポーリング失敗は削除。回復処理そのものの確定した失敗は維持 |
| recording-recovery.tsx:13 | "recording-recovery-result" | 維持：実際の回復成功・失敗。会議の保存状態と保持された元音声を確認 |
| recording-workspace.tsx:68 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| residency-notifications.tsx:22 | "residency-operation" | 変更：自動トレイ初期化失敗はログだけ。収納失敗・終了中止は画面を保持し元の操作や停止処理を確認 |
| residency-notifications.tsx:22 | "residency-monitor" | 削除：両OSの常駐監視は内部ログと自動再確認を継続 |
| speaker-completion-settings.tsx:57 | "settings" | 維持：設定値を修正し保存を再試行 |
| speaker-structure-dialog.tsx:39 | p | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| startup-attention.tsx:75 | "desktop-connection" | 維持：既存画面の表示・起動再試行、保存先やアクセス権を確認して再起動 |
| startup-attention.tsx:93 | "StartupAttention-operation" | 維持：既存画面の表示・起動再試行、保存先やアクセス権を確認して再起動 |
| startup-instance.tsx:42 | "StartupInstanceRecovery-operation" | 維持：起動の復旧操作を再確認・再試行 |
| startup-instance.tsx:48 | "StartupInstanceRecovery-operation" | 維持：起動の復旧操作を再確認・再試行 |
| theme.tsx:74 | "result" | 維持：明示的な設定保存・確認の完了通知 |
| theme.tsx:75 | "operation" | 変更：定期読込は削除。ユーザーが変更した設定の保存失敗・受付不明は保存再試行・結果確認を維持 |
| toast.tsx:33 | {type} | 共通部品：元の発火元に従う。通知内操作の失敗は元の操作を再確認 |
| toast.tsx:43 | {type} | 共通部品：元の発火元に従う。通知内操作の失敗は元の操作を再確認 |
| toast.tsx:66 | toasts.show | 共通部品：元の発火元に従う。通知内操作の失敗は元の操作を再確認 |
| toast.tsx:91 | toasts.show | 共通部品：元の発火元に従う。通知内操作の失敗は元の操作を再確認 |
| transcript-audio.tsx:245 | "playback-error" | 維持：音声の選択・再生を再試行 |
| transcript.tsx:343 | "transcript-consolidation" | 維持：発言の統合・文字起こし保存を修正・再試行 |
| transcript.tsx:345 | "transcript-save" | 維持：発言の統合・文字起こし保存を修正・再試行 |
| vad-settings.tsx:43 | "VAD設定" | 維持：VAD設定を修正し保存を再試行 |
| vocabulary.tsx:101 | "vocabulary-save" | 維持：空欄・重複の修正、保存再試行、競合時の再読込 |
| voice-matching-settings.tsx:57 | "settings" | 維持：設定値を修正し保存を再試行 |
| voice.tsx:108 | "VoicePanel-operation" | 維持：人物・声の候補選択、登録・名前変更・削除・照合の再試行 |
| voice.tsx:116 | div | 維持：入力修正・対象再確認・データ損失や未完了状態の判断に必要な画面内説明 |
| voice.tsx:129 | "VoicePanel-operation" | 維持：人物・声の候補選択、登録・名前変更・削除・照合の再試行 |
| voice.tsx:168 | "VoicePeopleSettings-operation" | 維持：人物・声の候補選択、登録・名前変更・削除・照合の再試行 |
| voice.tsx:172 | "VoicePeopleSettings-operation" | 維持：人物・声の候補選択、登録・名前変更・削除・照合の再試行 |
| voice.tsx:173 | "VoicePeopleSettings-operation" | 維持：人物・声の候補選択、登録・名前変更・削除・照合の再試行 |
| window-titlebar.tsx:72 | "WindowTitlebar-operation" | 維持：ウィンドウの元の操作を再試行（Windowsのみ） |

## OS固有経路

- macOS：マイク権限と「システムオーディオ録音のみ」の許可、対象アプリの終了・再起動による再選択案内を維持。Dock・メニューバー常駐監視は内部ログ／自動回復のみ。実際の収納・停止確認・説明画面操作の失敗を維持。
- Windows：マイクのプライバシー設定、サウンド出力機器・排他使用の確認案内を維持。ウィンドウタイトルバー操作の失敗を維持。トレイ監視はmacOSと同じ基準。
- 共通IPC・通知配信のDTOとネイティブの受領・表示・終了条件を維持。時計異常で自動再確認が停止した場合のみ再起動案内を表示。

## 検証結果

- フロントエンド：815件成功、1件skip。TypeScript型検査と製品ビルド成功。
- Rust：常駐監視の共有ポリシー3件成功。Rust書式検査成功。
- Python：Tauriコマンド・権限・Windows/macOSの構成整合6件成功。
- 合成IPCブラウザ：13スイート・127ケース成功。録音後の一時的な読取り失敗、自動再確認・復旧、再読込、別録音の失敗の分離、表示・終了応答を伴わない履歴削除を確認。
- Windows／macOSそれぞれのplatform値とnavigator分岐で、不要通知の抑制とマイク・PC音声のOS固有権限案内の維持を確認。960pxライト／360pxダークの通知履歴を目視確認。macOSネイティブ実行・実録音の検証とは区別する。
- 既存の配信キュー、世代交代、表示確認、解析結果の照合、保存失敗・再試行、録音停止確認、常駐・起動診断の回帰を確認。実録音・音声再生・利用者データの操作は行っていない。

主なログは `output/notification-cleanup-tests-final.log`、`notification-cleanup-frontend-build.log`、`notification-cleanup-native.log`、`notification-cleanup-os-browser.log`、`notification-cleanup-history-browser.log`、`notification-cleanup-regressions.log`、`notification-cleanup-errors-browser.log`。最終ブラウザ結果は `output/notification-cleanup-browser-results.json`。

最初のEdge起動はサンドボックス内で失敗し、合成IPCテストのみ制限外で再実行した。旧い検証用画面のレイアウトと遮蔽条件は、現在の共通通知表示・アイコンのみのミニパネルに合わせて更新した。初回の検証失敗は各ログに残し、修正後の成功と区別している。

## Windows QAインストーラー

- `npm.cmd --prefix app run tauri:windows:qa:build` 成功。ビルド用Node.jsとVisual Studio同梱CMakeのPATHを指定し、制限外で実行した。
- 配布物：`output/windows-qa/MON-QA-setup.exe`（未署名QA版）。生成日時：2026-10-27 12:51:03 JST。268,858,533 bytes。
- SHA-256：`3dd04f4caa013f6ca4c9566f95bd8e854758314a5dc63358c7fa72b20ff4eed1`。
- 録音ヘルパーのC++テスト6件成功（コンテナー、メタデータ、時計、キュー、入力準備、非同期取得）。
- ビルド前後のソース、本体、録音ヘルパー、NSISインストーラーを照合し、成功後に配布先を更新。さらに配布EXEを明示した検証で `currentSourceMatches: true`、`publishedInstallerVerified: true` を確認した。
- ビルド記録：`output/windows-qa-work/20261027T034237Z-3a03bb862b80448fb61cc53be6e89936-build.json`。
- 再照合：`output/windows-qa-work/notification-cleanup-published-verification.json`。ビルドログ：`output/windows-qa-work/notification-cleanup-build.log`。
- インストール・新版アプリのネイティブ起動・実録音は実施していない。`installedVerification: pending` はその区別を示す。macOSのネイティブビルド・実機検証も未実施。
