"""Run missing canonical signed-history costs after prior timing/verification ends."""
import gzip,hashlib,json,os,subprocess,time
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;m=dict(pid=os.getpid(),status='waiting_for_prior_campaigns',checks=[])
def save():(out/'after-history.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
    while True:
        prior=json.loads((out/'queued-verification.json').read_text())
        if prior['status'] not in ['waiting_for_complete_performance','running']:
            assert prior['status']=='passed',prior
            break
        assert Path('/proc/'+str(prior['pid'])).exists(),'prior verification orchestrator stopped'
        time.sleep(15)
    m['status']='running';save()
    for label,args in [('history-build',[]),('history-measure',['--measure'])]:
        command=['python3',str(out/'canonical_history.py'),*args];start=time.monotonic();path=out/(label+'.live.log')
        with path.open('wb') as stream:r=subprocess.run(command,cwd=root,stdout=stream,stderr=subprocess.STDOUT)
        raw=path.read_bytes();data=gzip.compress(raw,mtime=0);artifact=label+'.log.gz';(out/artifact).write_bytes(data);path.unlink()
        m['checks'].append(dict(command=command,exit_code=r.returncode,seconds=time.monotonic()-start,artifact=artifact,sha256=hashlib.sha256(data).hexdigest()));save();assert r.returncode==0,label
        print(label,'passed',flush=True)
    m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:save()
print('Canonical signed-history timing/allocation complete; independent acceptance remains required.',flush=True)
