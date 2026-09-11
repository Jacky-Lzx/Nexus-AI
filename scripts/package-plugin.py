#!/usr/bin/env python3
"""Build one Rust plugin and produce an independently installable macOS bundle."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import tempfile
import tomllib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("source", type=Path, help="Plugin source directory (Cargo.toml and plugin.toml)")
parser.add_argument("--output", type=Path, default=Path("dist/plugins"))
parser.add_argument("--identity", default=os.environ.get("CLIPKEEP_CODESIGN_IDENTITY"), help="Optional code signing identity")
args = parser.parse_args()
source = args.source.resolve()
manifest = tomllib.loads((source / "plugin.toml").read_text())
plugin_id = manifest["id"]
entrypoint = manifest["entrypoint"]
if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]+", plugin_id):
    parser.error("Invalid plugin ID")
if not entrypoint or "/" in entrypoint or ".." in entrypoint:
    parser.error("Entrypoint must be a filename")
if manifest.get("protocol_version") != 1:
    parser.error("Unsupported protocol version")
cargo_manifest = source / "Cargo.toml"
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--no-deps", "--format-version", "1", "--manifest-path", str(cargo_manifest)
]))
package = next(p for p in metadata["packages"] if Path(p["manifest_path"]).resolve() == cargo_manifest)
if not any(t["name"] == entrypoint and "bin" in t["kind"] for t in package["targets"]):
    parser.error("Entrypoint must match a Cargo binary target")
env = dict(os.environ, MACOSX_DEPLOYMENT_TARGET="26.0")
subprocess.run(["cargo", "build", "--release", "--manifest-path", str(cargo_manifest),
                "--package", package["name"], "--bin", entrypoint], env=env, check=True)
output = args.output.resolve()
output.mkdir(parents=True, exist_ok=True)
destination = output / (plugin_id + ".clipkeepplugin")
with tempfile.TemporaryDirectory(prefix=".package-", dir=output) as temp:
    bundle = Path(temp) / destination.name
    contents = bundle / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    (contents / "Resources").mkdir()
    shutil.copy2(Path(metadata["target_directory"]) / "release" / entrypoint, contents / "MacOS" / entrypoint)
    if (source / "resources").is_dir():
        shutil.copytree(source / "resources", contents / "Resources", dirs_exist_ok=True)
    shutil.copy2(source / "plugin.toml", contents / "Resources" / "plugin.toml")
    with (contents / "Info.plist").open("wb") as stream:
        plistlib.dump(dict(CFBundleIdentifier=plugin_id, CFBundleName=manifest["name"],
                          CFBundleExecutable=entrypoint, CFBundlePackageType="BNDL",
                          CFBundleVersion=manifest["version"], CFBundleShortVersionString=manifest["version"]), stream)
    subprocess.run(["codesign", "--force", "--sign", args.identity or "-", "--timestamp=none", str(bundle)], check=True)
    subprocess.run(["codesign", "--verify", "--strict", str(bundle)], check=True)
    # Output artifacts are not the live installation directory. The host installs atomically.
    if destination.exists():
        shutil.rmtree(destination)
    shutil.move(str(bundle), destination)
print(destination)
