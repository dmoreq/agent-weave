#!/usr/bin/env python3
"""B on Grok Build + OpenCode: 2 tasks x 2 arms x 2 hosts x 2 runs."""
from __future__ import annotations
import json, os, re, shutil, signal, sqlite3, subprocess, sys, time, uuid
from pathlib import Path

LAB=Path('/tmp/kg-lab'); ROOT=LAB/'b'; KG=str(LAB/'target-base/release/kg')
GROK=str(Path.home()/'.grok/bin/grok'); OC=str(Path.home()/'.opencode/bin/opencode')
CORPUS=LAB/'corpora/noisy'; ANSWERS=ROOT/'answers'
STATUS=ROOT/'status-goc.txt'; METRICS=ROOT/'metrics-goc.jsonl'; MANIFEST=ROOT/'manifest-goc.txt'
OC_DB=Path.home()/'.local/share/opencode/opencode.db'; KILL=600
SUBS=('path','callers','callees','context','impact','dead','unresolved','symbols','search','clones','stats','hotspots','report','usage','build','doctor','map')
KG_RE=re.compile(r'(?:target/release/)?\bkg\s+(?:--[a-z-]+\s+\S+\s+)*('+'|'.join(SUBS)+r')\b')

def cells():
    out=[]
    for host in ('grok','opencode'):
        for task in ('B1','B2'):
            for arm,run in (('kgc',1),('kgs',1),('kgs',2),('kgc',2)):
                out.append((host,arm,task,run))
    return out

def grok_usage(sid):
    out={'tools':0,'model_calls':0,'tok_in':0,'tok_out':0,'cache_read':0,'model':None,'session':sid}
    for d in Path.home().glob(f'.grok/sessions/*/{sid}'):
        u=d/'usage.json'
        if u.exists():
            s=json.loads(u.read_text()).get('session',{})
            out['tok_in']=s.get('inputTokens',0); out['tok_out']=s.get('outputTokens',0)
            out['model']=s.get('primaryModelId'); out['model_calls']=s.get('modelCalls',0)
        h=d/'chat_history.jsonl'
        if h.exists():
            out['tools']=sum(1 for l in h.read_text(errors='replace').splitlines() if '"type":"tool_result"' in l or '"type": "tool_result"' in l)
    return out

def oc_usage(sid):
    out={'tools':0,'model_calls':0,'tok_in':0,'tok_out':0,'cache_read':0,'model':None,'session':sid}
    if not OC_DB.exists(): return out
    c=sqlite3.connect(f'file:{OC_DB}?mode=ro',uri=True)
    try:
        rows=c.execute("SELECT data FROM session_message WHERE session_id=? AND type='assistant'",(sid,)).fetchall()
    finally: c.close()
    for (data,) in rows:
        try: d=json.loads(data)
        except Exception: continue
        out['model_calls']+=1
        t=d.get('tokens') or {}
        out['tok_in']+=t.get('input',0); out['tok_out']+=t.get('output',0)
        out['cache_read']+=t.get('cacheRead',0) or t.get('cache_read',0) or 0
        m=d.get('model') or {}; out['model']=out['model'] or m.get('id')
        for p in d.get('content') or []:
            if p.get('type')=='tool': out['tools']+=1
    return out

def journal(runid):
    n=0; tools={}
    log=ROOT/'logs'/f'{runid}.usage.log'
    if not log.exists(): return n,tools
    for line in log.read_text().splitlines():
        try: r=json.loads(line)
        except Exception: continue
        n+=1; t=r.get('tool','?'); tools[t]=tools.get(t,0)+1
    return n,tools

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
    env['KG_SESSION']=rid; env['KG_ARM']=rid
    env['PATH']=str(LAB/'target-base/release')+':'+env.get('PATH','')
    sid=str(uuid.uuid4()) if host=='grok' else f'ses_goc_{arm}_{run}_{task}'
    tmp=None
    if host=='grok':
        tmp=ROOT/'tmp'; tmp.mkdir(exist_ok=True); pf=tmp/f'{rid}.prompt'; pf.write_text(prompt)
        cmd=[GROK,'--always-approve','--max-turns','40','--session-id',sid,'--prompt-file',str(pf)]
    else:
        cmd=[OC,'run','--auto','-m','openai/gpt-6-sol','--format','json','--session',sid,prompt]
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
        pass  # answer is already in ROOT/answers
    parsed=grok_usage(sid) if host=='grok' else oc_usage(sid)
    kn,kt=journal(rid)
    rec={'runid':rid,'host':host,'arm':arm,'task':task,'exit':rc,'duration_s':dur,**parsed,
         'kg_calls':kn,'kg_tools':kt,'feature_calls':kt.get('search',0),'answered':answer.exists()}
    rec['violations']=[]
    if arm=='kgc' and rec['feature_calls']>0: rec['violations'].append('control called search')
    if arm=='kgs' and rec['feature_calls']==0: rec['violations'].append('treatment did not call search')
    if rec['tools']==0: rec['violations'].append('zero tool calls')
    if rec['tools']>25: rec['violations'].append(f'budget {rec["tools"]}>25')
    with METRICS.open('a') as m: m.write(json.dumps(rec)+'\n')
    with MANIFEST.open('a') as m: m.write(f'{rid}|{host}|{arm}|{task}|{sid}\n')
    with STATUS.open('a') as st: st.write(f'[done] {rid} exit={rc} tools={rec["tools"]} kg={kn} feat={rec["feature_calls"]} {time.strftime("%H:%M:%S")}\n')
    print(f'[done] {rid} exit={rc} tools={rec["tools"]} kg={kn} feat={rec["feature_calls"]}',flush=True)

def main():
    if len(sys.argv)>1 and sys.argv[1]=='--cell':
        run_cell(*sys.argv[2:6]); return 0
    for host,arm,task,run in cells():
        rid=f'{host}-{arm}-r{run}-{task}'
        if STATUS.exists() and f'[done] {rid} ' in STATUS.read_text(): continue
        print(f'=== {rid}',flush=True); run_cell(host,arm,task,run)
    print('=== GOC done',flush=True); return 0

if __name__=='__main__': raise SystemExit(main())
