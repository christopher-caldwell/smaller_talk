"""Port v3's verified GUI-root detection without reading process arguments."""

from __future__ import annotations

import plistlib
from functools import lru_cache
from pathlib import Path

from .core import Process

CODEX_BUNDLE_ID = "com.openai.codex"


@lru_cache(maxsize=64)
def app_info(bundle: str) -> dict:
    try:
        with (Path(bundle) / "Contents/Info.plist").open("rb") as stream:
            data = plistlib.load(stream)
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError, plistlib.InvalidFileException):
        return {}


def codex_root_bundle(executable: str) -> str | None:
    marker = ".app/Contents/"
    if marker not in executable:
        return None
    bundle = executable.split(marker, 1)[0] + ".app"
    info = app_info(bundle)
    declared = info.get("CFBundleExecutable")
    if info.get("CFBundleIdentifier") != CODEX_BUNDLE_ID or not isinstance(declared, str):
        return None
    return bundle if executable == str(Path(bundle) / "Contents/MacOS" / declared) else None


def matches_root(target: str, process: Process) -> bool:
    if target == "codex-app":
        return codex_root_bundle(process.executable) is not None
    if target == "smaller-talk":
        return Path(process.executable).name.lower() in {
            "smaller-talk-desktop", "smallertalk", "smaller talk"
        }
    return False


def root_candidates(target: str, rows: dict[int, Process]) -> list[int]:
    matches = {p.pid for p in rows.values() if matches_root(target, p)}
    roots = []
    for pid in sorted(matches):
        ancestors = set()
        parent = rows[pid].ppid
        while parent in rows and parent not in ancestors:
            ancestors.add(parent)
            if parent in matches:
                break
            parent = rows[parent].ppid
        else:
            roots.append(pid)
    return roots
