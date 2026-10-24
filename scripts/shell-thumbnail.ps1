# Ask the shell for a file's thumbnail, the way File Explorer does, and save it
# as a PNG - a check of the Explorer extension (`crates/shell-windows`) that needs
# no Explorer window and no interactive desktop.
#
#   scripts/register-shell.ps1
#   scripts/shell-thumbnail.ps1 -Path model.fbx -Out thumb.png [-Size 256]
#
# It goes through the real thumbnail cache and COM Surrogate
# (`IShellItemImageFactory::GetImage`), so it exercises the registration and the
# process isolation too. The cache keeps what it has made: give it a file it has
# not seen (a fresh copy in a new folder) to see a new build's thumbnail.
#
# It also reports the alpha it got back. The cache keeps straight alpha, so a
# channel above alpha is expected; see docs/GOTCHAS.md, "The thumbnail cache keeps
# straight alpha, and reads back top row first".
param(
    [Parameter(Mandatory)] [string] $Path,
    [Parameter(Mandatory)] [string] $Out,
    [int] $Size = 256
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class ReviewThumbnail {
    [StructLayout(LayoutKind.Sequential)] public struct SIZE { public int cx, cy; }
    [ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IShellItemImageFactory { [PreserveSig] int GetImage(SIZE size, int flags, out IntPtr bitmap); }
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
    public static extern void SHCreateItemFromParsingName(string path, IntPtr bindContext, [MarshalAs(UnmanagedType.LPStruct)] Guid riid, [MarshalAs(UnmanagedType.Interface)] out IShellItemImageFactory item);
    [StructLayout(LayoutKind.Sequential)] public struct BITMAP { public int bmType, bmWidth, bmHeight, bmWidthBytes; public ushort bmPlanes, bmBitsPixel; public IntPtr bmBits; }
    [DllImport("gdi32.dll")] public static extern int GetObject(IntPtr handle, int size, ref BITMAP bitmap);
    [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr handle);

    public static int Get(string path, int size, out IntPtr bitmap) {
        IShellItemImageFactory factory;
        SHCreateItemFromParsingName(path, IntPtr.Zero, typeof(IShellItemImageFactory).GUID, out factory);
        SIZE s; s.cx = size; s.cy = size;
        return factory.GetImage(s, 0x08 /* SIIGBF_THUMBNAILONLY */, out bitmap);
    }
}
"@
$Path = (Resolve-Path $Path).Path
$started = Get-Date
$handle = [IntPtr]::Zero
$result = [ReviewThumbnail]::Get($Path, $Size, [ref]$handle)
$ms = ((Get-Date) - $started).TotalMilliseconds
if ($result -ne 0) { throw ("GetImage failed: 0x{0:X8} after {1:N0} ms" -f $result, $ms) }
$bitmap = New-Object ReviewThumbnail+BITMAP
[ReviewThumbnail]::GetObject($handle, [System.Runtime.InteropServices.Marshal]::SizeOf($bitmap), [ref]$bitmap) | Out-Null
$width = $bitmap.bmWidth; $height = [Math]::Abs($bitmap.bmHeight); $stride = $bitmap.bmWidthBytes
$bytes = New-Object byte[] ($stride * $height)
[System.Runtime.InteropServices.Marshal]::Copy($bitmap.bmBits, $bytes, 0, $bytes.Length)
$image = New-Object System.Drawing.Bitmap $width, $height, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$partial = 0; $transparent = 0
for ($y = 0; $y -lt $height; $y++) {
    for ($x = 0; $x -lt $width; $x++) {
        # Top row first, despite the positive height (see the header).
        $i = $y * $stride + $x * 4
        $alpha = $bytes[$i + 3]
        if ($alpha -eq 0) { $transparent++ } elseif ($alpha -lt 255) { $partial++ }
        $image.SetPixel($x, $y, [System.Drawing.Color]::FromArgb($alpha, $bytes[$i + 2], $bytes[$i + 1], $bytes[$i]))
    }
}
$image.Save($Out)
$image.Dispose()
[ReviewThumbnail]::DeleteObject($handle) | Out-Null
"{0}x{1} in {2:N0} ms ({3} transparent, {4} partly covered texels) -> {5}" -f $width, $height, $ms, $transparent, $partial, $Out
