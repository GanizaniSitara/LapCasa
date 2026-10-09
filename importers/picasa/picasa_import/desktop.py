"""Private JSON-file bridge for the desktop Settings component.

Preview saves the exact verified plan; apply never silently rebuilds it. The
desktop host owns the job directory and checks the active library before apply.
"""
import argparse
from collections import Counter
from dataclasses import asdict
import json
from pathlib import Path
import sqlite3
import sys

from .core import Candidate, ImportError, Mapping, Plan, apply_plan, assert_output_path, make_plan


def write_new(path, value):
    with path.open("x", encoding="utf-8") as handle:
        json.dump(value, handle, ensure_ascii=False, default=str, indent=2)


def preview(job: Path, progress=None):
    request = json.loads((job / "request.json").read_text(encoding="utf-8"))
    mappings = [Mapping(Path(m["source"]), Path(m["target"])) for m in request["mappings"]]
    plan = make_plan(Path(request["database"]), Path(request["contacts"]), mappings,
                     request.get("coordinates", "raw"), progress)
    assert_output_path(job / "preview.json", plan)
    assert_output_path(job / "plan.json", plan)
    saved = dict(vars(plan), counts=dict(plan.counts),
                 candidates=[asdict(c) for c in plan.candidates],
                 mappings=[asdict(m) for m in plan.mappings])
    write_new(job / "plan.json", saved)
    report = plan.report()
    write_new(job / "preview.json", report)
    return {"counts": report["counts"], "skipped": dict(Counter(i["reason"] for i in plan.issues))}


def load_plan(job: Path):
    data = json.loads((job / "plan.json").read_text(encoding="utf-8"))
    data["database"] = Path(data["database"])
    data["contacts_path"] = Path(data["contacts_path"])
    data["candidates"] = [Candidate(**c) for c in data["candidates"]]
    data["mappings"] = [Mapping(Path(m["source"]), Path(m["target"])) for m in data["mappings"]]
    data["counts"] = Counter(data["counts"])
    return Plan(**data)


def apply(job: Path, *, allow_live=False):
    if (job / "result.json").exists():
        return json.loads((job / "result.json").read_text(encoding="utf-8"))
    plan = load_plan(job)
    request = json.loads((job / "request.json").read_text(encoding="utf-8"))
    if plan.database.resolve() != Path(request["database"]).resolve():
        raise ImportError("Preview library does not match the request")
    result = apply_plan(plan, job / "before-import.db", allow_live=allow_live)
    write_new(job / "result.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("preview", "apply"))
    parser.add_argument("job", type=Path)
    parser.add_argument("--desktop-user-confirmed", action="store_true")
    args = parser.parse_args()
    try:
        progress = lambda value: print(json.dumps({"event": "progress", **value}), flush=True)
        result = (preview(args.job, progress) if args.mode == "preview" else
                  apply(args.job, allow_live=args.desktop_user_confirmed))
        print(json.dumps({"event": "result", **result}), flush=True)
        return 0
    except (ImportError, OSError, ValueError, KeyError, TypeError, sqlite3.Error) as exc:
        print(json.dumps({"error": str(exc)}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
