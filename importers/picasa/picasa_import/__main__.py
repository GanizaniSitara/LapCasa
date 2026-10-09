"""CLI for the offline Picasa importer. Preview by default; JSON output only."""
import argparse
import json
from pathlib import Path
import sqlite3
import sys

from .core import ImportError, Mapping, apply_plan, assert_output_path, make_plan


def main():
    parser = argparse.ArgumentParser(description="Preview/import Picasa named faces into a disposable LapCasa database.")
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--contacts", required=True, type=Path)
    parser.add_argument("--map", required=True, action="append", metavar="SOURCE=TARGET")
    parser.add_argument("--coordinates", required=True, choices=("raw", "display"),
                        help="Coordinate convention of Picasa rect64 regions; never guessed.")
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--ignored-only", action="store_true", help="Import only ignored regions; leave named labels unchanged.")
    parser.add_argument("--backup", type=Path)
    args = parser.parse_args()
    if args.apply and not args.backup:
        parser.error("--apply requires --backup pointing to a new file")
    try:
        mappings = []
        for value in args.map:
            if "=" not in value:
                raise ImportError("--map must be SOURCE=TARGET")
            source, target = value.split("=", 1)
            mappings.append(Mapping(Path(source), Path(target)))
        def progress(event):
            print(json.dumps(dict(event="verification_progress", **event)), file=sys.stderr, flush=True)
        plan = make_plan(args.database, args.contacts, mappings, args.coordinates, progress,
                         ignored_only=args.ignored_only)
        assert_output_path(args.report, plan)
        if args.backup and args.report.resolve() == args.backup.resolve():
            raise ImportError("Report and backup must use different paths")
        if args.report.exists():
            raise ImportError("Report path must not exist")
        report = plan.report()
        report["mode"] = "apply" if args.apply else "preview"
        args.report.parent.mkdir(parents=True, exist_ok=True)
        # Persist the preview before mutation, so applied intent is reviewable even
        # if writing the final result later fails.
        with args.report.open("x", encoding="utf-8") as handle:
            json.dump(report, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
        if args.apply:
            report["result"] = apply_plan(plan, args.backup)
        with args.report.open("w", encoding="utf-8") as handle:
            json.dump(report, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
        print(json.dumps(dict(mode=report["mode"], counts=report["counts"],
                              result=report.get("result"), report=str(args.report))))
        return 0
    except (ImportError, OSError, ValueError, sqlite3.Error) as exc:
        print(json.dumps(dict(error=str(exc))), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
