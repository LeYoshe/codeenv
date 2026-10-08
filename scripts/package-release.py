#!/usr/bin/env python3
"""Package a native release binary, documentation, and dependency notices."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "aarch64-apple-darwin": "macos-arm64",
    "aarch64-unknown-linux-gnu": "linux-arm64",
    "x86_64-unknown-linux-gnu": "linux-amd64",
}


def dependency_notices(target):
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--offline", "--format-version", "1",
         "--filter-platform", target], cwd=ROOT, text=True,
    ))
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    reachable = set()
    pending = [metadata["resolve"]["root"]]
    while pending:
        package_id = pending.pop()
        if package_id in reachable:
            continue
        reachable.add(package_id)
        pending.extend(nodes[package_id]["dependencies"])
    sections = ["Rust dependency notices\n\nIncludes resolved build dependencies for this target.\n"]
    for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
        if package["id"] not in reachable or package["source"] is None:
            continue
        root = Path(package["manifest_path"]).parent
        licenses = sorted(path for path in root.rglob("*") if path.is_file()
                          and path.name.lower().startswith(("license", "copying", "notice")))
        if package.get("license_file"):
            licenses = sorted(set(licenses + [root / package["license_file"]]))
        heading = f"{package['name']} {package['version']} — {package.get('license') or 'see license file'}"
        sections.append(heading + "\n" + (package.get("repository") or "") + "\n")
        if not licenses:
            # Keep metadata visible rather than silently omitting a dependency.
            raise RuntimeError(f"No license text found for {heading}")
        for path in licenses:
            sections.append(f"--- {path.relative_to(root)} ---\n{path.read_text(errors='replace').strip()}\n")
    return "\n".join(sections)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", choices=TARGETS)
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    name = f"codeenv-{version}-{TARGETS[args.target]}"
    binary = ROOT / "target" / args.target / "release" / "codeenv"
    if not binary.is_file():
        raise SystemExit(f"Build first: cargo build --locked --release --target {args.target}")
    notices = dependency_notices(args.target)
    destination = ROOT / "dist"
    destination.mkdir(exist_ok=True)
    archive = destination / f"{name}.tar.gz"
    with tempfile.TemporaryDirectory(prefix="codeenv-package-") as temporary:
        folder = Path(temporary) / name
        folder.mkdir()
        for filename in ["LICENSE", "README.md", "CONTRIBUTING.md", "SECURITY.md", "THIRD_PARTY_NOTICES.md", "config.example.toml"]:
            shutil.copy2(ROOT / filename, folder / filename)
        shutil.copy2(binary, folder / "codeenv")
        for directory in ["docs", "deploy"]:
            shutil.copytree(ROOT / directory, folder / directory)
        license_dir = folder / "licenses"
        license_dir.mkdir()
        for path in (ROOT / "web/vendor").glob("LICENSE*"):
            shutil.copy2(path, license_dir / path.name)
        (license_dir / "RUST-DEPENDENCIES.txt").write_text(notices)
        with tarfile.open(archive, "w:gz") as output:
            output.add(folder, arcname=name)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(archive.suffix + ".sha256").write_text(f"{digest}  {archive.name}\n")
    print(archive.name)


if __name__ == "__main__":
    main()
