# Catalogue auto-update (systemd user units)

Two user units rebuild the store's unified desktop-app catalogue from its
upstream seeds on a schedule. The catalogue is the store's single browse
source; package managers (flatpak, am, snap, brew, soar) are only used for
install/uninstall and installed-state tracking at runtime.

## Install

```bash
mkdir -p ~/.config/systemd/user
cp tools/systemd/* ~/.config/systemd/user/
systemctl --user daemon-reload

# fix the repo path inside the service (if not ~/Offerings)
sed -i "s|%h/Offerings|%h/<your-repo-path>|" \
  ~/.config/systemd/user/offerings-catalogue-update.service

systemctl --user enable --now offerings-catalogue-update.timer
```

## Verify

```bash
systemctl --user list-timers offerings-catalogue-update
journalctl --user -u offerings-catalogue-update.service -n 50
```

## What happens after a rebuild

The store fingerprints the catalogue files (path + mtime). The next time the
user presses 🔄 **Refresh** — or the next app launch — the new catalogue is
served automatically; no restart is required.

## Notes

- Seeds are cached in `~/.local/share/offerings/catalogue-seeds/`; a failed
  download never destroys the previous snapshot, and a failed rebuild never
  replaces a working catalogue (validated temp-file + atomic swap).
- To run the updater without systemd:
  `~/Offerings/tools/update_catalogue.py`
- `--out <path>` overrides the output location (defaults to the repo's
  `assets/metadata-catalog.json` when a dev checkout is detected, plus
  `~/.local/share/offerings/metadata-catalog.json`).
