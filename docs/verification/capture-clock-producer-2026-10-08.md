# 録音取得時刻と保存フレームの接続

2026-10-08に、macOS録音ヘルパーの取得時刻・保存フレーム・共通UTC原点を、形式版2のmanifestと既存のDB確定処理へ接続した。Tauriが使う時間上限なしの録音では、両トラックの時刻情報が揃わない場合に保存完了を報告せず、元のCAFを保持する。同期精度の合格や実録音の長時間安定性を示すものではない。

## 取得時計と共通原点

マイクの `AVAudioTime.hostTime` とCore Audio Tapsの有効な `mHostTime` を使う。ScreenCaptureKitは `synchronizationClock` から `CMSyncConvertTime` でホスト時計へ変換する。コールバックの到着時刻や機器固有のサンプル番号を、二系統の共通時刻として保存しない。[AVAudioTime](https://developer.apple.com/documentation/avfaudio/avaudiotime)、[ScreenCaptureKitの同期時計](https://developer.apple.com/documentation/screencapturekit/scstream/synchronizationclock)

Machの単位換算は整数の全幅乗除算で行い、絶対時計を浮動小数へ変換しない。ホスト時計の前後の読込みでスリープを含む時計との対応を校正し、その対応とUTC基準を固定する。二系統で最初に保存したフレームを共通原点とし、相対時刻の補正区間とUTC原点・校正の不確かさだけを出力する。生の起動後時計値はmanifestへ出力しない。録音途中で壁時計が変更されても、確定済みのUTC基準へ付け替えない。

ホスト時計とスリープを含む時計の対応が変わった場合、取得時刻が校正前・現在より未来の場合、有効な取得時刻がない場合は補正の証拠を拒否する。時計変更後の原点を、遅延していた音声へ適用しない。現段階は時計変化を検知して保持する処理であり、明示スリープからの復帰と機器再接続は後続対応である。[Mach時計のスリープ時の性質](https://developer.apple.com/documentation/driverkit/mach_absolute_time)

## 保存したPCMとの対応

補正観測は書込みに成功したPCMにだけ登録する。マイクとCore Audio Tapsのサンプルカウンターは、連続性・飛び・逆行の検出に使い、共通原点には使わない。連続フレームの名目終了との差は時計ドリフトとして補正する。時刻だけを持つScreenCaptureKitでは、前バッファの名目終了から1ソースフレームを超える正の空白を、取得時刻の欠損として保持する。原因を機器切断やスリープと断定しない。

Core Audio Tapsはコールバックで変換した取得時刻をコピーしたPCMと一緒にキューへ渡し、実際のファイル書込み後にフレーム数と補正を更新する。ファイルが閉じた状態の任意書込みを成功扱いにしない。

ScreenCaptureKitの保存用サンプルは、保存済みフレーム数に従う連続時刻へ置き換える。PCM自体は変更せず、取得時刻の空白をAVAssetWriterが挿入する無音にしない。最後の保存フレームで書込みセッションを終了する。途中の形式変更や失敗した書込みは保存完了を拒否する。

## manifestと配布ヘルパー

Tauriの `--until-stopped` は形式版2を出力し、両トラックの全保存フレームを覆う `clockSegments` と `captureClock` を要求する。最大1024区間の生成と、manifest全体の256KiB上限を維持する。容量不足時に補正を切り捨てたり、形式版1へ自動的に戻したりしない。

時間指定の正式評価プローブは既存の形式版1を維持する。評価ハーネスの補正時刻対応は保留中の正式評価と分けて扱い、旧形式を同期確認済みにはしない。アプリとヘルパーは同じソースへ再ビルドする必要があり、以前のreleaseバイナリでは今回の接続を検証できない。

ヘルパーの `PrivacyInfo.xcprivacy` に、アプリ内イベント間の時間計測と絶対イベント時刻算出の理由を宣言し、バンドル内のResourcesへ署名前に配置する。ヘルパーのアドホック署名と実ファイルの宣言は確認した。製品全体の署名・公証・必要理由APIの確認は配布工程の残件である。[AppleのSystem Boot Time利用理由](https://developer.apple.com/documentation/bundleresources/app-privacy-configuration/nsprivacyaccessedapitypes/nsprivacyaccessedapitype)

## 自動検査

最終Swift33件は終了コード0で成功した。追加9件は整数時計変換、固定UTC、時刻欠落・範囲外、時計対応の変化と遅延観測、サンプルカウンターとドリフト・欠損、部分証拠の拒否、実ホスト時計の変換、PCMを変更しない連続時刻への置換、実CAFへの保存と読戻しを扱う。実ホスト時計の読込みは行うが、録音・再生・録音権限の要求は行わない。ログは `/private/tmp/mon-swift-capture-clock-producer-final.log` にある。

合成のマイク4800フレームとPC音声2400＋2400フレームをAVAssetWriterで保存し、200msの欠損がファイル内の無音にならないことを、全フレーム・全チャンネルの値で検査した。実際のSwiftの統計・補正生成・manifestエンコードを使って二つのCAFとmanifestを生成する。

`make check-capture-clock` は、このSwift生成物をRustの実際の録音確定処理へ渡す独立の統合検査である。UTC原点、物理フレーム数、欠損登録、既知区間の補正付き再生、欠損を横断する入力の拒否、原本の保持を検査する。最終Swift変更とヘルパーバンドル再構築後の再確認も、Swift生成1件・Rust統合1件が終了コード0で成功した。ログは `/private/tmp/mon-swift-capture-clock-integration-final.log` にある。環境依存のRust検査を通常テストではignoredとし、通常の `make check` の専用工程から明示実行する。

製品ソース `031f3e5` の通常の `make check` は終了コード0で完了した。Python65件、Swift33件、フロントエンド406件と型検査・ビルド、Rust通常1389件・失敗0件・37件ignoredが成功した。CAF4800/4800フレームとAVFoundation製CAF読出し1件、専用工程のSwift生成1件・Rust統合1件も成功した。通常Rustのignored区分は既存36件と専用統合検査1件で、成功した専用検査を通常件数へ重複加算しない。ログは `/private/tmp/mon-capture-clock-producer-full-check.log` にある。後続の話者色・トラック別発言表示のフロントエンド410件は別の最終検査であり、この406件へ読み替えない。

検査終了後に専用Rustキャッシュ約2.1GiBを削除し、ソース・検証ログ・既存release成果物と `app/src-tauri/target` 約38MiBを保持した。Rustの既存のリンカー警告とSwiftPMのユーザー用キャッシュ書込警告は残り、検査の失敗とは区別する。

前段の製品ソース `4cdfd42` の全体検査は終了コード0で完了した。Rust通常1389件・失敗0件・既存36件ignored、Python65件、Swift24件、フロントエンド406件と型検査・ビルド、CAF4800/4800フレームとAVFoundation製CAF読出し1件が成功した。この結果を今回のSwift接続後の全体成功と混同しない。

## 残対応

明示スリープ・機器切断・同じ機器への復帰、部分CAFの救出、欠損を含む録音の既知区間だけを解析するジョブ構成、実機での同期・エコー・重複・4時間以上の録音と停止後解析は残る。録音準備画面、三画面連携、残るUI・異常時対応・配布・先行試用も進行中である。

正式評価・10話者・Windows固有部分は保留し、Windows CI・アプリ起動・実録音は実行していない。計画全体を完了扱いにしない。
