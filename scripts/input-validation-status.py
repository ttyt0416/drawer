"""Local evidence/status ledger. Simulated input never certifies a physical pen."""
from pathlib import Path
import copy
import hashlib
import json
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
PROCESS = ROOT / 'docs/process'
IDS = list(range(38, 49)) + [50, 53, 55, 61, 63, 66, 72, 73, 74]
GROUPS = {
    38: ('drawing', 'verification::strokes', 'input_replay'),
    39: ('pointer_input',), 40: ('pressure', 'verification::pressure'),
    41: ('pressure', 'verification::pressure'), 42: ('pointer_input', 'drawing'),
    43: ('pointer_input',), 44: ('pointer_input', 'input_replay'),
    45: ('input_replay', 'verification::pressure'), 46: ('pointer_input',),
    47: ('pen_controls', 'pointer_input', 'pressure'),
    48: ('pointer_input', 'verification::coordinates'),
    50: ('pointer_input', 'drawing'), 53: ('stroke_ends', 'mouse_pressure', 'drawing'),
    55: ('verification::strokes',), 61: ('verification::strokes',),
    63: ('stroke_filter', 'pressure'), 66: ('tip_texture', 'brush_dynamics', 'pointer_input'),
    72: ('brush_composite', 'brush_material'), 73: ('precision', 'drawing'),
    74: ('mouse_pressure', 'brush_dynamics', 'pressure'),
}


def read(name):
    p = PROCESS / name
    if p.stat().st_size > 4 * 1024 * 1024:
        raise ValueError('Evidence exceeds4MiB')
    return json.loads(p.read_text(encoding='utf-8'))


def write(name, value):
    (PROCESS / name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def build():
    proof = read('check-307-suite.json')
    titles = {int(n): title for n, title in re.findall(
        r'^- \[.\] \*\*(\d+)\.\*\* (.+)$',
        (ROOT / 'docs/specs/feature-checklist.md').read_text(encoding='utf-8'), re.M)}
    features = []
    for n in IDS:
        tests = [t for t in proof['selectedInputTests'] if t['name'].startswith(GROUPS[n])]
        if not tests:
            raise ValueError(f'Missing test evidence for{n}')
        features.append({
            'checklist': n, 'feature': titles[n],
            'implementation': 'implemented',
            'virtualInput': {'status': 'passed_simulated_input', 'tests': tests},
            'mouse': {'status': 'passed_simulated_mouse_pipeline' if n in [38, 39, 43, 44, 46, 48, 50, 53, 55, 61, 63, 66, 72, 73, 74] else 'not_a_pressure_device',
                      'evidence': ['check-287-evidence.json', 'check-288-evidence.json', 'check-289-evidence.json', 'check-301-evidence.json']},
            'physicalPen': {'status': 'pending_by_user', 'validated': False,
                            'device': None, 'driver': None, 'os': None, 'backend': None,
                            'samplingHz': None, 'testedAt': None, 'evidence': [],
                            'pendingChecks': [296, 297, 298, 299, 300]},
        })
    value = {'format': 'Drawer input validation status', 'version': 1,
             'asOfExecutableSha256': proof['executableSha256'], 'nativeVersion': proof['nativeVersion'],
             'suiteEvidence': 'check-307-suite.json',
             'definitions': {'virtualInput': 'Production pipeline with synthetic events; not physical-device evidence',
                             'mouse': 'Synthetic mouse events through the editor; no OS/human mouse certification',
                             'adapterInitialization': 'Initialization alone does not prove receipt of real pen input',
                             'physicalPen': 'Deferred by user; device/driver/result remain empty until measured'},
             'features': features,
             'aggregate': {'implemented': len(features), 'physicalPenValidated': 0}}
    validate(value)
    write('input-validation-status.json', value)
    lines = ['# 입력 기능별 검증 상태 (307)', '',
             '구현·가상 입력·합성 마우스 처리·실물 펜 검증을 분리한다. 마우스 통과와 어댑터 초기화를 실물 펜 호환으로 집계하지 않는다. 실물 펜은 사용자 지시대로 대기하며 장치/드라이버/표본율/실측 결과가 없다.', '',
             '| 번호 | 기능 | 구현 | 가상 입력 | 마우스 | 실물 펜 |',
             '|---|---|---|---|---|---|']
    for f in features:
        mouse = '합성 입력 통과' if f['mouse']['status'].startswith('passed') else '압력 장치 아님'
        lines.append(f"| {f['checklist']:03} | {f['feature']} | 완료 | 통과 | {mouse} | 대기 |")
    lines += ['', f"기록한 배포본 SHA-256: `{proof['executableSha256']}` · Native{proof['nativeVersion']}. 시험{proof['passed']}개 중 입력 관련{len(proof['selectedInputTests'])}개의 이름·출처·소스/시험 로그 해시를 [시험 증거](check-307-suite.json)에 보관한다. 개별 CLI 증거는 각기 해당 배포 해시를 가진 과거 시험이며 현재의 실물 장치 결과로 해석하지 않는다.", '',
              '[기계 판독 기록](input-validation-status.json)은20개 기능을 구분한다. 실제 장치 검증을 수행할 때는296~300 절차에 맞춰 별도 기록을 추가해야 한다. 이 기록의 true/false를 가상 시험 결과에서 자동 승격하지 않는다.', '',
              '`python -X utf8 -B scripts/input-validation-status.py --check`는 상태·시험 참조·원본 펜 메타데이터 공란·실물 합격0을 확인한다. `--write`는 고정된 시험 증거로 표를 생성한다.']
    (PROCESS / 'input-validation-status.md').write_text('\n'.join(lines) + '\n', encoding='utf-8')


def validate(value):
    proof = read('check-307-suite.json')
    if value['format'] != 'Drawer input validation status' or value['version'] != 1:
        raise ValueError('Status identity mismatch')
    if value['asOfExecutableSha256'] != proof['executableSha256'] or value['nativeVersion'] != proof['nativeVersion']:
        raise ValueError('Build evidence mismatch')
    if [x['checklist'] for x in value['features']] != IDS:
        raise ValueError('Missing/repeated input feature')
    tests = {(x['crate'], x['name']) for x in proof['selectedInputTests']}
    if proof['passed'] != sum(proof['counts'].values()) or not tests:
        raise ValueError('Invalid suite result')
    for f in value['features']:
        p = f['physicalPen']
        if p['status'] != 'pending_by_user' or p['validated'] is not False or p['evidence']:
            raise ValueError('Simulation cannot certify a physical pen')
        if any(p[k] is not None for k in ['device', 'driver', 'os', 'backend', 'samplingHz', 'testedAt']):
            raise ValueError('Pending pen must not invent device metadata')
        if p['pendingChecks'] != [296, 297, 298, 299, 300]:
            raise ValueError('Physical procedure coverage incomplete')
        if f['implementation'] != 'implemented' or f['virtualInput']['status'] != 'passed_simulated_input':
            raise ValueError('Unexpected virtual/implementation state')
        if not f['virtualInput']['tests'] or any((t['crate'], t['name']) not in tests for t in f['virtualInput']['tests']):
            raise ValueError('Missing or invented test reference')
        for name in f['mouse']['evidence']:
            report = read(name)
            if report['result']['physicalDeviceValidated'] is not False:
                raise ValueError('Mouse evidence conflated with physical pen')
    if value['aggregate'] != {'implemented': len(IDS), 'physicalPenValidated': 0}:
        raise ValueError('Physical total conflated')


def check():
    value = read('input-validation-status.json')
    validate(value)
    for case in ['physical', 'metadata', 'count', 'test']:
        bad = copy.deepcopy(value)
        if case == 'physical': bad['features'][0]['physicalPen']['validated'] = True
        elif case == 'metadata': bad['features'][0]['physicalPen']['device'] = 'invented'
        elif case == 'count': bad['aggregate']['physicalPenValidated'] = 1
        else: bad['features'][0]['virtualInput']['tests'][0]['name'] = 'not_run'
        try:
            validate(bad)
        except ValueError:
            continue
        raise ValueError(f'Invalid status accepted:{case}')
    print(f'Input status verified: {len(IDS)} features; physical pen passes=0;4 invalid fixtures rejected')


if __name__ == '__main__':
    if sys.argv[1:] == ['--write']: build()
    elif sys.argv[1:] == ['--check']: check()
    else: raise SystemExit('Use --write or --check')
