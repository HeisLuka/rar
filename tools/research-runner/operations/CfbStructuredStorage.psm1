Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not ("PubResearch.StructuredStorage" -as [type])) {
    $source = @"
using System;
using System.Runtime.InteropServices;

namespace PubResearch
{
    [ComImport]
    [Guid("0000000B-0000-0000-C000-000000000046")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IStorage
    {
        [PreserveSig] int CreateStream([MarshalAs(UnmanagedType.LPWStr)] string name, uint mode, uint reserved1, uint reserved2, out IntPtr stream);
        [PreserveSig] int OpenStream([MarshalAs(UnmanagedType.LPWStr)] string name, IntPtr reserved1, uint mode, uint reserved2, out IntPtr stream);
        [PreserveSig] int CreateStorage([MarshalAs(UnmanagedType.LPWStr)] string name, uint mode, uint reserved1, uint reserved2, out IntPtr storage);
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

    public static class StructuredStorage
    {
        private const uint STGM_READ = 0x00000000;
        private const uint STGM_READWRITE = 0x00000002;
        private const uint STGM_SHARE_EXCLUSIVE = 0x00000010;
        private const int STG_E_FILENOTFOUND = unchecked((int)0x80030002);

        [DllImport("ole32.dll", CharSet = CharSet.Unicode, PreserveSig = true)]
        private static extern int StgOpenStorage(
            string path,
            IntPtr priority,
            uint mode,
            IntPtr exclude,
            uint reserved,
            out IStorage storage);

        private static void Check(int hr, string operation)
        {
            if (hr < 0)
            {
                throw new COMException(operation + " failed", hr);
            }
        }

        private static IStorage OpenRoot(string path, bool write)
        {
            IStorage root;
            uint mode = (write ? STGM_READWRITE : STGM_READ) | STGM_SHARE_EXCLUSIVE;
            int hr = StgOpenStorage(path, IntPtr.Zero, mode, IntPtr.Zero, 0, out root);
            Check(hr, "StgOpenStorage");
            return root;
        }

        private static IStorage OpenChild(IStorage root, string storageName, bool write)
        {
            IStorage child;
            uint mode = (write ? STGM_READWRITE : STGM_READ) | STGM_SHARE_EXCLUSIVE;
            int hr = root.OpenStorage(storageName, IntPtr.Zero, mode, IntPtr.Zero, 0, out child);
            Check(hr, "IStorage.OpenStorage(" + storageName + ")");
            return child;
        }

        private static bool StreamExistsIn(IStorage storage, string elementName)
        {
            IntPtr stream = IntPtr.Zero;
            int hr = storage.OpenStream(
                elementName,
                IntPtr.Zero,
                STGM_READ | STGM_SHARE_EXCLUSIVE,
                0,
                out stream);

            if (hr >= 0)
            {
                if (stream != IntPtr.Zero)
                {
                    Marshal.Release(stream);
                }
                return true;
            }

            if (hr == STG_E_FILENOTFOUND)
            {
                return false;
            }

            throw new COMException("IStorage.OpenStream(" + elementName + ") failed", hr);
        }

        public static bool StreamExists(string path, string storageName, string elementName)
        {
            IStorage root = null;
            IStorage target = null;
            try
            {
                root = OpenRoot(path, false);
                target = String.IsNullOrEmpty(storageName) ? root : OpenChild(root, storageName, false);
                return StreamExistsIn(target, elementName);
            }
            finally
            {
                if (target != null && !Object.ReferenceEquals(target, root))
                {
                    Marshal.FinalReleaseComObject(target);
                }
                if (root != null)
                {
                    Marshal.FinalReleaseComObject(root);
                }
            }
        }

        public static void RemoveStream(string path, string storageName, string elementName)
        {
            IStorage root = null;
            IStorage target = null;
            try
            {
                root = OpenRoot(path, true);
                target = String.IsNullOrEmpty(storageName) ? root : OpenChild(root, storageName, true);

                if (!StreamExistsIn(target, elementName))
                {
                    throw new InvalidOperationException("Target stream is already absent: " + elementName);
                }

                Check(target.DestroyElement(elementName), "IStorage.DestroyElement(" + elementName + ")");
                Check(target.Commit(0), "IStorage.Commit(target)");
                if (!Object.ReferenceEquals(target, root))
                {
                    Check(root.Commit(0), "IStorage.Commit(root)");
                }
            }
            finally
            {
                if (target != null && !Object.ReferenceEquals(target, root))
                {
                    Marshal.FinalReleaseComObject(target);
                }
                if (root != null)
                {
                    Marshal.FinalReleaseComObject(root);
                }
            }

            if (StreamExists(path, storageName, elementName))
            {
                throw new InvalidOperationException("Stream still exists after DestroyElement: " + elementName);
            }
        }
    }
}
"@

    Add-Type -TypeDefinition $source -Language CSharp
}

function Test-CfbStreamExists {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $false)]
        [AllowEmptyString()]
        [string]$StorageName = "",
        [Parameter(Mandatory = $true)]
        [string]$ElementName
    )

    return [PubResearch.StructuredStorage]::StreamExists(
        (Resolve-Path -LiteralPath $Path).Path,
        $StorageName,
        $ElementName
    )
}

function Remove-CfbStream {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $false)]
        [AllowEmptyString()]
        [string]$StorageName = "",
        [Parameter(Mandatory = $true)]
        [string]$ElementName
    )

    [PubResearch.StructuredStorage]::RemoveStream(
        (Resolve-Path -LiteralPath $Path).Path,
        $StorageName,
        $ElementName
    )
}

Export-ModuleMember -Function @(
    "Test-CfbStreamExists",
    "Remove-CfbStream"
)
