# Register (or -Unregister) the Explorer shell extension - the FBX thumbnail
# provider and preview handler in `crates/shell-windows` - for the current user,
# from a build in this checkout. The installer registers the shipped copy the same
# way (`packaging/3d-review.iss`, its [Registry] section); keep the two in step.
#
#   cargo build --profile release-shell -p review-shell-windows
#   scripts/register-shell.ps1             # register target/release-shell's build
#   scripts/register-shell.ps1 -Unregister
#
# The DLL is *copied* before it is registered, to a fresh folder under
# `target/shell-registered/`: Explorer keeps the DLL loaded in its surrogates, which
# locks the file, and a registered copy is what lets the next `cargo build` write
# the original. Every identifier comes from `product.json`'s `shellExtension`.
#
# Per-user (HKCU) needs no elevation. It does not override a thumbnail handler a
# *machine-wide* install of 3D Review registered; unregister that one first.
param(
    [switch] $Unregister,
    [string] $Dll
)
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$product = Get-Content (Join-Path $repo 'product.json') -Raw | ConvertFrom-Json
$shell = $product.shellExtension
$name = $product.productName
$thumbnail = $shell.thumbnailClsid
$preview = $shell.previewClsid
$appId = $shell.previewAppId
$ext = $product.fileAssociation.extension
$progId = $product.fileAssociation.progId

# The shell's own interface ids the handlers register under.
$thumbnailSlot = '{e357fccd-a995-4576-b01f-234630154e96}'  # IThumbnailProvider
$previewSlot = '{8895b1c6-b41f-4c1c-a562-0d564250836f}'    # IPreviewHandler

$classes = 'HKCU:\Software\Classes'
$previewHandlers = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\PreviewHandlers'

function Set-Default([string] $key, [string] $value) {
    New-Item -Path $key -Force | Out-Null
    Set-ItemProperty -Path $key -Name '(default)' -Value $value
}

function Remove-Key([string] $key) {
    if (Test-Path $key) { Remove-Item -Path $key -Recurse -Force }
}

# Remove a handler slot only while it is still ours, so unregistering never takes
# out another program's handler.
function Remove-Slot([string] $key, [string] $clsid) {
    if ((Test-Path $key) -and ((Get-Item $key).GetValue('') -eq $clsid)) {
        Remove-Item -Path $key -Recurse -Force
    }
}

if ($Unregister) {
    foreach ($owner in @("$classes\$ext", "$classes\$progId")) {
        Remove-Slot "$owner\ShellEx\$thumbnailSlot" $thumbnail
        Remove-Slot "$owner\ShellEx\$previewSlot" $preview
    }
    Remove-Key "$classes\CLSID\$thumbnail"
    Remove-Key "$classes\CLSID\$preview"
    Remove-Key "$classes\AppID\$appId"
    if (Test-Path $previewHandlers) {
        Remove-ItemProperty -Path $previewHandlers -Name $preview -ErrorAction SilentlyContinue
    }
    Write-Host "unregistered the $name shell extension"
} else {
    if (-not $Dll) { $Dll = Join-Path $repo 'target\release-shell\review_shell.dll' }
    if (-not (Test-Path $Dll)) {
        throw "no DLL at $Dll - build it with: cargo build --profile release-shell -p review-shell-windows"
    }
    $staging = Join-Path $repo ("target\shell-registered\" + (Get-Date -Format 'yyyyMMdd-HHmmss'))
    New-Item -ItemType Directory -Force $staging | Out-Null
    $registered = Join-Path $staging $shell.dllName
    Copy-Item $Dll $registered

    Set-Default "$classes\CLSID\$thumbnail" "$name Thumbnail Provider"
    Set-Default "$classes\CLSID\$thumbnail\InprocServer32" $registered
    Set-ItemProperty "$classes\CLSID\$thumbnail\InprocServer32" -Name 'ThreadingModel' -Value 'Apartment'

    Set-Default "$classes\CLSID\$preview" "$name Preview Handler"
    Set-ItemProperty "$classes\CLSID\$preview" -Name 'DisplayName' -Value "$name Preview Handler"
    Set-ItemProperty "$classes\CLSID\$preview" -Name 'AppID' -Value $appId
    Set-Default "$classes\CLSID\$preview\InprocServer32" $registered
    Set-ItemProperty "$classes\CLSID\$preview\InprocServer32" -Name 'ThreadingModel' -Value 'Apartment'

    # The preview handler's own prevhost.exe, so it never shares a process with
    # another vendor's handler.
    New-Item -Path "$classes\AppID\$appId" -Force | Out-Null
    New-ItemProperty -Path "$classes\AppID\$appId" -Name 'DllSurrogate' -PropertyType ExpandString `
        -Value '%SystemRoot%\system32\prevhost.exe' -Force | Out-Null

    # Under the extension (whoever is the default app) and under our own ProgID
    # (for when 3D Review is the default app, which Explorer looks under first).
    foreach ($owner in @("$classes\$ext", "$classes\$progId")) {
        Set-Default "$owner\ShellEx\$thumbnailSlot" $thumbnail
        Set-Default "$owner\ShellEx\$previewSlot" $preview
    }
    New-Item -Path $previewHandlers -Force | Out-Null
    Set-ItemProperty -Path $previewHandlers -Name $preview -Value "$name Preview Handler"
    Write-Host "registered $registered"
}

# Tell Explorer the associations changed, so it asks the new handlers.
Add-Type -Namespace Review -Name Shell -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("shell32.dll")]
public static extern void SHChangeNotify(int eventId, uint flags, System.IntPtr item1, System.IntPtr item2);
'@
[Review.Shell]::SHChangeNotify(0x08000000, 0, [System.IntPtr]::Zero, [System.IntPtr]::Zero)
