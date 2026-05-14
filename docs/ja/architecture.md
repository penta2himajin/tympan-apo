# アーキテクチャ

*他の言語で読む: [English](../architecture.md).*

本ドキュメントは、実装済みのアーキテクチャ — モジュール構成、4 層
モデル、ユーザーが実装する中核の抽象 — を記述します。
[`overview.md`](overview.md) の「スコープに含まれるもの」の機能群は
実装完了しています。検証戦略については
[`decisions/0001-ci-verification-strategy.md`](decisions/0001-ci-verification-strategy.md)
と [`testing.md`](testing.md) を参照してください。

## モジュール構成

フレームワーククレートは `rlib` のみを生成します。4 つの `Dll*` COM
エントリポイントは `register_apo!` / `register_aec_apo!` マクロが
**利用側**クレートのルートに展開するため、フレームワーク自身は
`cdylib` を生成しません。`rlib` と `cdylib` を同時に生成すると並列
リンクの競合 (`rust-lang/cargo#6313`) を踏むため、それを避ける構成
です。`examples/` 配下の各リファレンス APO はそれぞれ独立した
`cdylib` です。

```
tympan-apo/
├── src/
│   ├── lib.rs            # 再エクスポート、公開 API 表面
│   ├── apo.rs            # ProcessingObject トレイト、ProcessInput、
│   │                     #   ApoCategory、SystemEffect
│   ├── instance.rs       # ApoInstance<T> + AnyApoInstance:
│   │                     #   フレームワーク側のライフサイクルラッパー
│   ├── buffer.rs         # BufferFlags、ConnectionProperty
│   ├── format.rs         # Format、FormatNegotiation、
│   │                     #   WAVEFORMATEX(TENSIBLE) 変換
│   ├── error.rs          # HResult ラッパー + APO HRESULT 定数
│   ├── clsid.rs          # Clsid (クロスプラットフォームな GUID)
│   ├── inf.rs            # INF ファイルジェネレータ
│   ├── fx_properties.rs  # FxProperties エンドポイントバインディング
│   ├── macros.rs         # register_apo! / register_aec_apo!
│   ├── raw/              # 低レベル COM バインディング (Windows 専用)
│   │   ├── mod.rs
│   │   ├── abi.rs            # コンパイル時 ABI 不変条件
│   │   ├── class_factory.rs  # ApoVTable + ApoClassFactory
│   │   ├── instance_com.rs   # ApoInstanceCom: IAudioProcessingObject
│   │   │                     #   ファミリ + IAudioSystemEffects v1/v2/v3
│   │   ├── dispatch.rs       # 共有 COM メソッド本体
│   │   ├── media_type.rs     # IAudioMediaType <-> Format ブリッジ
│   │   ├── reg_properties.rs # APO_REG_PROPERTIES ペイロード生成
│   │   ├── register.rs       # HKCU CLSID レジストリ書き込み/削除
│   │   └── exports.rs        # Dll* ディスパッチヘルパー
│   ├── realtime/         # リアルタイム安全プリミティブ (クロスプラットフォーム)
│   │   ├── mod.rs
│   │   ├── context.rs    # RealtimeContext マーカー型
│   │   ├── ring.rs       # ロックフリー SPSC リングバッファ
│   │   ├── state.rs      # StateCell ライフサイクル状態機械
│   │   └── refcount.rs   # COM 形式のアトミック参照カウント
│   └── aec/              # Windows 11 AEC APO 対応
│       │                 #   (Windows + `aec` フィーチャー)
│       ├── mod.rs            # AecProcessingObject、AecApoInstance<T>、
│       │                     #   AnyAecApoInstance、AuxiliaryInputBuffer
│       ├── class_factory.rs  # AecApoVTable + AecApoClassFactory
│       ├── instance_com.rs   # AecApoInstanceCom: 9 つの AEC IID
│       └── exports.rs        # AEC Dll* ディスパッチヘルパー
├── examples/
│   ├── passthrough.rs    # 最小の APO: 入力を出力へコピー
│   ├── gain.rs           # 固定線形ゲイン、インスタンスごとの状態
│   └── aec_scaffold.rs   # AEC APO スケルトン (`aec` フィーチャー必須)
└── tests/
    ├── realtime_safety.rs    # RT パスの assert_no_alloc ガード
    ├── register_apo.rs       # マクロ展開エクスポートの結線
    ├── tier3_lifecycle.rs    # インプロセス COM 起動 (SISO)
    └── tier3_aec_lifecycle.rs# インプロセス COM 起動 (AEC)
```

## 層モデル

モジュール境界で隔離された 4 つの概念的な層です。

### 第 1 層: `raw` — COM バインディング

Windows 専用 (`#[cfg(windows)]`)。

- `windows` / `windows-core` クレートの APO インターフェース型を消費
  する唯一の場所であり、`windows_core::implement` ベースの vtable
  構築を所有する唯一の場所です。
- `instance_com::ApoInstanceCom` は `Arc<dyn AnyApoInstance>` を
  `IAudioProcessingObject` ファミリ (`IAudioProcessingObject`、
  `IAudioProcessingObjectConfiguration`、`IAudioProcessingObjectRT`)
  と `IAudioSystemEffects` v1/v2/v3 に橋渡しします。
- `dispatch` は COM メソッド本体を `&dyn AnyApoInstance` を取る自由
  関数に切り出し、SISO キャリアと AEC キャリアがコピペなしで歩調を
  揃えられるようにします。
- `class_factory` は `ApoVTable` (CLSID + メタデータ + 生成関数) と、
  そこからインスタンスを生成する `IClassFactory` を公開します。
- `exports` はマクロが展開する `Dll*` エントリポイントが呼び出す
  再利用可能な本体を提供します。`register` は
  `HKCU\Software\Classes\CLSID\{…}` サブツリーを書き込み、
  `reg_properties` は可変長の `APO_REG_PROPERTIES` ペイロードを構築し、
  `media_type` は `IAudioMediaType` を `Format` に橋渡しし、`abi` は
  `windows-rs` のレイアウトドリフトを防ぐコンパイル時の
  `size_of` / `align_of` アサーションを保持します。

`tympan-apo` の利用者がこのモジュールに触れることは想定されていません。
上級ユーザーとフレームワーク自身のテストハーネスのために `pub` に
なっています。

### 第 2 層: `realtime` — ゼロアロケーションプリミティブ

クロスプラットフォーム — リアルタイム不変条件は Windows 固有 API に
依存せず、どのホストでもユニットテストできることのほうが
`#[cfg(windows)]` でゲートするより価値があるためです。

- アロケータ使用なし、`std::sync::Mutex` なし、`std::collections` なし。
- `RealtimeContext` — リアルタイムの `APOProcess` パスから呼んで安全な
  任意の関数に引数として要求されるゼロサイズマーカー。ユーザーコード
  からは構築できず (フレームワークが `process` ハーネスから参照で
  渡す)、コールスタックに存在すること自体がリアルタイム安全性の
  コンパイル時の証明になります。
- `ring` — ロックフリーな単一生産者・単一消費者リングバッファ。
  `Producer` / `Consumer` は `Send` だが `Sync` ではなく、容量は構築時
  に固定されるため、`try_push` / `try_pop` は待機フリーかつ
  ヒープ非接触です。
- `state` — `StateCell`。アトミックなライフサイクル状態機械
  (`Uninitialized → Initialized → Locked`) で、不正な遷移は静かな破壊
  ではなく `TransitionError` として表面化します。
- `refcount` — `Refcount`。COM `IUnknown` の `AddRef` / `Release` 契約
  を支える待機フリーのアトミックカウンタです。

### 第 3 層: 公開 API — 安全でイディオマティック

大多数のユーザーが触れる層です。クレートルートと、クロスプラット
フォームなモジュール `apo`、`buffer`、`clsid`、`error`、`format`、
`instance`、`inf`、`fx_properties` に存在します。

- `ProcessingObject` — ユーザーが実装するトレイト (後述)。
- `ApoInstance<T>` / `AnyApoInstance` — `StateCell`、`Refcount`、
  `UnsafeCell<T>` を 1 つのオブジェクトに束ね、オーディオエンジンに
  渡すフレームワーク側ラッパー。`AnyApoInstance` は COM ブリッジが
  ディスパッチに用いる型消去ビューです。
- `Format` / `FormatNegotiation` — PCM ストリーム記述と、Accept /
  Suggest のネゴシエーション結果。
- `ProcessInput` / `BufferFlags` / `ConnectionProperty` — バッファ
  ごとのペイロードとホストのフラグワード。
- `Clsid` / `HResult` — クロスプラットフォームな GUID / HRESULT 値型。
  `windows-core` 側の対応型とレイアウト互換です。

### 第 4 層: `aec` — Windows 11 AEC APO 対応

`#[cfg(all(windows, feature = "aec"))]` でゲートされており、非 AEC
プラグインが Windows 11 SDK 表面を引き込まないようにしています。

- `AecProcessingObject` — `ProcessingObject` を拡張する補助入力
  ライフサイクルフック (`add_aux_input`、`remove_aux_input`、
  `is_aux_format_supported`、`accept_aux_input`) を追加する
  トレイト。
- `AecApoInstance<T>` / `AnyAecApoInstance` — `ApoInstance<T>` の上に
  構築された AEC ラッパーで、SISO 状態機械を再利用します。
- `AuxiliaryInputBuffer` — リアルタイムスレッドで `accept_aux_input`
  に渡されるバッファごとの参照信号ペイロード。
- `class_factory` / `instance_com` / `exports` — `raw` キャリアの
  AEC 版。`AecApoInstanceCom` は 9 つの COM インターフェースを
  広告します: 6 つの SISO インターフェースに加えて
  `IApoAcousticEchoCancellation`、
  `IApoAuxiliaryInputConfiguration`、`IApoAuxiliaryInputRT`。

## 中核の抽象

### `ProcessingObject`

利用者が実装する最上位トレイト。各実装者は 1 つの CLSID で識別される
APO です。フレームワークの COM ハーネスが `new` で型を構築し、
フォーマットネゴシエーション / `LockForProcess` / `APOProcess` /
`UnlockForProcess` のシーケンスを駆動し、オーディオエンジンの呼び出し
をトレイトメソッドに振り分けます。

```text
pub trait ProcessingObject: Sized + Send {
    const CLSID: Clsid;
    const NAME: &'static str;
    const COPYRIGHT: &'static str;
    const CATEGORY: ApoCategory;          // Sfx / Mfx / Efx

    fn new() -> Self;

    // フォーマットネゴシエーション — デフォルトは任意の IEEE-float32
    // ストリームを受理し、それ以外には float32 の代替を Suggest。
    fn is_input_format_supported(&self, format: &Format) -> FormatNegotiation { … }
    fn is_output_format_supported(&self, format: &Format) -> FormatNegotiation { … }

    // システムエフェクトの列挙 / トグル (IAudioSystemEffects2/3)。
    // デフォルト: 列挙可能なエフェクトなし、トグルは no-op。
    fn system_effects(&self) -> &[SystemEffect] { &[] }
    fn set_system_effect_state(&mut self, id: &Clsid, state: SystemEffectState) { … }

    // ライフサイクル。lock_for_process で事前確保、unlock で解放。
    fn lock_for_process(&mut self, input: &Format, output: &Format)
        -> Result<(), HResult> { Ok(()) }
    fn unlock_for_process(&mut self) {}

    // リアルタイム: アロケーションフリー、ロックフリー、syscall なし。
    fn process(
        &mut self,
        rt: &RealtimeContext,
        input: ProcessInput<'_>,
        output: &mut [f32],
    ) -> BufferFlags;
}
```

`new` と関連定数を除けば、必須メソッドは `process` のみで、それ以外
には妥当なデフォルトがあります。戻り値はホストの出力
`APO_CONNECTION_PROPERTY` の `u32BufferFlags` になります。

フレームワークは COM インプロセスサーバのエントリポイントをマクロで
展開します:

```text
tympan_apo::register_apo!(MyApo);
```

これは呼び出し側クレートのルートに、`ApoVTable` の static、要素 1 個
のレジストリ、そして `raw::exports` のディスパッチヘルパーに結線された
4 つの `#[no_mangle]` な `Dll*` エクスポート (`DllGetClassObject`、
`DllCanUnloadNow`、`DllRegisterServer`、`DllUnregisterServer`) を
展開します。展開されるシンボルは固定名なので、`cdylib` ごとにちょうど
1 回呼び出す必要があります。

### `Format` とフォーマットネゴシエーション

`Format` は `WAVEFORMATEX` に `WAVEFORMATEXTENSIBLE` 拡張
(チャンネルマスク、有効ビット数、サブフォーマット) を加えたものを
反映します。型付きコンストラクタ (`pcm_int16`、`pcm_int24`、
`pcm_int32`、`pcm_float32`、`pcm_float64`) は基本バリアントを生成し、
`with_extensible` で拡張ワイヤフォーマットにオプトインしてデフォルトの
チャンネルマスクを埋めます。`raw::media_type` がホストの
`IAudioMediaType` との相互変換を行います。

```text
fn is_input_format_supported(&self, format: &Format) -> FormatNegotiation {
    if format.sample_rate() == 48_000 && format.channels() == 1 {
        FormatNegotiation::Accept
    } else {
        FormatNegotiation::Suggest(Format::pcm_float32(48_000, 1))
    }
}
```

### `RealtimeContext`

リアルタイム安全性をコンパイル時に検査するゼロサイズマーカー。
フレームワークが `APOProcess` ハーネスから `ProcessingObject::process`
へ参照で渡します。フィールドを持たず、ユーザーから到達可能な
コンストラクタもありません (テストはクレート内部の `new_unchecked` を
使用)。

### `aec::AecProcessingObject`

AEC APO 向けの拡張トレイト。`ProcessingObject` の上に補助入力
(参照ストリーム) のライフサイクルを追加します:

```text
pub trait AecProcessingObject: ProcessingObject {
    fn add_aux_input(&mut self, id: u32, format: &Format, init_data: &[u8])
        -> Result<(), HResult> { Ok(()) }
    fn remove_aux_input(&mut self, id: u32) {}
    fn is_aux_format_supported(&self, format: &Format) -> FormatNegotiation { … }
    fn accept_aux_input(&mut self, rt: &RealtimeContext, input: AuxiliaryInputBuffer<'_>) {}
}
```

4 つのメソッドすべてにデフォルトがあるため、実装者はエコー
キャンセリングアルゴリズムが必要とするものだけをオーバーライドします。
`accept_aux_input` はリアルタイムスレッドで動作し、`process` と同じ
アロケーションフリー / ロックフリー制約を負います。

## 横断的な関心事

### CLSID の割り当て

APO は COM クラス ID で識別されます。`Clsid` はクロスプラット
フォームで `#[repr(C)]`、GUID とレイアウト互換の型で、`from_u128` /
`from_parts` コンストラクタを備えるため、作者はどのホストでも CLSID
を宣言・ユニットテストできます。`Clsid::NIL` は COM が
`CLASS_E_CLASSNOTAVAILABLE` として拒否するセンチネルです。

### 登録

プラットフォーム固有性が増す 3 段階の登録ヘルパー:

- `raw::register` — `DllRegisterServer` / `DllUnregisterServer` が
  `HKCU\Software\Classes\CLSID\{…}` サブツリーを書き込み・削除する
  ため、`regsvr32 /n /i:user` が管理者権限なしで動作します。
- `inf` — `generate(&InfConfig)` が Windows のコンポーネント化モデル
  に統合する本番配布向けの最小 INF を出力します。
- `fx_properties` — `HKLM\…\MMDevices\Audio` 配下の `FxProperties`
  サブツリーを書き込み、登録済み CLSID を特定のオーディオ
  エンドポイントに紐づけます。昇格が必要です。

### リアルタイムロギング

リアルタイムコードは `tracing` や `log` でログ出力できません
(どちらもアロケートする)。`realtime::ring` の SPSC バッファが
「リアルタイムスレッドからログを出し、別スレッドで吸い出す」
パターンの基盤です: `process` から小さな `Copy` イベントを push し、
非リアルタイムスレッドから吸い出します。

## 決着した設計判断

設計フェーズで未決だった問いはその後決着しています:

- **アグリゲーション。** APO は単一入力・単一出力です (AEC モードでは
  補助入力をオプションで持つ)。フレームワークは SISO を型レベルで
  強制し、クラスファクトリはアグリゲーションを
  `CLASS_E_NOAGGREGATION` で拒否します。
- **最小 Windows バージョン。** MSRV は Rust 1.80 で、`windows`
  クレートに合わせています。非 AEC パスは Windows 10 以降を対象とし、
  `aec` フィーチャーは Windows 11 23H2+ を対象として、非 AEC ビルドが
  新しい SDK を要求しないようゲートされています。
- **AEC 参照ストリーム。** 参照 (ループバック) 信号は
  `IApoAuxiliaryInputRT::AcceptInput` 経由で配信され、ユーザーコード
  には `AuxiliaryInputBuffer` として渡されます。フレームワークは独自
  の WASAPI ループバックを開きません。
- **信号処理モード。** APO は `ApoCategory` (`Sfx` / `Mfx` / `Efx`)
  でスロットを宣言します。それ以外はフレームワーク層ではモード非依存
  です。
- **エフェクトの動的な ON/OFF。** `IAudioSystemEffects2` /
  `IAudioSystemEffects3` を実装済みです: `ProcessingObject::system_effects`
  がエフェクト一覧を広告し、`set_system_effect_state` がエンジンの
  トグル呼び出しを受け取ります。

## 既知の制限

- `IApoAuxiliaryInputRT::AcceptInput` は補助バッファのジオメトリを
  プライマリ入力のロック済みフォーマットから推測します。補助入力が
  プライマリ入力と異なるフォーマットを使う AEC APO には補助入力ごとの
  明示的なフォーマット追跡が必要ですが、これは未実装です。
- `raw::reg_properties` はキャリアごとに固定のインターフェース一覧
  (SISO は 3 IID、AEC キャリアは 9 IID) を広告します。独自の
  インターフェースセットを持つ APO 向けに広げるにはコード変更が
  必要です。
- Tier 4 検証 — 実際の `audiodg.exe` を駆動する — は GitHub ホストの
  ランナーでは実行できず、手動 / セルフホストのステップです。
  [`testing.md`](testing.md) を参照してください。
