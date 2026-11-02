#!/usr/bin/env python3
import json, sys
from pathlib import Path
ROOT=Path('/tmp/kg-lab/c')
def score(runid):
    task=runid.rsplit('-',1)[-1]
    exp=json.loads((ROOT/'expected'/f'{task}.json').read_text())
    f=ROOT/'answers'/f'{runid}.json'
    if not f.exists(): return (False,'no answer file')
    try: a=json.loads(f.read_text())
    except Exception as e: return (False,f'invalid JSON: {e}')
    got_entry=str(a.get('entry') or '')
    if got_entry not in exp['entries']:
        return (False,f"entry {got_entry} not in {exp['entries']}")
    got=set(str(x) for x in (a.get('files') or []))
    want=set(exp['files'])
    missing=want-got
    if missing:
        return (False,f"missing files: {sorted(missing)}")
    if len(got) > len(want) + 2:
        return (False,f"too many files ({len(got)} > {len(want)}+2)")
    return (True,f"all {len(want)} files + entry")
if __name__=='__main__':
    ok=0
    for rid in sys.argv[1:]:
        good,why=score(rid); ok+=good
        print(f'| {rid} | {"OK" if good else "FAIL"} | {why} |')
    print(f'\ncorrect cells: {ok}/{len(sys.argv)-1}')
    raise SystemExit(0 if ok==len(sys.argv)-1 else 1)
