import json, os, re, shutil, subprocess, sys, tempfile, zipfile, hashlib, time
ROOT=os.getcwd()
UNKO=os.path.join(ROOT,'target/release/unko')
CAPTURE=os.path.join(ROOT,'tools/snes-oracle/ares-capture.sh')
MANIFEST=os.environ.get('UNKO_PAL_MANIFEST','/var/home/Lordra/.cache/nes-pal-manifest.jsonl')
LEDGER=os.path.join(ROOT,'compatibility/accuracy-ares-pal.jsonl')
FRAMES=int(os.environ.get('UNKO_ACC_FRAMES','120'))
TMP_ROOT=os.environ.get('UNKO_ACC_TMP','/var/home/Lordra/.cache/unko-nes-ares-pal')
os.makedirs(TMP_ROOT,exist_ok=True)
os.makedirs(os.path.dirname(LEDGER),exist_ok=True)
TMP=tempfile.mkdtemp(prefix='run-',dir=TMP_ROOT)

def extract(path):
    with zipfile.ZipFile(path) as z:
        names=[n for n in z.namelist() if n.lower().endswith(('.nes','.unh')) and not n.endswith('/')]
        if not names: raise RuntimeError('no .nes member')
        data=z.read(names[0])
    out=os.path.join(TMP,'game.nes'); open(out,'wb').write(data)
    return out,hashlib.sha256(data).hexdigest()

def read_ppm(path):
    b=open(path,'rb').read(); pos=0
    for _ in range(3): pos=b.index(b'\n',pos)+1
    return b[pos:]

def metrics(ours, ares):
    n=min(len(ours),len(ares))//3*3
    exact=sum(1 for i in range(0,n,3) if ours[i:i+3]==ares[i:i+3])
    # palette-normalized: map each image's RGB values to sorted canonical indices
    def seq(img):
        cols=sorted({img[i:i+3] for i in range(0,n,3)})
        m={c:i for i,c in enumerate(cols)}
        return [m[img[i:i+3]] for i in range(0,n,3)]
    a=seq(ours); b=seq(ares)
    pal=sum(x==y for x,y in zip(a,b))/max(1,len(a))
    return round(exact/(n//3)*100,4), round(pal*100,4)

def done_keys():
    paths=set(); shas=set()
    if os.path.exists(LEDGER):
        for line in open(LEDGER):
            try:
                r=json.loads(line)
                if r.get('path'): paths.add(r['path'])
                if r.get('sha256'): shas.add(r['sha256'])
            except Exception: pass
    return paths,shas

def main(limit=None):
    global TMP
    manifest=[json.loads(l) for l in open(MANIFEST)]
    done, done_shas=done_keys(); ledger=open(LEDGER,'a')
    started=time.time(); processed=ok=failed=0
    for r in manifest:
        if limit is not None and processed>=limit: break
        if r['path'] in done: continue
        processed+=1
        rec={'system':'nes','format':'Headered','title':r['title'],'path':r['path'],'oracle':'ares','region':'PAL','frames':FRAMES,'tested':'2026-11-27'}
        try:
            rom,sha=extract(r['path'])
            if sha in done_shas:
                processed-=1; continue
            rec['sha256']=sha
            ours_ppm=os.path.join(TMP,'ours.ppm'); ares_dir=os.path.join(TMP,'ares'); os.makedirs(ares_dir,exist_ok=True)
            p=subprocess.run([UNKO,'run',rom,'--frames',str(FRAMES),'--out',ours_ppm],capture_output=True,text=True)
            if p.returncode!=0: raise RuntimeError('ours: '+(p.stderr or p.stdout).strip()[:200])
            env=os.environ.copy(); env.update({'ARES_SYSTEM':'Famicom','ARES_REGION':'PAL','ARES_AUDIO_RATE':'44100','TMPDIR':TMP})
            p=subprocess.run([CAPTURE,rom,str(FRAMES),ares_dir],capture_output=True,text=True,env=env)
            if p.returncode!=0: raise RuntimeError('ares: '+(p.stderr or p.stdout).strip()[:200])
            ours=read_ppm(ours_ppm)
            ares=open(os.path.join(ares_dir,'frame.raw'),'rb').read()
            ares=bytes(b for i in range(0,len(ares),4) for b in ares[i:i+3])
            video,pal=metrics(ours,ares)
            rec.update({'video_pct':video,'palette_pct':pal,'status':'ok'}); ok+=1
        except Exception as e:
            rec['status']='error'; rec['error']=re.sub(r'/var/home/Lordra/[^: ]+','<tmp>',str(e))[:300]; failed+=1
        ledger.write(json.dumps(rec)+'\n'); ledger.flush(); os.fsync(ledger.fileno())
        shutil.rmtree(TMP,ignore_errors=True); TMP=tempfile.mkdtemp(prefix='run-',dir=TMP_ROOT)
        if processed%5==0:
            rate=processed/(time.time()-started); print(f'{processed} processed, {ok} ok, {failed} failed, {rate:.2f}/s',flush=True)
    print(f'done {processed} in {time.time()-started:.1f}s ({ok} ok, {failed} failed)',flush=True)

if __name__=='__main__':
    main(int(sys.argv[1]) if len(sys.argv)>1 else None)
