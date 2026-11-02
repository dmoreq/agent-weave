#!/usr/bin/env python3
import json,re,sys
from pathlib import Path
ROOT=Path('/tmp/kg-lab/r')
def norm(s): return ' '.join(str(s).split())
def parse(task,items):
    out=[]
    for it in items:
        it=norm(it)
        if task in ('R1','R2'):
            m=re.match(r'^(\S+):(\d+) (\S+)$',it)
            if not m: return None
            out.append((m.group(1),int(m.group(2)),m.group(3)))
        elif task=='R3':
            m=re.match(r'^(\S+) \((\d+) sites\)$',it)
            if not m: return None
            out.append((m.group(1),int(m.group(2))))
        else:
            m=re.match(r'^(\S+):(\d+) (\S+) (\S+) (.*)$',it)
            if not m: return None
            sig=norm(m.group(5))
            if sig.startswith('def '): sig=sig[4:]
            out.append((m.group(1),int(m.group(2)),m.group(3),m.group(4),sig))
    return out
def score(runid):
    task=runid.rsplit('-',1)[-1]
    exp=json.loads((ROOT/'expected'/f'{task}.json').read_text())['items']
    f=ROOT/'answers'/f'{runid}.json'
    if not f.exists(): return (False,'no answer file')
    try: a=json.loads(f.read_text())
    except Exception as e: return (False,f'invalid JSON: {e}')
    got=parse(task,a.get('items') or [])
    if got is None: return (False,'unparseable items')
    want=parse(task,exp)
    if set(got)!=set(want):
        miss=[x for x in want if x not in got]; extra=[x for x in got if x not in want]
        return (False,f'missing={len(miss)} extra={len(extra)}; first_missing={miss[:1]} first_extra={extra[:1]}')
    return (True,f'{len(want)} items')
if __name__=='__main__':
    ok=0
    for rid in sys.argv[1:]:
        good,why=score(rid); ok+=good
        print(f'| {rid} | {"OK" if good else "FAIL"} | {why} |')
    print(f'\ncorrect cells: {ok}/{len(sys.argv)-1}')
    raise SystemExit(0 if ok==len(sys.argv)-1 else 1)
