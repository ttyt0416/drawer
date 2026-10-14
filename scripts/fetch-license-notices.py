"""Fetch missing notices from the exact upstream revision declared by cached crates."""
import concurrent.futures
import hashlib
import json
from pathlib import Path
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
metadata = json.loads((ROOT / "target/latest/dependency-metadata.json").read_text(encoding="utf-8-sig"))
selected = {node["id"] for node in metadata["resolve"]["nodes"]}
groups = {}
for package in metadata["packages"]:
    if package["id"] not in selected or not package["source"]:
        continue
    crate = Path(package["manifest_path"]).parent
    if any(file.is_file() and file.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE")) for file in crate.iterdir()):
        continue
    url = urllib.parse.urlparse(package["repository"])
    if url.hostname != "github.com":
        raise RuntimeError(f"Review upstream host for {package['name']}")
    repository = "/".join(url.path.strip("/").split("/")[:2])
    revision = json.loads((crate / ".cargo_vcs_info.json").read_text())["git"]["sha1"]
    groups.setdefault((repository, revision), []).append(package["name"])

destination = ROOT / "docs/licenses/upstream"
destination.mkdir(parents=True, exist_ok=True)

def fetch(job):
    repository, revision, name = job
    url = f"https://raw.githubusercontent.com/{repository}/{revision}/{name}"
    try:
        with urllib.request.urlopen(url, timeout=25) as response:
            data = response.read()
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        raise
    data.decode("utf-8")
    path = destination / (repository.replace("/", "-") + "-" + revision[:12] + "-" + name + ".txt")
    path.write_bytes(data)
    return {"repository": repository, "revision": revision, "packages": groups[(repository, revision)], "source": url, "path": path.relative_to(ROOT).as_posix(), "sha256": hashlib.sha256(data).hexdigest()}

jobs = [(repository, revision, name) for repository, revision in groups for name in ["LICENSE", "LICENSE.txt", "LICENSE.md", "LICENSE-MIT", "LICENSE-APACHE", "COPYING", "NOTICE"]]
with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
    results = [result for result in pool.map(fetch, jobs) if result]
for repository, revision in groups:
    if not any(result["repository"] == repository and result["revision"] == revision for result in results):
        raise RuntimeError(f"No notice found at exact revision: {repository} {revision}")
(ROOT / "docs/licenses/upstream-sources.json").write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8", newline="\n")
print(f"Fetched {len(results)} notices for {len(groups)} upstream revisions")
