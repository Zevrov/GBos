"""Packages an Adventure Pad release for Windows: the app, and its source without the Android app.

    python rust/tools/package.py [--no-build]

Builds `AdventurePad.exe` (release; needs the toolchain on PATH, as rust/README.md's Build says) and writes to
`rust/target/dist/`:

- `AdventurePad-<version>-windows-x64.zip`: the app's folder - AdventurePad.exe with README, LICENSE and NOTICE.
  One exe, no installer, no DLLs beside it (the build is checked for any it would need).
- `AdventurePad-<version>-source.zip`: what builds it - rust/, the mGBA submodule, and the two EXPAND files the
  build takes from app/src/main/cpp (pk_wide.c and its headers) - with LICENSE and NOTICE. The GPL's source for
  the app, without the Android project around it. The files git has, or would take (not ignored ones).
- `SHA256SUMS.txt` for both.
"""

import hashlib
import re
import subprocess
import sys
import zipfile
from pathlib import Path

RUST = Path(__file__).resolve().parents[1]
REPO = RUST.parent
DIST = RUST / "target" / "dist"

# What the build needs from outside rust/ (rust/crates/mgba-sys/CMakeLists.txt).
APP_CPP = ["app/src/main/cpp/pk_wide.c", "app/src/main/cpp/pk_wide.h", "app/src/main/cpp/pk_wide_games.h"]

# The DLLs a Windows install always has. Anything else (libgcc_s, libwinpthread...) would have to ship beside the exe.
SYSTEM_DLL = re.compile(
    r"^(kernel32|advapi32|api-ms-win-.*|bcrypt.*|combase|dwmapi|gdi32|imm32|msvcrt|ntdll|ole32|oleaut32|"
    r"opengl32|shell32|shlwapi|user32|uxtheme|ws2_32|userenv|winmm|comdlg32|comctl32|dbghelp|propsys|"
    r"setupapi|version|cfgmgr32|hid|xinput.*|dinput8|dsound|avrt|mmdevapi|secur32|crypt32|ucrtbase)\.dll$",
    re.I,
)

SOURCE_NOTE = """Adventure Pad {version}: the source of the Windows app.

Based on PokeDaisy by Lidor Itzhari - https://github.com/lidor30/pokedaisy

This is the part of https://github.com/Zevrov/GBos that builds AdventurePad.exe: rust/ (the app), third_party/mgba
(mGBA {mgba}, which runs the games) and EXPAND's pk_wide.c from the Android app (app/src/main/cpp). Build it as
rust/README.md says (the mGBA submodule is already here: skip `git submodule update`).

Left out: the Android app, and the RAM dumps some tests read (app/src/test/resources/fixtures): those tests need
the whole repository.

GNU GPL v3 with PokeDaisy's additional term: see LICENSE and NOTICE.
"""


def run(cmd, cwd):
    return subprocess.run(cmd, cwd=cwd, check=True, capture_output=True, text=True).stdout


def version():
    text = (RUST / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'\[workspace\.package\][^\[]*?version\s*=\s*"([^"]+)"', text, re.S)
    if not m:
        sys.exit("no version in rust/Cargo.toml's [workspace.package]")
    return m.group(1)


def check_dlls(exe):
    """Fails if the exe imports a DLL Windows doesn't have (objdump from MinGW; skipped without it)."""
    try:
        dump = run(["objdump", "-p", str(exe)], RUST)
    except (OSError, subprocess.CalledProcessError):
        print("objdump not found: the DLL check is skipped")
        return
    dlls = sorted({m.group(1) for m in re.finditer(r"DLL Name: (\S+)", dump)}, key=str.lower)
    extra = [d for d in dlls if not SYSTEM_DLL.match(d)]
    if extra:
        sys.exit(f"AdventurePad.exe needs DLLs that would have to ship with it: {', '.join(extra)}")
    print(f"needs only Windows' own DLLs ({len(dlls)})")


def tracked(repo, paths=()):
    """The files git has under `paths`, and new ones it would take (not ignored: no target/, no ROMs)."""
    out = run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", *paths], repo)
    return sorted({p for p in out.split("\0") if p})


def write_zip(path, files):
    """files: (name in the zip, file on disk)."""
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for name, src in files:
            z.write(src, name)
    print(f"{path.name}: {len(files)} files, {path.stat().st_size / 1e6:.1f} MB")


def main():
    ver = version()
    if "--no-build" not in sys.argv:
        print("building AdventurePad.exe (release)...")
        subprocess.run(["cargo", "build", "--release", "-p", "adventurepad"], cwd=RUST, check=True)
    exe = RUST / "target" / "release" / "AdventurePad.exe"
    if not exe.is_file():
        sys.exit(f"no {exe}: build it first")
    check_dlls(exe)
    DIST.mkdir(parents=True, exist_ok=True)

    app = DIST / f"AdventurePad-{ver}-windows-x64.zip"
    write_zip(
        app,
        [
            ("AdventurePad/AdventurePad.exe", exe),
            ("AdventurePad/README.md", RUST / "README.md"),
            ("AdventurePad/LICENSE", REPO / "LICENSE"),
            ("AdventurePad/NOTICE", REPO / "NOTICE"),
        ],
    )

    mgba = REPO / "third_party" / "mgba"
    if not (mgba / "CMakeLists.txt").is_file():
        sys.exit("the mGBA submodule is missing: git submodule update --init third_party/mgba")
    mgba_ver = run(["git", "describe", "--tags", "--always"], mgba).strip()
    top = f"AdventurePad-{ver}-source"
    files = [(f"{top}/{p}", REPO / p) for p in tracked(REPO, ["rust", *APP_CPP, "LICENSE", "NOTICE"])]
    # Not mGBA's cinema/: its video regression tests, which the build doesn't use (and whose paths are too long
    # for Windows to unzip).
    files += [(f"{top}/third_party/mgba/{p}", mgba / p) for p in tracked(mgba) if not p.startswith("cinema/")]
    files = [(n, s) for n, s in files if s.is_file()]  # a submodule's gitlink, a deleted file
    note = DIST / "SOURCE.txt"
    note.write_text(SOURCE_NOTE.format(version=ver, mgba=mgba_ver), encoding="utf-8")
    files.append((f"{top}/SOURCE.txt", note))
    src = DIST / f"{top}.zip"
    write_zip(src, files)

    sums = "".join(f"{hashlib.sha256(z.read_bytes()).hexdigest()}  {z.name}\n" for z in (app, src))
    (DIST / "SHA256SUMS.txt").write_text(sums, encoding="utf-8")
    print(f"in {DIST}")


if __name__ == "__main__":
    main()
