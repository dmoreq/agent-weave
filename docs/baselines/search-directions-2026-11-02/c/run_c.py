#!/usr/bin/env python3
"""B: noisy-corpus search A/B, 2 tasks x 2 arms x 2 hosts x 2 runs."""
from __future__ import annotations
import json, os, re, shutil, signal, subprocess, sys, time
from pathlib import Path

LAB = Path('/tmp/kg-lab')
ROOT = LAB / 'c'
KG = str(LAB / 'target-map/release/kg')
AGY = str(Path.home() / '.local/bin/agy')
ZCODE = '/opt/homebrew/bin/zcode'
CORPUS = LAB / 'corpora/vihap'
ANSWERS = ROOT / 'answers'
STATUS = ROOT / 'status.txt'
METRICS = ROOT / 'metrics.jsonl'
MANIFEST = ROOT / 'manifest.txt'
KILL = 600
SUBS = ('path','callers','callees','context','impact','dead','unresolved','symbols','search','clones','stats','hotspots','report','usage','build','doctor','map')
KG_RE = re.compile(r'(?:target/release/)?\bkg\s+(?:--[a-z-]+\s+\S+\s+)*(' + '|'.join(SUBS) + r')\b')

def cells():
    out=[]
    for host in ('agy','zcode'):
        for task in ('C1','C2'):
            order=[('kgc',1),('kgm',1),('kgm',2),('kgc',2)]
            for arm,run in order:
                out.append((host,arm,task,run))
    return out

def parse_host(host, log):
    out={'tools':0,'model_calls':0,'tok_in':0,'tok_out':0,'cache_read':0,'model':None,'session':None}
    if not log.exists(): return out
    for line in log.read_text(errors='replace').splitlines():
        try: d=json.loads(line)
        except json.JSONDecodeError: continue
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
                out['tok_in']=use.get('input_tokens',0); out['tok_out']=use.get('output_tokens',0)
                out['cache_read']=use.get('cache_read_tokens',0)
        else:
            t=d.get('type')
            if t=='system' and d.get('subtype')=='init':
                out['model']=d.get('model'); out['session']=d.get('session_id')
            elif t=='assistant':
                for part in (d.get('message') or {}).get('content') or []:
                    if part.get('type')=='tool_use': out['tools']+=1
            elif t=='result':
                use=d.get('usage') or {}
                out['session']=d.get('session_id') or out['session']
                out['tok_in']=use.get('input_tokens',0); out['tok_out']=use.get('output_tokens',0)
                out['cache_read']=use.get('cache_read_input_tokens',0)
                out['model_calls']=d.get('num_turns',0)
    return out

def cmds(host, log):
    out=[]
    if not log.exists(): return out
    for line in log.read_text(errors='replace').splitlines():
        try: d=json.loads(line)
        except json.JSONDecodeError: continue
        if host=='agy':
            if d.get('event')=='step_update':
                u=d.get('step_update') or {}
                if u.get('tool_name')=='run_command' and u.get('state')=='DONE':
                    c=(u.get('tool_info') or {}).get('parameters',{}).get('CommandLine')
                    if isinstance(c,str): out.append(c)
        else:
            if d.get('type')=='assistant':
                for part in (d.get('message') or {}).get('content') or []:
                    if part.get('type')=='tool_use' and part.get('name')=='Bash':
                        c=(part.get('input') or {}).get('command')
                        if isinstance(c,str): out.append(c)
    return out

def kg_counts(host, log):
    n=0; tools={}; sqlite=0
    for c in cmds(host,log):
        if 'sqlite3' in c or '.kg/index.db' in c: sqlite+=1
        for m in KG_RE.finditer(c):
            n+=1; tools[m.group(1)]=tools.get(m.group(1),0)+1
    return n,tools,sqlite

def run_cell(host,arm,task,run):
    rid=f'{host}-{arm}-r{run}-{task}'
    if STATUS.exists() and f'[done] {rid} ' in STATUS.read_text(): return
    env=dict(os.environ)
    env.pop('KG_ARM',None); env.pop('KG_SESSION',None)
    subprocess.run([KG,'build','--root',str(CORPUS)],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=120)
    for f in (CORPUS/'.kg').glob('usage.log*'): f.unlink(missing_ok=True)
    for f in (CORPUS/'.kg').glob('usage-results.jsonl*'): f.unlink(missing_ok=True)
    ANSWERS.mkdir(exist_ok=True)
    answer=ANSWERS/f'{rid}.json'; answer.unlink(missing_ok=True)
    prompt=(ROOT/'prompts'/f'{arm}-{task}.txt').read_text().replace('{ANSWER_PATH}',str(answer))
    log=ROOT/'logs'/f'{rid}.jsonl'; err=ROOT/'logs'/f'{rid}.err'
    if host=='agy':
        cmd=[AGY,'--output-format','stream-json','--dangerously-skip-permissions','--model','gemini-3.8-flash-high','--print-timeout',f'{KILL}s',f'-p={prompt}']
    else:
        cmd=[ZCODE,'--permission-mode','yolo','--output-format','stream-json','--verbose','-p',prompt]
    env['PATH']=str(LAB/'target-map/release')+':'+env.get('PATH','')
    with STATUS.open('a') as st: st.write(f'[start] {rid} {time.strftime("%H:%M:%S")}\n')
    t0=time.time()
    with log.open('wb') as o, err.open('wb') as e:
        p=subprocess.Popen(cmd,cwd=CORPUS,env=env,stdout=o,stderr=e,start_new_session=True)
        try: rc=p.wait(timeout=KILL)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid,signal.SIGKILL); rc=-9
    dur=round(time.time()-t0,1)
    if (CORPUS/'.kg/usage.log').exists(): shutil.move(str(CORPUS/'.kg/usage.log'),ROOT/'logs'/f'{rid}.usage.log')
    if answer.exists():
        pass  # answer already lives in ROOT/answers
    parsed=parse_host(host,log); kn,kt,sql=kg_counts(host,log)
    rec={'runid':rid,'host':host,'arm':arm,'task':task,'exit':rc,'duration_s':dur,**parsed,
         'kg_calls':kn,'kg_tools':kt,'feature_calls':kt.get('map',0),'direct_sqlite':sql,
         'answered':answer.exists()}
    rec['violations']=[]
    if arm=='kgc' and rec['feature_calls']>0: rec['violations'].append('control called map')
    if arm=='kgm' and rec['feature_calls']==0: rec['violations'].append('treatment did not call map')
    if rec['tools']==0: rec['violations'].append('zero tool calls')
    if rec['tools']>25: rec['violations'].append(f'budget {rec["tools"]}>25')
    if sql: rec['violations'].append('direct sqlite')
    with METRICS.open('a') as m: m.write(json.dumps(rec)+'\n')
    with MANIFEST.open('a') as m: m.write(f'{rid}|{host}|{arm}|{task}|{parsed.get("session") or ""}\n')
    with STATUS.open('a') as st: st.write(f'[done] {rid} exit={rc} tools={rec["tools"]} kg={kn} feat={rec["feature_calls"]} {time.strftime("%H:%M:%S")}\n')
    print(f'[done] {rid} exit={rc} tools={rec["tools"]} kg={kn} feat={rec["feature_calls"]}',flush=True)

def main():
    if len(sys.argv)>1 and sys.argv[1]=='--cell':
        run_cell(*sys.argv[2:6]); return 0
    for host,arm,task,run in cells():
        rid=f'{host}-{arm}-r{run}-{task}'
        if STATUS.exists() and f'[done] {rid} ' in STATUS.read_text(): continue
        print(f'=== {rid}',flush=True); run_cell(host,arm,task,run)
    print('=== B done',flush=True); return 0

if __name__=='__main__': raise SystemExit(main())
