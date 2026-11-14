"""Select affected PR checks; unknown paths and main code changes fail closed."""
import json
import os
from pathlib import PurePosixPath
import re
import subprocess

LANGUAGES = {
    "actions": ("ubuntu-24.04", "none"),
    "javascript-typescript": ("ubuntu-24.04", "none"),
    "python": ("ubuntu-24.04", "none"),
    "rust": ("macos-15", "none"),
    "c-cpp": ("windows-2022", "none"),
    "swift": ("macos-15-intel", "manual"),
}
CHECKS = ("python", "browser", "native", "dependencies")
NATIVE_TOOLS = ("mon_ci_", "mon_rust_", "mon_contracts", "mon_model_catalog",
                "mon_inference_", "mon_llama_", "mon_whisper_", "mon_windows_",
                "mon_macos_", "mon_owned_engine", "mon_minutes_models_native")


def documentation(name):
    # Markdown shipped with the app and license notices are runtime inputs.
    return (name in ("README.md", "SECURITY.md") or name.startswith(("docs/", "assets/readme/"))
            or (name.startswith("evaluation/") and name.endswith(".md")))


def select(names, *, full=False, main=False):
    checks, languages = set(), set()
    for name in filter(None, names):
        path = PurePosixPath(name)
        if documentation(name):
            continue
        if name.startswith(("tools/", "tests/")) and path.suffix in (".py", ".json"):
            checks.add("python")
            languages.add("python")
            stem = path.stem.removeprefix("test_")
            if stem.startswith(NATIVE_TOOLS) or name == "tools/rust-test-suites.json":
                checks |= {"native", "browser"}
                languages |= {"rust", "c-cpp", "swift"}
            if "security" in stem or name == "tools/security-exceptions.json":
                checks.add("dependencies")
        elif name.startswith(("app/src/", "app/tests/", "app/scripts/")) or name in (
                "app/index.html", "app/tsconfig.json", "app/vite.config.ts", "app/playwright.config.mjs"):
            checks.add("browser")
            languages.add("javascript-typescript")
            if "contract" in name or name.startswith("app/src/generated/"):
                checks.add("native")
                languages.add("rust")
        elif name.startswith(("app/src-tauri/", "crates/", "probes/", ".cargo/")) or name in ("Cargo.toml", "Cargo.lock"):
            checks.add("native")
            languages.add("rust")
            if path.suffix == ".swift" or name.startswith("probes/macos-"):
                languages.add("swift")
            if path.suffix in (".c", ".cpp", ".h", ".hpp") or name.startswith("probes/windows-"):
                languages.add("c-cpp")
            if (path.name in ("lib.rs", "build.rs", "Cargo.toml", "Cargo.lock") or
                    name.startswith(("app/src-tauri/capabilities/", "crates/mon-contracts/")) or
                    "contract" in name or path.suffix == ".json"):
                checks.add("browser")
                languages.add("javascript-typescript")
            if path.name in ("Cargo.toml", "Cargo.lock"):
                checks.add("dependencies")
        else:
            # Includes workflows, build scripts, dependency/license assets, and
            # newly introduced directories until an explicit scope is reviewed.
            checks.update(CHECKS)
            languages.update(LANGUAGES)
    code = bool(checks)
    if full or (main and code):
        checks.update(CHECKS)
        languages.update(LANGUAGES)
    return {**{key: key in checks for key in CHECKS}, "code": code or full,
            "has_codeql": bool(languages),
            "codeql": {"include": [{"language": key, "os": LANGUAGES[key][0], "mode": LANGUAGES[key][1]}
                                    for key in LANGUAGES if key in languages]}}


def changed_paths(base):
    if not re.fullmatch(r"[0-9a-f]{40}", base or "") or base == "0" * 40:
        return None
    # Renames are delete+add so moving code into docs never hides its deletion.
    result = subprocess.run(["git", "diff", "--no-renames", "--name-only", "-z", base, "HEAD"],
                            capture_output=True, check=True)
    return result.stdout.decode("utf8").split("\0")


def main():
    full = os.environ.get("MON_FULL") == "true"
    names = [] if full else changed_paths(os.environ.get("MON_BASE", ""))
    values = select(names or [], full=full or names is None,
                    main=os.environ.get("MON_MAIN") == "true")
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf8") as stream:
        for key, value in values.items():
            stream.write(f"{key}={json.dumps(value, separators=(',', ':'))}\n")


if __name__ == "__main__":
    main()
