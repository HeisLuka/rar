param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$modulePath = Join-Path $root "operations\CfbStructuredStorage.psm1"
Import-Module $modulePath -Force

if (-not ("PubResearchTest.CfbTestFactory" -as [type])) {
    $source = @"
using System;
using System.Runtime.InteropServices;

namespace PubResearchTest
{
    [ComImport]
    [Guid("0000000B-0000-0000-C000-000000000046")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IStorage
    {
        [PreserveSig] int CreateStream([MarshalAs(UnmanagedType.LPWStr)] string name, uint mode, uint reserved1, uint reserved2, out IntPtr stream);
        [PreserveSig] int OpenStream([MarshalAs(UnmanagedType.LPWStr)] string name, IntPtr reserved1, uint mode, uint reserved2, out IntPtr stream);
        [PreserveSig] int CreateStorage([MarshalAs(UnmanagedType.LPWStr)] string name, uint mode, uint reserved1, uint reserved2, out IStorage storage);
        [PreserveSig] int OpenStorage([MarshalAs(UnmanagedType.LPWStr)] string name, IntPtr priority, uint mode, IntPtr exclude, uint reserved, out IStorage storage);
        [PreserveSig] int CopyTo(uint ciidExclude, IntPtr rgiidExclude, IntPtr snbExclude, IntPtr destination);
        [PreserveSig] int MoveElementTo([MarshalAs(UnmanagedType.LPWStr)] string name, IntPtr destination, [MarshalAs(UnmanagedType.LPWStr)] string newName, uint flags);
        [PreserveSig] int Commit(uint flags);
        [PreserveSig] int Revert();
        [PreserveSig] int EnumElements(uint reserved1, IntPtr reserved2, uint reserved3, out IntPtr enumerator);
        [PreserveSig] int DestroyElement([MarshalAs(UnmanagedType.LPWStr)] string name);
        [PreserveSig] int RenameElement([MarshalAs(UnmanagedType.LPWStr)] string oldName, [MarshalAs(UnmanagedType.LPWStr)] string newName);
        [PreserveSig] int SetElementTimes([MarshalAs(UnmanagedType.LPWStr)] string name, IntPtr creation, IntPtr access, IntPtr modification);
        [PreserveSig] int SetClass(ref Guid clsid);
        [PreserveSig] int SetStateBits(uint stateBits, uint mask);
        [PreserveSig] int Stat(IntPtr stat, uint flags);
    }

    public static class CfbTestFactory
    {
        private const uint STGM_CREATE = 0x00001000;
        private const uint STGM_READWRITE = 0x00000002;
        private const uint STGM_SHARE_EXCLUSIVE = 0x00000010;

        [DllImport("ole32.dll", CharSet = CharSet.Unicode, PreserveSig = true)]
        private static extern int StgCreateDocfile(
            string path,
            uint mode,
            uint reserved,
            out IStorage storage);

        private static void Check(int hr, string operation)
        {
            if (hr < 0) throw new COMException(operation + " failed", hr);
        }

        public static void Create(string path)
        {
            IStorage root = null;
            IStorage child = null;
            IntPtr stream = IntPtr.Zero;
            IntPtr nested = IntPtr.Zero;
            try
            {
                Check(StgCreateDocfile(path, STGM_CREATE | STGM_READWRITE | STGM_SHARE_EXCLUSIVE, 0, out root), "StgCreateDocfile");
                Check(root.CreateStream("Envelope", STGM_CREATE | STGM_READWRITE | STGM_SHARE_EXCLUSIVE, 0, 0, out stream), "CreateStream Envelope");
                if (stream != IntPtr.Zero) { Marshal.Release(stream); stream = IntPtr.Zero; }

                Check(root.CreateStorage("Escher", STGM_CREATE | STGM_READWRITE | STGM_SHARE_EXCLUSIVE, 0, 0, out child), "CreateStorage Escher");
                Check(child.CreateStream("EscherDelayStm", STGM_CREATE | STGM_READWRITE | STGM_SHARE_EXCLUSIVE, 0, 0, out nested), "CreateStream EscherDelayStm");
                if (nested != IntPtr.Zero) { Marshal.Release(nested); nested = IntPtr.Zero; }
                Check(child.Commit(0), "Commit child");
                Check(root.Commit(0), "Commit root");
            }
            finally
            {
                if (nested != IntPtr.Zero) Marshal.Release(nested);
                if (stream != IntPtr.Zero) Marshal.Release(stream);
                if (child != null) Marshal.FinalReleaseComObject(child);
                if (root != null) Marshal.FinalReleaseComObject(root);
            }
        }
    }
}
"@
    Add-Type -TypeDefinition $source -Language CSharp
}

$temp = Join-Path $env:RUNNER_TEMP ("synth-r1-cfb-" + [Guid]::NewGuid().ToString("N") + ".cfb")
try {
    [PubResearchTest.CfbTestFactory]::Create($temp)

    if (-not (Test-CfbStreamExists -Path $temp -StorageName "" -ElementName "Envelope")) {
        throw "Envelope fixture stream missing before test"
    }
    if (-not (Test-CfbStreamExists -Path $temp -StorageName "Escher" -ElementName "EscherDelayStm")) {
        throw "EscherDelayStm fixture stream missing before test"
    }

    Remove-CfbStream -Path $temp -StorageName "" -ElementName "Envelope"
    if (Test-CfbStreamExists -Path $temp -StorageName "" -ElementName "Envelope") {
        throw "Envelope stream survived deletion"
    }

    Remove-CfbStream -Path $temp -StorageName "Escher" -ElementName "EscherDelayStm"
    if (Test-CfbStreamExists -Path $temp -StorageName "Escher" -ElementName "EscherDelayStm") {
        throw "Nested stream survived deletion"
    }

    Write-Host "CFB Structured Storage helper smoke passed."
}
finally {
    Remove-Item -LiteralPath $temp -Force -ErrorAction SilentlyContinue
}
