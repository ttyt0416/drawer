"""Own-child Windows x64 debug probe. No attach, host clock changes, API arguments or payload logs.

Clock APIs return a moving simulated date through a one-byte RET stub in the owned child.
Network entries fail the run before the original API executes, including denied attempts.
This monitors named user-mode API entry points, not kernel syscalls or unrelated services.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import datetime as dt
import hashlib
import json
import ntpath
import os
from pathlib import Path
import struct
import subprocess
import sys
import time

if os.name != "nt" or c.sizeof(c.c_void_p) != 8:
    raise SystemExit("Windows x64 host required")
k = c.WinDLL("kernel32", use_last_error=True)
P = c.c_void_p
U64 = c.c_uint64
SIZE = c.c_size_t

class Record(c.Structure):
    _fields_ = [("code", w.DWORD), ("flags", w.DWORD), ("record", P), ("address", P), ("count", w.DWORD), ("info", U64 * 15)]
class ExceptionInfo(c.Structure):
    _fields_ = [("record", Record), ("first", w.DWORD)]
class CreateProcess(c.Structure):
    _fields_ = [("file", P), ("process", P), ("thread", P), ("base", P), ("offset", w.DWORD), ("size", w.DWORD), ("tls", P), ("start", P), ("name", P), ("unicode", w.WORD)]
class CreateThread(c.Structure):
    _fields_ = [("thread", P), ("tls", P), ("start", P)]
class LoadDll(c.Structure):
    _fields_ = [("file", P), ("base", P), ("offset", w.DWORD), ("size", w.DWORD), ("name", P), ("unicode", w.WORD)]
class Union(c.Union):
    _fields_ = [("exception", ExceptionInfo), ("process", CreateProcess), ("thread", CreateThread), ("dll", LoadDll), ("exit_code", w.DWORD), ("base", P)]
class Event(c.Structure):
    _fields_ = [("code", w.DWORD), ("pid", w.DWORD), ("tid", w.DWORD), ("u", Union)]
class Startup(c.Structure):
    _fields_ = [("cb", w.DWORD), ("reserved", w.LPWSTR), ("desktop", w.LPWSTR), ("title", w.LPWSTR), ("x", w.DWORD), ("y", w.DWORD), ("xs", w.DWORD), ("ys", w.DWORD), ("xc", w.DWORD), ("yc", w.DWORD), ("fill", w.DWORD), ("flags", w.DWORD), ("show", w.WORD), ("reserved2", w.WORD), ("reserved_ptr", P), ("stdin", P), ("stdout", P), ("stderr", P)]
class ProcessInfo(c.Structure):
    _fields_ = [("process", P), ("thread", P), ("pid", w.DWORD), ("tid", w.DWORD)]
assert (c.sizeof(Event), Event.u.offset, c.sizeof(Record), c.sizeof(Startup)) == (176, 16, 152, 104)

def api(name, args, result):
    f = getattr(k, name); f.argtypes = args; f.restype = result; return f
create = api("CreateProcessW", [w.LPCWSTR, w.LPWSTR, P, P, w.BOOL, w.DWORD, P, w.LPCWSTR, c.POINTER(Startup), c.POINTER(ProcessInfo)], w.BOOL)
wait = api("WaitForDebugEvent", [c.POINTER(Event), w.DWORD], w.BOOL)
cont = api("ContinueDebugEvent", [w.DWORD, w.DWORD, w.DWORD], w.BOOL)
read_memory = api("ReadProcessMemory", [P, P, P, SIZE, c.POINTER(SIZE)], w.BOOL)
write_memory = api("WriteProcessMemory", [P, P, P, SIZE, c.POINTER(SIZE)], w.BOOL)
protect = api("VirtualProtectEx", [P, P, SIZE, w.DWORD, c.POINTER(w.DWORD)], w.BOOL)
allocate = api("VirtualAllocEx", [P, P, SIZE, w.DWORD, w.DWORD], P)
flush = api("FlushInstructionCache", [P, P, SIZE], w.BOOL)
get_context = api("GetThreadContext", [P, P], w.BOOL)
set_context = api("SetThreadContext", [P, P], w.BOOL)
close = api("CloseHandle", [P], w.BOOL)
terminate = api("TerminateProcess", [P, w.UINT], w.BOOL)
mapped_name = api("K32GetMappedFileNameW", [P, P, w.LPWSTR, w.DWORD], w.DWORD)
final_name = api("GetFinalPathNameByHandleW", [P, w.LPWSTR, w.DWORD, w.DWORD], w.DWORD)

CLOCK = {"GetSystemTimeAsFileTime": "filetime", "GetSystemTimePreciseAsFileTime": "filetime", "GetSystemTime": "systemtime", "GetLocalTime": "localtime"}
NETWORK = {
    "ws2_32.dll": {"socket", "WSASocketA", "WSASocketW", "connect", "WSAConnect", "WSAConnectByNameA", "WSAConnectByNameW", "WSAConnectByList", "bind", "listen", "accept", "WSAAccept", "send", "sendto", "WSASend", "WSASendTo", "getaddrinfo", "GetAddrInfoW", "GetAddrInfoExA", "GetAddrInfoExW", "gethostbyname", "WSALookupServiceBeginA", "WSALookupServiceBeginW"},
    "dnsapi.dll": {"DnsQuery_A", "DnsQuery_W", "DnsQuery_UTF8", "DnsQueryEx", "DnsServiceBrowse", "DnsServiceResolve", "DnsServiceRegister"},
    "winhttp.dll": {"WinHttpOpen", "WinHttpConnect", "WinHttpSendRequest", "WinHttpWebSocketCompleteUpgrade"},
    "wininet.dll": {"InternetOpenA", "InternetOpenW", "InternetConnectA", "InternetConnectW", "InternetOpenUrlA", "InternetOpenUrlW", "HttpOpenRequestA", "HttpOpenRequestW", "HttpSendRequestA", "HttpSendRequestW"},
    "urlmon.dll": {"URLDownloadToFileA", "URLDownloadToFileW", "URLDownloadToCacheFileA", "URLDownloadToCacheFileW", "URLOpenBlockingStreamA", "URLOpenBlockingStreamW"},
    "cryptnet.dll": {"CryptRetrieveObjectByUrlA", "CryptRetrieveObjectByUrlW"},
    "wer.dll": {"WerReportSubmit"},
}

def checked(ok, operation):
    if not ok: raise OSError(c.get_last_error(), operation)

def read(h, address, length):
    if length < 0 or length > 8 * 1024 * 1024: raise ValueError("Remote read bound")
    b = c.create_string_buffer(length); n = SIZE()
    checked(read_memory(h, address, b, length, c.byref(n)), "ReadProcessMemory")
    if n.value != length: raise ValueError("Partial remote read")
    return b.raw

def write(h, address, data):
    b = c.create_string_buffer(data); n = SIZE()
    checked(write_memory(h, address, b, len(data), c.byref(n)), "WriteProcessMemory")
    if n.value != len(data): raise ValueError("Partial remote write")

def patch(h, address, data):
    old = w.DWORD(); checked(protect(h, address, len(data), 0x40, c.byref(old)), "Protect owned instruction")
    try: write(h, address, data); checked(flush(h, address, len(data)), "Flush owned instruction")
    finally:
        ignored = w.DWORD(); checked(protect(h, address, len(data), old.value, c.byref(ignored)), "Restore instruction permissions")

def exports(h, base):
    head = read(h, base, 4096)
    if head[:2] != b"MZ": raise ValueError("Loaded module lacks MZ")
    pe = struct.unpack_from("<I", head, 0x3c)[0]
    if pe > 3500 or head[pe:pe + 4] != b"PE\0\0": raise ValueError("Loaded module PE header bound")
    optional = pe + 24
    if struct.unpack_from("<H", head, optional)[0] != 0x20b: raise ValueError("Only PE64 module supported")
    size = struct.unpack_from("<I", head, optional + 56)[0]
    rva, length = struct.unpack_from("<II", head, optional + 112)
    if not rva: return size, []
    if length < 40 or rva + length > size: raise ValueError("Invalid export directory")
    data = read(h, base + rva, length)
    count, names, functions, strings, ordinals = struct.unpack_from("<IIIII", data, 20)
    if count > 65536 or names > 65536: raise ValueError("Export count bound")
    fs = read(h, base + functions, count * 4); ns = read(h, base + strings, names * 4); os_ = read(h, base + ordinals, names * 2)
    out = []
    for i in range(names):
        n = struct.unpack_from("<I", ns, i * 4)[0]
        if not rva <= n < rva + length: raise ValueError("Export name outside retained directory")
        at = n - rva; end = data.find(b"\0", at, at + 512)
        if end < 0: raise ValueError("Export name bound")
        name = data[at:end].decode("ascii")
        ordinal = struct.unpack_from("<H", os_, i * 2)[0]
        if ordinal >= count: raise ValueError("Export ordinal bound")
        address = struct.unpack_from("<I", fs, ordinal * 4)[0]
        if address >= size: raise ValueError("Export address outside image")
        out.append((name, base + address, rva <= address < rva + length))
    return size, out

def name_of(h, base, file):
    b = c.create_unicode_buffer(32768)
    n = final_name(file, b, len(b), 0) if file else 0
    if not n: n = mapped_name(h, base, b, len(b))
    if not n or n >= len(b): raise ValueError("Loaded module identity unavailable")
    return ntpath.basename(b.value).lower()

def context(h):
    raw = c.create_string_buffer(1248); address = (c.addressof(raw) + 15) & ~15
    c.memset(address, 0, 1232); c.c_uint32.from_address(address + 48).value = 0x100003
    checked(get_context(h, address), "Get owned thread context")
    return raw, address

class Probe:
    def __init__(self, executable, output, fake_date, timeout, env, clock_scope):
        self.exe = executable; self.output = output; self.timeout = timeout; self.environment = env; self.clock_scope = clock_scope; self.images = {}; self.real_calls = 0
        self.fake = dt.datetime.fromisoformat(fake_date).replace(tzinfo=dt.timezone.utc) if fake_date else None
        self.start = time.perf_counter(); self.processes = {}; self.threads = {}; self.hooks = {}; self.stubs = {}
        self.exceptions=[]; self.stack_code=[]; self.clock_sites={}; self.clock_calls = {}; self.network_calls = []; self.modules = set(); self.coverage = []; self.root = None; self.codes = {}; self.pi = None

    def module(self, pid, base, file):
        h = self.processes[pid]
        try:
            name = name_of(h, base, file); self.modules.add(name)
            wanted = dict(CLOCK) if self.fake and name in ("kernel32.dll", "kernelbase.dll") else {}
            wanted.update({n: "network" for n in NETWORK.get(name, ())})
            if not wanted: return
            size, symbols = exports(h, base); hooked = []; forward = []
            for symbol, address, forwarded in symbols:
                if symbol not in wanted: continue
                if forwarded: forward.append(symbol); continue
                kind = wanted[symbol]
                key = pid, address
                if key not in self.hooks:
                    old = read(h, address, 1)
                    if old == b"\xcc": raise ValueError("API entry already contains a breakpoint")
                    patch(h, address, b"\xcc")
                    self.hooks[key] = (kind, name, symbol, base, size)
                hooked.append(symbol)
            self.coverage.append({"pid": pid, "module": name, "hooks": sorted(hooked), "forwarded": sorted(forward)})
        finally:
            if file: close(file)

    def fake_clock(self, pid, tid, kind, symbol):
        h = self.processes[pid]; raw, address = context(self.threads[pid, tid])
        pointer = U64.from_address(address + 128).value
        stack=U64.from_address(address+152).value
        caller=struct.unpack('<Q',read(h,stack,8))[0]
        base,size=self.images[pid]
        simulated=self.clock_scope=='all' or base<=caller<base+size
        now = self.fake + dt.timedelta(seconds=time.perf_counter() - self.start) if simulated else dt.datetime.now(dt.timezone.utc)
        if kind == "filetime":
            epoch = dt.datetime(1601, 1, 1, tzinfo=dt.timezone.utc)
            delta = now - epoch; ticks = (delta.days * 86400 + delta.seconds) * 10000000 + delta.microseconds * 10
            write(h, pointer, struct.pack("<Q", ticks))
        else:
            if kind == "localtime": now = now.astimezone()
            write(h, pointer, struct.pack("<8H", now.year, now.month, (now.weekday() + 1) % 7, now.day, now.hour, now.minute, now.second, now.microsecond // 1000))
        if pid not in self.stubs:
            stub = allocate(h, None, 4096, 0x3000, 4); checked(stub, "Allocate owned RET stub")
            write(h, stub, b"\xc3"); old = w.DWORD(); checked(protect(h, stub, 4096, 0x20, c.byref(old)), "Seal owned RET stub")
            checked(flush(h, stub, 1), "Flush owned RET stub"); self.stubs[pid] = stub
        # Execute RET normally, preserving the call's regular/shadow-stack return semantics.
        U64.from_address(address + 248).value = self.stubs[pid]
        U64.from_address(address + 120).value = 0
        checked(set_context(self.threads[pid, tid], address), "Set owned clock result")
        if simulated:
            self.clock_calls[symbol] = self.clock_calls.get(symbol, 0) + 1
            site=f"{symbol}+image-rva-{caller-base:x}";self.clock_sites[site]=self.clock_sites.get(site,0)+1
        else: self.real_calls += 1

    def run(self, arguments):
        si = Startup(); si.cb = c.sizeof(si); si.flags = 1; si.show = 0; pi = ProcessInfo()
        command = c.create_unicode_buffer(subprocess.list2cmdline([str(self.exe), *arguments]))
        environment = dict(os.environ); environment.update(self.environment)
        block = c.create_unicode_buffer("\0".join(f"{key}={value}" for key, value in sorted(environment.items(), key=lambda e: e[0].upper())) + "\0\0")
        checked(create(str(self.exe), command, None, None, False, 0x1 | 0x400 | 0x08000000, block, str(self.output.parent), c.byref(si), c.byref(pi)), "Launch owned debug child")
        self.pi = pi; self.root = pi.pid; event = Event(); failure = None
        self.processes[pi.pid]=pi.process;self.threads[pi.pid,pi.tid]=pi.thread
        try:
            while True:
                if time.perf_counter() - self.start > self.timeout: raise TimeoutError("Owned API observation deadline")
                if not wait(c.byref(event), 100):
                    if c.get_last_error() == 121: continue
                    checked(False, "WaitForDebugEvent")
                code, pid, tid = event.code, event.pid, event.tid; status = 0x10002
                try:
                    if code == 3:
                        info = event.u.process; self.processes[pid] = info.process; self.threads[pid, tid] = info.thread
                        if pid == self.root:
                            if pi.process != info.process: close(pi.process)
                            if pi.thread != info.thread: close(pi.thread)
                        header=read(info.process,info.base,4096);pe=struct.unpack_from('<I',header,0x3c)[0]
                        if header[pe:pe+4]!=b'PE\0\0' or pe>3500:raise ValueError('Owned image PE bound')
                        self.images[pid]=(info.base,struct.unpack_from('<I',header,pe+24+56)[0])
                        self.module(pid, info.base, info.file)
                    elif code == 2: self.threads[pid, tid] = event.u.thread.thread
                    elif code == 6: self.module(pid, event.u.dll.base, event.u.dll.file)
                    elif code == 7:
                        base = event.u.base
                        for key, value in list(self.hooks.items()):
                            if key[0] == pid and value[3] == base: del self.hooks[key]
                    elif code == 1:
                        record = event.u.exception.record; hook = self.hooks.get((pid, record.address))
                        if record.code!=0x80000003 and len(self.exceptions)<64:
                            buf=c.create_unicode_buffer(32768);mapped_name(self.processes[pid],record.address,buf,len(buf))
                            base,size=self.images[pid];where=f"image-rva-{record.address-base:x}" if base<=record.address<base+size else ntpath.basename(buf.value).lower() or "private-or-unmapped-memory"
                            self.exceptions.append({"pid":pid,"code":hex(record.code),"firstChance":bool(event.u.exception.first),"location":where})
                            if record.code==0xc00000fd:
                                raw,context_address=context(self.threads[pid,tid]);rsp=U64.from_address(context_address+152).value
                                try:
                                    stack=read(self.processes[pid],rsp+512,32768)
                                    self.stack_code=[f"{word-base:x}" for (word,) in struct.iter_unpack('<Q',stack) if base<=word<base+size][:256]
                                except OSError:pass
                        if record.code == 0x80000003 and hook:
                            kind, module, symbol, _, _ = hook
                            if kind == "network":
                                self.network_calls.append({"pid": pid, "module": module, "api": symbol})
                                raise RuntimeError("Network API attempt observed, including attempts which would fail offline")
                            self.fake_clock(pid, tid, kind, symbol)
                        elif record.code == 0x80000003:
                            pass  # Windows-owned initial process breakpoint.
                        else:
                            status = 0x80010001
                    elif code == 4: self.threads.pop((pid, tid), None)
                    elif code == 5:
                        self.codes[pid] = event.u.exit_code; self.processes.pop(pid, None)
                        for key in list(self.threads):
                            if key[0] == pid: del self.threads[key]
                except Exception:
                    # Terminate while all debuggee threads are paused. Never resume into a patched API.
                    for handle in self.processes.values(): terminate(handle, 0xEE)
                    raise
                finally:
                    checked(cont(pid, tid, status), "Continue owned debug event")
                if self.root in self.codes and not self.processes: break
        except Exception as e:
            failure = f"{type(e).__name__}: {e}"
        finally:
            if self.processes:
                for handle in self.processes.values(): terminate(handle, 0xEE)
                deadline = time.perf_counter() + 10
                while self.processes and time.perf_counter() < deadline:
                    if wait(c.byref(event), 100):
                        if event.code == 5: self.codes[event.pid] = event.u.exit_code; self.processes.pop(event.pid, None)
                        cont(event.pid, event.tid, 0x10002)
            if self.processes: failure = (failure or "") + "; owned process cleanup incomplete"
        if not failure and self.codes.get(self.root) != 0: failure = "Owned application exited unsuccessfully"
        if not failure and self.fake and not self.clock_calls: failure = "No clock API hit: simulated date was not verified"
        result = {"version": 1, "passed": failure is None, "error": failure, "exeSha256": hashlib.sha256(self.exe.read_bytes()).hexdigest(), "simulatedUtcStart": self.fake.isoformat() if self.fake else None, "clockScope": self.clock_scope, "systemClockCallsPassedThrough": self.real_calls, "hostUtcEnd": dt.datetime.now(dt.timezone.utc).isoformat(), "clockCalls": self.clock_calls, "clockSites": self.clock_sites, "exceptions": self.exceptions, "stackImageRvas": self.stack_code, "networkAttempts": self.network_calls, "networkApiCount": sum(map(len, NETWORK.values())), "loadedModules": sorted(self.modules), "coverage": self.coverage, "processExitCodes": self.codes, "ownedProcessesRemaining": len(self.processes), "limitations": ["Named Windows user-mode API entries only; direct syscalls and unrelated OS/vendor services are outside observation", "Only this finite scenario and descendants launched under its debug object are observed", "No native target files or host clock changed; RET stub and breakpoints exist only in owned child memory"]}
        self.output.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
        return result

def main():
    p = argparse.ArgumentParser(); p.add_argument("--exe", required=True, type=Path); p.add_argument("--output", required=True, type=Path); p.add_argument("--fake-date"); p.add_argument("--clock-scope", choices=("image","all"),default="image"); p.add_argument("--timeout", type=float, default=180); p.add_argument("--data", required=True, type=Path); p.add_argument("arguments", nargs=argparse.REMAINDER); args = p.parse_args()
    if args.output.exists() or args.data.exists(): raise SystemExit("Fresh owned output/data paths required")
    args.output = args.output.absolute(); args.exe = args.exe.resolve(strict=True); args.data = args.data.absolute()
    if not args.output.parent.is_dir() or not args.data.parent.is_dir(): raise SystemExit("Create an owned parent directory first")
    if args.exe.suffix.lower() != ".exe": raise SystemExit("Explicit local executable required")
    arguments = args.arguments[1:] if args.arguments[:1] == ["--"] else args.arguments
    result = Probe(args.exe, args.output, args.fake_date, args.timeout, {"DRAWER_DATA_DIR": str(args.data), "DRAWER_SMOKE_DIAGNOSTICS": str(args.output.parent)}, args.clock_scope).run(arguments)
    print(json.dumps({key: result[key] for key in ("passed", "error", "clockCalls", "networkAttempts", "ownedProcessesRemaining")}, ensure_ascii=False), flush=True)
    return 0 if result["passed"] else 1

if __name__ == "__main__": raise SystemExit(main())
