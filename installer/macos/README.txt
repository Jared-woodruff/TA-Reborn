TA Reborn for macOS
===================

Requires macOS 13 or newer. Choose x64 for an Intel Mac or arm64 for Apple silicon.
Each package contains native code for its named architecture; no Rosetta dependency
is needed for the matching build.

Open the disk image and drag TA Reborn.app into Applications, then open the app.
The ZIP contains the same app if you prefer to extract and move it yourself.
Do not run directly from the read-only disk image.

TA Reborn is an unofficial reimplementation of Total Annihilation and includes no
original game data. In the launcher, select the data folder from your own copy of
Total Annihilation. The original Windows executable is not required by Reborn.
ZIP imports use macOS's archive tools; some RAR packages may need Homebrew 7-Zip.

The build-info.json inside Contents/Resources records the signing mode and whether
notarization was requested. Ad-hoc signing is not Apple notarization: macOS may
require you to approve an unnotarized download in System Settings > Privacy &
Security. Do not disable Gatekeeper system-wide.

Profiles, settings, saves and imported mods are kept in your user configuration
directory rather than inside the app. Removing the app does not delete them.
Use the launcher's Mods and Maps pages to add content; do not modify the signed app.

Documentation, licences and third-party notices are in Contents/Resources. The
separate symbols archive contains dSYM files for crash diagnosis.

Project: https://github.com/Jared-woodruff/TA-Reborn
