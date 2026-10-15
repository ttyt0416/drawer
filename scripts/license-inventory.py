"""Generate and verify notices from the locked Windows dependency graph."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser()
parser.add_argument("--check", action="store_true")
check = parser.parse_args().check
result = subprocess.run(["cargo", "metadata", "--offline", "--locked", "--format-version", "1", "--filter-platform", "x86_64-pc-windows-msvc"], cwd=ROOT, check=True, capture_output=True)
metadata = json.loads(result.stdout)
selected = {node["id"] for node in metadata["resolve"]["nodes"]}
lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
checksums = {(package["name"], package["version"]): package.get("checksum") for package in lock["package"]}
upstream = json.loads((ROOT / "docs/licenses/upstream-sources.json").read_text())
patched = json.loads((ROOT / "docs/licenses/patched-sources.json").read_text())
for patch in patched:
    directory = ROOT / patch["path"]
    actual = {file.relative_to(directory).as_posix(): hashlib.sha256(file.read_bytes()).hexdigest() for file in directory.rglob("*") if file.is_file()}
    if actual != patch["files"]:
        raise RuntimeError(f"Patched dependency source changed: {patch['name']}")
for source in upstream:
    data = (ROOT / source["path"]).read_bytes()
    if hashlib.sha256(data).hexdigest() != source["sha256"]:
        raise RuntimeError(f"Upstream notice changed: {source['path']}")

components = []
notices = ["Drawer third-party notices\nLocked Windows x64 dependency graph; generated from Cargo.lock and upstream notices.\n"]
for package in sorted(metadata["packages"], key=lambda package: (package["name"], package["version"])):
    patch = next((p for p in patched if (p["name"], p["version"]) == (package["name"], package["version"])), None)
    if package["id"] not in selected or (not package["source"] and not patch):
        continue
    if not package["license"]:
        raise RuntimeError(f"Missing license declaration: {package['name']}")
    crate = Path(package["manifest_path"]).parent
    if patch and crate.resolve() != (ROOT / patch["path"]).resolve():
        raise RuntimeError("Patched dependency path does not match its source inventory")
    files = [file for file in crate.iterdir() if file.is_file() and file.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE"))]
    if package["license_file"]:
        files.append(crate / package["license_file"])
    if package["name"] == "epaint_default_fonts":
        files.extend((crate / "fonts").glob("*.txt"))
    supplement = [source for source in upstream if package["name"] in source["packages"]]
    files.extend(ROOT / source["path"] for source in supplement)
    files = sorted(set(files))
    if not files:
        raise RuntimeError(f"Missing redistribution notice: {package['name']}")
    component = {"name": package["name"], "version": package["version"], "source": f"https://crates.io/crates/{package['name']}/{package['version']}", "repository": package["repository"], "license": package["license"], "crate_sha256": patch["crate_sha256"] if patch else checksums[(package["name"], package["version"])], "notice_files": [{"name": file.name, "sha256": hashlib.sha256(file.read_bytes()).hexdigest()} for file in files], "supplement_sources": [source["source"] for source in supplement]}
    if patch:
        component["local_patch"] = {"manifest": "docs/licenses/patched-sources.json", "upstream_revision": patch["upstream_revision"], "files": patch["files"]}
    components.append(component)
    notices.append(f"\n{'=' * 72}\n{package['name']} {package['version']}\n{component['source']}\nDeclared license: {component['license']}\n")
    if patch:
        notices.append("\nLocal modification: Windows Ink StylusButtonUp reports release as pressed=false.\nUpstream source and exact patched file hashes: COMPONENTS.json / local_patch.\n")
    for file in files:
        notices.append(f"\n--- {file.name} ---\n" + file.read_text(encoding="utf-8") + "\n")

font_crate = next(Path(package["manifest_path"]).parent for package in metadata["packages"] if package["name"] == "epaint_default_fonts")
fonts = [{"name": name, "version": "bundled with epaint_default_fonts 0.36.2", "source": "https://github.com/emilk/egui/tree/49682f8baa058bf49e011035cfbd6e825f88a5ef/crates/epaint_default_fonts/fonts", "license_notice": notice, "sha256": hashlib.sha256((font_crate / "fonts" / name).read_bytes()).hexdigest()} for name, notice in [("Hack-Regular.ttf", "Hack-Regular.txt"), ("NotoEmoji-Regular.ttf", "OFL.txt"), ("Ubuntu-Light.ttf", "UFL.txt"), ("emoji-icon-font.ttf", "emoji-icon-font-mit-license.txt")]]
inventory = {"target": "x86_64-pc-windows-msvc", "cargo_lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest(), "libraries": components, "embedded_fonts": fonts, "own_assets": "docs/process/own-assets.md", "external_models": [], "unverified_external_components": []}
outputs = {"docs/licenses/inventory.json": json.dumps(inventory, ensure_ascii=False, indent=2) + "\n", "docs/licenses/third-party-notices.txt": "".join(notices).replace("\r\n", "\n")}
for name, text in outputs.items():
    destination = ROOT / name
    if check:
        if not destination.exists() or destination.read_text(encoding="utf-8") != text:
            raise RuntimeError(f"Inventory needs regeneration: {name}")
    else:
        destination.write_text(text, encoding="utf-8", newline="\n")
print(f"{'Verified' if check else 'Recorded'} {len(components)} libraries, {len(fonts)} embedded fonts; missing licenses/notices=0")
