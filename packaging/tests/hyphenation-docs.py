#!/usr/bin/env python3
"""Verify shipped Russian source/license bytes using stub binaries, without compiling Rust.

Needs Bash, GNU install, tar, zip and PyYAML. All package staging is temporary.
"""

import hashlib
import functools
import http.server
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tarfile
import tempfile
import threading
import urllib.request
import zipfile

import yaml


ROOT = Path(__file__).resolve().parents[2]
DATA = Path("crates/compose/src/hyphen")
DOCS = ("hyph-ru.tex", "LPPL-1.3c.txt", "README.md")
SOURCE_SHA = "ce988fcc66e5e29d7474df5ab5b489d3f52a50ebccf570997f992d84221b922a"


def run(args, fixture, env, check=True, timeout=None):
    result = subprocess.run(args, cwd=fixture, env=env, capture_output=True, text=True, timeout=timeout)
    if check and result.returncode:
        raise AssertionError(f"{args}: exit {result.returncode}\n{result.stdout}\n{result.stderr}")
    return result


def check_docs(directory):
    for name in DOCS:
        assert (directory / name).read_bytes() == (ROOT / DATA / name).read_bytes(), directory / name


def main():
    assert shutil.which("zip"), "zip is needed to exercise the actual browser packaging script"
    assert hashlib.sha256((ROOT / DATA / "hyph-ru.tex").read_bytes()).hexdigest() == SOURCE_SHA
    with tempfile.TemporaryDirectory(prefix="designcraft-hyphenation-packaging-") as temporary:
        fixture = Path(temporary) / "fixture with spaces"
        fixture.mkdir()
        files = [
            "packaging/env.sh", "packaging/linux/package.sh", "packaging/freebsd/package.sh",
            "packaging/web/package.sh", "packaging/web/README.md", "packaging/web/_headers",
            "packaging/web/.htaccess", "README.md", "LICENSE-MIT", "LICENSE-APACHE",
            "packaging/linux/ai.storyteller.designcraft.desktop",
            "packaging/linux/ai.storyteller.designcraft.mime.xml",
            "packaging/linux/ai.storyteller.designcraft.metainfo.xml.in",
        ] + [str(DATA / name) for name in DOCS]
        for name in files:
            destination = fixture / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, destination)
        shutil.copytree(ROOT / "assets/app-icon/hicolor", fixture / "assets/app-icon/hicolor")
        env = os.environ.copy()
        env.pop("CRAFT_FONTS_DIR", None)
        env.update(
            DESIGNCRAFT_VERSION="0.0.0-hyphenation-test",
            DESIGNCRAFT_BUILD_SHA="packaging-fixture",
            DESIGNCRAFT_BUILD_DATE="2026-10-08",
            CARGO_TARGET_DIR=str(fixture / "target"), DIST=str(fixture / "dist/release"),
        )
        helper = 'set -euo pipefail; . "$1/packaging/env.sh"; copy_font_licences "$2"'
        resources = fixture / "bundle resources"
        run(["bash", "-c", helper, "test", str(fixture), str(resources)], fixture, env)
        check_docs(resources / "hyphenation")
        ofl = fixture / "craft-fonts/fonts/Example/OFL.txt"
        ofl.parent.mkdir(parents=True)
        ofl.write_text("synthetic font license fixture\n")
        fonts_env = env | {"CRAFT_FONTS_DIR": str(fixture / "craft-fonts")}
        run(["bash", "-c", helper, "test", str(fixture), str(resources)], fixture, fonts_env)
        check_docs(resources / "hyphenation")
        assert (resources / "OFL-Example.txt").read_bytes() == ofl.read_bytes()

        binaries = fixture / "target/release"
        binaries.mkdir(parents=True)
        for name in ("designcraft", "designcraft-cli"):
            binary = binaries / name
            binary.write_text("#!/bin/sh\necho 'packaging verification stub'\n")
            binary.chmod(0o755)
        run(["bash", "packaging/linux/package.sh", "--skip-build", "--formats", "tar"], fixture, env)
        archive = next((fixture / "dist/release").glob("*-linux-*.tar.gz"))
        with tarfile.open(archive) as tar:
            base = archive.name.removesuffix(".tar.gz")
            for name in DOCS:
                member = tar.extractfile(f"{base}/share/doc/designcraft/hyphenation/{name}")
                assert member is not None
                assert member.read() == (ROOT / DATA / name).read_bytes()

        # Inspect the real AppDir passed by package.sh; no AppImage/FUSE tool is needed.
        appimage_marker = fixture / "appimage inspected.txt"
        appimagetool = fixture / "stub appimagetool"
        appimagetool.write_text(
            "#!/usr/bin/env python3\n"
            "import os, sys\n"
            "from pathlib import Path\n"
            "appdir = Path(sys.argv[-2])\n"
            "source = Path(os.environ['DESIGNCRAFT_TEST_HYPHENATION_SOURCE'])\n"
            "docs = appdir / 'usr/share/doc/designcraft/hyphenation'\n"
            "for name in ('hyph-ru.tex', 'LPPL-1.3c.txt', 'README.md'):\n"
            "    assert (docs / name).read_bytes() == (source / name).read_bytes(), name\n"
            "assert not list((appdir / 'usr/share/doc/designcraft').rglob('OFL-*.txt'))\n"
            "Path(os.environ['DESIGNCRAFT_TEST_APPIMAGE_MARKER']).write_text('inspected AppDir\\n')\n"
            "Path(sys.argv[-1]).write_text('stub AppImage\\n')\n"
        )
        appimagetool.chmod(0o755)
        for package_env in (env, fonts_env):
            appimage_marker.unlink(missing_ok=True)
            appimage_env = package_env | {
                "APPIMAGETOOL": str(appimagetool),
                "DESIGNCRAFT_TEST_HYPHENATION_SOURCE": str(fixture / DATA),
                "DESIGNCRAFT_TEST_APPIMAGE_MARKER": str(appimage_marker),
            }
            run(
                ["bash", "packaging/linux/package.sh", "--skip-build", "--formats", "appimage"],
                fixture, appimage_env, timeout=30,
            )
            assert appimage_marker.read_text() == "inspected AppDir\n", "appimagetool stub must inspect the actual AppDir"
            appdir = fixture / "target/linux-package/DesignCraft.AppDir"
            check_docs(appdir / "usr/share/doc/designcraft/hyphenation")
            if "CRAFT_FONTS_DIR" in package_env:
                staged_font = fixture / "target/linux-package/root/usr/share/doc/designcraft/OFL-Example.txt"
                removed_font = fixture / "target/linux-package/doc-unused/designcraft/OFL-Example.txt"
                assert staged_font.read_bytes() == ofl.read_bytes()
                assert removed_font.read_bytes() == ofl.read_bytes()

        site = fixture / "dist/web"
        site.mkdir(parents=True)
        (site / "index.html").write_text('<html><script src="./fixture.js"></script></html>\n')
        (site / "fixture.js").write_text("// synthetic static site\n")
        run(["bash", "packaging/web/package.sh", "--skip-build"], fixture, env)
        archive = next((fixture / "dist/release").glob("designcraft-web-*.zip"))
        with zipfile.ZipFile(archive) as archive_file:
            base = archive.stem
            for name in DOCS:
                assert archive_file.read(f"{base}/hyphenation/{name}") == (ROOT / DATA / name).read_bytes()

        served = fixture / "served"
        with zipfile.ZipFile(archive) as archive_file:
            archive_file.extractall(served)
        handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(served))
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            for name in DOCS:
                url = f"http://127.0.0.1:{server.server_port}/{archive.stem}/hyphenation/{name}"
                with urllib.request.urlopen(url, timeout=5) as response:
                    assert response.read() == (ROOT / DATA / name).read_bytes()
        finally:
            server.shutdown()
            thread.join(timeout=5)
            server.server_close()

        companion = fixture / "designcraft-hyphenation-source.zip"
        run(["python3", str(ROOT / "packaging/hyphenation-source.py"), str(companion)], fixture, env)
        with zipfile.ZipFile(companion) as source_archive:
            for name in DOCS:
                assert source_archive.read(f"hyphenation/{name}") == (ROOT / DATA / name).read_bytes()
            for name in ("crates/compose/src/lib.rs", "crates/compose/src/hyphen/mod.rs", "LICENSE-MIT", "LICENSE-APACHE"):
                assert source_archive.read(name) == (ROOT / name).read_bytes()

        run(["bash", "packaging/freebsd/package.sh", "--dry-run"], fixture, env)
        stage = next(p for p in (fixture / "target/freebsd-package").glob("designcraft-*-freebsd-*") if p.is_dir())
        check_docs(stage / "share/doc/designcraft/hyphenation")

        source = yaml.safe_load((ROOT / "packaging/linux/flatpak/ai.storyteller.designcraft.yml").read_text())
        bundle = yaml.safe_load((ROOT / "packaging/linux/flatpak/ai.storyteller.designcraft.bundle.yml").read_text())
        for key in ("runtime", "runtime-version", "sdk", "finish-args"):
            assert source[key] == bundle[key], key
        app = fixture / "source-flatpak/app"
        commands = [c for c in source["modules"][0]["build-commands"] if c.startswith(f"install -Dm644 {DATA}/")]
        assert len(commands) == len(DOCS)
        for command in commands:
            args = shlex.split(command)
            args[-1] = str(app / Path(args[-1]).relative_to("/app"))
            run(args, fixture, env)
        check_docs(app / "share/doc/designcraft/hyphenation")
        app = fixture / "bundle-flatpak/app"
        commands = [c for c in bundle["modules"][0]["build-commands"] if "cp -R share/doc/designcraft" in c]
        assert len(commands) == 1
        # Exercise the manifest's existing documentation-tree copy in a private staging root.
        command = commands[0].replace("/app", shlex.quote(str(app)))
        linux_stage = fixture / "target/linux-package/root/usr"
        run(["bash", "-c", command], linux_stage, env)
        check_docs(app / "share/doc/designcraft/hyphenation")

        (fixture / DATA / "hyph-ru.tex").unlink()
        result = run(["bash", "-c", helper, "test", str(fixture), str(resources)], fixture, env, check=False)
        assert result.returncode != 0, "missing source must fail packaging"
    print("PASS: shared copies with/without optional fonts; Linux tar and actual AppDir inspected by stub appimagetool with/without optional fonts; browser ZIP and FreeBSD dry-run; Flatpak source/bundle data copies; exact source/license/provenance bytes; HTTP retrieval under a prefix; generic release source ZIP; missing source rejected. Stub binaries only, no cargo build.")


if __name__ == "__main__":
    main()
