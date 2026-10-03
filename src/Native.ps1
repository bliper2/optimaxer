# Native helpers: working-set trimming and standby-list purge (needs admin for the latter).
if (-not ('OptiMem' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;

public static class OptiMem {
    [DllImport("psapi.dll")] static extern bool EmptyWorkingSet(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("ntdll.dll")] static extern int NtSetSystemInformation(int cls, IntPtr info, int len);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool OpenProcessToken(IntPtr h, uint access, out IntPtr tok);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool LookupPrivilegeValue(string sys, string name, out long luid);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool AdjustTokenPrivileges(IntPtr tok, bool disable, ref TokPriv tp, int len, IntPtr prev, IntPtr ret);

    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    struct TokPriv { public int Count; public long Luid; public int Attr; }

    static bool EnablePrivilege(string name) {
        IntPtr tok;
        if (!OpenProcessToken(GetCurrentProcess(), 0x28, out tok)) return false;
        TokPriv tp = new TokPriv(); tp.Count = 1; tp.Attr = 2;
        if (!LookupPrivilegeValue(null, name, out tp.Luid)) return false;
        AdjustTokenPrivileges(tok, false, ref tp, 0, IntPtr.Zero, IntPtr.Zero);
        return Marshal.GetLastWin32Error() == 0;
    }

    public static int TrimWorkingSets() {
        int n = 0;
        foreach (Process p in Process.GetProcesses()) {
            IntPtr h = OpenProcess(0x0500, false, p.Id);
            if (h == IntPtr.Zero) continue;
            if (EmptyWorkingSet(h)) n++;
            CloseHandle(h);
        }
        return n;
    }

    public static bool PurgeStandbyList() {
        if (!EnablePrivilege("SeProfileSingleProcessPrivilege")) return false;
        IntPtr m = Marshal.AllocHGlobal(4);
        try { Marshal.WriteInt32(m, 4); return NtSetSystemInformation(80, m, 4) == 0; }
        finally { Marshal.FreeHGlobal(m); }
    }
}
'@
}
