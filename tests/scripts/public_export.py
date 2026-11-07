#!/usr/bin/env python3
"""Build the public export tree and prove it clean (R6b gate).

Exports `git archive HEAD`, removes private evidence and reports
(docs/baselines and docs/reports), scrubs private identifiers/paths/emails, rounds the
per-corpus table, then runs zero-assert checks over the staged tree.

Usage: python3 tests/scripts/public_export.py --src <repo> --out <empty dir>
Exit 0 only when every check reports zero.
"""
from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPLACEMENTS = [
    ("--vihap-root", "--corpus-a-root"),
    ("--new-vihap", "--new-corpus-a"),
    ("--old-vihap", "--old-corpus-a"),
    ("vihap", "corpus-a"),
    ("finguard", "corpus-b"),
    ("mapkit", "workspace"),
    ("mskit-ui", "frontend-a"),
    ("qauto-ui", "frontend-b"),
    ("be-pool-collector", "service-a"),
    ("be-ds-forecast", "service-b"),
    ("be-map-search-v2", "service-c"),
    ("ds-team-dag", "service-d"),
    ("be-parser-geocoder", "service-e"),
    ("be-mercury", "service-f"),
    ("food-dispatch", "service-g"),
    ("agent-skills", "companion-skills"),
    ("the private evidence vault's transcripts/SUMMARY.md", "the internal evidence records"),
    ("the private evidence vault's", "the internal evidence records'"),
    ("the private evidence vault", "the internal evidence records"),
    ("HierarchicalAddressParser", "WidgetParser"),
    ("get_knowledge_base", "get_widget_base"),
    ("normalize_ascii", "normalize_text"),
    ("knowledge.marshal_io", "pkg.io"),
    ("marshal_io", "io_utils"),
    ("default_artifacts_dir", "default_artifacts_path"),
    ("knowledge_artifacts_dir", "knowledge_artifacts_path"),
    ("hierarchical address parser parse", "widget parser parse"),
    ("get knowledge base", "get widget base"),
    ("normalize ascii", "normalize text"),
    ("gso_loader", "module_loader"),
    ("AddressPairwiseScorer", "PairScorer"),
    ("AddressQualityScorer", "QualityScorer"),
    ("GsoIndex", "SymbolIndex"),
    ("MicroStreetParser", "StreetParser"),
    ("MacroAdminResolver", "AdminResolver"),
    ("/Users/quy.doan", "<home>"),
    ("~/Workspace", "<workspace>"),
    ("~/.grok", "$GROK_HOME"),
    ("~/.config/opencode", "$XDG_CONFIG_HOME"),
    ("~/.zcode", "$ZCODE_HOME"),
    ("~/.agents", "$AGENTS_HOME"),
    ("~/", "<home>/"),
    ("quy.doan@be.com.vn", "noreply@example.invalid"),
    ("be.com.vn", "example.invalid"),
    ("f7ca48c", "<frozen-rev>"),
    ("56f448a", "<frozen-rev>"),
    ("80761de", "<frozen-rev>"),
    ("82d0b02", "<frozen-rev>"),
    ("6a53414", "<frozen-rev>"),
    ("d0c6136", "<frozen-rev>"),
]

TEXT_SUFFIXES = {".md", ".rs", ".py", ".sh", ".toml", ".sql", ".yml", ".yaml", ".json"}

# Scripts that depend on private ground-truth files. The exporter itself and
# its regression test are also private to the source repository.
PRIVATE_SCRIPTS = {
    "public_export.py",
    "test_public_export.py",
    "import_recall.py",
    "inheritance_recall.py",
    "path_recall.py",
    "search_gap_recall.py",
    "search_recall.py",
}


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=True, **kw)


def export(src: Path, out: Path) -> None:
    src = src.resolve()
    out = out.resolve()
    if out == src or src.is_relative_to(out) or out.is_relative_to(src):
        raise SystemExit("--out must be outside --src and must not overlap it")
    # Validate the source archive before touching the destination.
    archive = subprocess.run(
        ["git", "-C", str(src), "archive", "--format=tar", "HEAD"],
        check=True, capture_output=True,
    ).stdout
    if out.exists():
        if not out.is_dir():
            raise SystemExit("--out exists and is not a directory")
        if any(out.iterdir()):
            raise SystemExit("refusing non-empty --out; choose a fresh directory")
    out.mkdir(parents=True, exist_ok=True)
    subprocess.run(["tar", "-x", "-C", str(out)], input=archive, check=True)
    shutil.rmtree(out / "docs" / "baselines", ignore_errors=True)
    shutil.rmtree(out / "docs" / "reports", ignore_errors=True)
    for name in PRIVATE_SCRIPTS:
        (out / "tests" / "scripts" / name).unlink(missing_ok=True)


def scrub(out: Path) -> None:
    for path in out.rglob("*"):
        if path.is_symlink():
            raise SystemExit(f"refusing symlink in export: {path}")
        if not path.is_file() or path.suffix not in TEXT_SUFFIXES:
            continue
        text = path.read_text(errors="replace")
        for old, new in REPLACEMENTS:
            text = text.replace(old, new)
        path.write_text(text)
    # Doc-only path references: flatten links, then leave a single readable
    # token. Never apply this to .yml/.toml/.py (it broke the CI guard and
    # the Cargo exclude in round 2).
    for path in out.rglob("*.md"):
        if path.is_symlink():
            continue
        t = path.read_text(errors="replace")
        t = re.sub(r"\[([^\]]+)\]\([^)]*(?:docs/)?baselines[^)]*\)", r"\1", t)
        t = t.replace("docs/baselines/", "evidence/").replace("baselines/", "evidence/")
        path.write_text(t)
    # The gate script itself carries the private-pattern vocabulary; it stays
    # in the private repo (R6 review finding).
    for name in PRIVATE_SCRIPTS:
        (out / "tests" / "scripts" / name).unlink(missing_ok=True)
    # Round the per-corpus table (exact counts are fingerprints).
    ev = out / "docs" / "EVALUATION.md"
    if ev.exists():
        text = ev.read_text()
        text = re.sub(
            r"\| corpus A \|.*?\n\| aw self \|.*?\n",
            "| corpora | six repositories, ~1.2k files / ~8.8k symbols / "
            "~19.2k edges total (per-corpus detail withheld) | - | - | - |\n"
            "| aw self | this repository (Rust) | - | - | - |\n",
            text, flags=re.S,
        )
        text = text.replace(
            "Corpora are referred to by neutral labels throughout:",
            "Corpora are referred to by neutral labels; per-corpus detail is withheld.",
        )
        ev.write_text(text)


def gate(out: Path) -> int:
    run(["git", "init", "-q"], cwd=out)
    run(["git", "add", "-A"], cwd=out)
    # Patterns are assembled from pieces so this gate file does not match
    # itself when the export is scanned.
    checks = {
        "private_names": "|".join(["vi" + "hap", "fin" + "guard", "map" + "kit",
            "mskit" + "-ui", "qauto" + "-ui", "be-pool-" + "collector",
            "ds-team-" + "dag", "be-map-search-" + "v2", "be-ds-" + "forecast",
            "be-parser-" + "geocoder", "be-" + "mercury", "food-" + "dispatch",
            "agent-" + "skills"]),
        "private_symbols": "|".join(["Hierarchical" + "AddressParser", "marshal" + "_io",
            "get_knowledge" + "_base", "normalize_" + "ascii",
            "_gso_" + "loader", "AddressPairwise" + "Scorer",
            "AddressQuality" + "Scorer", "Gso" + "Index",
            "MicroStreet" + "Parser", "MacroAdmin" + "Resolver"]),
        "personal_paths": "|".join(["/Us" + "ers/", "/home/[A-Za-z0-9._-]+/",
            "/Work" + "space/", "/private/" + "tmp", "/tmp/utility-" + "corpus"]),
        "employer_email": "quy" + "\\.doan@|@" + "be" + "\\.com\\.vn",
        "home_tilde": "~" + "/",
        "secrets": "|".join(["(AKIA|ASIA)[0-9A-Z]{16}",
            "gh[pousr]_[A-Za-z0-9]{36,}", "github_pat_[A-Za-z0-9_]{22,}",
            "xox[abprs]-[A-Za-z0-9-]{10,}",
            "-----BEGIN [A-Z ]*PRIVATE KEY-----"]),
    }
    rc = 0
    for label, pat in checks.items():
        try:
            res = subprocess.run(
                ["git", "grep", "--cached", "-I", "-n", "-E", pat, "--", "."],
                cwd=out, capture_output=True, text=True,
            )
        except OSError as exc:
            print(f"{label:16s} scan-error: {exc}")
            rc = 1
            continue
        if res.returncode not in (0, 1):
            print(f"{label:16s} scan-error {res.returncode}")
            print(res.stderr[:2000])
            rc = 1
            continue
        n = len([l for l in res.stdout.splitlines() if l.strip()])
        print(f"{label:16s} {n}")
        if n:
            print(res.stdout[:2000])
            rc = 1
    for label, pat in {
        "vault_prose": "the private evidence vault",
        "empty_link": "\\]\\(\\)",
    }.items():
        try:
            res = subprocess.run(
                ["git", "grep", "--cached", "-I", "-n", "-E", pat, "--", "."],
                cwd=out, capture_output=True, text=True,
            )
        except OSError as exc:
            print(f"{label:16s} scan-error: {exc}")
            rc = 1
            continue
        if res.returncode not in (0, 1):
            print(f"{label:16s} scan-error {res.returncode}")
            print(res.stderr[:2000])
            rc = 1
            continue
        n = len([l for l in res.stdout.splitlines() if l.strip()])
        print(f"{label:16s} {n}")
        if n:
            print(res.stdout[:2000]); rc = 1
    cargo = (out / "Cargo.toml").read_text()
    if "include = [" not in cargo or "exclude = [" in cargo:
        print("Cargo.toml include allowlist is missing or exclude is still set")
        rc = 1
    for bad in ("docs/baselines", "docs/reports", ".aw", "target", ".zcode"):
        p = out / bad
        if p.exists():
            print(f"forbidden path present: {bad}")
            rc = 1
    for name in PRIVATE_SCRIPTS:
        if (out / "tests" / "scripts" / name).exists():
            print(f"private script present: tests/scripts/{name}")
            rc = 1
    return rc


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    src, out = Path(args.src), Path(args.out)
    export(src, out)
    scrub(out)
    rc = gate(out)
    if rc == 0:
        # The export must pass its own test suite (R6 round-2 Blocker).
        test = subprocess.run(
            ["cargo", "test", "--locked", "--release"],
            cwd=out, capture_output=True, text=True,
        )
        print("export tests:", "PASS" if test.returncode == 0 else "FAIL")
        if test.returncode != 0:
            print(test.stderr[-3000:])
            rc = 1
    files = len(subprocess.run(
        ["git", "ls-files", "--cached"], cwd=out, capture_output=True, text=True
    ).stdout.splitlines())
    print(f"exported files: {files}")
    if files < 40:
        print(f"export too small ({files} files) — expected the full source tree")
        rc = 1
    print("PUBLIC EXPORT: " + ("PASS" if rc == 0 else "FAIL"))
    return rc


if __name__ == "__main__":
    sys.exit(main())
