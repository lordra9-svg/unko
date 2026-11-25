import json, os, subprocess, sys, tempfile, zipfile, hashlib, time, re
ROOT=os.getcwd()
UNKO=os.path.join(ROOT,'target/release/unko')
CAPTURE=os.path.join(ROOT,'tools/snes-oracle/ares-capture.sh')
MANIFEST=os.environ.get('UNKO_MANIFEST','/tmp/opencode/compat-manifest.jsonl')
DEFAULT_DIR=os.environ.get('UNKO_NES_DIR','/var/home/Lordra/ai/refs/Minerva_Myrient/No-Intro/Nintendo - Nintendo Entertainment System (Headered)')
LEDGER=os.path.join(ROOT,'compatibility/accuracy-ares.jsonl')
FRAMES=int(os.environ.get('UNKO_ACC_FRAMES','120'))
TMP=tempfile.mkdtemp(prefix='unko-ares-acc-')
os.makedirs(os.path.dirname(LEDGER), exist_ok=True)

def parse_trace(path):
    frames=[]
    for line in open(path, errors='replace'):
        p=line.split()
        if p and p[0]=='frame':
            frames.append(tuple(int(x,16) for x in p[2:5]))
    return frames

def extract_nes(path):
    with zipfile.ZipFile(path) as z:
        names=[n for n in z.namelist() if n.lower().endswith(('.nes','.unh')) and not n.endswith('/')]
        if not names: raise RuntimeError('no .nes member')
        data=z.read(names[0])
    out=os.path.join(TMP,'game.nes')
    open(out,'wb').write(data)
    return out, hashlib.sha256(data).hexdigest()

def best_alignment(ours, ref):
    n=min(len(ours),len(ref))
    if not n: return None
    best=(0.0,0)
    for shift in range(-3,4):
        vals=[]
        for i in range(n):
            j=i+shift
            if 0<=j<n: vals.append(ours[i][2]==ref[j][2])
        if vals:
            v=sum(vals)/len(vals)
            if v>best[0]: best=(v,shift)
    return best

def metrics(ours, ref):
    n=min(len(ours),len(ref))
    if not n: return None
    same=sum(ours[i][2]==ref[i][2] for i in range(n))/n
    best,shift=best_alignment(ours,ref)
    start=30
    vals=[]
    for i in range(start,n):
        j=i+shift
        if 0<=j<n: vals.append(ours[i][2]==ref[j][2])
    steady=sum(vals)/len(vals) if vals else None
    return {
        'frames':n,
        'geometry_pct':round(same*100,4),
        'best_geometry_pct':round(best*100,4),
        'best_shift':shift,
        'steady_geometry_pct':round(steady*100,4) if steady is not None else None,
    }

def done_paths():
    done=set()
    if os.path.exists(LEDGER):
        for line in open(LEDGER):
            try: done.add(json.loads(line).get('path'))
            except Exception: pass
    return done

def main(limit=None):
    if os.path.exists(MANIFEST):
        manifest=[json.loads(l) for l in open(MANIFEST)]
        rows=[r for r in manifest if r['system']=='nes' and r['format']=='Headered']
    else:
        rows=[]
        for name in os.listdir(DEFAULT_DIR):
            if name.lower().endswith('.zip'):
                rows.append({'system':'nes','format':'Headered','title':os.path.splitext(name)[0],'path':os.path.join(DEFAULT_DIR,name)})
    rows.sort(key=lambda r:r['title'].casefold())
    done=done_paths()
    ledger=open(LEDGER,'a')
    started=time.time(); processed=0; ok=0; failed=0
    for r in rows:
        if limit is not None and processed>=limit: break
        if r['path'] in done: continue
        processed+=1
        rec={'system':'nes','format':'Headered','title':r['title'],'path':r['path'],'oracle':'ares','frames':FRAMES,'tested':'2026-11-25'}
        try:
            rom,sha=extract_nes(r['path'])
            rec['sha256']=sha
            ours=os.path.join(TMP,'ours.trace'); refdir=os.path.join(TMP,'ares')
            os.makedirs(refdir,exist_ok=True)
            p=subprocess.run([UNKO,'trace',rom,'--frames',str(FRAMES),'--out',ours],capture_output=True,text=True)
            if p.returncode!=0: raise RuntimeError('ours: '+(p.stderr or p.stdout).strip()[:200])
            env=os.environ.copy()
            env['ARES_SYSTEM']='Famicom'
            env['ARES_AUDIO_RATE']='44100'
            p=subprocess.run([CAPTURE,rom,str(FRAMES),refdir],capture_output=True,text=True,env=env)
            if p.returncode!=0: raise RuntimeError('ares: '+(p.stderr or p.stdout).strip()[:200])
            m=metrics(parse_trace(ours), parse_trace(os.path.join(refdir,'trace.txt')))
            if not m: raise RuntimeError('empty trace')
            rec.update(m); rec['status']='ok'; ok+=1
        except Exception as e:
            rec['status']='error'; rec['error']=re.sub(r'/tmp/[^: ]+','<tmp>',str(e))[:300]; failed+=1
        ledger.write(json.dumps(rec)+'\n'); ledger.flush(); os.fsync(ledger.fileno())
        if processed%5==0:
            rate=processed/(time.time()-started)
            print(f'{processed} processed, {ok} ok, {failed} failed, {rate:.2f}/s', flush=True)
    print(f'done {processed} in {time.time()-started:.1f}s ({ok} ok, {failed} failed)', flush=True)

if __name__=='__main__':
    limit=int(sys.argv[1]) if len(sys.argv)>1 else None
    main(limit)
