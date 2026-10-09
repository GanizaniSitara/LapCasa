"""Generate a local coordinate-review sheet from an importer report (read-only inputs)."""
import argparse
from contextlib import closing
import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageOps

from .core import assert_not_live, decode_rect64, open_readonly, orient_box, target_box


def render(report_path: Path, output: Path, orientation=None):
    assert_not_live(output)
    if output.exists():
        raise ValueError("Review sheet output must not exist")
    report = json.loads(report_path.read_text(encoding="utf-8"))
    actions = []
    seen = set()
    ignored = report.get("ignored_regions", [])
    ignored_keys = {a["key"] for a in ignored}
    for action in ignored + [a for a in report["actions"] if a["key"] not in ignored_keys]:
        source = action["sources"][0]
        if ((action["key"] in ignored_keys or source["verification"]["source_orientation"] != 1)
                and (orientation is None or source["verification"]["source_orientation"] == orientation)
                and action["file_id"] not in seen):
            actions.append(action)
            seen.add(action["file_id"])
        if len(actions) == 10:
            break
    sheet = Image.new("RGB", (660, 190 * len(actions) + 28), "white")
    draw = ImageDraw.Draw(sheet)
    counts = report["counts"]
    draw.text((10, 6), f"Ignored: {counts.get('ignored_matched', 0)} matched / "
              f"{counts.get('ignored_unmatched', 0)} unmatched / "
              f"{counts.get('ignored_already_imported', 0)} previously imported", fill="black")
    with closing(open_readonly(Path(report["database"]))) as conn:
        for index, action in enumerate(actions):
            source = action["sources"][0]
            proof = source["verification"]
            row = conn.execute("SELECT b.path,a.name FROM afiles a JOIN afolders b ON b.id=a.folder_id WHERE a.id=?",
                               (action["file_id"],)).fetchone()
            path = Path(row[0]) / row[1]
            with Image.open(path) as raw:
                upright = ImageOps.exif_transpose(raw).convert("RGB")
            y = index * 190 + 28
            draw.text((10, y + 2), f"{action['action']} / file {action['file_id']} / EXIF {proof['source_orientation']} -> {proof['target_orientation']}", fill="black")
            thumb = ImageOps.contain(upright, (180, 150))
            sheet.paste(thumb, (10, y + 28))
            for convention, x in (("raw", 225), ("display", 435)):
                box = target_box(decode_rect64(source["rect64"]), proof["source_orientation"],
                                 proof["target_orientation"], convention)
                box = orient_box(box, proof["target_orientation"])
                pixels = tuple(round(v * (upright.width if n % 2 == 0 else upright.height))
                               for n, v in enumerate(box))
                crop = ImageOps.contain(upright.crop(pixels), (180, 150))
                draw.text((x, y + 10), f"{convention} rect64", fill="black")
                sheet.paste(crop, (x, y + 28))
    output.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(output)
    return dict(output=str(output), reviewed_photos=len(actions))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--orientation", type=int, choices=range(2, 9))
    args = parser.parse_args()
    print(json.dumps(render(args.report, args.output, args.orientation)))
