"""Locate immutable main build outputs and link their exact source to a draft."""
import argparse
import json
import os
from pathlib import Path
import re
from urllib.parse import urlencode

from mon_ci_scope import changed_paths, select
from mon_release import GitHub, validate_versions

PREFIX = "release-candidate-"
WORKFLOW = ".github/workflows/release-build.yml"


def find_candidate(api, source, *, completed=True):
    if not re.fullmatch(r"[0-9a-f]{40}", source or ""):
        raise ValueError("full candidate source SHA required")
    found, expired = [], False
    for page in range(1, 101):
        response = api.call(f"/actions/artifacts?name={PREFIX}{source}&per_page=100&page={page}")
        artifacts = response["artifacts"]
        for artifact in artifacts:
            producer = artifact.get("workflow_run", {})
            if artifact["name"] != PREFIX + source or producer.get("head_sha") != source:
                continue
            run = api.call(f"/actions/runs/{producer['id']}")
            if (run.get("path") != WORKFLOW or run.get("head_branch") != "main" or
                    run.get("head_sha") != source or run.get("event") not in ("push", "workflow_dispatch")):
                continue
            if completed and (run.get("status") != "completed" or run.get("conclusion") != "success"):
                continue
            if not completed:
                admitted = run
                # Re-running a successful run changes its current status before
                # guard starts. Keep the successful prior attempt's artifact.
                if (str(run["id"]) == os.environ.get("GITHUB_RUN_ID") and
                        int(os.environ.get("GITHUB_RUN_ATTEMPT", "1")) > 1):
                    attempt = int(os.environ["GITHUB_RUN_ATTEMPT"]) - 1
                    admitted = api.call(f"/actions/runs/{run['id']}/attempts/{attempt}")
                if admitted.get("status") != "completed" or admitted.get("conclusion") != "success":
                    raise ValueError("candidate already exists in an incomplete or failed run; reconcile it without overwriting")
            if artifact.get("expired"):
                expired = True
                continue
            found.append({"artifactId": artifact["id"], "runId": run["id"], "sourceSha": source})
        if len(artifacts) < 100:
            break
    else:
        raise ValueError("candidate pagination exceeds limit")
    if len(found) > 1:
        raise ValueError("multiple main candidates for this SHA; select and reconcile explicitly")
    if not found and expired:
        raise ValueError("main candidate expired; no rebuild or source substitution is performed")
    return found[0] if found else None


def guard(api):
    if os.environ.get("GITHUB_REF") != "refs/heads/main":
        raise ValueError("main builds must run on main")
    source = os.environ["GITHUB_SHA"]
    if not re.fullmatch(r"[0-9a-f]{40}", source):
        raise ValueError("full main source SHA required")
    manual = os.environ["GITHUB_EVENT_NAME"] == "workflow_dispatch"
    names = [] if manual else changed_paths(os.environ.get("MON_BASE", ""))
    code = select(names or [], main=True, full=manual or names is None)["code"]
    previous = find_candidate(api, source, completed=False) if code else None
    tag = "v" + json.loads(Path("app/package.json").read_text())["version"]
    if code:
        validate_versions(tag, lambda name: Path(name).read_text())
    values = {"source_sha": source, "tag": tag, "build": str(code and previous is None).lower(),
              "reuse": str(previous is not None).lower()}
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf8") as stream:
        stream.writelines(f"{key}={value}\n" for key, value in values.items())
    if previous:
        print(f"Retaining existing candidate from run {previous['runId']}; it will not be overwritten")


def summary():
    source, tag = os.environ["MON_SOURCE_SHA"], os.environ["MON_TAG"]
    repository = os.environ["GITHUB_REPOSITORY"]
    run = f"https://github.com/{repository}/actions/runs/{os.environ['GITHUB_RUN_ID']}"
    body = f"Source: `{source}`\nMain build: {run}\n\nWindows EXEは未署名です。配布証跡の未確認項目を確認してください。"
    url = f"https://github.com/{repository}/releases/new?" + urlencode({
        "tag": tag, "target": source, "title": tag, "body": body,
    })
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf8") as stream:
        stream.write(f"Source: `{source}`\n\n[最終ローカル検証用の配布物]({os.environ['MON_ARTIFACT_URL']})（30日保存）\n\n"
                     f"ローカル確認後、[このコミットのRelease下書きを作成]({url})してください。\n\n"
                     "下書きへの自動添付完了を確認して、GitHubのPublish releaseで公開します。"
                     "公開時の再ビルド・テストはありません。Store送信はActionsの承認後です。\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("guard", "summary"))
    args = parser.parse_args()
    if args.mode == "guard":
        guard(GitHub())
    else:
        summary()
