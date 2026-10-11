# 版を固定した依存通知文の補完資料

このディレクトリのライセンス本文は、それぞれの第三者のソフトウェアに関するもの。MON自身のコードのライセンスを指定・変更しない。

`index.json`は対象crateの名前・版・Cargo配布checksum・申告ライセンス・公開VCS commit／pathと、資料のHTTPS出典・容量・SHA-256を結ぶ。収集時に対象crateと照合し、相違・資料欠落・改変では使用しない。通常のビルドでは通信しない。対象版を変える場合は、新しい版の条件を確認してから別途更新する。

| 資料 | 用途 |
|---|---|
| alloc-stdlibのLICENSE | 同じVCS commitの上流から取得した著作権付きBSD本文 |
| defmtのLICENSE-MIT／APACHE | 同じVCS commitの上流から取得した両ライセンス本文 |
| SymphoniaのLICENSE | 同じVCS commitの上流から取得したMPL本文 |
| MozillaのMPL 2.0 | selectorsのソースが明示する公式ライセンス本文。個別著作権を創作しない |
| objc2の4版のLICENSE.md | 本文ではなく上流のライセンス説明・参照。`upstream_license_reference`として扱い、missingNoticeTextsを解消しない |
| SPDX v3.26.0のMIT本文template | 2026-10-11に取得。上流説明がMITを選択肢として示す12件へ、参照本文をオフラインで読むため添付。`license_template`として扱い、個別著作権表示・missingNoticeTextsの不足を解消しない |

資料は出典の全バイトを保持する。改行のない原文はそのまま改行なしで保持する。取得日は2026-10-10。すべての本文・追加NOTICE・著作権・対応ソース・Apple SDK由来等の条件が満たされたとの承認ではない。`licenseReview: not_verified`と`distributionReady: false`を維持する。

MIT templateの`<year>`・`<copyright holders>`は原文のまま残す。MONや上流作者の名前・年を推測して置き換えない。templateを個別ソフトウェアの著作権通知として扱わず、生成通知にもその区別を表示する。Apple SDK由来の上流注意も削除しない。本文は[版固定のSPDX資料](https://github.com/spdx/license-list-data/blob/v3.26.0/text/MIT.txt)に全バイト一致する（1087bytes／SHA256 `6d7c9e9d7d12c8d3753bdb24417b84819ba4d6c2f025c37948113d86dc81eef9`）。

MPL対象のソース取得先とSHAは、アプリ同梱の依存一覧・通知文へ記録する。ビルドはソースarchiveを取得せず、参照の状態は`source_reference_not_download_verified`。2026-10-10の別の実取得確認は[11件の検証記録](../../docs/verification/mpl-source-availability-2026-10-10.json)に保持する。将来の配布時にもソースの提供方法・通知・取得可否を確認する。
