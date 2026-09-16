#!/usr/bin/env python3
"""Check that every file naming vitri's release version names the same one.

    check-versions.py [tag]

With a tag (as the release workflow passes it, e.g. `v0.2.0`), the tag must
name that version too. Prints each disagreement and exits 1.

A release archive is named after its tag, and the crate reports the version in
Cargo.toml, so a tag cut before the bump publishes files that disagree about
what they are. A binding package adds its manifest to `SOURCES`; the reader is
chosen by file name.
"""

import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# The first entry is the version the others are compared with.
SOURCES = [
    "Cargo.toml",
    "Cargo.lock",
    "CHANGELOG.md",
    "bindings/python/pyproject.toml",
    "bindings/python/Cargo.toml",
    "bindings/python/Cargo.lock",
    "bindings/c/Cargo.toml",
    "bindings/wasm/Cargo.toml",
]


def text(path):
    return path.read_text(encoding="utf-8")


def cargo_manifest(path):
    version = tomllib.loads(text(path))["package"]["version"]
    if not isinstance(version, str):
        raise ValueError("`package.version` is not a literal string")
    return version


def cargo_lock(path):
    entries = [p for p in tomllib.loads(text(path))["package"] if p["name"] == "vitri"]
    if len(entries) != 1:
        raise ValueError(f"{len(entries)} entries for the vitri package")
    return entries[0]["version"]


def pyproject(path):
    """`project.version`, or, when maturin supplies the version, the version of
    the Cargo.toml beside the file, which is where maturin reads it."""
    manifest = tomllib.loads(text(path))
    project = manifest["project"]
    if "version" in project:
        return project["version"]
    backend = manifest.get("build-system", {}).get("build-backend")
    if "version" in project.get("dynamic", []) and backend == "maturin":
        return cargo_manifest(path.with_name("Cargo.toml"))
    raise ValueError("no static `project.version`; list the manifest the version comes from")


def package_json(path):
    return json.loads(text(path))["version"]


def changelog(path):
    """The newest released section: the first `## ` heading other than `Unreleased`."""
    for heading in re.findall(r"^## +(.+?)\s*$", text(path), re.M):
        if heading != "Unreleased":
            return heading
    raise ValueError("no released section")


READERS = {
    "Cargo.toml": cargo_manifest,
    "Cargo.lock": cargo_lock,
    "pyproject.toml": pyproject,
    "package.json": package_json,
    "CHANGELOG.md": changelog,
}


def read(source):
    path = ROOT / source
    try:
        return READERS[path.name](path), None
    except (OSError, KeyError, ValueError, TypeError) as error:
        return None, f"{source}: cannot read the version ({type(error).__name__}: {error})"


def main():
    if len(sys.argv) > 2:
        sys.exit(__doc__)
    tag = sys.argv[1] if len(sys.argv) == 2 else ""

    problems = []
    versions = []
    for source in SOURCES:
        version, problem = read(source)
        if problem:
            problems.append(problem)
        versions.append((source, version))

    reference_source, reference = versions[0]
    if reference is not None:
        for source, version in versions[1:]:
            if version is not None and version != reference:
                problems.append(f"{source} names {version}, but {reference_source} names {reference}")
        if tag and tag != f"v{reference}":
            problems.append(f"tag {tag} does not name version {reference} (expected v{reference})")

    if problems:
        print("\n".join(problems))
        sys.exit(1)
    print(f"{', '.join(SOURCES)}{' and tag ' + tag if tag else ''} name version {reference}")


if __name__ == "__main__":
    main()
