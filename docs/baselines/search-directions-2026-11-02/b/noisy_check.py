#!/usr/bin/env python3
import json, sys
from pathlib import Path
ROOT=Path('/tmp/kg-lab/b')
def score(runid):
    task=runid.rsplit('-',1)[-1]
    exp=json.loads((ROOT/'expected'/f'{task}.json').read_text())
    f=ROOT/'answers'/f'{runid}.json'
    if not f.exists(): return (False,'no answer file')
    try: a=json.loads(f.read_text())
    except Exception as e: return (False,f'invalid JSON: {e}')
    if a.get('exists') is not True: return (False,'exists != true')
    if a.get('hops') != 0: return (False,f"hops {a.get('hops')} != 0")
    if a.get('qualname') != exp['qualname']:
        return (False,f"qualname {a.get('qualname')} != {exp['qualname']}")
    route=[str(x) for x in (a.get('route') or [])]
    if route != exp['route']:
        return (False,f"route {route} != {exp['route']}")
    return (True,'')
if __name__=='__main__':
    ok=0
    for rid in sys.argv[1:]:
        good,why=score(rid); ok+=good
        print(f'| {rid} | {"OK" if good else "FAIL"} | {why} |')
    print(f'\ncorrect cells: {ok}/{len(sys.argv)-1}')
    raise SystemExit(0 if ok==len(sys.argv)-1 else 1)
