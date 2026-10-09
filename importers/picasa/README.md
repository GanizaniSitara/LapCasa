# Picasa named people importer

An offline importer that copies confirmed named face regions from Picasa
(`contacts.xml` and the `.picasa.ini` files beside the photos) into a LapCasa
library's `persons` and `faces` tables. It needs Python 3.11+ with Pillow. It
does not load AI models and never writes photo metadata.

In the app: Settings > Advanced > **Import people from Picasa**. Choose the
contacts file and source folders, preview the counts and skipped cases, then
confirm **Back up and import**. Nothing imports automatically.

From the command line:

```
python -m picasa_import --help
python -m picasa_import --database library-copy.db --contacts contacts.xml --map OLD_ROOT=NEW_ROOT --coordinates raw --report preview.json
```

Preview is the default and opens the database read-only. Source roots map to
target roots by relative path. `--coordinates raw|display` is required: the
importer never guesses which convention a collection uses. To apply, add
`--apply --backup NEW_BACKUP_PATH` with a new report path; the command line
refuses the live app library, so review the import in a copy first.

An apply run verifies its inputs again, backs the database up with SQLite's
backup API and applies everything in one transaction, rolling back on any
failure. A photo is matched only when its dimensions, orientation and content
agree; uncertain photos and regions are skipped and listed in the report.
Imported people are anchored, existing labels are never relabelled, and reruns
create no duplicates.

## Tests

```
python -m unittest discover -s tests -v
```

The tests use disposable SQLite databases and real JPEGs.
