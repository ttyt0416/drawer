"""Local release integrity record; no publisher signature is claimed."""
import hashlib
import json
from pathlib import Path
import re
import tomllib

root = Path(__file__).resolve().parent.parent
package = root / 'dist/Drawer-windows-x64'
names = ['Drawer.exe', 'THIRD-PARTY-NOTICES.txt', 'COMPONENTS.json', 'Cargo.lock',
         'OWN-ASSETS.md', 'START-HERE.md', 'MOVIE-IMPORT.md', 'IMAGE-IMPORT.md',
         'HELP.md', 'MODEL-FORMATS.md', 'Install.ps1', 'Uninstall.ps1']
assert set(p.name for p in package.iterdir()) <= set(names + ['PACKAGE.json'])
version = tomllib.loads((root/'Cargo.toml').read_text(encoding='utf-8'))['workspace']['package']['version']
native = int(re.search(r'const VERSION: u32 = (\d+)', (root/'crates/drawer-core/src/files.rs').read_text(encoding='utf-8'))[1])
manifest = {'format': 1, 'product': 'Drawer', 'version': version, 'nativeMin': 1,
            'nativeMax': native, 'settingsVersion': 1,
            'files': [{'name': n, 'bytes': (package/n).stat().st_size,
                       'sha256': hashlib.sha256((package/n).read_bytes()).hexdigest()} for n in names]}
target = package/'PACKAGE.json'
temporary = package/'.PACKAGE.write.tmp'
with temporary.open('x', encoding='utf-8', newline='\n') as f:
    json.dump(manifest, f, ensure_ascii=False, indent=2)
    f.write('\n')
temporary.replace(target)
print('Unsigned local package manifest SHA256:', hashlib.sha256(target.read_bytes()).hexdigest())
