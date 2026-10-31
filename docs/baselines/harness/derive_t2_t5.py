#!/usr/bin/env python3
"""Freeze expected sets for T2-T5 from recorded transcript outputs and write
the baseline-arm answers (T1-T5).

Truth derivation is audited from transcripts/out/T<n>-*.txt at vihap f7ca48c.
The baseline answers equal the frozen truth because the same recorded grep arm
produced them (self-derived by construction) — stated in SUMMARY.md, flagged
for R0 review.

Usage rules per task (mechanical, no hand-picking):
  T2: lines matching `get_knowledge_base(` in T2-001.txt, minus `def ...`.
  T3: lines matching `make_alternation(` in T3-001.txt, minus `def ...`.
  T4: fixed pointer chain + essence ordering (audited T4-001..003).
  T5: defining lines of one-hop cross-module callees (audited T5-001).
"""
import json
import os
import re

BASE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
TR = os.path.join(BASE, "transcripts")
EXP = os.path.join(BASE, "expected")
CALL_RE = re.compile(r"^./([^:]+):(\d+):(.*)$")


def call_lines(out_file, callee):
    ptrs = []
    with open(os.path.join(TR, "out", out_file)) as f:
        for line in f:
            m = CALL_RE.match(line.rstrip("\n"))
            if not m:
                continue
            path, lineno, content = m.groups()
            if f"{callee}(" not in content:
                continue
            if content.lstrip().startswith(("def ", "class ")):
                continue
            ptrs.append(f"{path}:{lineno}")
    return ptrs


def write(task, data):
    os.makedirs(EXP, exist_ok=True)
    with open(os.path.join(EXP, f"{task}.json"), "w") as f:
        json.dump(data, f, indent=1, ensure_ascii=False)
    ans = dict(data)
    ans["baseline_answer"] = True
    ans["note"] = ("Baseline arm answer; equals frozen truth by construction "
                   "(same recorded grep arm). See SUMMARY.md, R0 flag.")
    with open(os.path.join(TR, f"{task}.answer.json"), "w") as f:
        json.dump(ans, f, indent=1, ensure_ascii=False)
    print(f"{task}: {len(data['pointers'])} pointers")


def main():
    t2_ptrs = call_lines("T2-001.txt", "get_knowledge_base")
    write("T2", {
        "task": "T2",
        "symbol": "vihap.knowledge.registry.get_knowledge_base",
        "provenance": "transcripts/out/T2-001.txt + T2-002.txt at vihap f7ca48c",
        "pointers": t2_ptrs,
        "ordering": [
            "vihap/validators/gso_loader.py (admin-node resolution, GsoIndex build)",
            "vihap/validators/street_validator.py (street validation, local import)",
            "vihap/reform/mapping.py (reform mapping)",
            "vihap/reform/kb_loader.py (deprecated wrapper, still live callers)",
            "vihap/knowledge/reform_utils.py (reform utilities)",
            "vihap/knowledge/access.py (lazy access wrapper)",
            "scripts/extract_poi_knowledge.py (offline, outside parse request)",
        ],
    })

    t3_ptrs = call_lines("T3-001.txt", "make_alternation")
    write("T3", {
        "task": "T3",
        "symbol": "vihap.grammar.fragments.make_alternation",
        "provenance": "transcripts/out/T3-001.txt at vihap f7ca48c; call sites only "
        "(imports unaffected by a signature change)",
        "pointers": t3_ptrs,
    })

    write("T4", {
        "task": "T4",
        "provenance": "transcripts/out/T4-001..003 at vihap f7ca48c; ordering is the "
        "tasks.md expected essence",
        "pointers": [
            "vihap/pipeline.py:40",
            "vihap/parsers/hierarchical_parser.py:86",
            "vihap/parsers/hierarchical_parser.py:102",
            "vihap/parsers/hierarchical_parser.py:121",
            "vihap/parsers/hierarchical_parser.py:137",
            "vihap/parsers/hierarchical_parser.py:151",
            "vihap/parsers/hierarchical_parser.py:179",
            "vihap/parsers/hierarchical_parser.py:197",
            "vihap/parsers/hierarchical_parser.py:426",
            "vihap/parsers/hierarchical_parser.py:427",
        ],
        "ordering": [
            "cleaners",
            "parsers (HierarchicalAddressParser + grammar)",
            "validators (GsoIndex)",
            "quality (QualityScorer)",
            "formatters (AddressFormatter)",
            "models.ParsedAddress",
        ],
    })

    write("T5", {
        "task": "T5",
        "symbol": "vihap.parsers.hierarchical_parser.HierarchicalAddressParser.parse",
        "provenance": "transcripts/out/T5-001.txt + T4-002/003 at vihap f7ca48c",
        "pointers": [
            "vihap/parsers/macro_resolver.py:303",
            "vihap/parsers/macro_resolver.py:383",
            "vihap/parsers/macro_resolver.py:398",
            "vihap/formatters/standard_formatter.py:154",
            "vihap/quality/confidence.py:145",
        ],
        "correction": "tasks.md T5 essence listed grammar/, validators/, knowledge/ as "
        "one-hop callees; measured one-hop cross-module callees are macro_resolver "
        "(same dir, cross-module) + formatter.format and quality score (one hop via "
        "_assemble_parsed_address). grammar/validators/knowledge are >= 2 hops. "
        "Scoring uses this frozen set, not the prose essence.",
    })
    return 0


if __name__ == "__main__":
    main()
