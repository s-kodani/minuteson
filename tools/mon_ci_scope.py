"""Select native CI without skipping the always-running final PR gate."""
import os
import re
import subprocess


def needs_native(names):
    prefixes = ("app/", "crates/", ".cargo/", "probes/", "tools/", "tests/", "third-party/", ".github/workflows/")
    return any(name in ("Cargo.toml", "Cargo.lock", "Makefile") or name.startswith(prefixes) for name in names)


def main():
    base = os.environ.get("MON_BASE", "")
    if os.environ.get("MON_FULL") == "true" or not re.fullmatch(r"[0-9a-f]{40}", base) or base == "0" * 40:
        value = True
    else:
        result = subprocess.run(["git", "diff", "--name-only", "-z", base, "HEAD"], capture_output=True, check=True)
        value = needs_native(result.stdout.decode().split("\0"))
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf8") as stream:
        stream.write(f"native={str(value).lower()}\n")


if __name__ == "__main__":
    main()
