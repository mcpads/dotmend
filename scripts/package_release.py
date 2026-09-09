"""Archive a native build, then verify the executable extracted from that archive."""

from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile

from check_mcp import check


def package(target):
    version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
    windows = target.endswith("windows-msvc")
    name = "dotmend.exe" if windows else "dotmend"
    binary = Path("target") / target / "release" / name
    destination = Path("dist")
    destination.mkdir(exist_ok=True)
    archive = destination / f"dotmend-{target}{'.zip' if windows else '.tar.gz'}"
    if windows:
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
            bundle.write(binary, name)
            bundle.write("LICENSE", "LICENSE")
    else:
        with tarfile.open(archive, "w:gz") as bundle:
            bundle.add(binary, arcname=name)
            bundle.add("LICENSE", arcname="LICENSE")
    with tempfile.TemporaryDirectory(prefix="dotmend-package-") as directory:
        if windows:
            with zipfile.ZipFile(archive) as bundle:
                bundle.extractall(directory)
        else:
            with tarfile.open(archive) as bundle:
                bundle.extractall(directory, filter="data")
        executable = Path(directory) / name
        reported = subprocess.check_output([str(executable), "--version"], text=True, timeout=10).strip()
        assert reported == f"dotmend {version}", reported
        check(executable)
    print(f"Verified {archive}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("Usage: python scripts/package_release.py RUST_TARGET")
    package(sys.argv[1])
