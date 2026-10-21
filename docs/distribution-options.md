# 配布方法の調査: Microsoft Store / winget

署名のないインストーラーを GitHub Releases から配布しているため、Windows SmartScreen の警告が出る。費用を抑えて警告を減らす手段として、Microsoft Store と winget での配布を調べた結果をまとめる。調査日は 2026-10-21。事実は Microsoft の公式ドキュメントで裏取りし、確認できなかったものは「未確認」と明記する。

## 結論(推奨)

1. **winget への掲載を採用する**(費用ゼロ、署名不要、工数は小さい)。ただし「SmartScreen の警告がなくなる」ことは公式には保証されていないため、掲載後に実機で確認する。
2. **Microsoft Store は、今は見送る**。MSIX なら署名の費用はかからず SmartScreen の警告も出ないが、このアプリには**大きな前提の変更**が要る(下の「Store の障害」)。v1.0.0 の後に、別の大きな案件として再検討する。
3. コード署名の証明書は、引き続き購入しない。個人で日本在住の場合、Microsoft の署名サービス(Artifact Signing)は使えない(下記)。

## 現状の配布で起きていること

署名のないファイルは、バージョンごとに SmartScreen の評価がゼロから始まり、警告が出る。評価が溜まって警告が消えるまでには、数週間かかり、幅広い利用者の数百回規模のインストールが要る(Microsoft の説明)。署名なしの場合は、新しいバージョンのたびにやり直しになる。EV 証明書も、いまは SmartScreen の警告を回避しない。([SmartScreen reputation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation))

## 比較表

| | 費用 | コード署名 | SmartScreen の警告 | 更新 | 工数 | 判断 |
|---|---|---|---|---|---|---|
| **GitHub Releases(現状)** | 0 | なし | **出る**(版ごとに評価ゼロから) | アプリ内更新(v0.8.5〜) | — | 継続 |
| **winget** | 0 | **不要**(maintainer の回答) | 公式な記載なし(**未確認**。下記) | `winget upgrade` と、アプリ内更新が併存 | 小(マニフェストを版ごとに PR。自動化できる) | **採用** |
| **Store(MSIX)** | 0(個人の登録が無料。本人確認が必要) | **不要**(Store が Microsoft の証明書で署名し直す) | **出ない**(Microsoft の説明) | Store が配信 | **大**(下記) | 見送り |
| **Store(EXE/MSI の提出)** | 証明書の費用が掛かる | **必須**(CA 発行の証明書。自己署名は不可) | Store 経由なら出ない | アプリ側の責任 | 中 | 不採用(費用の問題) |
| コード署名の購入(OV 証明書) | 年 約 $150〜300(目安) | あり | **出る**(評価が溜まるまで)。EV でも同じ | — | 小 | 今回は対象外 |
| Artifact Signing(旧 Trusted Signing) | 月 約 $9.99 | あり | 同上(評価が溜まるまで) | — | 中 | **個人・日本在住は対象外** |

### Microsoft Store

- **登録**: 個人の開発者の登録は 2025 年 9 月から無料。政府発行の ID の撮影と、自撮りによる本人確認が要る。個人から会社のアカウントへの変更はできない。([Windows Developer Blog](https://blogs.windows.com/windowsdeveloper/2025/09/10/free-developer-registration-for-individual-developers-on-microsoft-store/))
- **署名と SmartScreen**: MSIX で提出すると、Store が提出後に Microsoft の証明書で署名し直すので、CA の証明書は不要。Store 経由でインストールしたアプリは、SmartScreen のダウンロード警告の対象にならない。([MSIX の要件](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements)、[SmartScreen reputation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation))
- **EXE/MSI のまま提出する道**もあるが、その場合は Store が署名しない。Microsoft の信頼されたルートにつながる CA の証明書で、自分で署名する必要がある。つまり、費用の問題は解決しない。([配布経路の選び方](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/choose-distribution-path))
- **更新**: MSIX は Store が更新を配信する。EXE/MSI 提出は、アプリ側の責任。

#### Store(MSIX)の障害 — このアプリに固有の事情

1. **データの置き場所**: このアプリは、データ(DB、サムネイル、ログ、バックアップ)を、**実行ファイルと同じ場所**(`%LOCALAPPDATA%\GrayBrowser\`)に置く(`gb_core::paths::resolve_app_dir`)。MSIX のインストール先は読み取り専用なので、そのままでは動かない。データの置き場所の再設計と、既存の利用者のデータの移行が必要になる。データを失う危険のある、慎重な作業になる。
2. **MSIX のパッケージ化**: Tauri v2 は MSIX を公式にはサポートしていない(公式ドキュメントに記載なし。第三者の報告では独自に組む必要がある)。マニフェスト、アセット、署名なしのパッケージの生成を、CI に自作する必要がある。
3. **FFmpeg が同梱されていない**: アプリは、外部の FFmpeg / FFprobe を PATH から探す。Store の認証は、追加のソフトを前提にしないことを求める方向なので、そのままでは認証に通らない恐れがある(公式の方針の全文は確認できていない: **未確認**)。同梱すると、サイズとライセンスの対応が増える。
4. **アプリ内更新との併存**: Store 版は Store が更新するので、アプリ内更新(v0.8.5〜)を無効にする別ビルドが要る。配布経路が 2 本になり、ビルドとテストが増える。
5. その他: プライバシーポリシーの URL、年齢区分、ストアの説明文とスクリーンショットなど、Store の掲載の準備も要る。

→ SmartScreen の警告を消す効果は大きいが、1〜3 だけでも大きな案件になる。今のタイミング(v1.0.0 の前)に混ぜるべきではない。

### winget

- **掲載手順**: `microsoft/winget-pkgs` に、マニフェスト(版ごとに 1 つ)を Pull Request で提出する。自動検証(マニフェストの形式、URL、ウイルス対策ソフトによる検査、インストールとアンインストールの試験)のあと、人間のレビューを経て掲載される。費用はかからない。([提出の手順](https://learn.microsoft.com/en-us/windows/package-manager/package/repository))
- **要件**: インストーラーの URL は、**提供元の公開場所から直接**(リダイレクトなし、HTTPS)。無人(サイレント)でインストールでき、管理者と非管理者の両方でインストール・アンインストールできること。インストーラーの SHA256 を記載する。GitHub Releases の URL はこの条件を満たす。NSIS のインストーラーは、サイレントインストールに対応している。
- **署名**: **不要**。winget の maintainer は「WinGet doesn't require code signing, but some users are likely to be concerned about unsigned installers.」と回答している。([winget-cli Discussion #4327](https://github.com/microsoft/winget-cli/discussions/4327))署名が要るのは、MSIX 形式のときだけ。
- **SmartScreen への効果(未確認)**: 公式のドキュメントに、署名なしの exe を winget でインストールしたときの SmartScreen の挙動の記載はない。winget は、ブラウザ経由でないダウンロードなので、ブラウザが付ける「Mark of the Web」が付かず、警告が出にくいという第三者の説明はあるが、公式には確認できていない。また、署名なしの exe は、評価がゼロのため、Microsoft Defender が警告する例が報告されている。**掲載後に、実機でインストールして確認する必要がある。**
- **リスク**: 自動検証のウイルス検査で、署名のない Tauri の exe が誤検知されると、掲載が拒否される(Binary-Validation-Error)。その場合は、Microsoft Defender に誤検知として提出して解消する手順がある。
- **更新**: `winget upgrade` で更新できる。アプリ内更新とは別の経路だが、競合はしない(どちらも新しいインストーラーを実行するだけ)。掲載後の更新は、マニフェストの PR を版ごとに出す必要があるが、`wingetcreate` や GitHub Actions で自動化できる。

### 署名サービス(参考。今回は対象外)

Azure Artifact Signing(旧 Trusted Signing)は月 約 $9.99 だが、**個人の開発者は、米国とカナダ在住の場合に限られる**。日本では、組織(法人)としてのみ利用できる。個人で日本在住の場合は、CA の OV 証明書(年 約 $150〜300)が選択肢になる。いずれも、署名しただけでは SmartScreen の警告はすぐには消えず、評価が溜まるまで待つ必要がある。([Quickstart: Set up Artifact Signing](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart)、[配布経路の選び方](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/choose-distribution-path))

## 推奨する次の作業

1. **winget への掲載**(別 Issue): マニフェストの作成と初回の PR、リリース時のマニフェスト更新の自動化(タグの push 後に PR を出す仕組み)、掲載後の実機確認(SmartScreen の挙動)、README への `winget install` の案内。
2. **Store(MSIX)**は、v1.0.0 の後に、大きな案件として別途検討する。その前提として、データの置き場所の再設計(実行ファイルと切り離す)が、独立して価値のある改善になる。

## 参照した情報源

- [Choose a distribution path for your Windows app](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/choose-distribution-path)
- [SmartScreen reputation for Windows app developers](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)
- [App package requirements for MSIX app](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements)
- [Free developer registration for individual developers on Microsoft Store](https://blogs.windows.com/windowsdeveloper/2025/09/10/free-developer-registration-for-individual-developers-on-microsoft-store/)
- [Submit your manifest to the repository (winget)](https://learn.microsoft.com/en-us/windows/package-manager/package/repository)
- [winget-cli Discussion #4327: Is code signing necessary?](https://github.com/microsoft/winget-cli/discussions/4327)
- [Quickstart: Set up Artifact Signing](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart)
- [Tauri v2 Windows Installer](https://v2.tauri.app/distribute/windows-installer/)
