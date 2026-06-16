#!/usr/bin/env python3
"""rustchm regression test harness.

Compiles each test project, asserts the expected internal CHM files are present,
and round-trips through rustchm's own --extract asserting byte-identical source
files. Self-contained: no hh.exe / OS dependency.

Usage: python test/run_tests.py [path-to-rustchm(.exe)]
Exit 0 = all passed, 1 = a failure.
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
from chmdir import read_dir  # noqa: E402

if len(sys.argv) > 1:
    RUSTCHM = sys.argv[1]
else:
    exe = "rustchm.exe" if os.name == "nt" else "rustchm"
    RUSTCHM = os.path.join(ROOT, "target", "release", exe)

passed = 0
failed = 0


def check(name, cond, detail=""):
    global passed, failed
    if cond:
        passed += 1
        print("  ok   %s" % name)
    else:
        failed += 1
        print("  FAIL %s %s" % (name, detail))
        print("::error::FAIL %s %s" % (name, detail))


def run(args):
    r = subprocess.run([RUSTCHM] + args, capture_output=True, text=True)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def sha(path):
    return hashlib.sha256(open(path, "rb").read()).hexdigest()


def source_files(proj_dir):
    exts = (".htm", ".html", ".css", ".hhc", ".hhk", ".js", ".gif", ".png", ".jpg")
    out = {}
    for root, _d, files in os.walk(proj_dir):
        for f in files:
            if f.lower().endswith(exts):
                out.setdefault(f.lower(), os.path.join(root, f))
    return out


def extract_roundtrip(proj_dir, chm):
    tmp = tempfile.mkdtemp(prefix="rustchm_x_")
    try:
        run(["--extract", os.path.abspath(chm), tmp])
        srcs = source_files(proj_dir)
        checked, bad = 0, []
        for root, _d, files in os.walk(tmp):
            for f in files:
                src = srcs.get(f.lower())
                if not src:
                    continue
                checked += 1
                if sha(os.path.join(root, f)) != sha(src):
                    bad.append(f)
        return checked, bad
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def test_project(hhp, must_have, proj_dir=None):
    print("[project] %s" % os.path.basename(hhp))
    proj_dir = proj_dir or os.path.dirname(hhp)
    rc, out = run([hhp])
    check("compiles", rc == 0, out.strip())
    if rc != 0:
        return
    stem = os.path.splitext(os.path.basename(hhp))[0]
    chm = os.path.join(proj_dir, stem + ".chm")
    check("output exists", os.path.exists(chm), chm)
    if not os.path.exists(chm):
        return
    entries = read_dir(chm)
    for nm in must_have:
        check("contains %s" % nm, nm in entries)
    xchecked, xbad = extract_roundtrip(proj_dir, chm)
    check("round-trip byte-identical (%d files)" % xchecked, xchecked > 0 and not xbad, str(xbad))


def main():
    if not os.path.exists(RUSTCHM):
        print("rustchm not found at %s" % RUSTCHM)
        return 1

    common = ["/#SYSTEM", "/#STRINGS", "/#TOPICS",
              "::DataSpace/Storage/MSCompressed/Content"]

    test_project(os.path.join(ROOT, "test", "basic", "basic.hhp"), common)

    test_project(
        os.path.join(ROOT, "test", "featured", "featured.hhp"),
        common + ["/#WINDOWS", "/#TOCIDX", "/#IDXHDR",
                  "/$WWKeywordLinks/BTree", "/$WWAssociativeLinks/BTree",
                  "/$FIftiMain", "/$OBJINST"],
    )

    test_project(os.path.join(ROOT, "test", "utf8", "utf8.hhp"), common)
    print("[unicode] utf8 #STRINGS codepage check")
    tmp = tempfile.mkdtemp(prefix="rustchm_u_")
    try:
        run(["--extract", os.path.join(ROOT, "test", "utf8", "utf8.chm"), tmp])
        s = open(os.path.join(tmp, "#STRINGS"), "rb").read()
        check("title stored as cp1252 (0xE9/0xFC, no 0xC3)",
              0xE9 in s and 0xFC in s and 0xC3 not in s, s.hex())
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    print("\n%d passed, %d failed" % (passed, failed))
    return 1 if failed else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:
        import traceback
        print("::error::harness crashed: %r" % e)
        traceback.print_exc()
        sys.exit(2)
