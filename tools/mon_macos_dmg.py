"""Write Finder layout without launching Finder (also works on CI runners)."""
import argparse
import os
from pathlib import Path
import shutil
import sys
import tempfile

from mon_macos_release import command

BACKGROUND = Path(__file__).resolve().parent / "dmg/background.tiff"


def dependencies():
    try:
        from ds_store import DSStore
        from mac_alias import Alias
    except ImportError:
        raise ValueError("Install tools/requirements-macos-release.txt in the build Python environment") from None
    if not BACKGROUND.is_file():
        raise ValueError("DMG installation background is missing")
    return DSStore, Alias


def write_layout(mount, bundle_name):
    DSStore, Alias = dependencies()
    # Resolve the alias on the image's filesystem, never on the build machine.
    # This keeps the background usable after the DMG is downloaded elsewhere.
    background = Alias.for_file(str(mount / ".background.tiff")).to_bytes()
    with DSStore.open(str(mount / ".DS_Store"), "w+") as store:
        store["."]["vSrn"] = ("long", 1)
        store["."]["icvl"] = ("type", b"icnv")
        store["."]["bwsp"] = {
            "WindowBounds": "{{200, 160}, {660, 440}}",
            "ShowStatusBar": False, "ShowTabView": False,
            "ShowToolbar": False, "ShowPathbar": False,
            "ShowSidebar": False, "ContainerShowSidebar": False,
            "PreviewPaneVisibility": False,
            "SidebarWidth": 0,
        }
        store["."]["icvp"] = {
            "viewOptionsVersion": 1, "backgroundType": 2,
            "backgroundColorRed": 1.0, "backgroundColorGreen": 1.0,
            "backgroundColorBlue": 1.0,
            "backgroundImageAlias": background,
            "gridOffsetX": 0.0, "gridOffsetY": 0.0, "gridSpacing": 80.0,
            "arrangeBy": "none", "showIconPreview": False,
            "showItemInfo": False, "labelOnBottom": True,
            "textSize": 14.0, "iconSize": 96.0,
            "scrollPositionX": 0.0, "scrollPositionY": 0.0,
        }
        store[bundle_name]["Iloc"] = (170, 220)
        store["Applications"]["Iloc"] = (490, 220)


def create_image(source, destination, product_name):
    dependencies()
    bundle_name = product_name + ".app"
    if not (source / bundle_name).is_dir():
        raise ValueError("DMG application is missing")
    if destination.exists():
        raise ValueError("DMG destination already exists")
    shutil.copyfile(BACKGROUND, source / ".background.tiff")
    # Keep the original staged app and final image separate. Layout is written
    # before conversion, signing and notarization; the accepted DMG never changes.
    temporary = Path(tempfile.mkdtemp(prefix="mon-dmg-", dir=destination.parent))
    try:
        writable = temporary / "layout.dmg"
        mount = temporary / "mount"
        mount.mkdir()
        command(["/usr/bin/hdiutil", "create", "-volname", product_name,
                 "-srcfolder", str(source), "-fs", "HFS+", "-format", "UDRW",
                 "-nospotlight", str(writable)], timeout=300)
        attached = False
        try:
            command(["/usr/bin/hdiutil", "attach", "-nobrowse", "-noautoopen",
                     "-owners", "off", "-mountpoint", str(mount), str(writable)], timeout=120)
            attached = True
            write_layout(mount, bundle_name)
        finally:
            # Detach on metadata failure too; never compress a mounted image.
            if attached or os.path.ismount(mount):
                command(["/usr/bin/hdiutil", "detach", str(mount)], timeout=120)
        command(["/usr/bin/hdiutil", "convert", str(writable), "-format", "UDZO",
                 "-o", str(destination)], timeout=300)
    except BaseException:
        # In particular, do not recursively delete a volume whose detach failed.
        print(f"DMG layout failed; retained working image: {temporary}", file=sys.stderr)
        raise
    else:
        shutil.rmtree(temporary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--source", type=Path)
    parser.add_argument("--destination", type=Path)
    parser.add_argument("--product-name")
    args = parser.parse_args()
    dependencies()
    if not args.check:
        if args.source is None or args.destination is None or args.product_name is None:
            parser.error("--source, --destination and --product-name are required")
        create_image(args.source, args.destination, args.product_name)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
