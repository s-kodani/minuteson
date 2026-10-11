# 同梱文字起こし実行ファイルの署名経路

2026-10-11、基準 `4007692874b4f78a882269d6a9cff55386c82d58`。M4-02の配布準備として、コピーしたapp内のWhisper CLIをDeveloper IDで署名してからTARへ再封入する経路を追加した。証明書／公証profileは未準備であり、実Developer ID署名・公証・Gatekeeper・署名版の導入は未検証。CI・実録音・追加モデル取得・本番起動は行わない。

## 署名の順序と保持条件

従来はTAR内のCLIがアドホックのまま、外側のappだけを正式署名する経路だった。今回は固定3ファイル・全容量／SHAを照合した同じbytesから専用領域へ展開し、CLIのarm64・最低macOS 15・システム依存だけ・既存署名の整合を検査する。明示Developer ID／Team・timestamp・Hardened Runtime・追加例外なしで署名し、OSのApple anchor／証明書OID／Team要件と実entitlementsを検査した後に圧縮し直す。noticeとbuild情報の変更は拒否する。元の開発キャッシュは変更せず、コピーしたappの既知2資源だけを置換してからヘルパー／親を署名する。内側から署名する順序は[Apple TN2206](https://developer.apple.com/library/archive/technotes/tn2206/_index.html)に基づく。

最終appからもTARを展開し直してCLIの実署名を検査する。署名失敗・未知Team・アドホックへの後戻り・改変・権限の追加は成功にしない。JSONだけを署名証拠にせず、`asrRuntime.developerIdSigning`は明示Teamで実検査を通ったときだけ`verified`となる。外側のticketからCLI単体の公証やGatekeeper／導入成功は推測しない。

ネイティブ導入は既存の`ad_hoc_only`と、新しい`developer_id_signed`＋10桁英大文字／数字の`signingTeamId`を受け付ける。null・欠落／余分Team・未知状態は拒否する。署名情報は配置情報であり、公開元真正性・ライセンス確認・準備完了は未検証のまま。固定出典・CPU構成・全SHA・レビュー時の取得内容・コピー・CPU確認・明示適用の制約は維持する。署名でCLIのbytesが変われば取得内容のhashも変わり、古いレビューを流用できない。

## 回帰と実素材

Pythonの合成署名はCLI bytesを変更するstand-inで、実証明書・実公証を使わない。再封入の順序、元packageの保持、署名後SHAの更新、notice／出典保持、失敗時の親署名／送信拒否、JSONだけでの自己承認拒否、CLIへのlibrary-validation例外拒否を確認した。展開は固定3パスだけ・create-new・全検査後で、traversal／symlink／容量超過／改変を拒否する。Rustは新形式の読み取り、準備状態非昇格、取得内容の再照合と改変／link拒否を検査した。

変更後の関連Rust8件は成功。拡大した`model_`検査の初回は244成功・8失敗・8ignoredで、失敗8件すべて`TcpListener::bind("127.0.0.1:0")`の権限制限だった。同じ検査を必要な権限で再実行し252成功・0失敗・8ignored。外部取得や録音を行わず、ignoredの専用実モデル試験を通常成功へ含めない。通常Rust全体は今回再実行していない。

既存の実アドホックpackageをscratchへ展開し、実lipo／otool／codesign strictで整合を確認。CLIは実行せず、検査前後のindexは一致。TAR 2,478,080bytes、SHA `4a5968c63b56993be5ffc263844e03833742290b46bfcb7fb92298bc74d5493c`。CLI SHA `bb8a270033f83570737f5644dc08913f294b82d603d5ab88d12215c026a38a15`。検査0.169秒。

新QA `/private/tmp/mon-asr-signing-qa.E4zrL5/MON QA.app` は `jp.mon.desktop.qa`、39.18MiB、main SHA `64a9c0279f7c058117192b12b5fb08b9d85e27b59269988cabb73aa815210357`。deep／strictアドホック検査成功。実同梱TARは上記と一致、依存通知を現在のoffline入力から再収集して278依存・不足12件・未承認一覧の一致を確認した。この読取り照合は1.637秒。GUI起動・モデル導入・CPU試行・会議DBの検査／変更は今回行わない。

## 時間と残件

実装・調査・回帰／記録は約20〜30分の概算。QAのVite1.86秒、Cargo1分20秒、全体は個別未計測（ログ生成時刻差は約92秒の参考値）。既存targetを再利用し非増分、専用target2,215MiB／保持QA約40MiB。ソナーdirective／chunkサイズ・未公証・未承認noticeのwarningは残す。

初回関連Rustの構築99.30秒・runner実行区間7.86秒（libtest本体0.03秒）。拡大初回は構築0.57秒・実行102.16秒（本体102.13秒）で上記8失敗。再実行は構築55.57秒・runner区間93.53秒（本体86.20秒）。runner区間には容量管理も含む。初回4,153.5MiB、再実行4,123.7MiBの再生成可能cacheを回収し、最終8,268.9MiB／上限10,240MiB。上限は実行後の管理目安で、ビルド中の一時使用量や別のrelease targetを含む全ディスク上限ではない。元モデル・音声・会議・QAは削除しない。

フロント854成功・0失敗・0skip、本体3.189890042秒。最終Python163件中162成功・専用1skip、本体10.987秒、全体11.11秒。途中Python全体も163件中162成功／1skip（12.949秒）、関連48件と追加回帰の反復も成功し、これらを最終成功件数に重複加算しない。上記の並行工程を単純合算して経過時間とはしない。ログは `/private/tmp/mon-asr-signing-python-final-20261011.log`、`mon-asr-signing-rust-20261011.log`（初回失敗）、`mon-asr-signing-rust-retry-20261011.log`、`mon-asr-signing-frontend-20261011.log`、`mon-asr-signing-qa-build-20261011.log`。

| 終端ログ | SHA-256 |
|---|---|
| mon-asr-signing-python-final-20261011.log | 85640d389c2e415d87a455f2fcb0965f5ccc75f4fb10b9b16a6665ebbd3498ac |
| mon-asr-signing-rust-20261011.log | 5f5718aab94ee171d8ab662c2de161dd4c77fe233024f7eb30a95e98f4547370 |
| mon-asr-signing-rust-retry-20261011.log | 2864e0c35a1a3d74535ddf66472ed512196aef66da111a2e70047bfaa007fdc2 |
| mon-asr-signing-frontend-20261011.log | ac367a09c21714ba7101669ac9b01e44eddf4d29294a7680fd0ca43c5c91a213 |
| mon-asr-signing-qa-build-20261011.log | ab0cb2a00b17c88a173f5ad0162934080075bc8746344cec4af25037d3b62922 |

全23タスクの[監査](implementation-completion-audit-2026-10-11.md)を維持。暫定残工数12〜29時間以上は、非録音の未網羅UI／編集／異常経路4〜8時間、現在実施しない実録音／耐久5〜8時間、配布／更新／試用3〜13時間。証明書等の外部準備待ち、正式評価とWindows保留分は別。内包CLIの公証／Gatekeeper・署名版導入、配布条件不足12件／モデル、更新と実会議試用を残し、M4-02や目標全体の完了にはしない。
