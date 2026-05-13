# Verify a single cdylib: dumpbin /exports + dumpbin /dependents +
# signtool sign + Authenticode read-back. Shared by Tier 2's
# per-DLL checks.
#
# Usage:
#   verify-cdylib.ps1 -DllPath <path> -CertThumbprint <thumb> -CertSubject <subj>

param(
    [Parameter(Mandatory = $true)][string]$DllPath,
    [Parameter(Mandatory = $true)][string]$CertThumbprint,
    [Parameter(Mandatory = $true)][string]$CertSubject
)

$ErrorActionPreference = "Stop"
Write-Output "=== verify-cdylib: $DllPath ==="

if (-not (Test-Path $DllPath)) {
    Write-Error "::error::DLL not found: $DllPath"
    exit 1
}

# 1) dumpbin /exports — the four Dll* entry points must be present
#    and unmangled.
$output = (dumpbin /exports "$DllPath") | Out-String
Write-Output $output
$required = @(
    "DllGetClassObject",
    "DllCanUnloadNow",
    "DllRegisterServer",
    "DllUnregisterServer"
)
$missing = @()
foreach ($sym in $required) {
    $pattern = "(?m)^\s*\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]+\s+$sym\b"
    if ($output -notmatch $pattern) {
        $missing += $sym
    }
}
if ($missing.Count -gt 0) {
    Write-Error "::error::$DllPath missing required exports: $($missing -join ', ')"
    exit 1
}
Write-Output "[$DllPath] all four Dll* entry points present and unmangled."

# 2) dumpbin /dependents — every import-table dependency must be
#    in the OS allow-list.
$output = (dumpbin /dependents "$DllPath") | Out-String
Write-Output $output
$deps = @()
$inBlock = $false
$sawAny = $false
foreach ($line in ($output -split "`r?`n")) {
    if ($line -match "Image has the following dependencies:") {
        $inBlock = $true
        continue
    }
    if (-not $inBlock) { continue }
    if ($line -match "^\s*Summary") { break }
    $dep = $line.Trim()
    if ($dep) {
        $deps += $dep
        $sawAny = $true
    } elseif ($sawAny) {
        break
    }
}
$allowlist = @(
    "ADVAPI32.dll", "KERNEL32.dll", "USER32.dll", "ntdll.dll",
    "ole32.dll", "OLE32.dll", "combase.dll", "OLEAUT32.dll",
    "msvcrt.dll", "VCRUNTIME140.dll", "VCRUNTIME140_1.dll",
    "bcrypt.dll", "ws2_32.dll", "RPCRT4.dll", "SHELL32.dll"
)
$disallowed = @()
foreach ($dep in $deps) {
    $matched = $false
    if ($dep -like "api-ms-win-*") { $matched = $true }
    foreach ($a in $allowlist) {
        if ($dep -ieq $a) { $matched = $true; break }
    }
    if (-not $matched) { $disallowed += $dep }
}
if ($disallowed.Count -gt 0) {
    Write-Error "::error::$DllPath disallowed dependents: $($disallowed -join ', ')"
    exit 1
}
Write-Output "[$DllPath] all dependents within allow-list."

# 3) signtool sign — exercises the signing wiring with the ad-hoc
#    cert generated once per job.
signtool sign /sha1 $CertThumbprint /fd sha256 "$DllPath"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# 4) Read the signature back via PowerShell. We do not run
#    `signtool verify /pa` because the ad-hoc cert is not in the
#    trusted root store; what we validate is that the signature
#    bytes attached correctly and the signer subject matches.
$sig = Get-AuthenticodeSignature "$DllPath"
Write-Output "[$DllPath] Status: $($sig.Status)"
if (-not $sig.SignerCertificate) {
    Write-Error "::error::$DllPath has no signer certificate attached"
    exit 1
}
if ($sig.Status -eq "NotSigned") {
    Write-Error "::error::$DllPath reports NotSigned"
    exit 1
}
if ($sig.SignerCertificate.Subject -ne $CertSubject) {
    Write-Error "::error::$DllPath signer subject mismatch: got '$($sig.SignerCertificate.Subject)', expected '$CertSubject'"
    exit 1
}
Write-Output "[$DllPath] signed by $($sig.SignerCertificate.Subject); verify OK."
