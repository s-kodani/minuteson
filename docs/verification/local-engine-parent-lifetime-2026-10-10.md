# Mac解析プロセスの親寿命と所有付き終了

- 対象: M2-03の強制終了時の解析子プロセス対策、M4-01のMac統合準備
- 更新日: 2026-10-10
- 状態: 製品経路・通常全件・専用連携・製品headless入口の検査済み。実モデル・製品GUIの強制終了は未検証

## 未対応だった経路

従来の`ManagedChild`は通常の取消・timeout・エラー時にRustの`Drop`で子へkillとwaitを行っていた。しかし親アプリがSIGKILLされた場合はDropが走らず、whisper.cpp・sherpa-onnx・特徴量抽出・llama.cppが親の後に残る可能性があった。録音ヘルパーと音声取り込みの親PID監視とは別の未対応である。

4つのローカル推論経路を`OwnedCommand`へ接続した。各起動で匿名Unix stream pairを作り、親側の片端を`ManagedChild`だけが保持する。専用の受信端を継承して同じMON実行ファイルの私有headless入口を起動する。親の端はCLOEXECで他のプロセスへ渡さず、headless側もネイティブエンジンへ端を再継承しない。通常Dropで閉じた場合も、親のSIGKILLでdescriptorが閉じた場合も、EOFを停止条件にできる。

headless入口はTauriのBuilder・保存先・モデル探索・GUIを初期化する前に分岐する。私有protocol、直近の親PID、継承descriptor、socket種別と匿名Unix pair、絶対エンジンパスを検査する。通常のユーザー操作・IPCに新しい任意実行コマンドは追加しない。所有付き起動の入力条件、モデル／エンジンの既存SHA照合、環境の除外、作業場所、argv、標準出力／標準エラー、終了成功後の原本／結果照合は維持する。私有入口はモデルをダウンロード・選択・適用する権限ではない。

headlessプロセスがエンジンを新しいprocess groupへ起動して実Childを保持する。EOF・予期しない所有通信・エラーでは、その起動したgroupだけを停止して直接子をwaitする。エンジンが先に正常終了した場合も、`waitid(WNOWAIT)`で終了を見た後、まだPIDを回収していない間に同じgroupの残りを停止し、それからwaitして終了コードを返す。wait／try_waitでPIDを解放した後のgroup killは行わない。再起動時の台帳、保存したPID、プロセス名の検索から所有を推測しない。

Darwinはzombieだけのprocess groupでも`kill`にEPERMを返すことがある。[Apple XNUのkillpg1](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sig.c)ではgroupの列挙からSZOMBを除外し、対象数0をEPERMへ変換する。今回は同じ所有する合成子の正常終了後に、sandbox内・外の両方でこの挙動を再現した。EPERMを無条件に成功へ変換しない。未回収Childの終了をWNOWAITで確認した場合だけ、同じgroupをPROC_PGRP_ONLYで最大4,096件の固定bufferへ読み、未回収leaderを含め生存memberがいないことを確認する。[Appleのproc_info](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/proc_info.c)に従いBSD infoのarg=1でzombieも読む。全体を読めない場合、leaderが一致しない場合、生存memberがある場合は停止失敗のままにする。ここで得たPIDへの個別signal・名前検索・台帳からの採用は行わない。

親側の通常取消は通信端を閉じ、headless側の終了をwaitする。headlessを先にkillしてその子の後始末を失う手順にしない。guardの終了失敗を推論の成功にしない。未確定の既存処理単位・結果は既存のジョブ回復に任せ、音声／結果／ジョブをこの監視処理が削除・自動採用しない。

対象はMONが起動したローカル推論と同じprocess group内の子である。自ら別session／groupへ離脱したエンジン、監視プロセス自体のSIGKILL、OS停止／電源断後の処理や、任意の他アプリの終了を保証しない。名前で周囲をkillしてこの範囲を広げない。Windowsの実装・CIは保留のままであり、非Macの既存直接起動・Dropの意味は変えない。取り込みヘルパーの既存の親PID protocolは維持した。

Rustの[process_groupとpre_execの説明](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html)に沿い、fork後はdescriptor操作だけを行い、解析や割当てはexec後に実行する。[POSIXの状態保持](https://pubs.opengroup.org/onlinepubs/9799919799/functions/V2_chap02.html)ではWNOWAITの取得が状態を消費しない。実装では対象Mac SDKの`sys/wait.h`とlibcの定義も確認した。API説明だけを実モデル／実機GUIの動作証拠にはしない。

## 検査と失敗

私有headless coreは製品と同じソースを、テスト専用の小さい実行ファイルにも組み込む。テストハーネスのstdout bannerで推論JSONを汚さず、実デバイスやGUIを初期化しないためである。追加の依存パッケージは取得せず、現在のRust検査で構築済みのlibcを使う。テスト専用実行ファイルの内部構築はRustラッパー実行時間の内数であり、純粋な検査本体とは区別する。

先行構築でOsString／Stringの比較、PID parseの型注釈、既存sherpaテストのCommand import不備がそれぞれ発生した。3回とも検査実行前の構築失敗で、17.91＋14.07＋14.07＝46.05秒を参考時間に含める。書式コマンドも先行のunsafe式の括弧不足で1回失敗し、修正後に再確認する。拒否条件や既存テストの期待値を緩めない。

先行Rustの重点検査は構築361.30秒・実行8.27秒で4件成功・1件失敗・1件ignoredとなった。失敗は正常終了のcode27がEPERM由来の125となったもの。Nodeのnative検査もdescriptor99を含むstdio指定でexit134・ログ空となり、モデルを使わない最小の/usr/bin/true起動でも再現した。私有継承端を3にし、同じ匿名Unix streamの検査を維持した。最初のgroup確認ではBSD infoのarg=0がzombieを返さず、正常終了検査がまだ失敗した。arg=1で本当のzombie情報を確認する修正後、製品と同じcoreの小型実行ファイルで6ケースが成功した。先行失敗を最終QAの成功に混ぜない。

最初の通常全件は1,798件成功・8件失敗・75件ignored、構築57.01秒・実行812.46秒でexit101。失敗8件は全て`model_download/tests.rs:790`のlocalhost bindでEPERMとなった。所有付き終了の追加5件、whisper／sherpa／llamaの既存取消・timeout等はその実行でも成功した。sandboxを外して同じソースを4プロセス×16スレッドで再検査すると、最初の4 batch・256件で既存capture journalの所有未確認拒否が1件発生し、後続batchはラッパーの規則で実行されなかった（245件成功・1件失敗・10件ignored）。構築41.05秒・実行115.29秒でexit101。取得leaseのflockでEAGAINとなる拒否を緩めず、並列負荷との因果は未分離と記録する。4分割を既定へ昇格せず、同じ許可環境で既定の1プロセス×16スレッドによる全件を再確認した。

同梱のrelease特徴量ヘルパーと新規の合成C ABIを使い、通常全件ではignoredとなる専用検査も実施した。親runnerの実出力・改変・取消・timeout・overflow・失敗・子の回収1件と、保存済み特徴量を使うworkerの保持・停止／再開・再起動・追加復元・重複・自動handoff6件が成功した。既存の音声・編集・特徴量証拠を検査し、実モデル・実会議は使わない。

最終の通常全件は1,806件成功・失敗0件・75件ignored、main／docはそれぞれ0件でexit0。新規通常5件と専用補助1件を追加した。最後の通常全件ではlocalhostの8件と、4分割時に失敗した所有ロックの1件も成功した。製品や既存検査の拒否条件は変更していない。追加5件は全体の内数、上記の専用7件は通常のignoredを明示実行した別枠。フロント全件・全ブラウザ・Swift全件は今回再実行していない。QAの型検査／Vite構築は含むがGUI検査ではない。以下を実モデル・GUIの証拠と混同しない。

- native engineのstdout／stderrと終了コード・除外後の明示環境を正確に渡すこと。
- 所有通信の閉鎖で本当にengineと同じgroupの子が終了し、直接子をwaitすること。
- native engineの通常終了後もgroup内の残りを停止し、正常終了コードを維持すること。
- 継承端や親のない私有入口がnative engineを起動しないこと。
- 実テスト子の親へSIGKILLし、guard・engine・同じgroupの子の終了と、無関係な所有する対照プロセス・確認マーカーの保持を検査すること。
- 4つの既存推論ワーカー、録音優先、取消・timeout・結果照合の回帰検査。
- QAへ同じ製品入口を構築し、GUIを起動せずに私有headless経路を確認すること。

ログは`/private/tmp/mon-owned-engine-`接頭辞のfocused／全体／QA記録へ保持する。実モデルのSIGKILL・停止待ち時間・Mac WebViewの実録音優先・アプリ強制終了・4時間録音・正式評価の代替にはしない。

## 同じ製品入口を含むQA

資格情報を除外して新しい専用targetへQAを構築した。全コピー一致を確認後、`/private/tmp/mon-owned-engine-qa-artifact.OwfWTg/MON QA.app`を保持した。同じmainの私有headless入口で上記6ケースが成功した。検査用Nodeツールは起動前に既知バイナリのSHAと私有入口の存在を確認する。不一致SHAはspawn前に拒否する。古いQAを通常GUIとして起動してしまう確認手順を使わない。ツールの最終SHA確認追加後にも、正しいQAの6ケースと不一致SHAの拒否を再確認した。

deep／strict署名、アドホック署名、`jp.mon.desktop.qa`、最低OS15.0、全3実行ファイルのarm64、取得ヘルパーの識別子・音声用途宣言と同梱CLIのhelpを確認した。GUI・TCC・実デバイス・実モデルは起動していない。Developer ID署名・公証・Gatekeeper・実機のWebView操作の証拠ではない。

| 同梱実行ファイル | SHA-256 |
| --- | --- |
| mon-desktop | `a52055b689db09faafed8e7f9b42aaab54ca1e488aad28f3e6348ad1907d0cb2` |
| mon-macos-capture | `cf461e57b46cbdf06c526cf8a2a2ea310add1bcbaa58c27ccf04835d23c79920` |
| mon-speaker-embedding | `d9b217f5ebaced3957ebf77b3e35874fc21be06cf474eab0f550c313a21ba8a9` |

QA35,072KiB（約34.25MiB）を保持し、終了済みの新規専用target`/private/tmp/mon-owned-engine-qa-build.2jtB41`、1,238,384KiB（約1.18GiB）を回収した。生成時と同じdevice16777234／inode169709888／uid501／0700・非リンクを確認し、descriptor相対の安全な削除を使った。再生成可能な構築物だけで、既存QA／DMG・会議・音声・モデル・設定は削除していない。共有Rustは検査中の増分世代で一時的に約14.35GiBとなった。終了後は10,136.1MiB（約9.90GiB）、製品target38,664KiB（約37.76MiB）、取得Swift350,120KiB（約341.91MiB）。8〜10GiBは終了後の共有cache目安であり、構築中や全cacheのhard capではない。終了前にそのcacheへ手動削除しない。Rustラッパーが終了後に不要な増分世代だけを回収した。ユーザーのVite等は操作していない。

Rust／core／専用補助／4ワーカー／取り込み／lib入口の10ファイルは最終全件とQAの前後でSHA一致を確認した。Node検査ツールのSHA事前確認追加は別の検査用変更として固定し、最終製品入口6ケースで再確認した。最終11ファイルの記録は`/private/tmp/mon-owned-engine-source-v2.sha256`。小型coreや合成ABI・ログは診断用に保持する。

## 今回のcommit・pushに対する時間

| 工程 | 累積参考時間 | 範囲 |
| --- | --- | --- |
| 実装・調査・検査追加／修正・記録 | 約30〜45分（概算） | 専用ストップウォッチではない。全待機時間を能動工数へ加算しない |
| Rustラッパー構築 | 506.35秒（8分26.35秒） | 先行17.91＋14.07＋14.07＋361.30、全件57.01＋41.05＋0.23、専用0.47＋0.24。失敗も含む |
| 直接Rust構築 | 1.17秒 | stdio診断0.14、group診断0.14、core0.34＋0.35、合成ABI0.20。ラッパー内部のcore構築は別加算しない |
| QA構築コマンド | 198.71秒（3分18.71秒） | 型検査・Vite・Swift release・Rust release・バンドルと署名を内包 |
| 構築参考小計 | 706.23秒（11分46.23秒） | 先行の構築失敗・再試行を含む |
| Rustラッパー実行 | 1,711.41秒（28分31.41秒） | 重点8.27、通常全件812.46、4分割の先行部分115.29、最終全件767.35、専用2.00＋6.04。一覧・内部core構築・doc内部構築を含み純本体ではない |
| 直接診断／nativeと署名等 | 2.68秒 | stdio0.32＋0.04、group0.31、core失敗0.37＋成功0.45、QA native0.78＋0.31、期待した不一致SHA拒否0.06、署名0.03、help0.01 |
| 検査参考小計 | 1,714.09秒（28分34.09秒） | 失敗と再試行を含む。期待した拒否のexit1は成功条件である |

最後の通常全件は構築0.23秒・実行767.35秒（12分47.35秒）、libtest本体765.32秒。最終再利用時と新しいQAの専用構築を区別する。Node descriptor99の先行2回・最小診断、localhost bind診断、書式／構文、copy・plist・lipo・SHA・disk・Gitは個別未計測。待受け制限と4分割時の拒否を含む先行失敗を記録し、検査期待値は緩めない。QA構築と最初の全件実行は並行、記録作業も一部並行した。共有cacheロックの待ち時間と内部未分離を区別し、工程の和を実経過時間へ単純加算しない。

主なログは`mon-owned-engine-focused-v1`〜`v4`、`mon-owned-engine-full-v1`、`full-v2-escalated`、`full-v3-escalated`、`embedding-parent-v1`、`embedding-known-v1`、`qa-build-v1`、`qa-native-v1`／`v2`の各`.log`で、すべて`/private/tmp/`配下に保持した。

## 残対応

計画全体とM2-03・M4-01は進行中。入力形式・容量等の録音境界0.5〜1.5時間、旧形式／異常UIと実モデル3.5〜7.5時間、実配布元／ライセンス等0.5〜1.5時間、配布／公証／更新1.5〜3.5時間、Mac実機／長時間4〜8時間以上、先行試用1〜3時間以上。これらの暫定能動工数は維持する。今回4分割で見つかった既存所有ロックの拒否原因と速度効果は未分離なので、検査の並列安定性・効果確認0.5〜1.5時間を追加残件として明示する。既定1プロセスで全件成功しても4分割成功とはしない。項目別の合算は暫定11.5〜27時間以上。外部モデル・実機操作・署名資格情報・協力者等の準備待ちは別。正式評価・10話者・Windows固有部分とWindows CIは保留。
