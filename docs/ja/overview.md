# 概要

*他の言語で読む: [English](../overview.md).*

## 目的

`tympan-apo` は Windows の **Audio Processing Object (APO)** を実装する
ための Rust フレームワークです。APO は Windows Audio Engine
(`audiodg.exe`) の内部で動作し、特定のデバイスを流れるオーディオストリー
ムに対してデジタル信号処理を適用する、COM ベースのシステムエフェクト
プラグインです。

このフレームワークが目指すのは、Rust アプリケーションから以下を実現する
ことです。

- アプリケーション単位のオーディオを処理する Stream Effect (SFX) APO
  の実装
- 特定のモード (例: 通信、メディア) に紐づくオーディオを処理する
  Mode Effect (MFX) APO の実装
- Windows 11 AEC APO API を用いたアコースティックエコーキャンセリング
  APO の実装。これはマイクキャプチャパイプラインにおける AEC とその
  周辺処理の公式スロットです。
- 該当デバイスを利用する任意のアプリケーションに対してシステム全体で
  動作する、ノイズサプレッション・音声エフェクト・汎用マイク強化
  プラグインの構築

…C++ を書くことなく。

## なぜ存在するか

APO アーキテクチャは C++ の COM ヘッダ (`AudioEngineBaseAPO.h`,
`audioenginebaseapo.idl`) で定義されています。標準的な実装パスは
`CBaseAudioProcessingObject` を継承する形で、Microsoft の SYSVAD サンプル
や OSS の Equalizer APO プロジェクトに例があります。

Rust 向けの公式バインディングやフレームワークは存在しません。Rust 開発者
の既存の選択肢は以下のとおりです。

| アプローチ | 状態 | トレードオフ |
|---|---|---|
| `windows` クレートで COM を手書き | 可能だが複雑 | APO ごとに数百行、IUnknown を手作業で管理 |
| C++ ラッパー + FFI 経由の Rust コア | ハイブリッド | ビルドが複雑、純 Rust の魅力を失う |
| SYSVAD サンプルをそのまま土台に | C++ のみ | Rust の道筋なし |

このフレームワークが Rust 側のギャップを埋めます。COM の管理事項、
リアルタイム安全性の懸念、Windows 固有の癖を、安全な Rust トレイトの
背後にカプセル化します。

## スコープ

### スコープに含まれるもの

- APO の COM オブジェクト基盤 (IUnknown, IClassFactory, 登録)
- 必須インターフェース: `IAudioProcessingObject`,
  `IAudioProcessingObjectConfiguration`, `IAudioProcessingObjectRT`,
  `IAudioSystemEffects` (および v2/v3 派生)
- SFX および MFX APO カテゴリ
- Windows 11 23H2+ における AEC APO 対応:
  `IApoAcousticEchoCancellation`, `IApoAcousticEchoCancellation2`,
  `IApoAuxiliaryInputConfiguration`, `IApoAuxiliaryInputRT`
- フォーマットネゴシエーション補助 (サンプルレート、チャンネル数、
  ビット深度)
- APO 設定用のプロパティストアラッパー
- リアルタイム安全なプリミティブ (ロックフリーリングバッファ、
  アトミック状態ヘルパー)
- 登録補助 (CLSID 割り当て、INF ファイル生成、FxProperties 位置への
  レジストリ書き込みヘルパー)
- サンプル APO: 最小のパススルー、シンプルなゲイン、AEC リファレンス
  スケルトン

### スコープに含まれないもの

- Endpoint Effect (EFX) APO — 同じ API は適用できますが、EFX は本質的
  にデバイス全体を対象とするため、別途の検討を要します
- WDM オーディオのカーネルモードドライバ (これは APO ではなく、全く
  異なるプログラミングモデルです)
- Audio Driver Foundation (ハードウェアベンダがドライバスタック一式
  を出荷するために使用)
- DAW プラグイン形式 (VST3, ASIO) — 異なる API
- 信号処理アルゴリズム (DSP, ML) — `tympan-apo` に依存する利用側
  クレートに属します

## 名前について

*Tympan* は鼓膜器官 (tympanal organ) を指します。メイガ科 (Pyralidae)
やヤガ科 (Noctuidae) などの蛾の腹部にある膜状の聴覚器官です。コウモリの
エコーロケーションに対する防御として進化したもので、超音波を捕捉し、
付属する弦音器受容体を通じて振動を神経信号に変換します。

アナロジーは以下のとおりです。

- 鼓膜器官は外界と蛾の神経系の間に位置し、ある物理的領域 (空気圧) を
  別の領域 (神経インパルス) に変換します。
- `tympan-apo` は Windows Audio Engine とユーザー空間の Rust コードの
  間に位置し、あるプログラミング領域 (COM, IUnknown, リアルタイムの
  APOProcess コールバック) を別の領域 (安全な Rust 型、所有権、
  ライフタイム) に変換します。

2 つめの単語 `apo` は Microsoft による Audio Processing Object の略称
です。

## ステータス

**意図した機能は実装完了。** 上記「スコープに含まれるもの」の各項目は
すべて実装済みです。

- ✅ COM オブジェクト基盤 (IUnknown, IClassFactory, 登録)
- ✅ 必須インターフェース — `IAudioProcessingObject` ファミリと
  `IAudioSystemEffects` v1/v2/v3
- ✅ SFX・MFX・EFX カテゴリ
- ✅ AEC APO 対応 (基盤 + COM ブリッジ + `aec` フィーチャー派生)
- ✅ フォーマットネゴシエーション (`WAVEFORMATEX` +
  `WAVEFORMATEXTENSIBLE`)
- ✅ リアルタイム安全なプリミティブ (ロックフリーリングバッファ、
  アトミック状態、アトミック参照カウント)
- ✅ 登録補助 (CLSID、レジストリ、INF 生成、FxProperties エンド
  ポイントバインディング)
- ✅ サンプル APO (`passthrough`, `gain`, `aec_scaffold`)

CI は Tier 1 (fmt, clippy, build/test)、Tier 2 (複数 DLL の
エクスポート・依存関係・署名検証)、Tier 3 (AEC 派生を含む COM
ライフサイクルハーネスと AddressSanitizer nightly) をカバーします。
API 設計は [`architecture.md`](architecture.md)、CI 戦略は
[`testing.md`](testing.md) を参照してください。

## 対象読者

- ユーザーに仮想オーディオデバイスをインストールさせずに、システム
  全体で動作するオーディオエフェクトを必要とする Windows 向け
  オーディオアプリの Rust 開発者
- Windows Audio Engine をターゲットにするプラグイン作者
- Windows Audio Engine の層でオーディオ処理パイプラインを統合したい
  研究者

以下の用途は想定していません:

- アプリケーションレベルのオーディオ再生 (`cpal`, `wasapi-rs`、または
  `windows` クレートを直接利用してください)
- DAW 固有のプラグイン形式 (VST3, ASIO) — 全く異なる API
- クロスプラットフォームのプラグイン形式 (LADSPA, LV2 は Linux 中心)

## 既存実装との比較

### `windows` クレート (生の COM) との比較

Microsoft が公式に保守する `windows` クレートは、APO インターフェース
を含む Windows API 全体に対する Rust バインディングを提供します。これを
直接用いて APO を実装することは可能ですが、以下が必要になります。

- `IUnknown`, `IClassFactory`, COM ライフタイムプロトコルの手書き実装
- vtable の手書き構築、または `implement!` マクロの大量利用
- どのメソッドがリアルタイム安全でどれがそうでないかについての
  百科事典的な知識

`tympan-apo` は `windows` の上に (依存上必然的に) 構築されますが、より
高水準の抽象を提供し、利用者は完全な COM インターフェース群ではなく
`ProcessingObject` トレイトを実装します。

### Equalizer APO との比較

[Equalizer APO](https://sourceforge.net/projects/equalizerapo/) は最も
著名なオープンソース APO です。C++ で実装され、実行時に設定される
パラメトリックパイプラインによってシステム全体の DSP を提供します。
これは APO アーキテクチャが汎用 DSP を支えられることの実例ですが、
C++ 専用であり、他プラグインのためのフレームワークとしては設計され
ていません。

`tympan-apo` は精神的に最も近い類縁です。APO 機構を介したサードパー
ティ DSP を可能にする点で同じですが、Rust で、再利用可能なライブラリ
として提供します。

### SYSVAD AEC サンプルとの比較

Microsoft の [Windows-driver-samples](https://github.com/microsoft/Windows-driver-samples)
リポジトリには `audio/sysvad/` の下に AEC APO サンプルが含まれます。
これは Windows 11 AEC APO API の正典的リファレンスです。
`tympan-apo` は同じ API 表面を採用しつつ、それを Rust のイディオムで
公開します。

## 登録とデプロイ

APO は COM のインプロセスサーバです。デプロイは以下の手順を含みます。

1. APO を `cdylib` として `.dll` にビルド
2. `regsvr32` で COM クラスを登録
   (`HKLM\SOFTWARE\Classes\CLSID\{...}` に CLSID エントリを書き込み)
3. 対象オーディオエンドポイントと APO を関連付けるためのレジストリ編集
   (`HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture\{device-guid}\FxProperties`)
4. Windows オーディオサービスの再起動または OS の再起動

フレームワークはこれらの登録手順をカバーする INF ファイル生成の
ビルドスクリプトヘルパーと、インストール時に呼び出せる PowerShell
スニペットを提供します。

### 注意事項

- Windows Update がオーディオドライバを再インストールする際に APO
  の登録が上書きされることがあります。これはサードパーティ APO の
  既知の制限です (Equalizer APO のユーザーも日常的に遭遇します)。
  フレームワークでは防止できませんが、復旧手順は文書化します。
- コード署名: EV コード署名証明書があればインストール時の SmartScreen
  警告を回避できますが、APO のロードに厳密に必須ではありません。
  CLSID が登録されていれば、オーディオエンジンは未署名の APO を受け
  入れます。
