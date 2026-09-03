#!/usr/bin/env python3
"""Build a clean BIOS folder from a messy one, using the firmware manifest.

`data/bios-manifest.json` lists every firmware file the ES-DE system set can
ask for — 277 files across 95 systems — derived from ES-DE's own system
definitions joined with RetroArch's per-core `firmwareN_path` declarations.
This copies whatever a source folder has into a fresh one laid out exactly the
way the cores look it up.

Two details that make or break it:

* **Layout is part of the lookup.** 128 of the 277 sit in a subdirectory
  (`dc/dc_boot.bin`, `PPSSPP/ppge_atlas.zim`, `keropi/cgrom.dat`). A flat dump
  of the same files does not work.
* **Matching is by path first, then by name.** A file already at the right
  relative path is taken as-is; otherwise the basename is searched for
  anywhere in the source, because collections tend to nest things arbitrarily.

Nothing is deleted or moved — the destination is built alongside the original
so the two can be compared before anything is thrown away.
"""

import argparse
import json
import pathlib
import shutil
import sys
from collections import defaultdict


def index_source(root):
    """Every file under `root`, indexed by relative path and by basename."""
    by_rel, by_name = {}, defaultdict(list)
    for p in root.rglob("*"):
        if not p.is_file():
            continue
        rel = p.relative_to(root).as_posix()
        by_rel[rel] = p
        by_rel.setdefault(rel.lower(), p)
        by_name[p.name.lower()].append(p)
    return by_rel, by_name


# Files that are never firmware, whatever a collection accumulates.
JUNK_NAMES = {".ds_store", ".gitkeep", "readme.md", "readme.txt", "thumbs.db", ".localized"}
JUNK_SUFFIX = (".bak",)


def mirror_plan(source, manifest):
    """Everything in `source` except junk and duplicate content.

    The manifest is a floor, not a ceiling: plenty of real firmware is never
    declared in a core's `.info` file — `NstDatabase.xml`, `nds_firmware/`,
    `kronos/saturn_bios.bin`, the 3DO `panafz10` variants. Copying only what
    the manifest names would quietly break those systems, so this keeps
    everything and removes only what is provably useless.

    Identical *content* under two different names is NOT a duplicate here.
    Emulators resolve firmware by exact file name: `bios_E.sms` and
    `bios_U.sms` are how Genesis Plus GX picks the European and US Master
    System BIOS, `fbneo/spec128k.zip` is a romset name, and `cgb_boot.bin` and
    `gbc_bios.bin` are the same bytes wanted by different cores. Removing
    either half of such a pair silently breaks a system, so they are only
    reported.
    """
    import hashlib

    wanted = {r["path"] for r in manifest}
    by_hash = defaultdict(list)
    dropped = []

    for p in sorted(source.rglob("*")):
        if not p.is_file():
            continue
        rel = p.relative_to(source).as_posix()
        name = p.name.lower()
        if name in JUNK_NAMES or name.endswith(JUNK_SUFFIX) or name.startswith("._"):
            dropped.append((rel, "junk"))
            continue
        h = hashlib.md5(p.read_bytes()).hexdigest()
        by_hash[h].append((rel, p))

    plan = []
    twins = []
    for entries in by_hash.values():
        for rel, p in entries:
            plan.append((p, rel, "mirror", False))
        if len(entries) > 1:
            entries.sort(key=lambda e: (e[0] not in wanted, e[0].count("/"), len(e[0])))
            twins.append([r for r, _ in entries])
    plan.sort(key=lambda e: e[1])
    return plan, dropped, twins


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--manifest", default="data/bios-manifest.json")
    ap.add_argument("--source", required=True)
    ap.add_argument("--dest", required=True)
    ap.add_argument("--extra-source", action="append", default=[],
                    help="additional folder to fill gaps from")
    ap.add_argument("--keep-dir", action="append", default=[],
                    help="copy this whole directory across, manifest or not")
    ap.add_argument("--mirror", action="store_true",
                    help="copy everything except junk and content-duplicates, "
                         "rather than only what the manifest names")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    manifest = json.loads(pathlib.Path(args.manifest).read_text())
    source = pathlib.Path(args.source)
    dest = pathlib.Path(args.dest)
    if not source.is_dir():
        sys.exit(f"no source folder at {source}")

    by_rel, by_name = index_source(source)
    extras = [index_source(pathlib.Path(e)) for e in args.extra_source
              if pathlib.Path(e).is_dir()]

    plan, missing = [], []
    used = set()
    for rec in manifest:
        want = rec["path"]
        src = by_rel.get(want) or by_rel.get(want.lower())
        how = "path"
        if src is None:
            cands = by_name.get(pathlib.PurePath(want).name.lower(), [])
            if cands:
                src = sorted(cands, key=lambda p: len(p.as_posix()))[0]
                how = "name"
        if src is None:
            for e_rel, e_name in extras:
                src = e_rel.get(want) or e_rel.get(want.lower())
                if src is None:
                    c = e_name.get(pathlib.PurePath(want).name.lower(), [])
                    src = sorted(c, key=lambda p: len(p.as_posix()))[0] if c else None
                if src is not None:
                    how = "extra"
                    break
        if src is None:
            missing.append(rec)
        else:
            used.add(src.resolve())
            plan.append((src, want, how, rec["required"]))

    # Three manifest entries name a file but mean the folder around it —
    # "'Databases' folder", "Dolphin 'Sys' folder", "'Machines' folder".
    # Copying only the named file leaves blueMSX and Dolphin broken, so the
    # whole tree comes along.
    folder_roots = set()
    for rec in manifest:
        if "folder" in rec["desc"].lower():
            parts = pathlib.PurePath(rec["path"]).parts
            if len(parts) > 1:
                folder_roots.add(parts[0])
    folder_roots.update(args.keep_dir)

    kept_trees = []
    for d in sorted(folder_roots):
        tree = source / d
        if tree.is_dir():
            for f in tree.rglob("*"):
                if f.is_file() and f.resolve() not in used:
                    kept_trees.append((f, f.relative_to(source).as_posix(), "tree", False))
                    used.add(f.resolve())
    plan.extend(kept_trees)
    if kept_trees:
        print(f"whole folders  {', '.join(sorted(folder_roots))}"
              f"  ({len(kept_trees)} extra files)")

    req_missing = [m for m in missing if m["required"]]
    by_how = defaultdict(int)
    for _, _, how, _ in plan:
        by_how[how] += 1
    total = sum(p[0].stat().st_size for p in plan)

    print(f"manifest      {len(manifest)} files ({sum(1 for m in manifest if m['required'])} required)")
    print(f"found         {len(plan)}  (by path {by_how['path']}, by name {by_how['name']}"
          + (f", from extra {by_how['extra']}" if by_how["extra"] else "") + ")")
    print(f"size          {total/1e6:.0f} MB")
    print(f"absent        {len(missing)}  ({len(req_missing)} of them required)")
    for m in req_missing:
        print(f"   REQUIRED  {m['path']:<30} {m['desc'][:44]}")

    # What the source holds that no core ever asks for.
    unused = [p for p in source.rglob("*") if p.is_file() and p.resolve() not in used]
    unused_size = sum(p.stat().st_size for p in unused)
    print(f"\nnot in manifest {len(unused)} files ({unused_size/1e6:.0f} MB) — left where they are")
    for p in sorted(unused)[:12]:
        print(f"   {p.relative_to(source).as_posix()}")
    if len(unused) > 12:
        print(f"   ... and {len(unused)-12} more")

    if args.mirror:
        plan, dropped, twins = mirror_plan(source, manifest)
        total = sum(p[0].stat().st_size for p in plan)
        print(f"\nmirror mode: {len(plan)} files, {total/1e6:.0f} MB")
        print(f"dropped {len(dropped)} junk file(s):")
        for rel, why in dropped:
            print(f"   {why:<10} {rel}")
        if twins:
            dup_bytes = 0
            for group in twins:
                p0 = source / group[0]
                dup_bytes += p0.stat().st_size * (len(group) - 1)
            print(f"\n{len(twins)} set(s) of identical content under different names "
                  f"({dup_bytes/1e6:.0f} MB) — KEPT, since cores look firmware up by "
                  f"exact name:")
            for group in twins[:12]:
                print(f"   {' = '.join(group)}")
            if len(twins) > 12:
                print(f"   ... and {len(twins)-12} more")

    if not args.apply:
        print(f"\n(dry run — pass --apply to build {dest})")
        return

    if dest.exists():
        shutil.rmtree(dest)
    for src, want, _, _ in plan:
        out = dest / want
        out.parent.mkdir(parents=True, exist_ok=True)
        # copyfile, not copy2: copy2 preserves extended attributes, and on the
        # exFAT card macOS materialises those as `._name` AppleDouble sidecars
        # — one per file, doubling the file count and confusing emulators that
        # scan a directory. Content is all that matters for firmware.
        shutil.copyfile(src, out)
    n = sum(1 for _ in dest.rglob("*") if _.is_file())
    print(f"\nbuilt {dest}: {n} files, {total/1e6:.0f} MB")


if __name__ == "__main__":
    main()
