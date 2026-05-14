# tympan-apo

*他の言語で読む: [English](README.md).*

Windows Audio Processing Object (APO) を Rust で実装するためのフレームワーク。

`tympan-apo` は Windows Audio Processing Object の COM インターフェース
に対する Rust 抽象化を提供し、C++ を書かずに Rust アプリケーションから
Windows Audio Engine 内で動作するカスタムなシステムエフェクトプロセッサ
(SFX, MFX) を実装できるようにします。

Windows 11 の AEC APO API への対応も組み込まれており、マイク入力パイプ
ラインにおける公式のアコースティックエコーキャンセリング処理スロットに
Rust コードから参加できます。

## ステータス

**意図した機能は実装完了。** [`docs/ja/overview.md`](docs/ja/overview.md)
の「スコープに含まれるもの」の各項目はすべて実装済みです。

- COM オブジェクト基盤 — IUnknown、IClassFactory、登録、4 つの `Dll*`
  エクスポート ([`src/raw/`](src/raw/))。
- 必須インターフェース群 — `IAudioProcessingObject` ファミリと
  `IAudioSystemEffects` v1/v2/v3 — および SFX・MFX・EFX カテゴリ。
- AEC APO 対応 ([`src/aec/`](src/aec/)): `AecProcessingObject` 基盤、
  COM ブリッジ (`AecApoInstanceCom` + `register_aec_apo!`)、
  `aec_scaffold` サンプル。いずれも `aec` フィーチャーでゲートされます。
- `WAVEFORMATEX` と `WAVEFORMATEXTENSIBLE` 両方のフォーマット
  ネゴシエーション ([`src/format.rs`](src/format.rs))。
- リアルタイム安全なプリミティブ ([`src/realtime/`](src/realtime/)):
  ロックフリーリングバッファ、アトミック状態ヘルパー、アトミック
  参照カウント。
- 登録補助 ([`src/clsid.rs`](src/clsid.rs)、
  [`src/raw/register.rs`](src/raw/register.rs)、
  [`src/inf.rs`](src/inf.rs)、
  [`src/fx_properties.rs`](src/fx_properties.rs)): CLSID 割り当て、
  レジストリ書き込み、INF ジェネレータ、FxProperties エンドポイント
  バインディング。
- サンプル APO ([`examples/`](examples/)): `passthrough`、`gain`、
  `aec_scaffold`。

CI は Tier 1 (fmt、clippy、build/test)、Tier 2 (複数 DLL の
エクスポート・依存関係・署名検証)、Tier 3 (AEC 派生を含む COM
ライフサイクルハーネスと AddressSanitizer nightly) を実行します。
API 設計案は [`docs/architecture.md`](docs/architecture.md)、CI 戦略は
[`docs/testing.md`](docs/testing.md) を参照してください。
日本語版ドキュメントは [`docs/ja/`](docs/ja/) 配下にあります。

## 名前の由来

*Tympan* は蛾の鼓膜器官 (tympanal organ) を指します。メイガ科やヤガ科
などの蛾の腹部にある膜状の超音波センサで、コウモリのエコーロケーション
を検出するために進化しました。この名前は、本ライブラリが OS のオーディオ
エンジンとユーザー空間の Rust コードの間に位置する「薄い膜」としての役割
を反映しています。

## ライセンス

以下のいずれかのライセンスのもとで利用可能です。

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) または
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) または
  <http://opensource.org/licenses/MIT>)

利用者が選択できます。

### コントリビューション

明示的に別段の意思表示をしない限り、Apache-2.0 ライセンスに定義される
通り、あなたが本作品への取り込みを意図して提出したコントリビューション
は、上記の通り追加の条件なしにデュアルライセンスされるものとします。

## ドキュメント

| ドキュメント | 内容 |
|---|---|
| [`docs/ja/overview.md`](docs/ja/overview.md) | プロジェクトの目的、スコープ、既存実装との比較 |
| [`docs/ja/architecture.md`](docs/ja/architecture.md) | API 設計案とモジュール構成 |
| [`docs/ja/references.md`](docs/ja/references.md) | Microsoft 公式ドキュメント、先行事例、関連クレート |
| [`docs/ja/testing.md`](docs/ja/testing.md) | GitHub ホスト Windows ランナーを跨ぐテスト・CI 戦略 |
| [`docs/decisions/`](docs/decisions/) | アーキテクチャ決定記録 (ADR) (英語のみ) |
| [`docs/handoff-protocol.md`](docs/handoff-protocol.md) | 長期作業のセッション間引き継ぎプロトコル (英語のみ) |
