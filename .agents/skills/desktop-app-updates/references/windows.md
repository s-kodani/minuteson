# Windowsの更新境界

MSIX/AppX、Store掲載のEXE/MSI、直接配布のインストーラーは更新機構が異なる。言語やUIフレームワークだけで配布形式を判断しない。

- StoreのMSIX更新では、OSからの終了通知と使用中パッケージの扱いを確認する。マニフェストの生成元と実際の同梱XMLを調べる。
- Windows 11 24H2以降の`uap17:UpdateWhileInUse`は`defer`を選べる。使用中の更新を延期する方針に合う場合に使い、最低OSとの整合を確認する。トレイ常駐中にも適用が延期され得ることを利用者へ説明する。
- `WM_QUERYENDSESSION`、`WM_ENDSESSION`、`WM_CLOSE`と通常の終了要求は同じ経路とは限らない。利用中のランタイム・子プロセス・保存完了待ちまで追う。
- Store APIにはパッケージIdentity、UIスレッド、所有ウィンドウの初期化が必要なものがある。非同期完了までUIスレッドを待たせない。
- 更新の検出結果には配信の遅れやキャッシュがある。取得可能な更新がないことだけで、実行中の版が最新・登録完了とは断定しない。
- Store起点の再起動登録と、アプリ独自の再起動機能を分けて確認する。再起動が必要でも実行中の作業の保全を先に解決する。

公式資料:

- [使用中の更新延期](https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-uap17-updatewhileinuse)
- [Storeの更新API](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/package-updates-from-store)
- [Store更新時の終了と再起動](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/desktop-to-uwp-extensions#restart-automatically-after-receiving-an-update-from-the-microsoft-store)
- [StoreへのURI](https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-store-app)
