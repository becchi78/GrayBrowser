<#
.SYNOPSIS
  E2E / 実機確認を、開発機の実データに一切触れない隔離ビルド（サンドボックス）
  に対して実行する。

.DESCRIPTION
  背景（`sandboxed-verification` スキル参照）: GB_APP_PATH 未設定のまま E2E / 実機確認
  を実行し、開発機の実データ（target/{debug,release}/GrayBrowser/app.db）に
  書き込みが発生する事故があった。このスクリプトは CARGO_TARGET_DIR を専用の
  サンドボックスディレクトリに切り替えてビルドすることで、既存の target/ には
  一切触れないようにする。さらにビルド成果物のディレクトリに sentinel ファイル
  を作成した上で GB_APP_PATH をそのexeパスに設定する（sentinelのファイル名
  自体はこのスクリプトは知らず、e2e/sandboxGuard.mjs 経由でnodeに作成させる。
  ファイル名リテラルはsandboxGuard.mjs側に一本化されている）。
  e2e/session.mjs 側のガード（e2e/sandboxGuard.mjs の assertSandbox）が
  GB_APP_PATH の親ディレクトリにこの sentinel があることを検証するため、実データ
  を触らないことが二重に保証される。

.PARAMETER Mode
  "e2e"（既定）: tauri-driver を起動して `npm run test:e2e` を実行する。
  "manual": ビルドしたアプリを起動し、ユーザーが閉じるまで待つ（手動実機確認用）。
  いずれのモードも tauri-driver は起動しない点に注意（"manual" はアプリを
  素で起動するのみ）。

.PARAMETER Profile
  "release"（既定）または "debug"。

.PARAMETER SandboxRoot
  ビルド成果物の隔離先ルート。既定は $env:TEMP\gb-e2e-sandbox。

.EXAMPLE
  ./e2e/run-sandboxed.ps1
  既定（release + e2e）でサンドボックスビルド → E2Eテストを実行する。

.EXAMPLE
  ./e2e/run-sandboxed.ps1 -Mode manual -Profile debug
  debugビルドを作成し、アプリを起動して手動確認する。
#>

param(
    [ValidateSet("e2e", "manual")]
    [string]$Mode = "e2e",

    [ValidateSet("release", "debug")]
    [string]$Profile = "release",

    [string]$SandboxRoot = (Join-Path $env:TEMP "gb-e2e-sandbox")
)

if ($PSVersionTable.PSVersion.Major -lt 7) {
    Write-Error @"
このスクリプトは Windows PowerShell 5.1 では実行できません（PowerShell 7 以上が必要です）。
'pwsh' で実行し直してください（例: pwsh -File e2e\run-sandboxed.ps1）。
GB_APP_PATH を手動で設定して回避しないでください。GB_APP_PATH は開発機の実データ
（target/{debug,release}/GrayBrowser/app.db）への誤書き込み事故（詳細は
`sandboxed-verification` スキル参照）を防ぐための安全機構であり、手動設定はこの
安全機構を無効化します。
"@
    exit 1
}

$ErrorActionPreference = "Stop"

function ConvertTo-ComparablePath([string]$Path) {
    # 大文字小文字・区切り文字・末尾区切りの差を吸収して比較できる形にする。
    $full = [System.IO.Path]::GetFullPath($Path).Replace('/', '\')
    return $full.TrimEnd('\')
}

function Test-PathUnderRoot([string]$Path, [string]$Root) {
    $p = ConvertTo-ComparablePath $Path
    $r = ConvertTo-ComparablePath $Root
    return $p.StartsWith($r + '\', [System.StringComparison]::OrdinalIgnoreCase)
}

function Find-SandboxProcess([string[]]$ExePaths) {
    # 名前ではなくexeのフルパスで判定する。名前一致だと、同名exeである実アプリ
    # (%LOCALAPPDATA%\GrayBrowser)まで巻き込んでしまうため。
    $targets = @($ExePaths | ForEach-Object { ConvertTo-ComparablePath $_ })
    foreach ($proc in @(Get-Process -ErrorAction SilentlyContinue)) {
        # 権限不足のプロセスは Path が取れず null になる。サンドボックス配下の
        # exe は自分が起動したものなので、取れないものは対象外としてよい。
        $procPath = $null
        try { $procPath = $proc.Path } catch { }
        if ([string]::IsNullOrEmpty($procPath)) { continue }
        $comparable = ConvertTo-ComparablePath $procPath
        if ($targets | Where-Object { $_ -ieq $comparable }) {
            $proc
        }
    }
}

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Push-Location $RepoRoot
try {
    # 相対パス指定でも、以降の検証・削除が常に同じ絶対パスを指すようにする。
    $SandboxRoot = ConvertTo-ComparablePath (
        $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($SandboxRoot))

    # サンドボックスのアプリが動いたままだと、リンカがexeを上書きできず再リンクが
    # 失敗するか、古いバイナリのまま確認してしまう。自動終了はしない: 閉じるべきか
    # 判断できるのは起動した本人だけで、未保存の状態を壊す恐れもある。
    $sandboxExes = @(
        (Join-Path $SandboxRoot "debug\graybrowser.exe"),
        (Join-Path $SandboxRoot "release\GrayBrowser.exe")
    )
    $running = @(Find-SandboxProcess $sandboxExes)
    if ($running.Count -gt 0) {
        Write-Host "サンドボックスのアプリが起動したままです。ビルドしても古いバイナリが残るため中止します:"
        foreach ($p in $running) {
            Write-Host ("  PID {0}  開始 {1}  {2}" -f $p.Id, $p.StartTime, $p.Path)
        }
        Write-Host "そのアプリを閉じてから、このスクリプトを再実行してください（スクリプトはプロセスを終了しません）。"
        exit 1
    }

    # 実データを保持する既存の target/ には一切触れない、専用のビルド出力先。
    $env:CARGO_TARGET_DIR = $SandboxRoot
    Write-Host "CARGO_TARGET_DIR = $env:CARGO_TARGET_DIR (isolated from the real target/)"

    $exeRelative = if ($Profile -eq "release") { "release\GrayBrowser.exe" } else { "debug\graybrowser.exe" }
    $staleExe = Join-Path $SandboxRoot $exeRelative
    # 古いexeを残すと「再リンクされなかった」ことをビルド後に検知できない。
    # 削除対象はサンドボックス配下の当該exe1ファイルに限定する。
    if (-not (Test-PathUnderRoot $staleExe $SandboxRoot)) {
        throw "削除対象がSandboxRoot配下ではありません: $staleExe"
    }
    if (Test-Path -LiteralPath $staleExe -PathType Leaf) {
        try {
            Remove-Item -LiteralPath $staleExe -Force -ErrorAction Stop
        }
        catch {
            throw "既存のexeを削除できません（使用中の可能性があります。サンドボックスのアプリを閉じて再実行してください）: $staleExe"
        }
        Write-Host "Removed stale exe: $staleExe"
    }

    if ($Profile -eq "release") {
        Write-Host "Building release (npm run tauri build -- --no-bundle; frontend build runs automatically via beforeBuildCommand)..."
        npm run tauri build -- --no-bundle
        if ($LASTEXITCODE -ne 0) { throw "npm run tauri build failed with exit code $LASTEXITCODE" }
        $exePath = Join-Path $SandboxRoot "release\GrayBrowser.exe"
    }
    else {
        Write-Host "Building debug (npm run build, then cargo build -p graybrowser --features tauri/custom-protocol)..."
        npm run build
        if ($LASTEXITCODE -ne 0) { throw "npm run build failed with exit code $LASTEXITCODE" }
        # `tauri`'s own build.rs decides devUrl-vs-frontendDist purely from
        # whether its `custom-protocol` feature is active in this build --
        # unrelated to which command invoked cargo. A plain `cargo build`
        # (no `custom-protocol`) compiles the binary to always dial the Vite
        # dev server, which this script never starts, so the window shows
        # WebView2's ERR_CONNECTION_REFUSED instead of the app (Issue #17).
        # Passing the feature explicitly makes it load the `dist/` output
        # from `npm run build` above, the same as a real `cargo tauri build`.
        cargo build -p graybrowser --features tauri/custom-protocol
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
        $exePath = Join-Path $SandboxRoot "debug\graybrowser.exe"
    }

    if (-not (Test-Path $exePath)) {
        throw "Expected built exe not found at $exePath (ビルド前に削除済みのため、再リンクされなかったことを意味します)"
    }

    # ビルド出力先ディレクトリにsentinelを作成する（無ければ）。
    # sentinelファイル名のリテラルはe2e/sandboxGuard.mjs（JS側）のみが知っており、
    # このスクリプトはファイル名を一切知らない。node経由でensureSentinel()を
    # 呼び出し、作成したファイルのフルパスを標準出力から受け取る。
    # e2e/sandboxGuard.mjs のassertSandboxはGB_APP_PATHの親ディレクトリにこの
    # ファイルがあることを、実データディレクトリでないことの根拠として検証する。
    $exeDir = Split-Path $exePath -Parent
    $sentinelScript = Join-Path $PSScriptRoot "sandboxGuard.mjs"
    $sentinelOutput = & node $sentinelScript $exeDir 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "sentinel作成に失敗しました（node $sentinelScript $exeDir）。出力: $sentinelOutput"
    }
    $sentinelLines = @($sentinelOutput)
    if ($sentinelLines.Count -ne 1 -or [string]::IsNullOrWhiteSpace($sentinelLines[0])) {
        throw "sentinel作成の出力が想定外です（1行のフルパスを期待）。出力: $sentinelOutput"
    }
    $sentinelPath = $sentinelLines[0].Trim()
    Write-Host "Sentinel present at $sentinelPath"

    $env:GB_APP_PATH = $exePath
    Write-Host "GB_APP_PATH = $env:GB_APP_PATH"

    if ($Mode -eq "manual") {
        Write-Host "Launching app for manual verification. Close the app window to finish."
        Start-Process $env:GB_APP_PATH -Wait
        return
    }

    # Mode "e2e": tauri-driverの起動・待機・テスト実行をすべてこのスクリプト
    # プロセスのスコープ内で完結させる。ci.ymlのe2eジョブ「Run E2E tests
    # (start tauri-driver and test in one step)」ステップと同じパターン
    # （tauri-driverはこの呼び出し元プロセスが生きている間だけ生存させる）。
    $tauriDriverCmd = Get-Command tauri-driver -ErrorAction SilentlyContinue
    if (-not $tauriDriverCmd) {
        throw "tauri-driver が見つかりません。'cargo install tauri-driver --locked' でインストールしてください。"
    }

    $stdoutLog = Join-Path $env:TEMP "gb-e2e-tauri-driver-stdout.log"
    $stderrLog = Join-Path $env:TEMP "gb-e2e-tauri-driver-stderr.log"
    $proc = Start-Process tauri-driver -ArgumentList "--port 4444" -PassThru `
        -RedirectStandardOutput $stdoutLog `
        -RedirectStandardError $stderrLog
    Write-Host "tauri-driver PID: $($proc.Id)"

    try {
        $deadline = (Get-Date).AddSeconds(30)
        $ready = $false
        while ((Get-Date) -lt $deadline) {
            $result = Test-NetConnection -ComputerName localhost -Port 4444 -WarningAction SilentlyContinue
            if ($result.TcpTestSucceeded) {
                Write-Host "tauri-driver is listening on port 4444"
                $ready = $true
                break
            }
            Start-Sleep -Milliseconds 500
        }
        if (-not $ready) {
            throw "tauri-driver did not start listening on port 4444 within 30s"
        }

        npm run test:e2e
        if ($LASTEXITCODE -ne 0) {
            throw "E2E tests failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        if (-not $proc.HasExited) {
            Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        }
        Get-Process msedgedriver -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        Write-Host "--- tauri-driver stdout ---"
        Get-Content $stdoutLog -ErrorAction SilentlyContinue
        Write-Host "--- tauri-driver stderr ---"
        Get-Content $stderrLog -ErrorAction SilentlyContinue
    }
}
finally {
    Pop-Location
}
