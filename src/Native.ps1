# Native helpers for Free RAM: working sets, the memory lists (modified, standby, low-priority standby), the system
# file cache, page combining, the registry cache, and Windows' available-memory counters. Everything except trimming
# working sets and reading the counters needs administrator.
if (-not ('OptiMem' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;

public static class OptiMem {
    [DllImport("psapi.dll")] static extern bool EmptyWorkingSet(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll")] static extern void SetLastError(uint error);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool GlobalMemoryStatusEx(ref MemoryStatusEx status);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetSystemFileCacheSize(IntPtr min, IntPtr max, int flags);
    [DllImport("ntdll.dll")] static extern int NtSetSystemInformation(int cls, IntPtr info, int len);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool OpenProcessToken(IntPtr h, uint access, out IntPtr tok);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool LookupPrivilegeValue(string sys, string name, out long luid);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool AdjustTokenPrivileges(IntPtr tok, bool disable, ref TokPriv tp, int len, IntPtr prev, IntPtr ret);

    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    struct TokPriv { public int Count; public long Luid; public int Attr; }

    [StructLayout(LayoutKind.Sequential)]
    struct MemoryStatusEx {
        public uint Length, Load;
        public ulong TotalPhys, AvailPhys, TotalPageFile, AvailPageFile, TotalVirtual, AvailVirtual, AvailExtendedVirtual;
    }

    const int SystemMemoryListInformation = 80;
    const int SystemCombinePhysicalMemoryInformation = 130;
    const int SystemRegistryReconciliationInformation = 155;
    const int MemoryEmptyWorkingSets = 2, MemoryFlushModifiedList = 3, MemoryPurgeStandbyList = 4, MemoryPurgeLowPriorityStandbyList = 5;

    static bool EnablePrivilege(string name) {
        IntPtr tok;
        if (!OpenProcessToken(GetCurrentProcess(), 0x28, out tok)) return false;
        try {
            TokPriv tp = new TokPriv(); tp.Count = 1; tp.Attr = 2;
            if (!LookupPrivilegeValue(null, name, out tp.Luid)) return false;
            SetLastError(0);
            if (!AdjustTokenPrivileges(tok, false, ref tp, 0, IntPtr.Zero, IntPtr.Zero)) return false;
            return Marshal.GetLastWin32Error() == 0;
        } finally { CloseHandle(tok); }
    }

    // ullAvailPhys includes immediately reusable standby/cache pages; WMI FreePhysicalMemory does not.
    public static ulong[] GetMemoryStatus() {
        MemoryStatusEx m = new MemoryStatusEx();
        m.Length = (uint)Marshal.SizeOf(typeof(MemoryStatusEx));
        if (!GlobalMemoryStatusEx(ref m)) return null;
        return new ulong[] { m.AvailPhys, m.TotalPhys, m.Load };
    }

    static int MemoryListCommand(int command) {
        IntPtr m = Marshal.AllocHGlobal(4);
        try { Marshal.WriteInt32(m, command); return NtSetSystemInformation(SystemMemoryListInformation, m, 4); }
        finally { Marshal.FreeHGlobal(m); }
    }

    public static int TrimWorkingSets() {
        int n = 0;
        int self = Process.GetCurrentProcess().Id;
        foreach (Process p in Process.GetProcesses()) {
            using (p) {
                if (p.Id == self) continue;
                IntPtr h = OpenProcess(0x0500, false, p.Id);
                if (h == IntPtr.Zero) continue;
                try { if (EmptyWorkingSet(h)) n++; }
                finally { CloseHandle(h); }
            }
        }
        return n;
    }

    public static bool PurgeStandbyList() {
        if (!EnablePrivilege("SeProfileSingleProcessPrivilege")) return false;
        return MemoryListCommand(MemoryPurgeStandbyList) == 0;
    }

    // Every cache Windows lets a program empty, in the order RAMMap uses. One line per step: "ok <step>" or
    // "failed <step> (0xSTATUS)". Windows refills these caches as programs read files again.
    public static string[] ClearAll() {
        var r = new List<string>();
        bool prof = EnablePrivilege("SeProfileSingleProcessPrivilege");
        bool quota = EnablePrivilege("SeIncreaseQuotaPrivilege");
        Func<string, int, string> line = (step, status) => status == 0 ? "ok " + step : string.Format("failed {0} (0x{1:X8})", step, status);

        r.Add("ok process working sets (" + TrimWorkingSets() + " processes)");
        r.Add(line("system working set", prof ? MemoryListCommand(MemoryEmptyWorkingSets) : -1));
        if (!quota) r.Add("failed system file cache (SeIncreaseQuotaPrivilege unavailable)");
        else {
            SetLastError(0);
            r.Add(SetSystemFileCacheSize(new IntPtr(-1), new IntPtr(-1), 0) ? "ok system file cache" : "failed system file cache (0x" + Marshal.GetLastWin32Error().ToString("X8") + ")");
        }
        r.Add(line("modified page list", prof ? MemoryListCommand(MemoryFlushModifiedList) : -1));
        r.Add(line("standby list", prof ? MemoryListCommand(MemoryPurgeStandbyList) : -1));
        r.Add(line("low-priority standby list", prof ? MemoryListCommand(MemoryPurgeLowPriorityStandbyList) : -1));

        // MEMORY_COMBINE_INFORMATION_EX: Handle, PagesCombined, Flags (24 bytes on x64)
        IntPtr c = Marshal.AllocHGlobal(24);
        try {
            for (int i = 0; i < 24; i++) Marshal.WriteByte(c, i, 0);
            int st = prof ? NtSetSystemInformation(SystemCombinePhysicalMemoryInformation, c, 24) : -1;
            r.Add(st == 0 ? "ok combined identical pages (" + Marshal.ReadInt64(c, 8) + " pages)" : line("combine identical pages", st));
        } finally { Marshal.FreeHGlobal(c); }

        r.Add(line("registry cache", NtSetSystemInformation(SystemRegistryReconciliationInformation, IntPtr.Zero, 0)));
        return r.ToArray();
    }
}
'@
}
