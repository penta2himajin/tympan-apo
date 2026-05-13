# テストと CI

*他の言語で読む: [English](../testing.md).*

本ドキュメントは `tympan-apo` のテストおよび継続的インテグレーション
戦略を記述します。GitHub ホストの Windows ランナー上で自動的に検証
できるもの、手動または自己ホスト実行が必要なもの、および Windows
Audio Engine のアーキテクチャが課す制約を扱います。

決定そのものは
[`docs/decisions/0001-ci-verification-strategy.md`](../decisions/0001-ci-verification-strategy.md)
に記録しています。本ドキュメントは各階層がどのように動くかを示す運用
リファレンスです。

## 階層化された検証戦略

検証は深さと環境要件によって 4 つの階層に整理します。各階層は前の階層
を包含します。下位の階層は全プルリクエストで実行され、上位の階層は
スケジュール実行またはオンデマンドで実行されます。

### Tier 1: 静的検証および単体テスト

任意の GitHub ホストの Windows ランナーで実行可能な、標準 Rust
ツールチェーンのチェック。

| チェック | コマンド | 目的 |
|---|---|---|
| ビルド | `cargo build --release --target x86_64-pc-windows-msvc --all-targets` | クレートフィーチャ全体にわたるコンパイル |
| テスト | `cargo test` | COM 起動を要しないロジックの単体テスト |
| Lint | `cargo clippy --all-targets -- -D warnings` | プロジェクト固有のリアルタイム安全性 lint を含む |
| Format | `cargo fmt --check` | スタイル一貫性 |
| ドキュメント | `cargo doc --no-deps --document-private-items` | ドキュメント網羅性と rustdoc エラー |
| グローバル状態なし | `! git grep -nE 'static\s+mut' -- src/` | `CLAUDE.md` 規則と ADR の機械的強制 |

全プルリクエストで必須。所要時間: `windows-2025` / `windows-latest`
で 3-7 分。

### Tier 2: DLL と COM ABI 検証

ビルドされた `cdylib` がロード可能な APO COM in-process サーバとして
必要な構造的性質を備えていることを検証します。

| チェック | ツール | 目的 |
|---|---|---|
| エクスポートエントリポイント | `dumpbin /exports target\release\*.dll` | `DllGetClassObject`, `DllCanUnloadNow`, `DllRegisterServer`, `DllUnregisterServer` が存在しマングルされていない |
| INF 検証 | `infverif /v /w packaging\*.inf` | コンポーネント化 APO INF の正しさ (WDK 拡張) |
| モジュール依存 | `dumpbin /dependents` | `audioenginebaseapo.lib`, `propsys.lib`, `combase.dll` 系のみ — 予期しないユーザモード依存なし |
| ABI サイズ | コンパイル時の `static_assertions` | ブリッジされた構造体 (`WAVEFORMATEXTENSIBLE`, `APO_CONNECTION_PROPERTY`) のサイズが C 定義と一致 |
| アドホック署名スモーク | `signtool sign /fd SHA256 /a /n "Test Cert"` の後に `signtool verify /pa` | 署名経路が正しく配線されている (実証明書は検証しない) |

ABI サイズチェックは `windows` クレートが生成する型に対する
`static_assertions::assert_eq_size!` を用いて、ランタイム前にレイアウト
ドリフトを捕捉します。

ビルド可能な `cdylib` の例が存在するようになり次第、全プルリクエストで
実行します。

### Tier 3: in-process COM 起動

`audiodg.exe` やオーディオエンドポイントを介さず、Rust 統合テスト
プロセスから標準の COM 起動経由で APO のライフサイクル全体を駆動します。

この階層が Windows で利用可能なのは、
[Microsoft 公式ドキュメント][impl-apo] がオーディオエンジン自身も
APO を `CoCreateInstance` で起動すると確認しており、4 つのライフ
サイクルメソッド (`CoCreateInstance`, `IsInputFormatSupported`,
`IsOutputFormatSupported`, `LockForProcess`) はオーディオエンジンが
駆動するメソッドと同一だからです。`windows` クレートまたは
`libloading` を使うテストハーネスがこの駆動シーケンスを再現できます。

[impl-apo]: https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/implementing-audio-processing-objects

テスト毎のシーケンス:

1. `regsvr32 /s target\release\example_apo.dll` (ユーザハイブで管理者
   不要)
2. テストプロセスが `CoInitializeEx(COINIT_MULTITHREADED)` を呼ぶ
3. `CoCreateInstance(CLSID_EXAMPLE_APO, ..., IID_IAudioProcessingObject)`
4. 駆動: `Initialize` → `IsInputFormatSupported` →
   `LockForProcess` → `APOProcess` (合成バッファでループ) →
   `UnlockForProcess`
5. `APOProcess` 呼び出しは `assert_no_alloc` のグローバルアロケータ
   ガード下で実行します。アロケーションがあればテストを失敗させ、
   `CLAUDE.md` 禁止事項 #1 を機械的に強制します。
6. 出力バッファのアサーション: `NaN` なし、`±Inf` なし、プラグインごと
   の解析的境界 (例: `gain` サンプルでは output RMS = input RMS × gain)
7. 後処理として `regsvr32 /u /s` で登録解除

追加の Tier 3 ジョブ:

- **AEC APO バリアント**: 同じシーケンスを
  `IApoAcousticEchoCancellation` と合成 auxiliary input ストリーム
  で実施。`aec` cargo フィーチャでゲート。
- **失敗カウンタ挙動**: `IsInputFormatSupported` から失敗 HRESULT を
  意図的に 10 回返し、フレームワークが警告を出力することを検証。
  実オーディオエンジンが `PKEY_Endpoint_Disable_SysFx` を立てる閾値
  に対応。
- **AddressSanitizer**: nightly Rust で
  `RUSTFLAGS="-Zsanitizer=address"`、同じフィクスチャを並行ジョブで
  実行。FFI 境界の UB を捕捉。

`main` への merge ごと、および日次スケジュールで実行。

### Tier 4: 実オーディオエンジン統合

実際の Windows オーディオサービス経由で APO を `audiodg.exe` にロード。
標準の GitHub ホストランナーではスコープ外。理由:

- Windows Server 2025 (ホストランナーの基盤) では Windows Audio
  Service (`AudioSrv`) は既定で無効
- サービスを起動しても、ホストランナーには APO を `FxProperties`
  経由でバインドできる物理/仮想オーディオエンドポイントが存在しない
- macOS の HAL プラグインローディングモデル (SIP 有効なランナーでも
  `coreaudiod` 下にアドホック署名のプラグインをロード可能) と同等の
  仕組みはない — オーディオエンジンは FxProperties 経路に実 `MMDevice`
  エンドポイントを要求する

実施手段:

- 開発者ローカルの Windows マシン (PR レビュー時)
- GitHub Actions に登録した Windows ワークステーション (自己ホスト
  ランナー)
- 仮想オーディオデバイス (VB-CABLE, Scream 等) を導入したクラウド
  Windows サービス (Azure Windows 11 デスクトップ, AWS EC2 Windows)

スコープ:

- APO をエンドポイントの `FxProperties` レジストリキーへバインド
- `Restart-Service AudioSrv` の後、`audiodg.exe` が
  `PKEY_Endpoint_Disable_SysFx` にフォールバックせずに APO をロード
  することを検証
- ETW 取得: 代表ワークロードで `wpr -start AudioGlitches.wprp` を
  動かし、glitch 数と APO レイテンシを点検
- DAW / 通信モードアプリでの聴感テスト
- 製品版オーディオエンジンでの WHQL / EV 署名検証

## GitHub ホスト Windows ランナー

現行ランナー (2026年5月時点):

| ラベル | OS | アーキテクチャ | スペック | 公開リポでのコスト |
|---|---|---|---|---|
| `windows-2025` / `windows-latest` | Windows Server 2025 (Build 26100) | x86_64 | 4 vCPU, 16 GB RAM, 14 GB ディスク | 無料 |
| `windows-2022` | Windows Server 2022 | x86_64 | 同等 | 無料 |
| `windows-11-arm` | Windows 11 ARM | arm64 | 4 vCPU, 16 GB RAM | 無料 |
| `windows-latest-l` (large) | Windows Server 2025 | x86_64 | 8+ vCPU | 有料のみ |

公開リポジトリは全 GitHub プランで標準ランナーの無料分が無制限です。

`tympan-apo` は公開リポジトリなので、標準ランナーのコスト制約はあり
ません。ARM64 ランナーは ARM Windows 11 デバイス向けの
`aarch64-pc-windows-msvc` cdylib ビルドを可能にします。

### ランナーイメージのインベントリ

GitHub がイメージごとのソフトウェアインベントリを公開しています。APO
開発に関連する構成は以下のとおりです。

- Visual Studio Enterprise 2022 (17.14+) — フル MSVC ツールチェーン
- Windows SDK 10.1.26100.x (Windows 11 24H2) — Windows 11 AEC APO
  API 要件 (23H2+) を満たす
- Windows Driver Kit Visual Studio Extension 10.0.26100.x —
  `infverif.exe`, `audioenginebaseapo.h` の WDK ヘッダを提供
- Visual Studio 開発者コマンドプロンプト環境で `dumpbin.exe`,
  `signtool.exe`, `regsvr32.exe`, `reg.exe` が PATH に
- Rust stable + コンポーネント (rustup プリインストール)

テストハーネスでのアドホック署名や未署名 APO ロードに Microsoft
パートナーセンター登録は不要です。

## Windows Audio Service の考慮事項

GitHub ホストランナーで用いられる Windows Server SKU での観察:

| 操作 | 可否 | 備考 |
|---|---|---|
| APO DLL の `regsvr32` | 可 | ユーザハイブ (`HKCU\Software\Classes`) なら管理者不要 |
| APO CLSID の `CoCreateInstance` | 可 | Audio Service 状態に依存しない |
| `IAudioProcessingObject*` メソッドの駆動 | 可 | インターフェースは純粋な COM、オーディオエンジン関与不要 |
| `AudioSrv` の起動 | 可能 | `Set-Service -Name AudioSrv -StartupType Automatic; Start-Service AudioSrv` (Server SKU は既定で無効) |
| `MMDeviceEnumerator::EnumAudioEndpoints` | エンドポイント 0 件 | 物理/仮想サウンドカードが存在しない |
| FxProperties 経由で APO をエンドポイントにバインド | 不可 | 既存の `MMDevice` エンドポイントが必要 |
| `audiodg.exe` が APO をロード | 不可 | エンドポイントなし、audiodg パイプライングラフなし |
| WHQL テスト署名 | 不可 | Microsoft HLK サーバ送信が必要 |

重要な観察: **オーディオエンドポイントが存在しなくとも、APO の COM
起動とライフサイクル駆動はホストランナー上で完全に機能します**。
これが Tier 3 自動化を可能にする技術的基盤であり、LADSPA や macOS HAL
版の兄弟フレームワークが同程度には共有していない特性です。

## GitHub ホストランナーで検証できないもの

標準ランナー環境の確固たる限界:

- **物理スピーカーへのオーディオ出力** — ランナーにはアプリケーション
  に公開されるオーディオ出力ハードウェアが存在しない
- **マイクキャプチャ** — AEC リファレンスストリームテスト用も含めて、
  入力デバイスが存在しない
- **`audiodg.exe` レベルの統合** — エンドポイントなしではオーディオ
  エンジンパイプラインを組み立てられない
- **長時間安定性** — ジョブは 6 時間でタイムアウト。現実的な安定性
  テストはオーディオ負荷を継続して日単位で実行する
- **WHQL 署名済みドライバフロー** — WHQL は有料証明書を伴う Microsoft
  HLK サーバ送信が必要で、CI で commit ごとに実施することはできない
- **Windows Update 再登録シナリオ** — ドライバ再インストールによる
  上書きの検証には Windows Update イベントが必要で、CI では再現不可
- **通信モードアプリの挙動** — Teams, Discord, WhatsApp 等はログイン
  UI セッションを要求し、Server ランナーでは信頼性高く提供できない

これらのギャップが Tier 4 の手動 / 自己ホスト検証を必要とします。

## 自己ホストの代替手段

Tier 4 を自動化する必要が生じた場合の選択肢:

### 自己ホスト GitHub Actions ランナー

開発者の Windows マシンを GitHub Actions ランナーとして登録します。
個人開発に費用対効果が高い反面、CI 実行時にマシンを起動かつネットワーク
接続しておく必要があります。

公開リポジトリでは自己ホストランナーのプラットフォーム料金は無料。
非公開リポジトリは 2026 年 3 月から $0.002/min のプラットフォーム料金。

登録手順:

1. Settings > Actions > Runners > New self-hosted runner
2. 対象 Windows マシンで表示されたインストールスクリプトを実行
3. 必要に応じてランナーを Windows サービスとして登録し自動起動

Tier 4 には、加えて実または仮想のオーディオエンドポイント (VB-CABLE,
Scream, または物理サウンドデバイス) を登録マシン上に用意します。

### Windows-in-cloud サービス

| サービス | モデル | 概算コスト | 適用場面 |
|---|---|---|---|
| Azure Virtual Desktop / Windows 11 Cloud PC | 時間単位 | $0.10-0.40/hr | 永続状態、フル UI セッション |
| AWS EC2 Windows (`m5.large` 等) | 時間単位 | $0.10-0.20/hr | アドホック検証、仮想オーディオ追加可能 |
| GitHub Actions large Windows ランナー | 分単位 | $0.016/min | パイプライン統合、ただし仮想オーディオなし |
| Microsoft Dev Box | 月単位 | $30-100/mo 程度 | 永続的な開発環境 |

Tier 4 をパイプラインの一部として自動実行する必要があり、ローカル
開発マシンでは不十分な場合 (リリース検証等) に適合します。

## 推奨ワークフローファイル

実装開始後の `.github/workflows/` レイアウト:

```
.github/workflows/
├── tier1.yml           # cargo build/test/clippy/fmt/doc を毎 PR
├── tier2.yml           # DLL エクスポート、INF、ABI サイズを毎 PR
├── tier3.yml           # in-process COM 起動を merge 時 + nightly
└── release.yml         # タグ付きリリースの publish (cargo publish dry-run)
```

Tier 4 はワークフロー集合からは意図的に除外します。標準パイプライン
の外で、手動または自己ホストランナー上で実施します。

## CI におけるリアルタイム安全性の強制

Clippy が提供する lint に加えて、フレームワークはリアルタイム
コード経路にリアルタイム非安全なパターンが現れた場合に CI を失敗
させる、プロジェクト固有の lint を定義します。

`CLAUDE.md` 禁止事項と CI 強制手段の対応表:

| 禁止事項 | 強制手段 | Tier |
|---|---|---|
| 1. `APOProcess` 内のアロケート禁止 | Tier 3 統合テストでの `assert_no_alloc` ガード | 3 |
| 2. リアルタイムで `std::sync::Mutex::lock()` 禁止 | `realtime` モジュール向け `clippy.toml` の `disallowed-methods` | 1 |
| 3. async ランタイム禁止 | `cargo deny bans` で `tokio`, `async-std` 禁止 | 1 |
| 4. Windows オーディオ以外の C ライブラリ依存禁止 | native-dep クレートの `cargo deny` 許可リスト | 1 |
| 5. ドキュメントなしの公開 `unsafe fn` 禁止 | `clippy::missing_safety_doc` を deny 設定 | 1 |
| 6. リアルタイムで blocking syscall 禁止 | Tier 4 の ETW 取得 (CI では機械的に検証不可) | 4 |

これらの lint は以下の手段で強制します。

- `realtime` モジュールの許可される依存とメソッドを制限するカスタム
  `clippy.toml` 構成
- リアルタイム非安全な推移的依存の誤導入を防ぐ `cargo-deny` 規則
- モジュールレベル属性でのコンパイル時 `#[deny(...)]` 指示

最初のリアルタイムモジュールが入った時点で実装詳細を追加します。

## 兄弟 tympan クレートとの比較

| 観点 | tympan-ladspa | tympan-aspl | tympan-apo |
|---|---|---|---|
| ホスト OS | Linux | macOS | Windows |
| build/test/lint | Tier 1 | Tier 1 | Tier 1 |
| ABI / バンドル検証 | Tier 1 (`nm`) | Tier 2 (`plutil`, `lipo`) | Tier 2 (`dumpbin`, `infverif`) |
| CI 上のプラグインライフサイクル | Tier 2 (`applyplugin`) | Tier 3 (`coreaudiod` 下の HAL ロード) | Tier 3 (in-process `CoCreateInstance`) |
| CI 上の sanitizer | Tier 2 ASan, Tier 3 TSan | (記載なし) | Tier 3 ASan |
| 実オーディオ I/O | スコープ外 (Tier 4 手動) | スコープ外 (Tier 4 手動/自己ホスト) | スコープ外 (Tier 4 手動/自己ホスト) |

APO 移植版が異例にクリーンな Tier 3 経路を持つのは、COM 起動モデルが
オーディオエンジンパイプラインから分離されているからです。LADSPA 版は
`applyplugin` (外部 SDK ツール) を必要とし、ASPL 版は HAL プラグイン
配置を伴う実 `coreaudiod` の再起動を必要とします。

## 実装状況

CI は未構成です。最初のソースコードコミットと同時に実装する計画です。
初期 CI 構成は Tier 1 と 2 をカバーし、最初の APO サンプルがビルド
可能になった時点で Tier 3 を追加します。Tier 4 は無期限に手動、あるいは
オーディオハードウェアを持つ自己ホストランナーがプロジェクトに加わる
まで手動のままです。
