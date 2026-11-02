#!/usr/bin/env python3
"""Feature A/B: callers/callees/impact/outline on agy+zcode, n=2."""
from __future__ import annotations
import json, os, re, signal, subprocess, sys, time
from pathlib import Path

LAB=Path('/tmp/kg-lab'); ROOT=LAB/'r'; KG=str(LAB/'target-base/release/kg')
AGY=str(Path.home()/'.local/bin/agy'); ZCODE='/opt/homebrew/bin/zcode'
CORPUS=LAB/'corpora/vihap'; ANSWERS=ROOT/'answers'
STATUS=ROOT/'status.txt'; METRICS=ROOT/'metrics.jsonl'; MANIFEST=ROOT/'manifest.txt'
KILL=600
TASKS=('R1','R2','R3','R4')
def cells():
    out=[]
    for host in ('agy','zcode'):
        for task in TASKS:
            for arm,run in (('kgc',1),('kgt',1),('kgt',2),('kgc',2)):
                out.append((host,arm,task,run))
    return out

def parse_host(host,log):
    out={'tools':0,'model_calls':0,'tok_in':0,'tok_out':0,'cache_read':0,'model':None,'session':None}
    if not log.exists(): return out
    for line in log.read_text(errors='replace').splitlines():
        try: d=json.loads(line)
        except Exception: continue
        if host=='agy':
            ev=d.get('event')
            if ev=='init': out['model']=(d.get('init') or {}).get('model'); out['session']=d.get('conversation_id')
            elif ev=='step_update':
                u=d.get('step_update') or {}
                if u.get('step_type')=='tool' and u.get('state')=='DONE': out['tools']+=1
                if u.get('usage'): out['model_calls']+=1
            elif ev=='result':
                r=d.get('result') or {}; use=r.get('usage') or {}
                out['session']=r.get('conversation_id') or out['session']
                out['tok_in']=use.get('input_tokens',0); out['tok_out']=use.get('output_tokens',0); out['cache_read']=use.get('cache_read_tokens',0)
        else:
            t=d.get('type')
            if t=='system' and d.get('subtype')=='init': out['model']=d.get('model'); out['session']=d.get('session_id')
            elif t=='assistant':
                for p in (d.get('message') or {}).get('content') or []:
                    if p.get('type')=='tool_use': out['tools']+=1
            elif t=='result':
                use=d.get('usage') or {}
                out['session']=d.get('session_id') or out['session']
                out['tok_in']=use.get('input_tokens',0); out['tok_out']=use.get('output_tokens',0); out['cache_read']=use.get('cache_read_input_tokens',0)
                out['model_calls']=d.get('num_turns',0)
    return out


def cmds(host,log):
    out=[]
    if not log.exists(): return out
    for line in log.read_text(errors='replace').splitlines():
        try: d=json.loads(line)
        except Exception: continue
        if host=='agy':
            if d.get('event')=='step_update':
                u=d.get('step_update') or {}
                if u.get('tool_name')=='run_command' and u.get('state')=='DONE':
                    c=(u.get('tool_info') or {}).get('parameters',{}).get('CommandLine')
                    if isinstance(c,str): out.append(c)
        else:
            if d.get('type')=='assistant':
                for p in (d.get('message') or {}).get('content') or []:
                    if p.get('type')=='tool_use' and p.get('name')=='Bash':
                        c=(p.get('input') or {}).get('command')
                        if isinstance(c,str): out.append(c)
    return out

def cmd_feature(host,log,task):
    n=0
    for c in cmds(host,log):
        if 'kg' not in c: continue
        if task in ('R1','R2','R3'):
            tool={'R1':'callers','R2':'callees','R3':'impact'}[task]
            if re.search(r'\bkg\s+(?:--[a-z-]+\s+\S+\s+)*'+tool+r'\b',c): n+=1
        else:
            if re.search(r'\bkg\s+(?:--[a-z-]+\s+\S+\s+)*symbols\b',c) and '--outline' in c: n+=1
    return n

def journal(rid):
    recs=[]
    log=ROOT/'logs'/f'{rid}.usage.log'
    if log.exists():
        for line in log.read_text().splitlines():
            try: recs.append(json.loads(line))
            except Exception: pass
    return recs

def feature_count(task,recs):
    if task=='R1': return sum(1 for r in recs if r.get('tool')=='callers')
    if task=='R2': return sum(1 for r in recs if r.get('tool')=='callees')
    if task=='R3': return sum(1 for r in recs if r.get('tool')=='impact')
    return sum(1 for r in recs if r.get('tool')=='symbols' and str((r.get('args') or {}).get('outline','')).lower()=='true')

def run_cell(host,arm,task,run):
    rid=f'{host}-{arm}-r{run}-{task}'
    if STATUS.exists() and f'[done] {rid} ' in STATUS.read_text(): return
    env=dict(os.environ); env.pop('KG_ARM',None); env.pop('KG_SESSION',None)
    subprocess.run([KG,'build','--root',str(CORPUS)],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=120)
    for f in (CORPUS/'.kg').glob('usage.log*'): f.unlink(missing_ok=True)
    for f in (CORPUS/'.kg').glob('usage-results.jsonl*'): f.unlink(missing_ok=True)
    ANSWERS.mkdir(exist_ok=True); answer=ANSWERS/f'{rid}.json'; answer.unlink(missing_ok=True)
    prompt=(ROOT/'prompts'/f'{arm}-{task}.txt').read_text().replace('{ANSWER_PATH}',str(answer))
    log=ROOT/'logs'/f'{rid}.jsonl'; err=ROOT/'logs'/f'{rid}.err'
    if host=='agy':
        cmd=[AGY,'--output-format','stream-json','--dangerously-skip-permissions','--model','gemini-3.8-flash-high','--print-timeout',f'{KILL}s',f'-p={prompt}']
    else:
        cmd=[ZCODE,'--permission-mode','yolo','--output-format','stream-json','--verbose','-p',prompt]
    env['PATH']=str(LAB/'target-base/release')+':'+env.get('PATH','')
    with STATUS.open('a') as st: st.write(f'[start] {rid} {time.strftime("%H:%M:%S")}\n')
    t0=time.time()
    with log.open('wb') as o, err.open('wb') as e:
        p=subprocess.Popen(cmd,cwd=CORPUS,env=env,stdout=o,stderr=e,start_new_session=True)
        try: rc=p.wait(timeout=KILL)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid,signal.SIGKILL); rc=-9
    dur=round(time.time()-t0,1)
    recs=journal(rid)
    parsed=parse_host(host,log)
    feat=max(feature_count(task,recs), cmd_feature(host,log,task))
    kgn=max(len(recs), len([c for c in cmds(host,log) if 'kg' in c]))
    rec={'runid':rid,'host':host,'arm':arm,'task':task,'exit':rc,'duration_s':dur,
         **parsed,'kg_calls':kgn,'kg_tools':{},
         'feature_calls':feat,'answered':answer.exists()}
    for r in recs: rec['kg_tools'][r.get('tool','?')]=rec['kg_tools'].get(r.get('tool','?'),0)+1
    rec['violations']=[]
    if arm=='kgc' and feat>0: rec['violations'].append('control used feature')
    if arm=='kgt' and feat==0: rec['violations'].append('treatment did not use feature')
    if rec['tools']==0: rec['violations'].append('zero tool calls')
    if rec['tools']>25: rec['violations'].append(f'budget {rec["tools"]}>25')
    with METRICS.open('a') as m: m.write(json.dumps(rec)+'\n')
    with MANIFEST.open('a') as m: m.write(f'{rid}|{host}|{arm}|{task}|{parsed.get("session") or ""}\n')
    with STATUS.open('a') as st: st.write(f'[done] {rid} exit={rc} tools={rec["tools"]} kg={kgn} feat={feat} {time.strftime("%H:%M:%S")}\n')
    print(f'[done] {rid} exit={rc} tools={rec["tools"]} kg={kgn} feat={feat}',flush=True)

def main():
    if len(sys.argv)>1 and sys.argv[1]=='--cell':
        run_cell(*sys.argv[2:6]); return 0
    for host,arm,task,run in cells():
        rid=f'{host}-{arm}-r{run}-{task}'
        if STATUS.exists() and f'[done] {rid} ' in STATUS.read_text(): continue
        print(f'=== {rid}',flush=True); run_cell(host,arm,task,run)
    print('=== R done',flush=True); return 0

if __name__=='__main__': raise SystemExit(main())
