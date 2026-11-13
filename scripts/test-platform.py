"""Run the owned app on a real target host; never infer support from cross-checking."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import stat
import subprocess
import tempfile
import time


def target(system, machine):
    machine = machine.lower()
    return {("Windows", "amd64"): "x86_64-pc-windows-msvc",
            ("Windows", "x86_64"): "x86_64-pc-windows-msvc",
            ("Darwin", "arm64"): "aarch64-apple-darwin",
            ("Linux", "x86_64"): "x86_64-unknown-linux-gnu"}.get((system, machine))


def linked(path):
    info = path.lstat()
    return path.is_symlink() or bool(getattr(info, "st_file_attributes", 0) & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400))


def safe_tree(root):
    todo = [root]
    while todo:
        path = todo.pop()
        if linked(path):
            return False
        if path.is_dir():
            todo.extend(path.iterdir())
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--expect-target", required=True)
    args = parser.parse_args()
    current = target(platform.system(), platform.machine())
    if current is None or current != args.expect_target:
        raise SystemExit("Actual host does not match the requested target; no support result produced")
    repo = Path(__file__).resolve().parents[1]
    exe = args.exe.absolute()
    if str(repo).startswith("\\\\") or not exe.is_relative_to(repo) or any(linked(p) for p in reversed([exe, *exe.parents])) or not exe.is_file():
        raise SystemExit("Choose a regular executable inside this local checkout without linked ancestors")
    evidence = args.evidence.absolute()
    if not evidence.is_relative_to(repo) or any(linked(p) for p in reversed([evidence.parent, *evidence.parent.parents])):
        raise SystemExit("Evidence must be under this checkout without linked ancestors")
    root = Path(tempfile.mkdtemp(prefix=".verify-285-platform-", dir=repo)).resolve()
    started = time.monotonic()
    try:
        env = dict(os.environ, DRAWER_DATA_DIR=str(root / "data"), DRAWER_SMOKE_DIAGNOSTICS=str(root))
        env["PATH"] = str(Path(os.environ["SystemRoot"]) / "System32") if platform.system() == "Windows" else "/usr/bin:/bin"
        receipt = root / "receipt.txt"
        kwargs = {"creationflags": subprocess.CREATE_NO_WINDOW} if platform.system() == "Windows" else {}
        with (root / "stdout.txt").open("wb") as out, (root / "stderr.txt").open("wb") as err:
            child = subprocess.Popen([str(exe), "--smoke-test", str(receipt)], cwd=root, env=env, stdout=out, stderr=err, **kwargs)
            try:
                code = child.wait(timeout=120)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
                raise RuntimeError("Owned app timed out; support remains unverified") from None
        if code != 0 or not receipt.is_file():
            raise RuntimeError("Target app failed; support remains unverified")
        flags = dict(line.split("=", 1) for line in receipt.read_text(encoding="utf-8").splitlines() if "=" in line)
        required = ["controller_replay", "mouse_virtual_pressure", "native_generation", "atomic_save", "local_recovery", "saved_state", "sparse_raster", "template_bundles"]
        if any(flags.get(name) != "true" for name in required):
            raise RuntimeError("Target receipt lacks required common document/mouse/storage cases")
        evidence.write_text(json.dumps({"version": 1, "target": current, "system": platform.system(),
            "machine": platform.machine(), "executableSha256": hashlib.sha256(exe.read_bytes()).hexdigest(),
            "commonCases": required, "elapsedSeconds": round(time.monotonic()-started, 3),
            "separateHostDocumentExchange": "pending", "physicalDeviceAndGraphicsFailures": "pending",
            "grantsPlatformSupport": False}, indent=2)+"\n", encoding="utf-8")
    finally:
        if root.parent != repo or not root.name.startswith(".verify-285-platform-"):
            raise RuntimeError("Temporary cleanup escaped checkout")
        if not safe_tree(root):
            raise RuntimeError("Unexpected temporary link preserved for inspection")
        shutil.rmtree(root)


if __name__ == "__main__":
    main()
