"""Validate the repository's OKF bundle and local Markdown references offline."""
import argparse
from collections import Counter
from pathlib import Path
import re
import subprocess
import unicodedata
from urllib.parse import unquote, urlsplit

import yaml

ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"!?\[[^\]\n]*\]\(<?([^\s)>]+)>?(?:\s+[^)]*)?\)")
SOURCE = re.compile(r"https://github\.com/s-kodani/mon/(?:blob|tree)/([0-9a-f]{40})/(.+)")


def prose(text):
    # Ignore examples rather than treating their links/headings as live content.
    return re.sub(r"(?ms)^\s*(`{3,}|~{3,})[^\n]*\n.*?^\s*\1\s*$", "", text)


def anchors(text):
    counts, result = Counter(), set()
    for line in prose(text).splitlines():
        match = re.match(r"^#{1,6}\s+(.+?)\s*#*\s*$", line)
        if not match:
            continue
        label = re.sub(r"!?\[([^]]*)\]\([^)]*\)", r"\1", match[1])
        label = re.sub(r"<[^>]+>", "", label).strip().lower()
        slug = "".join(c for c in label if c in "_- " or unicodedata.category(c)[0] in "LN").replace(" ", "-")
        result.add(slug + (f"-{counts[slug]}" if counts[slug] else ""))
        counts[slug] += 1
    result.update(re.findall(r'<a\s+(?:id|name)=["\']([^"\']+)', text))
    return result


def validate(root=ROOT):
    root = root.resolve()
    pages = set((root / "docs").rglob("*.md"))
    files = pages | {root / "README.md"}
    if (root / "SECURITY.md").exists():
        files.add(root / "SECURITY.md")
    errors, graph = [], {}
    for file in sorted(files):
        text = file.read_text(encoding="utf8")
        relative = file.relative_to(root).as_posix()
        if file in pages:
            parts = text.split("---", 2) if text.startswith("---\n") else []
            try:
                metadata = yaml.safe_load(parts[1]) if len(parts) == 3 else None
                if file == root / "docs/index.md":
                    if not isinstance(metadata, dict) or metadata.get("okf_version") != "0.2":
                        raise ValueError("root index requires okf_version: '0.2'")
                elif file.name == "index.md":
                    if metadata is not None:
                        raise ValueError("child index must not have front matter")
                else:
                    if not isinstance(metadata, dict) or metadata.get("type") not in ("Guide", "Specification", "ADR"):
                        raise ValueError("missing or invalid OKF type")
                    if metadata.get("status") not in ("draft", "stable", "deprecated"):
                        raise ValueError("missing or invalid status")
                    for key in ("title", "description", "tags", "sources"):
                        if not metadata.get(key):
                            raise ValueError(f"missing {key}")
                if isinstance(metadata, dict):
                    for source in metadata.get("sources", []):
                        url = source["resource"]
                        match = SOURCE.fullmatch(url)
                        if url.startswith("https://github.com/s-kodani/mon/") and not match:
                            raise ValueError("repository sources must use a full commit SHA")
                        if match:
                            commit, name = match.groups()
                            name = unquote(name).split("#", 1)[0]
                            if subprocess.run(["git", "cat-file", "-e", f"{commit}:{name}"], cwd=root,
                                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
                                raise ValueError(f"source does not exist at its commit: {name}")
            except (yaml.YAMLError, ValueError, KeyError, TypeError) as error:
                errors.append(f"{relative}: {error}")
        body = text.split("---", 2)[-1] if text.startswith("---\n") else text
        graph[file] = set()
        for target in LINK.findall(prose(body)):
            url = urlsplit(target)
            if url.scheme or url.netloc:
                continue
            destination = (file.parent / unquote(url.path)).resolve() if url.path else file
            if not destination.is_relative_to(root) or not destination.exists():
                errors.append(f"{relative}: missing or escaping target: {target}")
                continue
            if destination.is_dir():
                destination = destination / "index.md"
            if destination in pages:
                graph[file].add(destination)
            if url.fragment and destination.suffix == ".md" and destination.is_file():
                if unquote(url.fragment) not in anchors(destination.read_text(encoding="utf8")):
                    errors.append(f"{relative}: missing anchor: {target}")
    visited, pending = set(), [root / "docs/index.md"]
    while pending:
        file = pending.pop()
        if file not in visited:
            visited.add(file)
            pending.extend(graph.get(file, set()))
    for file in sorted(pages - visited):
        errors.append(f"{file.relative_to(root)}: unreachable from docs/index.md")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args()
    errors = validate(args.root)
    print("\n".join(errors) if errors else "Documentation checks passed")
    return bool(errors)


if __name__ == "__main__":
    raise SystemExit(main())
