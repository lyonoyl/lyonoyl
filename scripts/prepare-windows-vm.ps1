<#
.SYNOPSIS
    Opens SSH on the Windows machine the engine questions are answered on.

.DESCRIPTION
    Installs the OpenSSH server, starts it, makes PowerShell the shell an SSH session is given,
    and authorises one public key. Every step is checked after it is taken, and the script stops
    at the first one that did not hold: a run that ends in "ready" has been verified rather than
    assumed.

    Two things about Windows cost an afternoon each if they are not known. Keys for an account in
    the Administrators group are read from C:\ProgramData\ssh\administrators_authorized_keys and
    not from the profile, and that file is ignored unless Administrators and SYSTEM are the only
    ones who can write it. Rights are granted here by SID rather than by name, because the names
    of those two groups are translated on a localised Windows and the grant then fails.

    The account configured is the elevated account this runs as, and that is the account to log
    in as afterwards.

    This file stays ASCII. Windows PowerShell 5.1 reads a file without a byte order mark as ANSI,
    and a UTF-8 em dash decoded that way ends in a right double quote, which PowerShell accepts as
    a string delimiter. One of those unbalances every quote below it, and the parse error is
    reported forty lines further on. scripts/preflight.sh fails if a byte above 0x7F appears here.

.PARAMETER PublicKey
    The public key line to authorise. When omitted, a .pub file beside this script is used, and
    failing that the key is asked for.

.EXAMPLE
    .\prepare-windows-vm.ps1

.EXAMPLE
    .\prepare-windows-vm.ps1 -PublicKey "ssh-ed25519 AAAA... comment"
#>
[CmdletBinding()]
param(
    [string] $PublicKey
)

$ErrorActionPreference = 'Stop'

$AdministratorsSid = 'S-1-5-32-544'
$SystemSid = 'S-1-5-18'
$AuthorizedKeys = Join-Path $env:ProgramData 'ssh\administrators_authorized_keys'

function Write-Step { param([string] $Text) Write-Host ""; Write-Host "== $Text" -ForegroundColor Cyan }
function Write-Ok { param([string] $Text) Write-Host "   ok: $Text" -ForegroundColor Green }
function Stop-Here { param([string] $Text) Write-Host "   FAILED: $Text" -ForegroundColor Red; exit 2 }

# --- Elevation -------------------------------------------------------------------------------

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host "This has to run elevated: Start, right-click, Terminal (Administrator)." -ForegroundColor Red
    exit 1
}

Write-Host "Configuring SSH for $($identity.Name) on $env:COMPUTERNAME" -ForegroundColor White
Write-Host "Windows $((Get-CimInstance Win32_OperatingSystem).Version), $env:PROCESSOR_ARCHITECTURE"

# Printed rather than left to be checked by hand: a disc attached from an earlier run is served
# from a cache, and an old script is indistinguishable from a new one until something is compared.
try {
    $mine = (Get-FileHash -Path $PSCommandPath -Algorithm SHA256).Hash.ToLower()
    Write-Host "This script: $mine"
} catch {
    Write-Host "This script: could not be hashed"
}

# --- The key ---------------------------------------------------------------------------------

Write-Step "Public key"

if ([string]::IsNullOrWhiteSpace($PublicKey)) {
    $beside = @(Get-ChildItem -Path $PSScriptRoot -Filter '*.pub' -File -ErrorAction SilentlyContinue)
    if ($beside.Count -eq 1) {
        $PublicKey = (Get-Content -Path $beside[0].FullName -Raw)
        Write-Ok "read from $($beside[0].Name)"
    }
    elseif ($beside.Count -gt 1) {
        Stop-Here "more than one .pub file sits beside this script; pass -PublicKey instead"
    }
    else {
        $PublicKey = Read-Host "Paste the public key line"
    }
}

$PublicKey = $PublicKey.Trim()
if ($PublicKey -notmatch '^(ssh-ed25519|ssh-rsa|ecdsa-sha2-nistp\d+|sk-ssh-ed25519@openssh\.com)\s+[A-Za-z0-9+/=]+') {
    Stop-Here "that does not look like a public key line"
}
if ($PublicKey -match "`n") {
    Stop-Here "a public key is one line; this has more"
}
Write-Ok "$($PublicKey.Substring(0, [Math]::Min(24, $PublicKey.Length)))... accepted"

# --- The server ------------------------------------------------------------------------------

Write-Step "OpenSSH server"

$capability = Get-WindowsCapability -Online -Name 'OpenSSH.Server*' | Select-Object -First 1
if (-not $capability) { Stop-Here "this Windows does not offer the OpenSSH server as a capability" }

if ($capability.State -ne 'Installed') {
    # Measured at ten minutes on an ARM64 guest, and it prints nothing at all while it works. The
    # message used to say "a minute", which turned an ordinary wait into a hunt for a hung script.
    $update = Get-Service -Name wuauserv -ErrorAction SilentlyContinue
    if ($update -and $update.StartType -eq 'Disabled') {
        Write-Host "   Windows Update is disabled, and this component is fetched through it." -ForegroundColor Yellow
        Write-Host "   If the next step never finishes, that is why." -ForegroundColor Yellow
    }
    Write-Host "   installing $($capability.Name) from Windows Update."
    Write-Host "   This prints nothing while it works and can take ten minutes. Leave it alone."
    Add-WindowsCapability -Online -Name $capability.Name | Out-Null
    $capability = Get-WindowsCapability -Online -Name $capability.Name
    if ($capability.State -ne 'Installed') { Stop-Here "the capability did not install" }
    Write-Ok "installed"
}
else {
    Write-Ok "already installed"
}

Set-Service -Name sshd -StartupType Automatic
Start-Service -Name sshd
$service = Get-Service -Name sshd
if ($service.Status -ne 'Running') { Stop-Here "sshd did not start" }
Write-Ok "sshd running, starts with the machine"

# --- The shell an SSH session gets -----------------------------------------------------------

Write-Step "Default shell"

$shell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
if (-not (Test-Path -Path $shell)) { Stop-Here "no PowerShell at $shell" }
if (-not (Test-Path -Path 'HKLM:\SOFTWARE\OpenSSH')) {
    New-Item -Path 'HKLM:\SOFTWARE\OpenSSH' -Force | Out-Null
}
New-ItemProperty -Path 'HKLM:\SOFTWARE\OpenSSH' -Name DefaultShell -PropertyType String -Force -Value $shell | Out-Null

$written = (Get-ItemProperty -Path 'HKLM:\SOFTWARE\OpenSSH' -Name DefaultShell).DefaultShell
if ($written -ne $shell) { Stop-Here "the default shell reads back as '$written'" }
Write-Ok "an SSH session gets PowerShell, not cmd"

# --- The authorised key ----------------------------------------------------------------------

Write-Step "Authorised key"

$existing = @()
if (Test-Path -Path $AuthorizedKeys) {
    $existing = @(Get-Content -Path $AuthorizedKeys | ForEach-Object { $_.Trim() } | Where-Object { $_.Length -gt 0 })
}

if ($existing -contains $PublicKey) {
    Write-Ok "already there, left alone"
}
else {
    # Appended rather than written over: another key in this file belongs to somebody.
    $keys = @($existing) + @($PublicKey)
    Set-Content -Path $AuthorizedKeys -Value $keys -Encoding ascii
    Write-Ok "added to administrators_authorized_keys ($($keys.Count) key(s) in it)"
}

# Granted by SID: on a localised Windows the group names are translated and a grant by name fails.
icacls.exe $AuthorizedKeys /inheritance:r /grant "*${AdministratorsSid}:F" /grant "*${SystemSid}:F" | Out-Null
if ($LASTEXITCODE -ne 0) { Stop-Here "icacls could not set the rights on the key file" }

$allowed = @($AdministratorsSid, $SystemSid)
$strangers = @()
foreach ($entry in (Get-Acl -Path $AuthorizedKeys).Access) {
    try { $sid = $entry.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value }
    catch { $sid = $entry.IdentityReference.Value }
    if ($allowed -notcontains $sid) { $strangers += $sid }
}
if ($strangers.Count -gt 0) {
    Stop-Here "the key file is also open to: $($strangers -join ', '); sshd will ignore it"
}
Write-Ok "readable by Administrators and SYSTEM only"

# --- The way in ------------------------------------------------------------------------------

Write-Step "Firewall"

$rule = Get-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -ErrorAction SilentlyContinue
if ($rule) {
    Enable-NetFirewallRule -Name 'OpenSSH-Server-In-TCP'
    Write-Ok "the rule the capability adds is enabled"
}
else {
    New-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -DisplayName 'OpenSSH Server (sshd)' `
        -Enabled True -Direction Inbound -Protocol TCP -Action Allow -LocalPort 22 | Out-Null
    Write-Ok "inbound TCP 22 allowed"
}

# Every profile, and not the Private one the capability leaves behind. The network is changed after
# this script runs -- that is the next step of the procedure -- and the adapter it produces is
# classified Public. A rule scoped to Private then blocks a running sshd that is listening on
# 0.0.0.0:22, and the only symptom is a connection that opens and never receives a banner.
Set-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -Profile Any
$profiles = (Get-NetFirewallRule -Name 'OpenSSH-Server-In-TCP').Profile
if ("$profiles" -ne 'Any') { Stop-Here "the rule still applies to '$profiles' only" }
Write-Ok "the rule applies to every network profile"

$where = Get-NetConnectionProfile -ErrorAction SilentlyContinue
foreach ($network in $where) {
    Write-Host "   this machine is on '$($network.Name)', classified $($network.NetworkCategory)"
}

Restart-Service -Name sshd
Start-Sleep -Seconds 2

$listening = @(Get-NetTCPConnection -State Listen -LocalPort 22 -ErrorAction SilentlyContinue)
if ($listening.Count -eq 0) { Stop-Here "nothing is listening on port 22" }
Write-Ok "listening on 22"

# --- What to do with it ----------------------------------------------------------------------

$account = $env:USERNAME
Write-Host ""
Write-Host "Ready. Log in as '$account'." -ForegroundColor Green
Write-Host ""
Write-Host "  In UTM, with the machine shut down: Network -> Emulated VLAN, then a port forward"
Write-Host "  with Guest Port 22 and Host Port 2223. In that order: the host port is the number"
Write-Host "  you type after -p, and it must be above 1024 or nothing binds and nothing says so."
Write-Host "  Not Shared Network: there the guest reaches the host and not the other way round."
Write-Host ""
Write-Host "  Then, from the host:"
Write-Host "      ssh -i <private key> -p <host port> $account@127.0.0.1" -ForegroundColor White
Write-Host ""
exit 0
