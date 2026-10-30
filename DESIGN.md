---
version: alpha
name: Sumi / blender-live
description: 常駐したBlenderの書き出し・ジョブ・状態をブラウザで確かめる画面のデザイン。
colors:
  primary: "#a84d00"
  accent: "#a84d00"
  accent-subtle: "rgba(168, 77, 0, 0.10)"
  surface: "#faf6ef"
  surface-raised: "#fffdf8"
  on-surface: "#3a2f28"
  muted: "#6f6257"
  border: "#e3d9c9"
  scrim: "rgba(58, 47, 40, 0.4)"
  link: "#14506e"
  danger: "#9c2b1d"
  danger-subtle: "#f9e9e4"
  wash-base: "#f6efe0"
  wash-raised: "#faf4ea"
  hover-1: "rgba(168, 77, 0, 0.10)"
  hover-2: "rgba(168, 77, 0, 0.16)"
typography:
  title:
    fontFamily: system-ui
    fontSize: 17px
    fontWeight: 600
    lineHeight: 1.3
  body:
    fontFamily: system-ui
    fontSize: 16px
    fontWeight: 400
    lineHeight: 1.6
  body-sm:
    fontFamily: system-ui
    fontSize: 14px
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: system-ui
    fontSize: 15px
    fontWeight: 500
    lineHeight: 1.2
  caption:
    fontFamily: system-ui
    fontSize: 12px
    fontWeight: 400
    lineHeight: 1.4
rounded:
  sm: 6px
  md: 8px
  lg: 12px
  full: 9999px
spacing:
  sp-1: 4px
  sp-2: 8px
  sp-3: 12px
  sp-4: 16px
  sp-5: 24px
components:
  app-header:
    backgroundColor: "{colors.wash-base}"
    textColor: "{colors.on-surface}"
    height: 48px
  sub-header:
    backgroundColor: "{colors.wash-raised}"
    textColor: "{colors.on-surface}"
    height: 40px
  hairline:
    backgroundColor: "{colors.border}"
    height: 1px
  card:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.md}"
    padding: 10px
  button:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: 8px
  button-hover:
    backgroundColor: "{colors.hover-1}"
  button-pressed:
    backgroundColor: "{colors.hover-2}"
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.surface-raised}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: 8px
  button-quiet:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.muted}"
    rounded: "{rounded.sm}"
  icon-button:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.sm}"
    size: 36px
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.on-surface}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    padding: 8px
  modal:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.lg}"
    padding: 16px
  modal-scrim:
    backgroundColor: "{colors.scrim}"
  radio-selected:
    backgroundColor: "{colors.accent-subtle}"
    rounded: "{rounded.sm}"
  link:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.link}"
  error-banner:
    backgroundColor: "{colors.danger-subtle}"
    textColor: "{colors.danger}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.sm}"
    padding: 8px
  spinner:
    textColor: "{colors.accent}"
    size: 18px
  badge:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.muted}"
    typography: "{typography.caption}"
    rounded: "{rounded.full}"
    padding: 4px
---

# blender-live

## Overview

coding agentがMCPで操縦するBlenderの成果物を、人がブラウザで確かめる画面。
主役は書き出し（静止画・動画）で、ジョブの進み具合とBlenderの状態はその判断材料として添える。
操作はジョブの取り消しとダウンロードだけとし、シーンの編集はagentに任せる。

rust-svelte-templateのSumi familyの原本から取り込み、以後は本書を正とする。
アクセントはBlenderを連想させる橙の明暗の組、テーマの保存キーは `blender-live:theme` とする。

## Colors

暗色はSumi、通常画面の明色はKinariとする。Sumiから設計し、両方で検証する。
Washiはe-paperを実際に扱う派生製品がKinariと置き換える。2つの明色テーマを重ねない。
色は `client/src/global.sass` のCSS変数を使う。frontmatterはKinariの値を持つ。

| 役割 | Kinari | Sumi |
|---|---|---|
| surface | #faf6ef | #191919 |
| surface-raised | #fffdf8 | #232323 |
| on-surface | #3a2f28 | #e6e6e6 |
| muted | #6f6257 | #9a9a9a |
| border | #e3d9c9 | #333333 |
| accent / primary | #a84d00 | #f5933a |
| accent-subtle | rgba(168,77,0,.10) | rgba(245,147,58,.15) |
| link | #14506e | #7fdbff |
| danger | #9c2b1d | #ff6b6b |
| danger-subtle | #f9e9e4 | #3a1a1a |
| scrim | rgba(58,47,40,.4) | rgba(0,0,0,.6) |
| wash-base | #f6efe0 | #232323 |
| wash-raised | #faf4ea | #191919 |
| hover-1 | rgba(168,77,0,.10) | #333333 |
| hover-2 | rgba(168,77,0,.16) | #3d3d3d |

`primary` はlint用に `accent` と同値を持つ。製品の色名はaccentを使う。
背景・補助情報・通常操作は無彩色を基本とする。Kinariでは表の淡い色を許す。
帯にはwash、ホバーにはhoverの変数を使い、accent-subtleを直接流用しない。
アクセントは主操作・フォーカス・小さな選択表示・処理中のスピナーに使う。
塗りつぶす主操作は画面に最大1つとし、領域を分けて増やさない。
大きな選択行は背景の濃淡で示し、強いアクセントの面と縁取りを重ねない。
小さなラジオの選択表示にはaccent-subtleを使える。

副アクセントは、主アクセントと異なる持続的な役割がある場合だけ製品側で定める。
分類などのデータ色は操作の色と分け、製品側で用途を定める。
色の意味は文字・形・アクセシビリティ属性でも示す。
通常文字は両テーマでWCAG AAの4.5:1以上を保つ。主ボタンの文字色はsurface-raisedとする。

### テーマの状態

`:root` をSumiの値と `color-scheme: dark` にする。
`data-theme="light"` でKinari、`data-theme="dark"` でSumiを明示指定する。
自動では属性と保存キーを削除し、OSの `prefers-color-scheme` に従う。
Kinariの明示指定とOS委任は同じSass mixinから出力し、`color-scheme: light` を設定する。
設定はlocalStorageへ保存し、初回描画前に適用する。状態をコンポーネント内だけに保持しない。

## Typography

書体はsystem-uiとし、Webフォントを追加しない。サイズ・太さ・行高はfrontmatterの5役割を使う。
タイトルは1行で省略し、本文は16px以上とする。補助文は14px、ラベルは15px、注記は12pxを使う。
注記はデータ色を持つ場合を除いてmutedとする。階層は色と太さで示し、独自サイズを増やさない。

## Layout

### 内容と補助情報

一覧や本文を最初の画面で見せる。現在地・選択対象・件数が既存の表示で分かるなら、説明帯を追加しない。
検索・主要操作は見出しの近くへまとめ、ヘルプ・確認・詳細説明は必要な操作から開く。
異常時の通知は必要だが、通常時に空の通知領域を予約しない。

縦方向はページの通常スクロールを使う。表の列見出しも行と一緒に流す。
固定見出しのための内側縦スクロール、高さ制限、残り高さの計算を標準の一覧へ持ち込まない。
狭幅の表は必要な横スクロールだけを表内へ収める。
カード向けの幅を密な表や全画面の画像へ適用しない。製品ごとの用途をroot DESIGN.mdで定める。

### 画面構成

- app headerは全幅・高さ48px・stickyとし、wash-baseと1pxの下境界を使う。
  左にホームへ戻るアプリ名、右に36pxのメニューボタンを置く。リンクはアプリ名だけとする。
- ホームは書き出しを主役とし、状態とジョブはその上に小さくまとめる。
  ジョブはあるときだけ出し、異常の通知も起きたときだけ出す。
- 個別の書き出しを開くと、高さ40pxのsub-headerにファイル名だけを1行で示し、
  その下に画像か動画を幅いっぱい（高さは画面の75%まで）に置く。
- 本文は通常の文書フローで続く。mainに独立した縦スクロール領域を作らない。

ブレークポイントは768px。画像を並べる画面は中央配置で最大1200pxとする。
左右の余白は12px、上下は狭幅16px・広幅24pxとする。幅320px以上でページを横にはみ出させない。
余白は4/8/12/16/24pxを使う。カード内10px、通常ボタンの横14pxは部品固有の値とする。

## Elevation & Depth

階層は面の濃淡と1pxの境界で示す。
影はメニューとモーダルの `0 8px 32px rgba(0,0,0,.25)` だけに使う。
フォーカスは共通の `:focus-visible` に2pxのaccent色の輪郭と2pxの間隔を設ける。
ブラウザ既定の輪郭を抑止する場合も、この可視リングを残す。

## Shapes

角丸は小部品6px、カード8px、モーダルとメニュー12pxとする。
9999pxは件数と状態のバッジだけに使う。同じ操作部品で角丸を混ぜず、円形ボタンを作らない。

## Components

### アイコン

`client/src/lib/Icon.svelte` を辞書の正とする。絵文字や文字記号をアイコンの代わりに使わない。
SVGは24×24、currentColorの2px線、丸い端と角、通常は塗りなしとする。
サイズは1.2emで文字の基準線にそろえる。filled版は同じ形状を塗り、状態は属性でも示す。
列挙には `ICON_NAMES` を使い、別の手書き一覧を作らない。未使用だけを理由に辞書項目を削らない。
汎用的な追加は原本の辞書へ採用し、派生製品が明示的に取り込む。実行時依存やsubmoduleは不要。

### メニューとテーマ設定

メニューは右上のボタンに接するドロップダウンとする。
上端をheader下端、右端をボタン右端へそろえる。最小幅180px、1pxの枠と12pxの角丸を使う。
背景はsurface-raised、項目は全幅の行とし、余白は上下8px・左右12pxを使う。
メニューにscrimは付けない。背面の透明な閉じるボタンで外側クリックを受ける。
Escでも閉じ、フォーカスをメニューボタンへ戻す。開閉は `aria-expanded` に反映する。
先頭は「テーマ設定」、以降は製品の画面リンクとする。ホーム項目は重複するため置かない。

テーマ設定は中央のモーダルで開く。
自動・ライト・ダークの3つのラジオにmonitor/sun/moonのアイコンを使う。
選択は即時反映し、確認できるようモーダルを閉じない。
閉じるボタン・Esc・scrimで閉じ、メニューボタンへフォーカスを戻す。

### 状態・ジョブ・書き出し・ファイル

Blenderの状態は1枚のカードにまとめ、状態名のバッジ、開いている.blend、未保存のバッジ、
版とGPUを置く。状態名は色だけに頼らず文字で示し、処理中はaccent、停止はdangerの縁と文字にする。
落ちて起こし直したときは、状態カードの下にerror-bannerで時刻と失ったものを1文で示し、
原文のエラーは既定で閉じた詳細に入れる。

ジョブは1件1カードとし、名前・段階のバッジ・完了frame数・取り消しのアイコンボタンを1行に並べ、
その下に高さ6pxの進み具合の棒を置く。棒はaccentで塗り、`role="progressbar"` で値を公開する。
完了したジョブの名前は動画を開くリンクにする。

書き出しは新しい順のカードの格子とし、各カードは最小220pxで折り返す。
幅480px以下では2列に固定し、間隔を8pxに詰める。
画像と動画は16:9の枠に収め、余白は黒で埋める。動画はカードの中でそのまま再生できる。
カードの下に名前（1行で省略）と時刻・大きさの注記を置く。名前は個別表示へのリンクとする。

ファイルはscenes・scripts・assetsのタブで切り替え、1行1ファイルの一覧にする。
各行はパス、大きさと時刻の注記、ダウンロードのアイコンボタンとする。狭幅では注記を隠す。

一覧の `data-state` はloading/empty/error/successとする。
読込中は控えめな14pxの文字とスピナーを示す。
空の説明と取得失敗を区別し、失敗時にはdanger色の説明と再試行を示す。
新しい書き出しやジョブの変化はサーバーからの通知で反映し、利用者に再読込を求めない。

### 入力と操作部品

- 通常ボタンはsurface-raised、1px枠、6px角丸、上下8px・左右14pxの余白とする。
  ホバーはhover-1、無効時は不透明度50%でポインターを付けない。
- 主ボタンはaccentで塗り、静かなアイコンボタンは透明背景とする。
- 入力欄はsurface、1px枠、6px角丸、本文サイズとする。
  ラベルは上にmutedの注記で置き、フォーカスはaccent色の枠と共通リングを使う。
- モーダルは中央配置、12px角丸、16px余白とscrimを使う。
  閉じるボタン・Esc・scrimで閉じ、内容は内部でスクロールできる最大80dvhとする。
- 動きは150ms以下の高さ・透明度の変化とスピナーに限る。
  `prefers-reduced-motion: reduce` では両方を止める。

## Verification

実装はSassの字下げ構文を使い、normalize.cssを先に読み込む。
変数の接頭辞は色が `--c-*`、余白が `--sp-1..5`、文字が `--fs-xs..xl`、角丸が `--radius-*` とする。
`designmd lint` は形式を検査する。UIへの適用は実ブラウザで次を確認する。

- 同じデータ・画面サイズで変更前後を比べ、主要情報の面積と見える件数、色の強さを確認する。
  新機能が動いても、重複情報や強い装飾で主役が隠れたら修正する。
- 明暗、320px以上の狭幅、長い名前、空、読込、失敗、キーボードを変更範囲に応じて確認する。
- Sumiの背景色をrgb(25,25,25)にする。Kinariではrgb(250,246,239)となる。
  明示指定がOSより優先され、自動へ戻すと属性と保存キーが消える。
- 375pxのheaderの操作対象はアプリ名とメニューボタンの2つだけとする。
  メニュー開閉時も横にはみ出さず、パネルの位置は指定する端と±1px以内で一致する。
- カード・アイコン・フォーカスの寸法、各状態、メニューとモーダルの閉じ方を部品規則と照合する。

通常フローと余白は `App.svelte` と `global.sass` の `.content` が所有する。

`npm --prefix client run test:e2e` は、stubのBlenderを相手にした実サーバーをChromiumで測る。
