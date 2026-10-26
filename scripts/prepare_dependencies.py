#!/usr/bin/env python3
"""Restore checksum-pinned crates and apply the application's local fixes."""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile
import tempfile


ROOT = Path(__file__).resolve().parents[1]
PATCHES = ROOT / "patches"


def tree_sha256(directory):
    digest = hashlib.sha256()
    for path in sorted(directory.rglob("*"), key=lambda p: p.relative_to(directory).as_posix()):
        if path.is_symlink():
            raise RuntimeError(f"Unexpected symlink: {path}")
        if path.is_file() and path.relative_to(directory).as_posix() != "PATCHES.md":
            relative = path.relative_to(directory).as_posix()
            digest.update(relative.encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def archive_bytes(package):
    filename = f"{package['package']}-{package['version']}.crate"
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    cache = ROOT / ".dev" / "dependency-archives"
    candidates = [cache / filename, *sorted((cargo_home / "registry" / "cache").glob(f"*/{filename}"))]
    for candidate in candidates:
        if candidate.is_file():
            data = candidate.read_bytes()
            if hashlib.sha256(data).hexdigest() != package["sha256"]:
                raise RuntimeError(f"Checksum mismatch: {candidate}; remove the corrupt archive and retry")
            return data
    url = f"https://static.crates.io/crates/{package['package']}/{filename}"
    print(f"Downloading {filename}", flush=True)
    data = subprocess.run(
        ["curl", "--fail", "--location", "--silent", "--show-error", "--max-time", "60", url],
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
    if hashlib.sha256(data).hexdigest() != package["sha256"]:
        raise RuntimeError(f"Checksum mismatch for {url}")
    cache.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=cache, delete=False) as temporary:
        temporary.write(data)
        temporary_path = Path(temporary.name)
    temporary_path.replace(cache / filename)
    return data


def unpack(data, destination, prefix):
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        for member in archive.getmembers():
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or path.parts[0] != prefix:
                raise RuntimeError(f"Unsafe archive path: {member.name}")
            if member.isdir():
                continue
            if not member.isfile():
                raise RuntimeError(f"Unsupported archive entry: {member.name}")
            target = destination.joinpath(*path.parts[1:])
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.extractfile(member) as source, target.open("wb") as output:
                shutil.copyfileobj(source, output)


def prepare(package, vendor):
    destination = vendor / package["directory"]
    if destination.exists():
        if tree_sha256(destination) != package["tree_sha256"]:
            raise RuntimeError(
                f"{destination} differs from the pinned patched source. "
                "Preserve any local edits, then move that directory aside and retry."
            )
        print(f"Ready: {package['package']} {package['version']}")
        return
    data = archive_bytes(package)
    vendor.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".prepare-", dir=vendor) as temporary:
        staging = Path(temporary) / package["directory"]
        staging.mkdir()
        unpack(data, staging, f"{package['package']}-{package['version']}")
        subprocess.run(
            ["patch", "--batch", "--forward", "-p1", "-i", str(PATCHES / f"{package['directory']}.patch")],
            cwd=staging,
            check=True,
        )
        shutil.copyfile(PATCHES / f"{package['directory']}.md", staging / "PATCHES.md")
        if tree_sha256(staging) != package["tree_sha256"]:
            raise RuntimeError(f"Patched source checksum mismatch: {package['package']}")
        staging.rename(destination)
    print(f"Prepared: {package['package']} {package['version']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vendor-dir", type=Path, default=ROOT / "vendor")
    args = parser.parse_args()
    try:
        for package in json.loads((PATCHES / "dependencies.json").read_text()):
            prepare(package, args.vendor_dir.resolve())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Dependency setup failed: {error}\n")


if __name__ == "__main__":
    main()
