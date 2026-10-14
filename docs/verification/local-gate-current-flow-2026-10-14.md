# 現行フローのローカル統合ゲート

2026-10-14。実録音、OS権限変更、使用中の会議・モデル設定変更、CIの実行はしていない。

`make check` は経過14分14.79秒で終了2。Python単体165件（明示skip1件）、Swift XCTest240件（237成功・3 skip）とSwift Testing1件、CAF 4800/4800フレーム、フロントの型検査・本番ビルド・単体は通過した。通常Rustは1830件成功・84件ignore、モデル取得のローカルHTTP待受け8件だけがサンドボックスの `Operation not permitted` で失敗し、後続ターゲットへ進まなかった。`mon_eval.py validate` はエラー0、正式評価素材・端末等の保留による警告7件。Viteの大きなチャンクとRust未使用項目等の警告も残る。

待受け可能な環境で `python3 tools/mon_rust_tests.py model_download::tests` を再実行し、40件成功・8件ignore、終了0。先ほどの8件はすべて成功した。構築0.59秒、テスト本体3.11秒、コマンド経過4.003秒。CIを有効化した結果ではない。

未到達分を `make check-caf-playback check-capture-clock check-input-levels check-input-status check-input-history check-input-journal check-capture-test check-microphone-recovery check-system-recovery check-capture-ending check-speaker-embedding` として実行。最後の話者分離検査までのターゲットは通過したが、旧仕様の1件が `review` を期待して失敗し、全体は31.724秒・終了2。単独再実行でも同じ1件が失敗し、偶発的な成功として扱わなかった。診断で現行の自動フローは既存の文字起こしと人間の編集を守り、明示的な再生成なしでは `attention` に移ることを確認した。このテストの名称・期待状態・理由文を現行の仕様に合わせ、既存本文と下書きが不変である確認を維持した。製品コードは変更していない。

最終 `make check-speaker-embedding` は終了0。実ヘルパーとfake C APIの単体、専用子プロセス1件、保存済み話者分離6件が成功した。Rustテストプロファイルの再構築459.29秒、テスト本体は子2.05秒と保存済み6件5.96秒、コマンド経過7分59.10秒。再生成可能なキャッシュ4561.1MiBを回収し、終了時9902.8MiB（上限10240MiB）。この再構築時間は実装時間ではない。

直前の同日 `make check-recording-raw-prefix` は7件・終了0で、今回の唯一のソース変更は話者分離のignoreテストの期待値だけ。したがって各ゲートの通過証拠は揃うが、単一の `make check` が終了0になったとは記録しない。ローカルHTTP待受けが禁止されるサンドボックスでも通るようにテストの意味を弱めたり、保留の評価警告を成功へ変換したりしない。実マイク・PC音声、4時間録音、電源断、同期精度、配布署名・公証、正式品質は未検証のまま。
