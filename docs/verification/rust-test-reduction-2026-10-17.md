# Rustテストの重複実行削減

## 方針と変更範囲

Rustテストの実行時間を優先し、同じ準備・削除・保存先移動を繰り返す検査を統合する。製品コード、公開API、IPC、DB形式、テスト用Cargoプロファイル、通常ランナー、並列数、ignored指定は変更しない。別の障害境界、実SIGKILL、Swift/Rustの連携は削除しない。

開始コミットは `ff4c2ae`。開始時の作業ツリーはクリーンだった。ソース上の `#[test]` と変更前のlibtest列挙はいずれも2,018件。変更後のソース上の件数は2,012件で、6件の登録を減らす。テスト名をまとめただけでなく、不要な準備・成功時の完了処理を取り除く。

## 削除したケースと残る検証

共通接頭辞を省いた関数名を示す。統合先の実装は元の各検証を実行する。

| 旧ケース | 統合先 | 維持する検証 |
| --- | --- | --- |
| `file_removal_completed_receipt_refuses_recreated_meeting_tree` | `file_removal_erases_fixed_tree_replays_and_refuses_recreated_content` | 完了した同じreceiptで再生成されたファイルを消さず、無関係なデータを保持する |
| `compound_deletion_requires_completed_file_phase_and_keeps_all_rows_when_refused` | `compound_deletion_requires_files_then_purges_fixed_rows_replays_and_refuses_recreated_content` | ファイル処理前のDB削除拒否、会議・音声の保持 |
| `compound_deletion_finished_retry_refuses_recreated_target_without_erasing_it` | 同上 | 完了後の再生成ファイルを拒否して保持する |
| `compound_deletion_start_acknowledgement_replays_the_same_progress_without_reconfirming_or_redeleting` | 同上 | 開始応答の再取得時の進捗一致、完了後の応答再取得 |
| `deletion_startup_admission_kept_audio_remains_present_and_old_consent_cannot_resume` | `deletion_repeat_moves_kept_audio_and_whole_meeting_never_become_current_consent` | 初回移動後と後続の各起動で、保持音声の存在、旧同意の拒否、元記録の不変性と通常書込みを確認 |
| `deletion_startup_admission_kept_whole_meeting_is_readable_without_current_root_consent` | 同上 | 会議の読込み、保持音声、旧会議削除の拒否と同意記録の不変性を確認 |

ファイル削除と複合削除は「拒否 → 実行 → 応答再取得 → 再生成ファイルの拒否」の順に一つの実DB・実ファイルのシナリオで検査する。保持音声／保持会議の検証は繰返し移動の前後に実行する。初回起動の検証を最終移動先だけの検証で置き換えない。保持会議については、初回admission後に同じ移動先をもう一度cold openする元の検証も維持する。

## ロールバック検査の準備の重複削減

同じテスト内で、各失敗後に元のDB状態・進捗が保持されることを確認してから、次の障害境界を検査する。成功時の承認・完了処理は最後に一度実行する。並列テスト間で可変fixtureを共有しない。

| 対象 | 維持する条件 | fixture作成回数の前後 |
| --- | --- | --- |
| 音声削除の範囲再確認 | audit保存・proof保存・commit直前の取消 | 3 → 1 |
| モデル削除の範囲再確認 | audit保存・proof保存・audit完了・commit直前の取消 | 4 → 1 |
| 会議削除の範囲再確認 | 4つの保存境界 × 取消／panic | 8 → 1 |
| 音声削除の完了 | asset・chunk・track更新とcommit直前の取消 | 4 → 1 |
| 複合削除のDB反映 | 行削除後とcommit直前の取消 | 2 → 1 |
| 保存先移動後のモデル削除の範囲再確認 | 4つの取消境界と各境界後のcold reopen | 4 → 1 |

25通りの障害条件をすべて維持する。各callbackの到達を明示確認し、単に別の理由でエラーになっただけでは成功にしない。会議削除ではpanic後のmutex poisonを解除してからDBの不変性を確認する。音声完了と移動後モデルの検査には、元の証拠・進捗との比較を追加した。

上記の準備回数は25回から6回へ減る。ケース統合による6回分と合わせ、25回分の独立したfixture作成を除く。これはDB openやSQL実行全体の回数ではない。保存先移動後の各取消に続く再起動検証は残す。

## 測定方法

通常ビルドの実行ファイルを共有キャッシュの排他ロック下で実行する。一時的な計測スクリプトだけで、実行時に `RUSTC_BOOTSTRAP=1` とlibtestの `-Z unstable-options --report-time` を指定する。計測用環境変数をCargoへ渡さず、製品やテストのコンパイル設定を変えない。

全件は変更前後とも1プロセス・10スレッド、対象ケースの比較は1プロセス・1スレッドでそれぞれ3回行い、中央値を比較する。対象は変更前15件、変更後9件で、残る検証内容を対応させる。全件中のケース別時間には並列処理の待ち時間が含まれるため、合計値を全体の実経過時間として扱わない。

最初の通常全対象ビルドは74.16秒で成功した。その後、誤って計測用環境変数をCargoにも渡したため不要な再構築が発生し、今回起動した構築プロセスを明示停止した。この中断は検査成功にも実行時間にも含めない。構築済みの変更前実行ファイルを保持し、以降は実行時だけ計測を有効にする方式へ修正した。

## 結果

変更前の通常ライブラリ全件は1,930件成功、失敗0件、既存専用88件ignored。実経過は1,993.94秒（33分13.94秒）。変更前実行ファイルのSHA-256は `eda076f2ec766a9ce89516eb9962f387645b5cd0dbc1181e523c7ce42e87d978`。

対象15件の変更前3回は37.922秒、40.240秒、33.950秒で、中央値は37.922秒。各回15件成功、失敗0件。全件の並列中に記録した各ケースの時間を、この単独比較の値と混ぜない。

RustランナーのPython検査26件は成功した（0.164秒）。

変更後の対象9件は19.626秒、19.108秒、17.748秒で、各回9件成功、失敗0件。中央値は19.108秒で、変更前の37.922秒から49.6%短縮した。この率は対応する対象ケース群の比較であり、全Rust検査の短縮率ではない。

取消時にauditだけを誤ってcommitする一時的な変異を`renewal.rs`へ入れたところ、統合後の会議削除再確認テストが`audit_count`の比較で失敗した（実際1行、期待0行）。変異版は0件成功・1件失敗。元の製品ソースをバイト単位で復元して全対象を再ビルドし、その版で上記9件×3回が成功した。最終差分に製品ソースの変更はない。

変更後の最初の構築は、bootstrap側テストから非公開DBフィールドを直接参照したため失敗した。既存のテスト用readerを使う形へ修正した後の全対象構築は733.34秒で成功した。変異版の構築は100.64秒、復元後の全対象構築は97.62秒で、これらをテスト実行の短縮時間に含めない。計測に使った最終実行ファイルのSHA-256は `00409d8fac4fa108a3d647a884ffbe1207b25e2a57a2c3482239a71caf504c84`。通常キャッシュ内の全件用と一時コピーの対象比較用は同じSHA-256である。

変更後の通常ライブラリ全件は1,924件成功、失敗0件、専用88件ignored。列挙2,012件と一致し、フィルター対象外は0件。全件実経過は1,758.36秒（29分18.36秒）で、変更前から235.59秒（3分55.59秒）、11.8%短縮した。

| 比較 | 変更前 | 変更後 | 差 |
| --- | --- | --- | --- |
| ライブラリの登録件数 | 2,018件 | 2,012件 | 重複6件を統合 |
| 対象群の独立したfixture作成 | 35回 | 10回 | 25回分を削減 |
| 対象群の3回中央値・1スレッド | 37.922秒（15件） | 19.108秒（9件） | 49.6%短縮 |
| 全件実行・10スレッド | 1,993.94秒 | 1,758.36秒 | 11.8%短縮 |

binとdoc testはそれぞれ0件で成功。RustランナーのPython26件、`cargo fmt --check`、`git diff --check`も成功した。全件中にも変更した9件すべての成功を確認した。通常ランナーの対象範囲・標準並列設定・専用テストの登録は変更していない。

全件時間は同じMacで各1回、順次実行した結果で、OSキャッシュや外部負荷の変動を完全には制御していない。一般的な短縮率や他環境の速度保証にはしない。専用88件の単独実行、実音声・実モデルの品質評価、Windows実行を今回の全件結果に含めない。親テストから起動される子プロセスの検証は親の成功に含む。

## 証跡

計測スクリプト、対象名のJSON、変更前後の全件列挙は `/private/tmp/mon-reduction-` 接頭辞で保持する。計測スクリプトはビルドを行わず、実行時だけ時間表示を有効にし、共有キャッシュの所有付きロックを保持して子へ引き継ぐ。

- 全件: `/private/tmp/mon-reduction-before-tests.log`、`/private/tmp/mon-reduction-after-tests.log`
- 対象3回: `/private/tmp/mon-reduction-focused-before.log`、`/private/tmp/mon-reduction-focused-after.log`
- 構築: `/private/tmp/mon-reduction-build-before.log`、`/private/tmp/mon-reduction-build-after-fixed.log`、`/private/tmp/mon-reduction-build-restored.log`
- 変異の検出と復元: `/private/tmp/mon-reduction-mutant.log`、`/private/tmp/mon-reduction-mutant-driver.log`
- その他: `/private/tmp/mon-reduction-bins.log`、`/private/tmp/mon-reduction-doc.log`、`/private/tmp/mon-reduction-runner-python.log`、`/private/tmp/mon-reduction-fmt.log`、`/private/tmp/mon-reduction-whitespace.log`
