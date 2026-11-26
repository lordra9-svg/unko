import json, os, subprocess, sys, tempfile, zipfile, hashlib, time, re, glob
ROOT=os.getcwd()
INFO=os.path.join(ROOT,'target/release/examples/snes-info')
CAPTURE=os.path.join(ROOT,'tools/snes-oracle/ares-capture.sh')
FRAME_DIFF=os.path.join(ROOT,'tools/snes-oracle/diff-frame.py')
AUDIO_DIFF=os.path.join(ROOT,'tools/snes-oracle/diff-audio-raw.py')
MANIFEST=os.environ.get('UNKO_MANIFEST','/tmp/opencode/compat-manifest.jsonl')
DEFAULT_DIR=os.environ.get('UNKO_SNES_DIR','/var/home/Lordra/ai/refs/Minerva_Myrient/No-Intro/Nintendo - Super Nintendo Entertainment System')
IPL=os.environ.get('UNKO_SNES_IPL','/var/home/Lordra/ai/refs/ares/ares/System/Super Famicom/ipl.rom')
LEDGER=os.path.join(ROOT,'compatibility/accuracy-snes.jsonl')
FRAMES=int(os.environ.get('UNKO_ACC_FRAMES','120'))
TMP=tempfile.mkdtemp(prefix='unko-snes-acc-')
os.makedirs(os.path.dirname(LEDGER), exist_ok=True)

def extract_rom(path):
    with zipfile.ZipFile(path) as z:
        names=[n for n in z.namelist() if n.lower().endswith(('.sfc','.smc','.swc','.fig')) and not n.endswith('/')]
        if not names: raise RuntimeError('no SNES member')
        data=z.read(names[0])
    # Strip a 512-byte copier header when present (ROM size % 1024 == 512).
    if len(data) % 1024 == 512:
        data=data[512:]
    out=os.path.join(TMP,'game.sfc')
    open(out,'wb').write(data)
    return out, hashlib.sha256(data).hexdigest()

def parse_pct(text, pattern):
    m=re.search(pattern,text)
    return float(m.group(1)) if m else None

def done_paths():
    done=set()
    if os.path.exists(LEDGER):
        for line in open(LEDGER):
            try: done.add(json.loads(line).get('path'))
            except Exception: pass
    return done

def rows_from_manifest():
    if os.path.exists(MANIFEST):
        m=[json.loads(l) for l in open(MANIFEST)]
        return [r for r in m if r['system']=='snes']
    out=[]
    for name in os.listdir(DEFAULT_DIR):
        if name.lower().endswith('.zip'):
            out.append({'system':'snes','format':'SNES','title':os.path.splitext(name)[0],'path':os.path.join(DEFAULT_DIR,name)})
    return out

def main(limit=None):
    rows=rows_from_manifest()
    rows.sort(key=lambda r:r['title'].casefold())
    done=done_paths(); ledger=open(LEDGER,'a')
    started=time.time(); processed=0; ok=0; failed=0
    for r in rows:
        if limit is not None and processed>=limit: break
        if r['path'] in done: continue
        processed+=1
        rec={'system':'snes','format':'SNES','title':r['title'],'path':r['path'],'oracle':'ares','frames':FRAMES,'tested':'2026-11-25'}
        try:
            rom,sha=extract_rom(r['path']); rec['sha256']=sha
            work=os.path.join(TMP,'run'); os.makedirs(work,exist_ok=True)
            ours_frame=os.path.join(work,'ours.raw'); ours_audio=os.path.join(work,'ours.f32')
            env=os.environ.copy(); env.update({'FRAMES':str(FRAMES),'FRAME_OUT':ours_frame,'AUDIO_OUT':ours_audio})
            p=subprocess.run([INFO,rom,IPL],capture_output=True,text=True,env=env)
            if p.returncode!=0: raise RuntimeError('ours: '+(p.stderr or p.stdout).strip()[:250])
            region='PAL' if 'region:      PAL' in p.stdout else 'NTSC'
            rec['region']=region
            ares_dir=os.path.join(work,'ares'); os.makedirs(ares_dir,exist_ok=True)
            env=os.environ.copy(); env['ARES_RAW_AUDIO']=os.path.join(ares_dir,'raw.i16')
            p=subprocess.run([CAPTURE,rom,str(FRAMES+2),ares_dir],capture_output=True,text=True,env=env)
            if p.returncode!=0: raise RuntimeError('ares: '+(p.stderr or p.stdout).strip()[:250])
            ares_frame=os.path.join(ares_dir,'frame.raw')
            size=os.path.getsize(ours_frame)
            if size==256*224*4:
                fcmd=['python3',FRAME_DIFF,ours_frame,ares_frame]
            elif size==256*240*4:
                fcmd=['python3',FRAME_DIFF,ours_frame,ares_frame,'--ours-h','240','--ares-h','288','--x0','22','--y0','29']
            elif size==512*224*4:
                fcmd=['python3',FRAME_DIFF,ours_frame,ares_frame,'--ours-w','512','--sx','1']
            else:
                raise RuntimeError(f'unsupported frame geometry {size} bytes')
            f=subprocess.run(fcmd,capture_output=True,text=True)
            video=parse_pct(f.stdout,r'exact:\s+\d+\s+\(([0-9.]+)%\)')
            if video is None: raise RuntimeError('frame diff: '+(f.stdout+f.stderr).strip()[:250])
            a=subprocess.run(['python3',AUDIO_DIFF,ours_audio,env['ARES_RAW_AUDIO']],capture_output=True,text=True)
            audio=parse_pct(a.stdout,r'([0-9.]+)% identical')
            rec.update({'video_pct':video,'audio_pct':audio,'status':'ok'}); ok+=1
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
