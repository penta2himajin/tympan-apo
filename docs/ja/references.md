# 参考資料

*他の言語で読む: [English](../references.md).*

設計時に参照した資料。

## Microsoft 公式ドキュメント

### APO の中核ドキュメント

- **Audio Processing Object Architecture**
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/audio-processing-object-architecture>
  - SFX / MFX / EFX のカテゴリ、ライフサイクル、スレッディング
- **Implementing Audio Processing Objects**
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/implementing-audio-processing-objects>
  - 必須インターフェース、ベースクラスの使い方、INF 登録
- **Audio Signal Processing Modes**
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/audio-signal-processing-modes>
  - APO とオーディオエンジンの処理モードの連携
- **Deep Noise Suppression**
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/audio-signal-processing-modes#deep-noise-suppression>
  - AI ベースのノイズサプレッションのための Windows 11 24H2 システム
    エフェクト

### Windows 11 AEC APO API

- **Windows 11 APIs for Audio Processing Objects**
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/windows-11-apis-for-audio-processing-objects>
  - `IApoAcousticEchoCancellation`, `IApoAcousticEchoCancellation2`,
    `IApoAuxiliaryInputConfiguration`, `IApoAuxiliaryInputRT`
- **AEC リファレンスストリーム用 WASAPI loopback**
  - 上記ドキュメントで説明あり。プライベートチャンネル経由のリファレ
    ンスストリームが利用できない場合の代替手段。

### オーディオエンジンの背景

- **AudioRenderEffectsManager**
  - <https://learn.microsoft.com/en-us/uwp/api/windows.media.audio.audiorendereffectsmanager>
  - エンドポイント上で有効なエフェクトの問い合わせ
- **Audio Effects Discovery サンプル**
  - エフェクト列挙を実演する Windows SDK 同梱のサンプルアプリ

## リファレンス実装

### Microsoft Windows-driver-samples

- <https://github.com/microsoft/Windows-driver-samples>
- 正典的なサンプル集。特に注目すべきは:
  - `audio/sysvad/EndpointsCommon/` — エンドポイント構造
  - `audio/sysvad/SwapAPO/` — チャンネル入れ替え APO (シンプルな
    SFX の例)
  - `audio/sysvad/AecAPO/` — Windows 11 AEC APO サンプル
  - `audio/sysvad/KwsAPO/` — キーワード検出 APO (loopback ストリッ
    ピング)
- ライセンス: MIT

### Equalizer APO

- <https://sourceforge.net/projects/equalizerapo/>
- 最も普及しているサードパーティ APO
- ライセンス: GPL-2.0
- 実演する内容: システム全体のパラメトリック DSP、テキストファイル
  による実行時設定、マルチチャンネル EQ、VST ホスト機能
- Microsoft のドキュメントが示唆する以上にサードパーティ APO で実現
  可能なことが多いことの証左として特筆に値します

### dechamps/APO

- <https://github.com/dechamps/APO>
- コミュニティが保守する APO 開発ノートのコレクション
- 特に有用な内容: 登録の仕組み、レジストリ構造、文書化された挙動と
  実際の挙動のギャップ
- ライセンス: MIT 系

### NoiseTorch (相互参照用の Linux 相当品)

- <https://github.com/noisetorch/NoiseTorch>
- APO ではないものの、Linux でシステム全体のマイクノイズサプレッ
  ションを提供する最も近いオープンソース相当品
- 参照に有用: ユーザ向け UX、オーディオパイプラインリセット時の
  復旧戦略、インプロセスのオーディオ強化の意義

## 関連 Rust クレート

### COM バインディング

- **windows** (Microsoft 公式)
  - <https://crates.io/crates/windows>
  - Windows API の公式 Rust バインディング
  - `IAudioProcessingObject*` と関連インターフェースの型定義を提供。
    `tympan-apo` はこの上に構築されます
- **windows-sys** (低レベル)
  - <https://crates.io/crates/windows-sys>
  - COM の利便性レイヤを持たない生の `extern "system"` バインディング

### クライアント側オーディオ (tympan-apo のスコープ外だが関連)

- **wasapi-rs**
  - <https://crates.io/crates/wasapi>
  - クライアント側オーディオ向けの WASAPI への親しみやすい Rust ラッパー
  - これはデバイスからオーディオを*キャプチャ*するためのもので、
    オーディオエンジン内部で*処理*するためではありません
- **cpal**
  - <https://crates.io/crates/cpal>
  - クロスプラットフォームのクライアント側オーディオ I/O

### リアルタイム / ロックフリー

- **crossbeam**
  - <https://crates.io/crates/crossbeam>
  - リアルタイムスレッドに適したロックフリーデータ構造
- **atomic-waker**
  - <https://crates.io/crates/atomic-waker>
  - スレッド間の非ブロッキング起床通知

### 汎用 DSP

- **rustfft**: スペクトル処理に用いる FFT
- **biquad**: 標準的なバイカッドフィルタ
- **realfft**: 実数値信号に最適化された FFT

## リアルタイムオーディオプログラミングの背景

- **Ross Bencina, "Real-time audio programming 101: time waits for nothing"**
  - <http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing>
  - リアルタイムオーディオ制約への正典的入門
- **Windows のリアルタイムスケジューリング**
  - APO スレッドは MMCSS サービス経由で `AVRT_PRIORITY_REALTIME`
    にて動作します
  - オーディオエンジンがスレッド優先度を調整するため、APO 自身で
    スケジューリングを変更しようとしてはなりません

## ビルド、署名、デプロイ

- **オーディオドライバ・APO 向けコード署名**
  - APO のロードには (カーネルドライバとは異なり) WHQL 署名は不要
  - ただしインストール時の SmartScreen は EV コード署名されたバイナリ
    を好みます
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/install/code-signing-best-practices>
- **APO 登録用 INF ファイル**
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/registering-an-apo>
  - 公式の機構。代替として直接 `regsvr32` + レジストリ編集もある
- **Windows Update との相互作用**
  - オーディオドライバの再インストールにより APO 登録が上書きされ得る
  - フレームワークはこれを文書化しますが、防止はできません
  - Equalizer APO のユーザはこれが最大の運用課題だと報告しています
