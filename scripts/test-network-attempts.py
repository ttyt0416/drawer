"""Fail-closed own-process API scenarios and negative controls; never attach."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid

repo = Path(__file__).resolve().parent.parent
probe = repo/'scripts/win-api-probe.py'


def run(exe, base, name, arguments, timeout=240):
    output = base/(name+'.json')
    command = [sys.executable, '-X', 'utf8', '-B', str(probe), '--exe', str(exe),
               '--output', str(output), '--data', str(base/(name+'-data')),
               '--timeout', str(timeout), '--', *map(str, arguments)]
    child = subprocess.run(command, capture_output=True, text=True, encoding='utf-8',
                           timeout=timeout+30)
    if not output.is_file():
        raise RuntimeError('Probe did not produce its owned-process completion record: '+name)
    result = json.loads(output.read_text(encoding='utf-8'))
    if result['ownedProcessesRemaining']:
        raise RuntimeError('Owned process cleanup incomplete: '+name)
    return child.returncode, result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--evidence', type=Path, default=repo/'docs/process/network-attempts-evidence.json')
    parser.add_argument('--controls-only', action='store_true')
    parser.add_argument('--item', type=int, default=278)
    args = parser.parse_args()
    if not 1 <= args.item <= 309:
        raise ValueError('Checklist item must be1..309')
    base = repo/(f'.verify-{args.item:03d}-network-'+uuid.uuid4().hex)
    base.mkdir()
    controls = [
        ('socket', 'ws2_32', 'socket', '[c.c_int]*3', '(2,1,6)'),
        ('loopback-connect', 'ws2_32', 'connect', '[c.c_size_t,c.c_void_p,c.c_int]', '(0,c.byref(address),c.sizeof(address))'),
        ('loopback-bind', 'ws2_32', 'bind', '[c.c_size_t,c.c_void_p,c.c_int]', '(0,c.byref(address),c.sizeof(address))'),
        ('loopback-listen', 'ws2_32', 'listen', '[c.c_size_t,c.c_int]', '(0,0)'),
        ('address-lookup', 'ws2_32', 'getaddrinfo', '[c.c_void_p]*4', '(None,None,None,None)'),
        ('dns', 'dnsapi', 'DnsQuery_W', '[c.c_void_p,c.c_ushort,c.c_ulong,c.c_void_p,c.c_void_p,c.c_void_p]', '(None,0,0,None,None,None)'),
        ('http', 'winhttp', 'WinHttpOpen', '[c.c_wchar_p,c.c_ulong,c.c_wchar_p,c.c_wchar_p,c.c_ulong]', "('Drawer local test',1,None,None,0)"),
        ('wininet', 'wininet', 'InternetOpenW', '[c.c_wchar_p,c.c_ulong,c.c_wchar_p,c.c_wchar_p,c.c_ulong]', "('Drawer local test',1,None,None,0)"),
        ('url-download', 'urlmon', 'URLDownloadToFileW', '[c.c_void_p,c.c_void_p,c.c_void_p,c.c_ulong,c.c_void_p]', '(None,None,None,0,None)'),
        ('error-report', 'wer', 'WerReportSubmit', '[c.c_void_p,c.c_int,c.c_ulong,c.c_void_p]', '(None,0,0,None)'),
    ]
    receipts = []
    for name, dll, api, types, values in controls:
        source = base/(name+'.py')
        prefix = 'import ctypes as c\n'
        if name in ('loopback-connect', 'loopback-bind'):
            prefix += "class Address(c.Structure):\n _fields_=[('family',c.c_ushort),('port',c.c_ushort),('host',c.c_ubyte*4),('zero',c.c_ubyte*8)]\naddress=Address(2,0,(c.c_ubyte*4)(127,0,0,1),(c.c_ubyte*8)())\n"
        source.write_text(prefix+'f=c.WinDLL('+repr(dll)+').'+api+'\nf.argtypes='+types+'\nf.restype=c.c_size_t\nf(*'+values+')\n', encoding='utf-8')
        code, result = run(Path(sys.executable), base, name, ['-B', source], 45)
        if code == 0 or result['passed'] or len(result['networkAttempts']) != 1 or result['networkAttempts'][0]['api'] != api:
            raise RuntimeError('Negative control did not detect the attempted API: '+name)
        receipts.append({'case': name, 'api': api, 'observedAttempts': 1,
                         'originalApiExecuted': False, 'ownedProcessesRemaining': 0})
        print('Detected before execution:', name, api, flush=True)
    if args.controls_only:
        print('All ten negative controls detected before original execution', flush=True)
        return
    package = repo/'dist/Drawer-windows-x64'
    executable = package/'Drawer.exe'
    receipt = base/'app-receipt.txt'
    code, app = run(executable, base, 'application', ['--smoke-test', receipt])
    text = receipt.read_text(encoding='utf-8') if receipt.is_file() else ''
    if code or not app['passed'] or app['networkAttempts'] or 'error_reporting_disabled=true' not in text or 'data_maintenance=true' not in text:
        raise RuntimeError('Application network/policy/function scenario failed')
    pin = hashlib.sha256((package/'PACKAGE.json').read_bytes()).hexdigest()
    powershell = Path(os.environ['SystemRoot'])/'System32/WindowsPowerShell/v1.0/powershell.exe'
    code, installer = run(powershell, base, 'manual-installer',
                          ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', package/'Install.ps1',
                           '-InstallDir', base/'installed', '-ExpectedManifestSha256', pin, '-AllowUnsigned'])
    if code or not installer['passed'] or installer['networkAttempts']:
        raise RuntimeError('Manual local installer network scenario failed')
    report = {
        'version': 1, 'executableSha256': app['exeSha256'],
        'installerSha256': hashlib.sha256((package/'Install.ps1').read_bytes()).hexdigest(),
        'negativeControls': receipts,
        'scenarios': [{'name': name, 'networkAttempts': len(r['networkAttempts']),
                       'networkApiCount': r['networkApiCount'], 'ownedProcessesRemaining': r['ownedProcessesRemaining'],
                       'coverage': r['coverage'], 'processCount': len(r['processExitCodes'])}
                      for name, r in [('application-full-smoke', app), ('manual-local-installer', installer)]],
        'windowsFaultReportingDisabledInApplication': True,
        'blocking': 'Named user-mode network entries stop all owned processes before the original API executes; would-fail calls also fail the scenario',
        'limitations': app['limitations'] + [
            'Host network adapters were not changed; this is API-level fail-closed observation, not the fresh offline PC272',
            'SEM_NOGPFAULTERRORBOX is verified in the app; unrelated OS service activity and pre-main faults are outside observation',
            'Negative socket/bind/listen/lookup controls use invalid handles/arguments and must still be detected; no server is opened'],
    }
    args.evidence.write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    print('Application and installer attempts=0; all ten controls detected; owned remaining=0', flush=True)


if __name__ == '__main__':
    main()
