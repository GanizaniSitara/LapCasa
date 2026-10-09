"""Read Picasa metadata, verify mapped photos, and transactionally import labels.

This module never writes photo files. It uses explicit database/root paths and
keeps imported contact IDs separate from display names and recognition models.
"""
from __future__ import annotations

from collections import Counter, defaultdict
from contextlib import closing
from dataclasses import dataclass, field
import hashlib
import json
import math
import os
from pathlib import Path
import re
import sqlite3
import time
from typing import Callable
import uuid
import xml.etree.ElementTree as ET

from PIL import Image, ImageChops, ImageOps, ImageStat

IGNORED_CONTACT = "ffffffffffffffff"


class ImportError(Exception):
    """A failed safety check or invalid input (not a partially applied import)."""


def path_key(path: Path | str) -> str:
    return str(Path(path).resolve()).replace("\\", "/").casefold()


def contact_id(value: str) -> str:
    value = value.strip().lower()
    if not re.fullmatch(r"[0-9a-f]{1,16}", value):
        raise ImportError("Invalid Picasa contact ID")
    return value.lstrip("0") or "0"


def read_contacts(path: Path) -> dict[str, str]:
    try:
        root = ET.parse(path).getroot()
    except (OSError, ET.ParseError) as exc:
        raise ImportError(f"Cannot parse contacts XML: {exc}") from exc
    contacts: dict[str, str] = {}
    for item in root.iter():
        if item.tag.rsplit("}", 1)[-1] != "contact":
            continue
        cid = contact_id(item.attrib.get("id", ""))
        name = item.attrib.get("name", "").strip()
        if not name or cid == "ffffffffffffffff":
            continue
        if cid in contacts and contacts[cid] != name:
            raise ImportError(f"Conflicting names for contact {cid}")
        contacts[cid] = name
    if not contacts:
        raise ImportError("The contacts XML contains no named contacts")
    return contacts


def decode_rect64(value: str) -> tuple[float, float, float, float]:
    """Picasa packs x0,y0,x1,y1 from MOST to LEAST significant 16-bit word.

    Leading zeroes are omitted. Coordinates use 65535, following ExifTool's
    picasa_faces.config. Return normalized left/top/right/bottom.
    """
    if not re.fullmatch(r"[0-9a-fA-F]{1,16}", value):
        raise ImportError("Invalid rect64 hex value")
    packed = int(value, 16)
    box = tuple(((packed >> shift) & 65535) / 65535 for shift in (48, 32, 16, 0))
    if box[0] >= box[2] or box[1] >= box[3]:
        raise ImportError("Empty or inverted rect64 region")
    return box


def orient_point(x: float, y: float, orientation: int) -> tuple[float, float]:
    """Map normalized raw pixels into EXIF display coordinates (all 8 cases)."""
    transforms = {
        1: (x, y), 2: (1 - x, y), 3: (1 - x, 1 - y), 4: (x, 1 - y),
        5: (y, x), 6: (1 - y, x), 7: (1 - y, 1 - x), 8: (y, 1 - x),
    }
    if orientation not in transforms:
        raise ImportError("Invalid EXIF orientation")
    return transforms[orientation]


def orient_box(box, orientation: int):
    corners = [orient_point(x, y, orientation) for x in (box[0], box[2])
               for y in (box[1], box[3])]
    return (min(p[0] for p in corners), min(p[1] for p in corners),
            max(p[0] for p in corners), max(p[1] for p in corners))


def target_box(box, source_orientation: int, target_orientation: int, coordinates: str):
    display = orient_box(box, source_orientation) if coordinates == "raw" else box
    inverse = {1: 1, 2: 2, 3: 3, 4: 4, 5: 5, 6: 8, 7: 7, 8: 6}
    return orient_box(display, inverse[target_orientation])


def box_iou(a, b) -> float:
    overlap = max(0, min(a[2], b[2]) - max(a[0], b[0])) * max(
        0, min(a[3], b[3]) - max(a[1], b[1]))
    union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - overlap
    return overlap / union if union > 0 else 0


def detector_box(box):
    """The corpus calibration also used by t_face_suggest::seed_named_embeddings."""
    x0, y0, x1, y1 = box
    w, h = x1 - x0, y1 - y0
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2 + h * 0.046
    return cx - w * 0.615 / 2, cy - h * 0.664 / 2, cx + w * 0.615 / 2, cy + h * 0.664 / 2


def read_ini(path: Path):
    data = path.read_bytes()
    if data.startswith((b"\xff\xfe", b"\xfe\xff")):
        text = data.decode("utf-16")
    else:
        try:
            text = data.decode("utf-8-sig")
        except UnicodeDecodeError:
            text = data.decode("cp1252")
    sections = []
    name, properties = "", defaultdict(list)
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith((";", "#")):
            continue
        if line.startswith("[") and line.endswith("]"):
            if name:
                sections.append((name, properties))
            name, properties = line[1:-1], defaultdict(list)
        elif "=" in line:
            key, value = line.split("=", 1)
            properties[key.strip().lower()].append(value.strip())
    if name:
        sections.append((name, properties))
    return sections


def inside(path: Path, root: Path) -> bool:
    candidate, base = path_key(path), path_key(root)
    return candidate == base or candidate.startswith(base.rstrip("/") + "/")


@dataclass
class Mapping:
    source: Path
    target: Path


@dataclass
class Photo:
    id: int
    path: Path
    width: int
    height: int
    orientation: int
    excluded: bool


@dataclass
class Candidate:
    file_id: int
    contact_id: str
    name: str
    box: tuple[float, float, float, float]
    width: int
    height: int
    sources: list[dict]
    key: str = ""
    action: str = ""
    face_id: int | None = None
    person_id: int | None = None

    @property
    def ignored(self) -> bool:
        return self.contact_id == IGNORED_CONTACT

    def bbox(self) -> dict:
        x0, y0, x1, y1 = self.box
        return dict(x=round(x0 * self.width, 6), y=round(y0 * self.height, 6),
                    width=round((x1 - x0) * self.width, 6),
                    height=round((y1 - y0) * self.height, 6), confidence=1.0)


@dataclass
class Plan:
    database: Path
    namespace: str
    fingerprint: str
    contacts: dict[str, str]
    candidates: list[Candidate]
    counts: Counter = field(default_factory=Counter)
    issues: list[dict] = field(default_factory=list)
    mappings: list[Mapping] = field(default_factory=list)
    contacts_path: Path | None = None
    coordinates: str = "raw"
    contacts_hash: str = ""

    def reject(self, reason: str, **detail):
        self.counts[reason] += 1
        self.issues.append(dict(reason=reason, **detail))

    def report(self) -> dict:
        return dict(version=1, database=str(self.database), namespace=self.namespace,
                    database_fingerprint=self.fingerprint, coordinates=self.coordinates,
                    contacts_sha256=self.contacts_hash,
                    counts=dict(sorted(self.counts.items())), issues=self.issues,
                    actions=[dict(key=c.key, file_id=c.file_id, contact_id=c.contact_id,
                                  name=c.name, bbox=c.bbox(), action=c.action,
                                  face_id=c.face_id, person_id=c.person_id, sources=c.sources)
                             for c in self.candidates if c.action],
                    ignored_regions=[dict(key=c.key, file_id=c.file_id, bbox=c.bbox(),
                                          sources=c.sources, action=c.action or "unmatched_or_skipped")
                                     for c in self.candidates if c.ignored],
                    mappings=[dict(source=str(m.source), target=str(m.target)) for m in self.mappings])


def open_readonly(path: Path) -> sqlite3.Connection:
    if not path.is_file():
        raise ImportError("Database file does not exist")
    conn = sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA query_only=ON")
    return conn


def tables(conn) -> set[str]:
    return {row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}


def fingerprint(conn) -> str:
    """Protect plans against concurrent metadata/face edits, including embeddings."""
    digest = hashlib.sha256()
    queries = (
        "SELECT id,path,faces_excluded FROM afolders ORDER BY id",
        "SELECT id,folder_id,name,width,height,e_orientation,has_faces,file_type FROM afiles ORDER BY id",
        "SELECT id,name,cover_face_id,is_anchored,created_at FROM persons ORDER BY id",
        "SELECT id,file_id,bbox,embedding,person_id,created_at FROM faces ORDER BY id",
    )
    for query in queries:
        digest.update(query.encode())
        for row in conn.execute(query):
            encoded = [hashlib.sha256(v).hexdigest() if isinstance(v, bytes) else v for v in row]
            digest.update(json.dumps(encoded, ensure_ascii=False, separators=(",", ":")).encode())
    for table in sorted(tables(conn) & {"picasa_import_people", "picasa_import_faces", "picasa_import_ignored", "face_ignores"}):
        for row in conn.execute(f"SELECT * FROM {table} ORDER BY 1,2"):
            digest.update(json.dumps(tuple(row), ensure_ascii=False).encode())
    return digest.hexdigest()


def load_thumbnail(path: Path):
    with Image.open(path) as im:
        if im.format not in {"JPEG", "PNG", "TIFF", "BMP", "WEBP"} or getattr(im, "is_animated", False):
            raise ImportError("Unsupported image format (RAW/HEIC/animated images are not guessed)")
        dimensions = im.size
        orientation = int(im.getexif().get(274, 1) or 1)
        if orientation not in range(1, 9):
            raise ImportError("Invalid image EXIF orientation")
        im.draft("RGB", (128, 128))
        thumb = ImageOps.exif_transpose(im).convert("RGB").resize((64, 64), Image.Resampling.LANCZOS)
        return dimensions, orientation, thumb


@dataclass
class VerifiedPhoto:
    proof: dict
    source_thumb: Image.Image
    target_thumb: Image.Image


def verify_photo(source: Path, photo: Photo) -> VerifiedPhoto:
    sd, so, source_thumb = load_thumbnail(source)
    td, to, target_thumb = load_thumbnail(photo.path)
    if td != (photo.width, photo.height) or to != photo.orientation:
        raise ImportError("Target image geometry differs from database metadata")
    sw, sh = sd[::-1] if so > 4 else sd
    tw, th = td[::-1] if to > 4 else td
    if abs((sw / sh) / (tw / th) - 1) > 0.01:
        raise ImportError("Source and target aspect ratios differ")
    stat = ImageStat.Stat(ImageChops.difference(source_thumb, target_thumb))
    mae = sum(stat.mean) / (3 * 255)
    rmse = math.sqrt(sum(v * v for v in stat.rms) / 3) / 255
    if path_key(source) == path_key(photo.path):
        method = "same-file"
    elif source.stat().st_size == photo.path.stat().st_size and file_hash(source) == file_hash(photo.path):
        method = "identical-bytes"
    else:
        if mae > 0.035 or rmse > 0.06:
            raise ImportError(f"Photo content mismatch (MAE={mae:.5f}, RMSE={rmse:.5f})")
        if max(ImageStat.Stat(source_thumb).stddev) / 255 < 0.03:
            raise ImportError("Too little image detail to verify a resized match safely")
        method = "relative-path+upright-pixel-similarity"
    proof = dict(method=method, source_dimensions=sd, target_dimensions=td,
                source_orientation=so, target_orientation=to,
                mae=round(mae, 7), rmse=round(rmse, 7),
                source_size=source.stat().st_size, target_size=photo.path.stat().st_size,
                source_mtime_ns=source.stat().st_mtime_ns,
                target_mtime_ns=photo.path.stat().st_mtime_ns)
    return VerifiedPhoto(proof, source_thumb, target_thumb)


def verify_region(photo: VerifiedPhoto, region, coordinates: str) -> dict:
    display = orient_box(region, photo.proof["source_orientation"]) if coordinates == "raw" else region
    pixels = (math.floor(display[0]*64), math.floor(display[1]*64),
              math.ceil(display[2]*64), math.ceil(display[3]*64))
    if pixels[2]-pixels[0] < 3 or pixels[3]-pixels[1] < 3:
        raise ImportError("Face region too small for safe resized-content verification")
    difference = ImageChops.difference(photo.source_thumb.crop(pixels), photo.target_thumb.crop(pixels))
    stat = ImageStat.Stat(difference)
    mae = sum(stat.mean)/(3*255)
    rmse = math.sqrt(sum(v*v for v in stat.rms)/3)/255
    if mae > 0.055 or rmse > 0.10:
        raise ImportError(f"Face-region content mismatch (MAE={mae:.5f}, RMSE={rmse:.5f})")
    return dict(face_region_mae=round(mae,7), face_region_rmse=round(rmse,7))


def file_hash(path: Path) -> bytes:
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").digest()


def make_plan(database: Path, contacts_path: Path, mappings: list[Mapping],
              coordinates: str = "raw", progress: Callable | None = None,
              *, ignored_only: bool = False) -> Plan:
    if coordinates not in {"raw", "display"}:
        raise ImportError("Coordinates must be raw or display")
    contacts = read_contacts(contacts_path)
    namespace = hashlib.sha256(path_key(contacts_path).encode()).hexdigest()
    mappings = [Mapping(m.source.resolve(), m.target.resolve()) for m in mappings]
    if not mappings or any(not m.source.is_dir() or not m.target.is_dir() for m in mappings):
        raise ImportError("Each source and target root must be an existing directory")
    with closing(open_readonly(database)) as conn:
        required = {"afiles", "afolders", "faces", "persons"}
        if not required <= tables(conn):
            raise ImportError("Database is not a LapCasa library")
        plan = Plan(database.resolve(), namespace, fingerprint(conn), contacts, [],
                    mappings=mappings, contacts_path=contacts_path.resolve(), coordinates=coordinates,
                    contacts_hash=file_hash(contacts_path).hex())
        index = defaultdict(list)
        photos = {}
        for row in conn.execute("""SELECT a.id,b.path,a.name,a.width,a.height,a.e_orientation,
                    COALESCE(b.faces_excluded,0) excluded FROM afiles a
                    JOIN afolders b ON b.id=a.folder_id WHERE a.file_type=1"""):
            photo = Photo(row["id"], Path(row["path"]) / row["name"], row["width"], row["height"],
                          row["e_orientation"] or 1, bool(row["excluded"]))
            index[path_key(photo.path)].append(photo)
            photos[photo.id] = photo
        faces = defaultdict(list)
        for row in conn.execute("SELECT id,file_id,bbox,person_id FROM faces ORDER BY id"):
            photo = photos.get(row["file_id"])
            if not photo or not photo.width or not photo.height:
                continue
            try:
                box = json.loads(row["bbox"])
                x, y, w, h = (float(box[k]) for k in ("x", "y", "width", "height"))
                if w <= 0 or h <= 0 or not all(math.isfinite(v) for v in (x, y, w, h)):
                    continue
                faces[photo.id].append(dict(id=row["id"], person_id=row["person_id"], bbox=row["bbox"],
                                           detected=float(box.get("confidence", 1)) < 1,
                                           box=(x/photo.width, y/photo.height,
                                                (x+w)/photo.width, (y+h)/photo.height)))
            except (TypeError, ValueError, KeyError):
                plan.reject("invalid_existing_bbox", face_id=row["id"])
        people = {row["id"]: dict(row) for row in conn.execute("SELECT * FROM persons")}
        known_people, known_faces, known_ignored = {}, {}, {}
        if "picasa_import_people" in tables(conn):
            known_people = {row["contact_id"]: dict(row) for row in conn.execute(
                "SELECT * FROM picasa_import_people WHERE namespace=?", (namespace,))}
        if "picasa_import_faces" in tables(conn):
            known_faces = {row["import_key"]: dict(row) for row in conn.execute(
                "SELECT * FROM picasa_import_faces WHERE namespace=?", (namespace,))}
        if "picasa_import_ignored" in tables(conn):
            known_ignored = {row["import_key"]: dict(row) for row in conn.execute(
                "SELECT * FROM picasa_import_ignored WHERE namespace=?", (namespace,))}
    plan.counts["contacts"] = len(contacts)
    plan.counts["library_images"] = len(photos)
    verified = {}
    by_file = defaultdict(list)
    for mapping in mappings:
        sidecars = sorted((p for p in mapping.source.rglob("*") if p.name.lower() in
                           {".picasa.ini", "picasa.ini"} and p.is_file()), key=path_key)
        for ini in sidecars:
            if not inside(ini, mapping.source):
                plan.reject("source_path_escape", source=str(ini))
                continue
            plan.counts["sidecars"] += 1
            try:
                sections = read_ini(ini)
                sidecar_hash = file_hash(ini).hex()
            except (OSError, UnicodeError) as exc:
                plan.reject("unreadable_sidecar", source=str(ini), error=str(exc))
                continue
            for name, properties in sections:
                if "faces" not in properties:
                    continue
                if not name or any(c in name for c in "/\\:") or name in {".", ".."}:
                    plan.reject("unsafe_photo_section", source=str(ini), section=name)
                    continue
                # Virtual rotations/crops are not guessed. RAW coordinate exceptions
                # also stay excluded by the image-format gate.
                edited = any(v not in {"", "rotate(0)", "0"}
                             for k in ("rotate", "crop", "filters") for v in properties.get(k, []))
                source = ini.parent / name
                target = mapping.target / ini.parent.relative_to(mapping.source) / name
                if not inside(source, mapping.source) or not inside(target, mapping.target):
                    plan.reject("photo_path_escape", source=str(source))
                    continue
                found = index.get(path_key(target), [])
                ignored_regions = []
                if ignored_only:
                    for value in properties["faces"]:
                        for raw in value.split(";"):
                            ignored = re.fullmatch(r"rect64\(([0-9a-fA-F]{1,16})\),\s*ffffffffffffffff", raw.strip(), re.I)
                            if ignored:
                                try: ignored_regions.append(decode_rect64(ignored[1]))
                                except ImportError: pass
                for entry in (e.strip() for v in properties["faces"] for e in v.split(";") if e.strip()):
                    plan.counts["face_entries"] += 1
                    match = re.fullmatch(r"rect64\(([0-9a-fA-F]{1,16})\),\s*([0-9a-fA-F]{1,16})", entry)
                    if not match:
                        plan.reject("malformed_region", source=str(ini), section=name, value=entry)
                        continue
                    cid = contact_id(match[2])
                    # An ignore-only pass need not decode thousands of unrelated photos.
                    # Keep overlapping labels in the plan so source conflicts still block.
                    if ignored_only and cid != IGNORED_CONTACT:
                        try:
                            if not any(box_iou(decode_rect64(match[1]), box) >= 0.6 for box in ignored_regions):
                                continue
                        except ImportError:
                            continue
                    if cid not in contacts and cid != IGNORED_CONTACT:
                        plan.reject("unknown_contact", contact_id=cid, source=str(source))
                        continue
                    if len(found) != 1:
                        plan.reject("ambiguous_photo_path" if found else "unmapped_photo", source=str(source))
                        continue
                    photo = found[0]
                    if photo.excluded:
                        plan.reject("faces_excluded", file_id=photo.id)
                        continue
                    if edited:
                        plan.reject("picasa_virtual_edit", source=str(source))
                        continue
                    try:
                        region = decode_rect64(match[1])
                        key = (path_key(source), photo.id)
                        if key not in verified:
                            try:
                                verified[key] = verify_photo(source, photo)
                                plan.counts["verified_photo_pairs"] += 1
                            except (OSError, ValueError, ImportError, Image.DecompressionBombError) as exc:
                                verified[key] = str(exc)
                            if progress and len(verified) % 100 == 0:
                                progress(dict(verified_pairs=len(verified), face_entries=plan.counts["face_entries"]))
                        verified_photo = verified[key]
                        if isinstance(verified_photo, str):
                            raise ImportError(verified_photo)
                        proof = dict(verified_photo.proof, **verify_region(verified_photo, region, coordinates))
                        box = target_box(region, proof["source_orientation"], photo.orientation, coordinates)
                    except (ImportError, KeyError) as exc:
                        plan.reject("unverified_photo_or_region", source=str(source), error=str(exc))
                        continue
                    evidence = dict(sidecar=str(ini), sidecar_sha256=sidecar_hash,
                                    photo=str(source), rect64=match[1], verification=proof)
                    if proof["source_orientation"] != 1:
                        plan.counts["rotated_source_entries"] += 1
                        for convention in ("raw", "display"):
                            alternative = target_box(region, proof["source_orientation"], photo.orientation, convention)
                            if any(box_iou(alternative, f["box"]) >= 0.6 for f in faces[photo.id]):
                                plan.counts[f"rotated_existing_overlap_{convention}"] += 1
                    duplicate = next((c for c in by_file[photo.id]
                                      if c.contact_id == cid and box_iou(c.box, box) >= 0.8), None)
                    if duplicate:
                        duplicate.sources.append(evidence)
                        plan.counts["duplicate_source_region"] += 1
                        continue
                    quantized = [round(v * 65535) for v in box]
                    c = Candidate(photo.id, cid, contacts.get(cid, "Ignored face"), box, photo.width, photo.height, [evidence])
                    c.key = hashlib.sha256(json.dumps([namespace, photo.id, cid, quantized]).encode()).hexdigest()
                    by_file[photo.id].append(c)
                    plan.candidates.append(c)
    for candidates in by_file.values():
        blocked = {c.key for c in candidates for other in candidates
                   if c.contact_id != other.contact_id and box_iou(c.box, other.box) >= 0.6}
        claimed_faces = set()
        for c in candidates:
            if c.key in blocked:
                plan.reject("conflicting_source_labels", file_id=c.file_id, contact_id=c.contact_id)
                continue
            if c.ignored:
                tracked = known_ignored.get(c.key)
                if tracked:
                    actual = next((f for f in faces[c.file_id] if f["id"] == tracked["face_id"]), None)
                    if actual and actual["bbox"] == tracked["imported_bbox"]:
                        plan.counts["ignored_already_imported"] += 1
                    else:
                        plan.reject("changed_or_removed_ignored_face", file_id=c.file_id, import_key=c.key)
                    # A user undo takes precedence over the old imported decision.
                    continue
                calibrated = detector_box(c.box)
                def overlap(face):
                    return max(box_iou(c.box, face["box"]),
                               box_iou(calibrated, face["box"]) if face["detected"] else 0)
                overlaps = [f for f in faces[c.file_id] if overlap(f) >= 0.6]
                if len(overlaps) != 1 or overlap(overlaps[0]) < 0.8:
                    plan.reject("ignored_unmatched", file_id=c.file_id,
                                ambiguous=bool(overlaps), import_key=c.key)
                    continue
                face = overlaps[0]
                if face["id"] in claimed_faces:
                    plan.reject("ignored_unmatched", file_id=c.file_id, ambiguous=True, import_key=c.key)
                    continue
                claimed_faces.add(face["id"])
                c.face_id = face["id"]
                c.action = "ignore_face"
                plan.counts["ignored_matched"] += 1
                if box_iou(c.box, face["box"]) < 0.8:
                    plan.counts["ignored_detector_overlap"] += 1
                continue
            tracked_person = known_people.get(c.contact_id)
            if tracked_person:
                c.person_id = tracked_person["person_id"]
                actual = people.get(c.person_id)
                if (not actual or not actual["is_anchored"] or actual["name"] != tracked_person["imported_name"]
                        or c.name != tracked_person["imported_name"]):
                    plan.reject("changed_or_removed_person", file_id=c.file_id, contact_id=c.contact_id)
                    continue
            tracked_face = known_faces.get(c.key)
            if tracked_face:
                actual_face = next((f for f in faces[c.file_id] if f["id"] == tracked_face["face_id"]), None)
                if (actual_face and actual_face["person_id"] == tracked_face["person_id"]
                        and actual_face["person_id"] == c.person_id
                        and actual_face["bbox"] == tracked_face["imported_bbox"]):
                    plan.counts["already_imported"] += 1
                else:
                    plan.reject("changed_or_removed_face", file_id=c.file_id, import_key=c.key)
                continue
            overlaps = [f for f in faces[c.file_id] if box_iou(c.box, f["box"]) >= 0.6]
            conflicts = [f for f in overlaps if f["person_id"] is not None and f["person_id"] != c.person_id]
            if conflicts:
                plan.reject("existing_label_conflict", file_id=c.file_id, contact_id=c.contact_id,
                            face_ids=[f["id"] for f in conflicts])
                continue
            strong = [f for f in overlaps if box_iou(c.box, f["box"]) >= 0.8]
            if len(overlaps) > 1 or (overlaps and not strong):
                plan.reject("ambiguous_existing_face", file_id=c.file_id, contact_id=c.contact_id)
                continue
            if strong:
                face = strong[0]
                if face["id"] in claimed_faces:
                    plan.reject("existing_face_claimed_twice", file_id=c.file_id)
                    continue
                claimed_faces.add(face["id"])
                c.face_id = face["id"]
                c.action = "assign_face" if face["person_id"] is None else "link_face"
            else:
                c.action = "create_face"
            plan.counts[c.action] += 1
    if ignored_only:
        for c in plan.candidates:
            if not c.ignored:
                if c.action: plan.counts[c.action] -= 1
                c.action = ""
    plan.counts["planned_faces"] = sum(bool(c.action) for c in plan.candidates)
    plan.counts["planned_people"] = len({c.contact_id for c in plan.candidates if c.action and not c.ignored and c.person_id is None})
    for key in ("ignored_matched", "ignored_unmatched", "ignored_already_imported"):
        plan.counts[key] += 0
    return plan


LEDGER_SQL = """
CREATE TABLE IF NOT EXISTS picasa_import_people(
 namespace TEXT NOT NULL, contact_id TEXT NOT NULL, person_id INTEGER NOT NULL,
 imported_name TEXT NOT NULL, PRIMARY KEY(namespace,contact_id));
CREATE TABLE IF NOT EXISTS picasa_import_faces(
 namespace TEXT NOT NULL, import_key TEXT NOT NULL, face_id INTEGER NOT NULL,
 person_id INTEGER NOT NULL, source_json TEXT NOT NULL, imported_bbox TEXT NOT NULL,
 PRIMARY KEY(namespace,import_key));
CREATE TABLE IF NOT EXISTS picasa_import_runs(
 id TEXT PRIMARY KEY, namespace TEXT NOT NULL, created_at INTEGER NOT NULL,
 report_json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS face_ignores(
 face_id INTEGER PRIMARY KEY REFERENCES faces(id) ON DELETE CASCADE,
 ignored_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS picasa_import_ignored(
 namespace TEXT NOT NULL, import_key TEXT NOT NULL, face_id INTEGER NOT NULL,
 source_json TEXT NOT NULL, imported_bbox TEXT NOT NULL,
 PRIMARY KEY(namespace,import_key));
"""


def assert_output_path(output: Path, plan: Plan):
    assert_not_live(output)
    protected = [plan.database, plan.contacts_path] + [Path(str(plan.database) + suffix)
                 for suffix in ("-wal", "-shm", "-journal")]
    if any(p and path_key(output) == path_key(p) for p in protected):
        raise ImportError("Output must not replace an input")
    if any(inside(output, root) for m in plan.mappings for root in (m.source, m.target)):
        raise ImportError("Output must be outside all photo roots")


def assert_not_live(database: Path):
    local = Path(os.environ.get("LOCALAPPDATA", str(Path.home() / "AppData/Local")))
    if any(inside(database, local / name / "libraries") for name in ("com.lapcasa.dev", "com.lapcasa", "com.lap")):
        raise ImportError("Applying to the live application library is disabled; use a disposable copy")


def apply_plan(plan: Plan, backup: Path, fail_after: int | None = None,
               *, allow_live: bool = False) -> dict:
    # The standalone CLI remains disposable-only. The desktop app supplies this
    # opt-in only after the user previews and explicitly requests an import.
    if not allow_live:
        assert_not_live(plan.database)
    assert_output_path(backup, plan)
    if backup.exists():
        raise ImportError("Backup path must not exist")
    if not any(c.action for c in plan.candidates):
        return dict(applied_faces=0, applied_people=0, already_imported=plan.counts["already_imported"],
                    applied_ignored=0, ignored_matched=plan.counts["ignored_matched"],
                    ignored_unmatched=plan.counts["ignored_unmatched"],
                    backup=None, database_modified=False)
    conn = sqlite3.connect(plan.database.resolve().as_uri() + "?mode=rw", uri=True)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys=ON")
    before_fk = {tuple(row) for row in conn.execute("PRAGMA foreign_key_check")}
    backup_created = False
    try:
        conn.execute("BEGIN IMMEDIATE")
        if fingerprint(conn) != plan.fingerprint:
            raise ImportError("Database changed after preview; rebuild the plan")
        if file_hash(plan.contacts_path).hex() != plan.contacts_hash:
            raise ImportError("Contacts changed after preview")
        # Recheck photo files before applying a potentially old plan.
        for c in plan.candidates:
            if not c.action:
                continue
            target = next((m.target / Path(s["photo"]).relative_to(m.source)
                           for s in c.sources for m in plan.mappings if inside(Path(s["photo"]), m.source)), None)
            for s in c.sources:
                if file_hash(Path(s["sidecar"])).hex() != s["sidecar_sha256"]:
                    raise ImportError("Picasa sidecar changed after preview")
                source = Path(s["photo"])
                stat = source.stat()
                if stat.st_size != s["verification"]["source_size"] or stat.st_mtime_ns != s["verification"]["source_mtime_ns"]:
                    raise ImportError("Source photo changed after verification")
            if target is None:
                raise ImportError("No target path for verified photo")
            stat = target.stat()
            proof = c.sources[0]["verification"]
            if stat.st_size != proof["target_size"] or stat.st_mtime_ns != proof["target_mtime_ns"]:
                raise ImportError("Target photo changed after verification")
        backup.parent.mkdir(parents=True, exist_ok=True)
        with backup.open("xb"):
            pass
        backup_created = True
        with closing(open_readonly(plan.database)) as reader, closing(sqlite3.connect(backup)) as dest:
            reader.backup(dest)
            if dest.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
                raise ImportError("Backup integrity check failed")
        # executescript implicitly commits, so use individual DDL statements.
        for statement in LEDGER_SQL.split(";"):
            if statement.strip():
                conn.execute(statement)
        person_ids = {c.contact_id: c.person_id for c in plan.candidates if c.person_id is not None}
        now = int(time.time())
        created_people = applied = applied_ignored = 0
        for c in plan.candidates:
            if not c.action:
                continue
            if c.ignored:
                stored = conn.execute("SELECT bbox FROM faces WHERE id=?", (c.face_id,)).fetchone()
                if not stored:
                    raise ImportError("An ignored face disappeared after preview")
                conn.execute("INSERT OR IGNORE INTO face_ignores VALUES(?,?)", (c.face_id, now))
                conn.execute("INSERT INTO picasa_import_ignored VALUES(?,?,?,?,?)",
                             (plan.namespace, c.key, c.face_id, json.dumps(c.sources, ensure_ascii=False), stored[0]))
                applied_ignored += 1
                applied += 1
                if fail_after is not None and applied >= fail_after:
                    raise ImportError("Injected validation failure")
                continue
            pid = person_ids.get(c.contact_id)
            if pid is None:
                pid = conn.execute("INSERT INTO persons(name,is_anchored,created_at) VALUES(?,1,?)",
                                   (c.name, now)).lastrowid
                conn.execute("INSERT INTO picasa_import_people VALUES(?,?,?,?)",
                             (plan.namespace, c.contact_id, pid, c.name))
                person_ids[c.contact_id] = pid
                created_people += 1
            if c.action == "create_face":
                fid = conn.execute("INSERT INTO faces(file_id,bbox,embedding,person_id,created_at) VALUES(?,?,NULL,?,?)",
                                   (c.file_id, json.dumps(c.bbox(), separators=(",", ":")), pid, now)).lastrowid
            else:
                fid = c.face_id
                if c.action == "assign_face":
                    changed = conn.execute("UPDATE faces SET person_id=? WHERE id=? AND person_id IS NULL", (pid, fid)).rowcount
                    if changed != 1:
                        raise ImportError("An existing face assignment changed")
            stored_bbox = conn.execute("SELECT bbox FROM faces WHERE id=?", (fid,)).fetchone()[0]
            conn.execute("INSERT INTO picasa_import_faces VALUES(?,?,?,?,?,?)",
                         (plan.namespace, c.key, fid, pid, json.dumps(c.sources, ensure_ascii=False), stored_bbox))
            conn.execute("UPDATE persons SET cover_face_id=? WHERE id=? AND cover_face_id IS NULL", (fid, pid))
            applied += 1
            if fail_after is not None and applied >= fail_after:
                raise ImportError("Injected validation failure")
        report = plan.report()
        conn.execute("INSERT INTO picasa_import_runs VALUES(?,?,?,?)",
                     (str(uuid.uuid4()), plan.namespace, now, json.dumps(report, ensure_ascii=False)))
        after_fk = {tuple(row) for row in conn.execute("PRAGMA foreign_key_check")}
        if after_fk - before_fk or conn.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
            raise ImportError("Import integrity check failed")
        conn.commit()
        return dict(applied_faces=applied, applied_people=created_people,
                    applied_ignored=applied_ignored, ignored_matched=plan.counts["ignored_matched"],
                    ignored_unmatched=plan.counts["ignored_unmatched"],
                    backup=str(backup), database_modified=True)
    except Exception:
        conn.rollback()
        # A completed backup is useful even when the transaction rolls back.
        if backup_created and backup.exists() and backup.stat().st_size == 0:
            backup.unlink()
        raise
    finally:
        conn.close()
