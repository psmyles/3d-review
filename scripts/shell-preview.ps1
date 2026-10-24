# Host the Explorer preview handler (`crates/shell-windows`) the way File
# Explorer's preview pane does, drive it with the pointer, and save what it shows
# - a check that needs no Explorer window and no interactive desktop.
#
#   scripts/register-shell.ps1
#   powershell.exe -NoProfile -STA -File scripts/shell-preview.ps1 -Path model.fbx -OutPrefix out/preview
#
# Windows PowerShell 5.1 (`powershell.exe`), not PowerShell 7: the harness is
# WinForms, which PowerShell 7's Add-Type cannot reference without listing a dozen
# split-out assemblies.
#
# The handler is created out of process (CLSCTX_LOCAL_SERVER), in the
# `prevhost.exe` of its own AppID, exactly as Explorer creates it; `-InProc`
# loads it into this process instead. The difference matters: it is what showed
# that prevhost calls a handler on threads that never pump messages (see
# docs/GOTCHAS.md, "A preview handler's calls arrive on threads that never pump
# messages"). Writes <OutPrefix>-1-loaded.png, -2-orbited.png (after a left
# drag) and -3-zoomed.png (after three wheel notches).
param(
    [Parameter(Mandatory)] [string] $Path,
    [Parameter(Mandatory)] [string] $OutPrefix,
    [int] $Width = 640,
    [int] $Height = 480,
    [int] $LoadSeconds = 4,
    [switch] $InProc
)
$ErrorActionPreference = 'Stop'
$product = Get-Content (Join-Path (Split-Path -Parent $PSScriptRoot) 'product.json') -Raw | ConvertFrom-Json
$clsid = $product.shellExtension.previewClsid

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -ReferencedAssemblies System.Windows.Forms, System.Drawing @"
using System;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Windows.Forms;
public static class ReviewPreviewHost {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [ComImport, Guid("8895b1c6-b41f-4c1c-a562-0d564250836f"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IPreviewHandler {
        void SetWindow(IntPtr hwnd, ref RECT rect);
        void SetRect(ref RECT rect);
        void DoPreview();
        void Unload();
        void SetFocus();
        void QueryFocus(out IntPtr hwnd);
        [PreserveSig] int TranslateAccelerator(IntPtr msg);
    }
    [ComImport, Guid("b7d14566-0509-4cce-a71f-0a554233bd9b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IInitializeWithFile { void Initialize([MarshalAs(UnmanagedType.LPWStr)] string path, uint mode); }
    [ComImport, Guid("196bf9a5-b346-4ef0-aa1e-5dcdb76768b1"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IPreviewHandlerVisuals { void SetBackgroundColor(uint color); void SetFont(IntPtr font); void SetTextColor(uint color); }
    [DllImport("ole32.dll")] public static extern int CoCreateInstance([MarshalAs(UnmanagedType.LPStruct)] Guid clsid, IntPtr outer, uint context, [MarshalAs(UnmanagedType.LPStruct)] Guid iid, [MarshalAs(UnmanagedType.IUnknown)] out object instance);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string windowClass, string title);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);

    static IPreviewHandler handler;
    static Form window;

    public static void Pump(int milliseconds) {
        var until = DateTime.Now.AddMilliseconds(milliseconds);
        while (DateTime.Now < until) { Application.DoEvents(); System.Threading.Thread.Sleep(10); }
    }

    public static void Open(string clsid, string path, int width, int height, bool inproc) {
        window = new Form();
        window.FormBorderStyle = FormBorderStyle.None;
        window.StartPosition = FormStartPosition.Manual;
        window.Location = new Point(40, 40);
        window.ClientSize = new Size(width, height);
        window.Show();
        Pump(200);
        object instance;
        int result = CoCreateInstance(new Guid(clsid), IntPtr.Zero, inproc ? 1u : 4u, typeof(IPreviewHandler).GUID, out instance);
        if (result != 0) throw new Exception("CoCreateInstance failed: 0x" + result.ToString("X8"));
        ((IInitializeWithFile)instance).Initialize(path, 0);
        // Explorer's dark theme.
        ((IPreviewHandlerVisuals)instance).SetBackgroundColor(0x00202020);
        ((IPreviewHandlerVisuals)instance).SetTextColor(0x00F0F0F0);
        handler = (IPreviewHandler)instance;
        RECT rect = new RECT(); rect.Right = width; rect.Bottom = height;
        handler.SetWindow(window.Handle, ref rect);
        handler.DoPreview();
    }

    public static void Capture(string file) {
        var bitmap = new Bitmap(window.ClientSize.Width, window.ClientSize.Height);
        using (var graphics = Graphics.FromImage(bitmap)) {
            var hdc = graphics.GetHdc();
            PrintWindow(window.Handle, hdc, 2 /* PW_RENDERFULLCONTENT */);
            graphics.ReleaseHdc(hdc);
        }
        bitmap.Save(file);
        bitmap.Dispose();
    }

    static IntPtr Pane() { return FindWindowEx(window.Handle, IntPtr.Zero, "3DReviewPreviewPane", null); }
    static IntPtr At(int x, int y) { return (IntPtr)((y << 16) | (x & 0xFFFF)); }

    public static void Drag(int x0, int y0, int x1, int y1) {
        IntPtr pane = Pane();
        SendMessage(pane, 0x0201 /* WM_LBUTTONDOWN */, (IntPtr)1, At(x0, y0));
        for (int i = 1; i <= 10; i++) {
            SendMessage(pane, 0x0200 /* WM_MOUSEMOVE */, (IntPtr)1, At(x0 + (x1 - x0) * i / 10, y0 + (y1 - y0) * i / 10));
            Pump(16);
        }
        SendMessage(pane, 0x0202 /* WM_LBUTTONUP */, IntPtr.Zero, At(x1, y1));
    }

    public static void Wheel(int notches, int x, int y) {
        SendMessage(Pane(), 0x020A /* WM_MOUSEWHEEL */, (IntPtr)((notches * 120) << 16), At(x, y));
    }

    public static bool HasPane() { return Pane() != IntPtr.Zero; }

    public static void Close() { handler.Unload(); Marshal.ReleaseComObject(handler); window.Close(); }
}
"@
[ReviewPreviewHost]::Open($clsid, (Resolve-Path $Path).Path, $Width, $Height, $InProc.IsPresent)
[ReviewPreviewHost]::Pump($LoadSeconds * 1000)
if (-not [ReviewPreviewHost]::HasPane()) { throw 'the handler made no pane window' }
[ReviewPreviewHost]::Capture("$OutPrefix-1-loaded.png")
[ReviewPreviewHost]::Drag([int]($Width * 0.3), [int]($Height * 0.5), [int]($Width * 0.55), [int]($Height * 0.45))
[ReviewPreviewHost]::Pump(1500)
[ReviewPreviewHost]::Capture("$OutPrefix-2-orbited.png")
[ReviewPreviewHost]::Wheel(3, [int]($Width / 2), [int]($Height / 2))
[ReviewPreviewHost]::Pump(1500)
[ReviewPreviewHost]::Capture("$OutPrefix-3-zoomed.png")
[ReviewPreviewHost]::Close()
"wrote $OutPrefix-{1-loaded,2-orbited,3-zoomed}.png"
