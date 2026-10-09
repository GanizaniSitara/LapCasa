"""Importer integration tests: actual image files and disposable SQLite libraries."""
from pathlib import Path
from contextlib import closing
import hashlib
import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from PIL import Image, ImageDraw, ImageOps

from picasa_import.core import (
    ImportError, Mapping, apply_plan, assert_not_live, assert_output_path,
    decode_rect64, file_hash, fingerprint, make_plan, orient_box, target_box,
)


SCHEMA = """
CREATE TABLE afolders(id INTEGER PRIMARY KEY,path TEXT,faces_excluded INTEGER DEFAULT 0);
CREATE TABLE afiles(id INTEGER PRIMARY KEY,folder_id INTEGER,name TEXT,width INTEGER,height INTEGER,
 e_orientation INTEGER,has_faces INTEGER DEFAULT 0,file_type INTEGER DEFAULT 1);
CREATE TABLE persons(id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT,cover_face_id INTEGER,
 thumbnail BLOB,is_anchored INTEGER DEFAULT 0,created_at INTEGER);
CREATE TABLE faces(id INTEGER PRIMARY KEY AUTOINCREMENT,file_id INTEGER,bbox TEXT,embedding BLOB,
 person_id INTEGER,created_at INTEGER,FOREIGN KEY(file_id) REFERENCES afiles(id),
 FOREIGN KEY(person_id) REFERENCES persons(id));
"""


def rect(box):
    return "".join(f"{round(x*65535):04x}" for x in box).lstrip("0") or "0"


class ImportTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.source = self.root / "source"
        self.target = self.root / "target"
        self.source.mkdir()
        self.target.mkdir()
        self.db = self.root / "library.db"
        self.xml = self.root / "contacts.xml"
        self.xml.write_text('<contacts><contact id="000a" name="Rad &amp; Ada"/>'
                            '<contact id="b" name="Other"/></contacts>', encoding="utf-8")
        self.conn = sqlite3.connect(self.db)
        self.conn.executescript(SCHEMA)
        self.conn.execute("INSERT INTO afolders(id,path) VALUES(1,?)", (str(self.target),))
        self.conn.commit()
        self.addCleanup(self.conn.close)
        self.box = (0.15, 0.2, 0.45, 0.65)
        self.image = Image.new("RGB", (240, 160), "navy")
        draw = ImageDraw.Draw(self.image)
        draw.rectangle((20, 15, 100, 70), fill="red")
        draw.rectangle((130, 80, 210, 145), fill="green")
        draw.ellipse((70, 45, 170, 140), fill="gold")
        self.photo()
        self.sidecar()

    def photo(self, orientation=1, target_orientation=None, different=False):
        exif = Image.Exif()
        exif[274] = orientation
        self.image.save(self.source / "pic.jpg", quality=95, exif=exif)
        target_orientation = orientation if target_orientation is None else target_orientation
        with Image.open(self.source / "pic.jpg") as loaded:
            target = loaded.copy()
            if target_orientation != orientation:
                target = ImageOps.exif_transpose(loaded)
            if different:
                target = target.transpose(Image.Transpose.FLIP_LEFT_RIGHT)
            target.thumbnail((120, 120))
            out_exif = Image.Exif()
            out_exif[274] = target_orientation
            target.save(self.target / "pic.jpg", quality=95, exif=out_exif)
            self.conn.execute("INSERT OR REPLACE INTO afiles(id,folder_id,name,width,height,e_orientation,has_faces,file_type) VALUES(1,1,'pic.jpg',?,?,?,0,1)",
                              (*target.size, target_orientation))
            self.conn.commit()

    def sidecar(self, entries=None, extra="", section="pic.jpg"):
        if entries is None:
            entries = [(self.box, "a")]
        value = ";".join(f"rect64({rect(box)}),{cid}" for box, cid in entries)
        (self.source / ".picasa.ini").write_text(f"[{section}]\nfaces={value}\n{extra}", encoding="utf-8")

    def plan(self, mappings=None, coordinates="raw"):
        return make_plan(self.db, self.xml, mappings or [Mapping(self.source, self.target)], coordinates)

    def apply(self, plan=None, name="backup.db", **kwargs):
        return apply_plan(plan or self.plan(), self.root / name, **kwargs)

    def existing(self, person_id=None, name="Existing", bbox=None):
        if person_id is not None:
            self.conn.execute("INSERT INTO persons(id,name,is_anchored) VALUES(?,?,1)", (person_id, name))
        bbox = bbox or dict(x=18, y=16, width=36, height=36, confidence=0.91)
        self.conn.execute("INSERT INTO faces(id,file_id,bbox,embedding,person_id) VALUES(1,1,?,?,?)",
                          (json.dumps(bbox), b"existing-embedding", person_id))
        self.conn.commit()

    def test_rect64_word_order_and_omitted_zeroes(self):
        self.assertEqual(decode_rect64("00000000ffffffff"), (0, 0, 1, 1))
        self.assertEqual(decode_rect64("ffffffff"), (0, 0, 1, 1))
        for value in ("xyz", "1" * 17, "ffffffff00000000", "0"):
            with self.assertRaises(ImportError):
                decode_rect64(value)

    def test_all_eight_exif_transforms_round_trip(self):
        for orientation in range(1, 9):
            oriented = orient_box(self.box, orientation)
            result = target_box(oriented, 1, orientation, "display")
            for a, b in zip(result, self.box):
                self.assertAlmostEqual(a, b)
        self.assertEqual(orient_box((0, 0, 1, 0.5), 6), (0.5, 0, 1, 1))

    def test_dry_run_never_changes_database(self):
        before = file_hash(self.db)
        plan = self.plan()
        self.assertEqual(plan.counts["planned_faces"], 1)
        self.assertEqual(file_hash(self.db), before)
        self.assertIsNone(self.conn.execute("SELECT name FROM sqlite_master WHERE name='picasa_import_faces'").fetchone())

    def test_apply_backup_and_second_run_are_idempotent(self):
        before = fingerprint(self.conn)
        result = self.apply()
        self.assertEqual(result["applied_faces"], 1)
        with closing(sqlite3.connect(self.root / "backup.db")) as backup:
            self.assertEqual(fingerprint(backup), before)
        row = self.conn.execute("SELECT name,is_anchored FROM persons").fetchone()
        self.assertEqual(row, ("Rad & Ada", 1))
        self.assertEqual(self.conn.execute("SELECT embedding FROM faces").fetchone(), (None,))
        self.assertEqual(self.conn.execute("SELECT has_faces FROM afiles").fetchone(), (0,))
        first = file_hash(self.db)
        second_plan = self.plan()
        self.assertEqual(second_plan.counts["already_imported"], 1)
        self.assertEqual(second_plan.counts["planned_faces"], 0)
        self.assertFalse(self.apply(second_plan, "second-backup.db")["database_modified"])
        self.assertEqual(file_hash(self.db), first)
        self.assertFalse((self.root / "second-backup.db").exists())

    def test_duplicate_roots_deduplicate_same_face(self):
        plan = self.plan([Mapping(self.source, self.target), Mapping(self.source, self.target)])
        self.assertEqual(plan.counts["duplicate_source_region"], 1)
        self.assertEqual(plan.counts["planned_faces"], 1)
        self.assertEqual(len(plan.candidates[0].sources), 2)

    def test_ignored_face_preserves_identity_and_embedding_and_is_undoable(self):
        self.existing(person_id=5)
        self.sidecar([(self.box, "ffffffffffffffff")])
        plan = self.plan()
        self.assertEqual(plan.counts["ignored_matched"], 1)
        self.assertEqual(plan.counts["planned_people"], 0)
        self.assertEqual(self.apply(plan)["applied_ignored"], 1)
        self.assertEqual(self.conn.execute("SELECT person_id,embedding FROM faces").fetchone(),
                         (5, b"existing-embedding"))
        self.assertEqual(self.conn.execute("SELECT count(*) FROM face_ignores").fetchone(), (1,))
        second = self.plan()
        self.assertEqual(second.counts["ignored_already_imported"], 1)
        self.assertFalse(self.apply(second, "second.db")["database_modified"])
        # Undo is a user decision: reimport must not reinstate an old Picasa ignore.
        self.conn.execute("DELETE FROM face_ignores")
        self.conn.commit()
        self.assertFalse(self.apply(self.plan(), "third.db")["database_modified"])

    def test_ignored_unmatched_and_ambiguous_regions_do_not_create_faces(self):
        self.sidecar([(self.box, "ffffffffffffffff")])
        plan = self.plan()
        self.assertEqual(plan.counts["ignored_unmatched"], 1)
        self.assertEqual(plan.counts["planned_faces"], 0)
        self.existing()
        self.conn.execute("INSERT INTO faces(file_id,bbox) SELECT file_id,bbox FROM faces")
        self.conn.commit()
        self.assertEqual(self.plan().counts["ignored_unmatched"], 1)

    def test_ignored_tight_detector_box_uses_existing_picasa_calibration(self):
        self.existing(bbox=dict(x=24.93, y=23.704, width=22.14, height=23.904, confidence=0.9))
        self.sidecar([(self.box, "ffffffffffffffff")])
        plan = self.plan()
        self.assertEqual(plan.counts["ignored_matched"], 1)
        self.assertEqual(plan.counts["ignored_detector_overlap"], 1)
        self.apply(plan)
        self.assertEqual(self.conn.execute("SELECT count(*) FROM faces").fetchone(), (1,))

    def test_ignored_state_changes_invalidate_preview_and_rollback(self):
        self.existing()
        self.sidecar([(self.box, "ffffffffffffffff")])
        plan = self.plan()
        with self.assertRaisesRegex(ImportError, "Injected"):
            self.apply(plan, fail_after=1)
        self.assertIsNone(self.conn.execute("SELECT name FROM sqlite_master WHERE name='face_ignores'").fetchone())
        self.apply(self.plan(), name="second.db")
        before = fingerprint(self.conn)
        self.conn.execute("DELETE FROM face_ignores")
        self.conn.commit()
        self.assertNotEqual(before, fingerprint(self.conn))

    def test_ignore_only_skips_unrelated_images_but_keeps_conflicting_labels(self):
        self.existing()
        self.sidecar([(self.box, "a"), (self.box, "ffffffffffffffff")])
        plan = make_plan(self.db, self.xml, [Mapping(self.source, self.target)], ignored_only=True)
        self.assertEqual(plan.counts["conflicting_source_labels"], 2)
        self.assertEqual(plan.counts["planned_faces"], 0)
        self.sidecar()
        with patch('picasa_import.core.verify_photo') as verify:
            plan = make_plan(self.db, self.xml, [Mapping(self.source, self.target)], ignored_only=True)
            verify.assert_not_called()
            self.assertEqual(plan.counts["planned_faces"], 0)

    def test_different_picture_with_same_filename_rejected(self):
        self.photo(different=True)
        plan = self.plan()
        self.assertEqual(plan.counts["planned_faces"], 0)
        self.assertEqual(plan.counts["unverified_photo_or_region"], 1)

    def test_rotated_resized_photo_maps_back_to_raw_database_coordinates(self):
        self.photo(orientation=6, target_orientation=1)
        plan = self.plan()
        self.assertEqual(plan.counts["planned_faces"], 1)
        expected = orient_box(decode_rect64(rect(self.box)), 6)
        for a, b in zip(plan.candidates[0].box, expected):
            self.assertAlmostEqual(a, b)

    def test_existing_named_assignment_and_embedding_preserved(self):
        self.existing(person_id=5)
        before = self.conn.execute("SELECT * FROM faces").fetchall()
        plan = self.plan()
        self.assertEqual(plan.counts["existing_label_conflict"], 1)
        self.assertEqual(plan.counts["planned_faces"], 0)
        self.assertFalse(self.apply(plan)["database_modified"])
        self.assertEqual(self.conn.execute("SELECT * FROM faces").fetchall(), before)

    def test_existing_unassigned_face_reused_without_losing_embedding(self):
        self.existing()
        plan = self.plan()
        self.assertEqual(plan.counts["assign_face"], 1)
        self.apply(plan)
        self.assertEqual(self.conn.execute("SELECT count(*) FROM faces").fetchone(), (1,))
        self.assertEqual(self.conn.execute("SELECT embedding FROM faces").fetchone(), (b"existing-embedding",))
        self.assertEqual(self.plan().counts["already_imported"], 1)

    def test_manual_person_rename_is_not_overwritten(self):
        self.apply()
        self.conn.execute("UPDATE persons SET name='User override'")
        self.conn.commit()
        plan = self.plan()
        self.assertEqual(plan.counts["changed_or_removed_person"], 1)
        self.assertEqual(plan.counts["planned_faces"], 0)

    def test_manual_face_edit_is_not_overwritten(self):
        self.apply()
        self.conn.execute("UPDATE faces SET bbox=?", (json.dumps(dict(x=19,y=16,width=36,height=36)),))
        self.conn.commit()
        self.assertEqual(self.plan().counts["changed_or_removed_face"], 1)

    def test_ledger_does_not_block_gui_deletion_and_does_not_recreate_face(self):
        self.apply()
        self.conn.execute("PRAGMA foreign_keys=ON")
        self.conn.execute("DELETE FROM faces")
        self.conn.commit()
        plan = self.plan()
        self.assertEqual(plan.counts["changed_or_removed_face"], 1)
        self.assertEqual(plan.counts["planned_faces"], 0)

    def test_overlapping_different_picasa_labels_blocked(self):
        self.sidecar([(self.box, "a"), (self.box, "b")])
        plan = self.plan()
        self.assertEqual(plan.counts["conflicting_source_labels"], 2)
        self.assertEqual(plan.counts["planned_faces"], 0)

    def test_same_display_name_does_not_merge_distinct_contact_ids(self):
        self.xml.write_text('<contacts><contact id="a" name="Same"/><contact id="b" name="Same"/></contacts>')
        self.sidecar([(self.box, "a"), ((0.65,0.2,0.9,0.65), "b")])
        result = self.apply()
        self.assertEqual(result["applied_people"], 2)

    def test_unknown_malformed_and_unsafe_entries_reported(self):
        self.sidecar([(self.box, "ffff")])
        self.assertEqual(self.plan().counts["unknown_contact"], 1)
        self.sidecar(section="../pic.jpg")
        self.assertEqual(self.plan().counts["unsafe_photo_section"], 1)
        (self.source / ".picasa.ini").write_text("[pic.jpg]\nfaces=garbage\n")
        self.assertEqual(self.plan().counts["malformed_region"], 1)

    def test_virtual_edits_and_excluded_folder_skipped(self):
        self.sidecar(extra="rotate=rotate(1)\n")
        self.assertEqual(self.plan().counts["picasa_virtual_edit"], 1)
        self.sidecar()
        self.conn.execute("UPDATE afolders SET faces_excluded=1")
        self.conn.commit()
        self.assertEqual(self.plan().counts["faces_excluded"], 1)

    def test_stale_geometry_and_changed_database_refused(self):
        plan = self.plan()
        self.conn.execute("UPDATE afiles SET width=999")
        self.conn.commit()
        with self.assertRaisesRegex(ImportError, "Database changed"):
            self.apply(plan)
        self.assertEqual(self.plan().counts["unverified_photo_or_region"], 1)

    def test_changed_source_photo_refused_after_preview(self):
        plan = self.plan()
        with (self.source / "pic.jpg").open("ab") as handle:
            handle.write(b"changed")
        with self.assertRaisesRegex(ImportError, "Source photo changed"):
            self.apply(plan)

    def test_changed_contacts_and_sidecars_refused_after_preview(self):
        plan = self.plan()
        original = self.xml.read_text()
        self.xml.write_text(original.replace("Other", "Changed"))
        with self.assertRaisesRegex(ImportError, "Contacts changed"):
            self.apply(plan)
        self.xml.write_text(original)
        self.sidecar(extra="comment=Changed\n")
        with self.assertRaisesRegex(ImportError, "sidecar changed"):
            self.apply(plan)

    def test_transaction_rolls_back_schema_and_data_on_failure(self):
        before = file_hash(self.db)
        logical_before = fingerprint(self.conn)
        with self.assertRaisesRegex(ImportError, "Injected"):
            self.apply(fail_after=1)
        self.assertEqual(self.conn.execute("SELECT count(*) FROM persons").fetchone(), (0,))
        self.assertEqual(file_hash(self.db), before)
        with closing(sqlite3.connect(self.root / "backup.db")) as backup:
            self.assertEqual(fingerprint(backup), logical_before)
        self.assertIsNone(self.conn.execute("SELECT name FROM sqlite_master WHERE name='picasa_import_faces'").fetchone())

    def test_live_db_and_input_output_overwrite_guards(self):
        import os
        local = Path(os.environ.get("LOCALAPPDATA", str(Path.home() / "AppData/Local")))
        with self.assertRaises(ImportError):
            assert_not_live(local / "com.lapcasa.dev/libraries/default.db")
        plan = self.plan()
        for output in (self.db, self.xml, Path(str(self.db)+"-wal"), self.target/"out.json"):
            with self.assertRaises(ImportError):
                assert_output_path(output, plan)

    def test_duplicate_conflicting_contact_ids_refused(self):
        self.xml.write_text('<contacts><contact id="a" name="First"/><contact id="000a" name="Second"/></contacts>')
        with self.assertRaisesRegex(ImportError, "Conflicting names"):
            self.plan()

    def test_cli_preview_and_apply_require_distinct_new_outputs(self):
        report = self.root / "report.json"
        command = [sys.executable, "-m", "picasa_import", "--database", str(self.db),
                   "--contacts", str(self.xml), "--map", f"{self.source}={self.target}",
                   "--coordinates", "raw", "--report", str(report)]
        before = file_hash(self.db)
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["mode"], "preview")
        self.assertEqual(file_hash(self.db), before)
        result = subprocess.run(command + ["--apply", "--backup", str(self.root / "cli-backup.db")],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("Report path must not exist", result.stderr)
        self.assertEqual(file_hash(self.db), before)
        self.assertFalse((self.root / "cli-backup.db").exists())

    def test_weak_region_and_ambiguous_detected_face_skip(self):
        self.sidecar([((0.2,0.2,0.21,0.21), "a")])
        self.assertEqual(self.plan().counts["unverified_photo_or_region"], 1)
        self.sidecar()
        self.existing()
        self.conn.execute("INSERT INTO faces(file_id,bbox) SELECT file_id,bbox FROM faces")
        self.conn.commit()
        self.assertEqual(self.plan().counts["ambiguous_existing_face"], 1)

    def desktop_job(self, database=None):
        job = self.root / "desktop-job"
        job.mkdir()
        (job / "request.json").write_text(json.dumps({
            "database": str(database or self.db), "contacts": str(self.xml),
            "mappings": [{"source": str(self.source), "target": str(self.target)}],
            "coordinates": "raw",
        }))
        return job

    def test_desktop_frozen_preview_backup_and_repeated_apply(self):
        from picasa_import.desktop import preview, apply, load_plan
        job = self.desktop_job()
        before = file_hash(self.db)
        summary = preview(job)
        self.assertEqual(summary["counts"]["planned_faces"], 1)
        self.assertEqual(file_hash(self.db), before)
        self.assertEqual(load_plan(job).fingerprint, self.plan().fingerprint)
        result = apply(job)
        self.assertEqual(result["applied_people"], 1)
        self.assertTrue((job / "before-import.db").is_file())
        after = file_hash(self.db)
        self.assertEqual(apply(job), result)
        self.assertEqual(file_hash(self.db), after)

    def test_desktop_stale_preview_requires_new_preview(self):
        from picasa_import.desktop import preview, apply
        job = self.desktop_job()
        preview(job)
        self.conn.execute("INSERT INTO persons(name) VALUES('Manual change')")
        self.conn.commit()
        with self.assertRaisesRegex(ImportError, "Database changed"):
            apply(job)
        self.assertFalse((job / "before-import.db").exists())

    def test_desktop_explicit_confirmation_does_not_weaken_cli_guard(self):
        from picasa_import.desktop import preview, apply
        local = self.root / "simulated-local-appdata"
        simulated_live = local / "com.lapcasa.dev/libraries/default.db"
        simulated_live.parent.mkdir(parents=True)
        with closing(sqlite3.connect(simulated_live)) as copy:
            self.conn.backup(copy)
        job = self.desktop_job(simulated_live)
        with patch.dict("os.environ", {"LOCALAPPDATA": str(local)}):
            preview(job)
            with self.assertRaisesRegex(ImportError, "live application library"):
                apply(job)
            result = apply(job, allow_live=True)
            self.assertEqual(result["applied_faces"], 1)


if __name__ == "__main__":
    unittest.main()
