#!/bin/bash
# Copies the pre-built AppIcon.icns into the app bundle, overriding what actool
# generates. actool on Xcode 14.2 drops several icon sizes; iconutil does not.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$SCRIPT_DIR/AppIcon.icns"
DEST="$TARGET_BUILD_DIR/$UNLOCALIZED_RESOURCES_FOLDER_PATH/AppIcon.icns"

cp "$SRC" "$DEST"
