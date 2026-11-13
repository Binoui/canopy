# Capture Chrome history and bookmarks

To search your Chrome history from Canopy itself, use `canopy --import-chrome` instead, as described in the main README. This script builds a separate research dataset.

From the project directory, run the importer with a Chrome profile and a new output directory:

```sh
python3 scripts/import_chrome.py \
  --profile "$HOME/.config/google-chrome/Profile 1" \
  --output .local-data/chrome-profile-1 \
  --timezone Europe/Paris
```

Use a different output directory for each capture. The importer refuses to overwrite an existing capture or write inside the selected Chrome profile.

Chrome can stay open. If it keeps changing its history during capture, wait until browsing is idle and retry.

Open the output directory's `README.md` for the record counts. Inspect `summary.json` for frequent pages, hostname counts, local-hour and weekday distributions, and recorded referrer pairs. Query `dataset.sqlite3` for the complete normalized records.

Keep the capture in `.local-data`, which the project's `.gitignore` excludes. The importer creates private directories and files. The capture includes complete URLs and titles, which can contain private information.

The importer reads the profile's History database and Bookmarks file. It does not load any saved URL. It imports neither cookies nor credentials. The dataset is separate from Canopy's live `forest.sqlite3` database.

Run the synthetic-data checks with this command:

```sh
python3 -m unittest discover -s scripts -p 'test_import_chrome.py' -v
```
