# アーキテクチャ

*他の言語で読む: [English](../architecture.md).*

本ドキュメントは計画中のアーキテクチャを記述します。実装は未着手で
あり、設計フィードバックの蓄積に応じて詳細は変わり得ます。

## モジュール構成

```
tympan-apo/
├── src/
│   ├── lib.rs            # 再エクスポート、公開 API 表面
│   ├── apo.rs            # ProcessingObject トレイト、ライフサイクル
│   ├── format.rs         # WAVEFORMATEX ヘルパー、フォーマット交渉
│   ├── property.rs       # IPropertyStore ラッパー
│   ├── registration.rs   # CLSID + INF + レジストリヘルパー
│   ├── raw/              # 低レベル: `windows` クレート経由の COM IF バインディング
│   │   ├── mod.rs
│   │   ├── interfaces.rs # IAudioProcessingObject* トレイト配線
│   │   ├── hresult.rs    # APO 固有の HRESULT コード
│   │   └── class.rs      # IClassFactory ボイラープレート
│   ├── realtime/         # リアルタイム安全なプリミティブ
│   │   ├── mod.rs
│   │   ├── context.rs    # RealtimeContext マーカ型
│   │   ├── ring.rs       # ロックフリー SPSC リングバッファ
│   │   └── state.rs      # アトミック状態機械ヘルパー
│   └── aec/              # Windows 11 AEC APO 対応
│       ├── mod.rs
│       ├── auxiliary.rs  # IApoAuxiliaryInput* 対応
│       └── reference.rs  # リファレンスストリーム処理 (WASAPI loopback)
├── examples/
│   ├── passthrough/      # 入力を出力にコピーするだけの最小 APO
│   ├── gain/             # 線形ゲイン APO
│   └── aec-scaffold/     # AEC APO スケルトン (実 DSP なし)
└── tests/
    └── ...               # 結合テスト
```

## レイヤモデル

モジュール境界によって分離された 4 つの概念レイヤ。

### レイヤ 1: `raw` — COM バインディング

- `windows` クレートの APO インターフェース型の唯一の消費者
- `implement!` ベースの vtable 構築の唯一の所有者
- `IAudioProcessingObject`, `IAudioProcessingObjectRT`,
  `IAudioProcessingObjectConfiguration`, `IAudioSystemEffects3`、
  および AEC APO 各インターフェースの直接マッピングを提供

`tympan-apo` の利用者は通常このモジュールに触れる必要はありません。
フレームワーク内部のために、また高水準抽象を回避する必要がある上級
ユーザのために存在します。

### レイヤ 2: `realtime` — ゼロアロケーションプリミティブ

- アロケータ未使用
- `std::sync::Mutex` 不使用、`std::collections::HashMap` 不使用
- (`crossbeam-utils` 上に構築された) ロックフリー SPSC リングバッファ
- プラグインライフサイクル用のアトミック状態機械
- 零サイズマーカ `RealtimeContext`:
  - `APOProcess` から呼び出して安全な関数のパラメタとして必須
  - フレームワーク外からは構築不可
  - リアルタイム安全性のコンパイル時証人として機能

このレイヤの不変条件: `APOProcess` から到達可能な全関数は
`&RealtimeContext` を受け取り、ヒープ操作を一切含まないこと。

### レイヤ 3: 公開 API — 安全でイディオマティック

- `ProcessingObject` トレイト
- `Format`, `PropertyStore`, `ConfigurationContext` 型
- ホスト所有バッファ (APO_CONNECTION_PROPERTY) へのライフタイム境界付き
  参照
- 初期化中に失敗し得る操作のための `Result` 型

利用者の 95% が触れるのはこのレイヤです。

### レイヤ 4: `aec` — Windows 11 AEC APO 対応

- AEC APO に要求される auxiliary input パターンの実装
- リアルタイムなリファレンスストリームアクセスのための
  `IApoAuxiliaryInputRT` ラップ
- プライベートチャンネルが利用できない場合に用いる WASAPI loopback
  経路のためのヘルパー
- オプション: 非 AEC プラグインが Windows 11 SDK 要件を引き込まない
  よう、`aec` cargo フィーチャでゲート

## 中核となる抽象

### `ProcessingObject`

利用者が実装するトップレベルのトレイト。APO の COM ライフサイクルに
対応します。

```text
trait ProcessingObject: Sized {
    const CLSID: GUID;
    const NAME: &'static str;
    const COPYRIGHT: &'static str;
    const CATEGORY: ApoCategory;  // Sfx / Mfx / Efx

    fn new() -> Self;

    fn is_input_format_supported(
        &self,
        format: &Format,
    ) -> FormatNegotiation;

    fn lock_for_process(
        &mut self,
        input: &Format,
        output: &Format,
    ) -> Result<(), HResult>;

    fn process(
        &mut self,
        rt: &RealtimeContext,
        input: ApoInput,
        output: ApoOutput,
    );

    fn unlock_for_process(&mut self) {}
}
```

フレームワークは COM オブジェクトの構築とクラスファクトリ登録をマクロ
として提供します。

```text
tympan_apo::register_apo!(MyApo);
```

これは `DllGetClassObject` のエントリポイントと、COM が APO を生成する
ために用いる `IClassFactory` 実装へ展開されます。

### `Format` とフォーマット交渉

APO はオーディオエンジンとサンプルレート・チャンネル数・ビット深度
を交渉します。フレームワークは `WAVEFORMATEX`/`WAVEFORMATEXTENSIBLE`
のラッパーとして `Format` を提供します。

```text
fn is_input_format_supported(
    &self,
    format: &Format,
) -> FormatNegotiation {
    if format.sample_rate() == 48_000 && format.channels() == 1 {
        FormatNegotiation::Accept
    } else {
        FormatNegotiation::Suggest(
            Format::pcm_float32(48_000, 1),
        )
    }
}
```

### `RealtimeContext`

姉妹 tympan クレートの同名型と同じ役割を持ちます。リアルタイム安全性
をコンパイル時に検査する零サイズマーカで、フレームワークの
`APOProcess` ハーネスから利用者コードへ参照で渡されます。フィールド
は持たず、利用者コードから構築する手段もありません。

### `aec::AecProcessingObject`

AEC APO 用の拡張トレイト。auxiliary input (レンダーエンドポイントから
のリファレンスストリーム) のサポートを追加します。

```text
trait AecProcessingObject: ProcessingObject {
    fn process_aec(
        &mut self,
        rt: &RealtimeContext,
        microphone: ApoInput,
        reference: ApoAuxiliaryInput,
        output: ApoOutput,
    );
}
```

フレームワークはオーディオエンジンへの auxiliary input の登録と、
マイク・リファレンスストリーム間のタイムスタンプ整合を担当します。

## 横断的関心事

### CLSID 割り当て

APO は COM Class ID (GUID) で識別されます。作者は APO ごとに一意の
GUID を生成しなければなりません。フレームワークは以下を提供します。

- `CLSID` が非ゼロかつ著名な Microsoft GUID でないことのコンパイル時
  検証
- 新規プラグイン用に新しい GUID を生成するビルドスクリプトヘルパー
  `tympan-apo-genclsid`

### 登録

フレームワークは以下のための INF ファイルテンプレートと PowerShell
スニペットを提供します。

- COM クラスの登録 (`regsvr32`)
- 対象エンドポイントの `FxProperties` レジストリエントリへの APO の
  関連付け
- アンインストール時のクリーンアップ

利用者はインストール手順を管理者として実行する必要があります。
フレームワーク自身は権限昇格を試みません。

### リアルタイムロギング

リアルタイムコードからは `tracing` や `log` 経由でログを取れません
(いずれもアロケートします)。`realtime` モジュールは `APOProcess`
からの診断イベントを捕捉するロックフリーログキューを提供します。
別の非リアルタイムスレッド (`LockForProcess` の中で生成) がキューを
排出します。

## オープン課題

設計フェーズで決着させるもの:

- [ ] アグリゲーションをどう扱うか? COM APO は本質的に単一入力単一
  出力 (任意で auxiliary input) です。実行時チェックではなく型レベル
  で強制すべきです。
- [ ] 最小サポート Windows バージョンは何か? Windows 10 21H2 が妥当、
  AEC APO は Windows 11 22H2+。非 AEC APO はより古いバージョンも
  サポートすべきか?
- [ ] AEC リファレンスストリームの WASAPI loopback 経路をどう扱うか?
  オーディオエンジンがプライベートチャンネル経由でリファレンスを
  提供する場合と、APO が自身で loopback を開く場合があります。
  フレームワークは両方を抽象化する必要があります。
- [ ] オーディオエンジンの信号処理モード (Raw, Default, Communications,
  Speech 等) とどう連携するか? APO は対応モードを宣言すべきか、
  モード非依存のままにすべきか?
- [ ] `IAudioSystemEffectsControl` による動的なエフェクト on/off の
  ための `IAudioSystemEffects2` 通知パターンをフレームワークが
  サポートすべきか?

これらは実装開始前に解決します。決定は `docs/decisions/` (今後作成)
に記録します。
