#!/usr/bin/env python3
"""Create the source companion shared by every release format, without compiling Rust."""

import argparse
import hashlib
from pathlib import Path
import zipfile


ROOT = Path(__file__).resolve().parents[1]
DATA = Path("crates/compose/src/hyphen")
SOURCE_SHA = "ce988fcc66e5e29d7474df5ab5b489d3f52a50ebccf570997f992d84221b922a"
INTEGRATION = (
    "crates/compose/Cargo.toml",
    "crates/compose/src/lib.rs",
    "crates/compose/src/hyphen/mod.rs",
    "crates/compose/src/hyphen/dict.rs",
    "crates/compose/src/hyphen/patterns.rs",
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    if hashlib.sha256((ROOT / DATA / "hyph-ru.tex").read_bytes()).hexdigest() != SOURCE_SHA:
        parser.error("Russian source does not match the documented upstream SHA-256")
    files = [(DATA / name, f"hyphenation/{name}") for name in ("hyph-ru.tex", "LPPL-1.3c.txt", "README.md")]
    files += [(Path(name), name) for name in INTEGRATION]
    files += [(Path(name), name) for name in ("LICENSE-MIT", "LICENSE-APACHE")]
    # Read all required sources before opening the output, so missing inputs fail closed.
    contents = [(name, (ROOT / source).read_bytes()) for source, name in files]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, content in contents:
            archive.writestr(name, content)
    print(f"wrote {args.output}")


if __name__ == "__main__":
    main()
