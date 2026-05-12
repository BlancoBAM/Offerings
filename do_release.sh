#!/bin/bash
set -e

REPO="/home/aegon/Offerings"
PAT="${GITHUB_TOKEN}"
REMOTE="https://${PAT}@github.com/BlancoBAM/Offerings.git"
TAG="v1.1.0"

cd "$REPO"

# Configure git
git config user.email "blancobam@lilithlinux.com"
git config user.name "BlancoBAM"

# Stage all changes
git add -A

# Check if there's anything to commit
if git diff --staged --quiet; then
    echo "Nothing to commit, working tree clean"
else
    git commit -m "fix: resolve all critical UI and categorization bugs

- Fix cards cut off on right (AppCard: min-width 220px, max-width 380px, horizontal-stretch, 3 columns)
- Fix AI/category miscategorization: removed dangerous name/summary substring matching
  ('ai' was matching FastMAIL, GAIa Sky, KAIdan, Iaito etc. - now category-metadata only)
- Fix Miscellaneous showing 1 package (case-sensitive 'miscellaneous' != 'Miscellaneous' bug)
- Fix progress bar stuck at 1% (now smoothly increments 2% per 300ms)
- Fix sidebar category counts (now shows true category count not home-page dedup count)
- Fix Lilith curated section populating via curated showcase fallback
- Fix PackageDetailView bottom cutoff (named Flickable viewport with padding-bottom 48px)
- Remove max-width/max-height window constraints for DE compatibility
- Fix CI: add all Slint/Wayland/XCB/font dependencies, APPIMAGE_EXTRACT_AND_RUN=1
- Fix AppImage script: use existing PNG icon, skip rebuild, CI-compatible
- Fix DEB package deps: remove GTK/WebKit, use correct Slint runtime deps
- Trim CI to only produce .deb and .AppImage releases"
fi

# Set remote URL with PAT
git remote set-url origin "$REMOTE"

# Push to main
git push origin HEAD

# Create and push tag (delete if exists)
git tag -d "$TAG" 2>/dev/null || true
git push origin --delete "$TAG" 2>/dev/null || true
git tag "$TAG"
git push origin "$TAG"

echo ""
echo "=== Done ==="
echo "Pushed to GitHub and tagged $TAG"
echo "CI will now build .deb and .AppImage releases"
echo "Check: https://github.com/BlancoBAM/Offerings/actions"
