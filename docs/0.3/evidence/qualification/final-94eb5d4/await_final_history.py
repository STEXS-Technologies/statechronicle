"""Wait for the actual history orchestrator before the final evidence recheck."""
import gzip,hashlib,json,os,subprocess,time
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent
m=dict(pid=os.getpid(),status='waiting_for_history',checks=[])
def save():(out/'await-final-history.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 while True:
  prior=json.loads((out/'after-history.json').read_text())
  if prior['status'] not in ['running','waiting_for_prior_campaigns']:
   assert prior['status']=='passed',prior
   break
  pid=prior['pid'];cmd=Path(f'/proc/{pid}/cmdline')
  assert cmd.exists() and b'after_history.py' in cmd.read_bytes(),'History orchestrator is no longer live.'
  time.sleep(15)
 m['status']='running';save();start=time.monotonic()
 command=['python3',str(out/'recheck_final_gates.py')]
 path=out/'final-recheck.live.log'
 with path.open('wb') as stream:r=subprocess.run(command,cwd=root,stdout=stream,stderr=subprocess.STDOUT)
 data=gzip.compress(path.read_bytes(),mtime=0);artifact='final-recheck.log.gz';(out/artifact).write_bytes(data);path.unlink()
 m['checks'].append(dict(command=command,exit_code=r.returncode,seconds=time.monotonic()-start,artifact=artifact,sha256=hashlib.sha256(data).hexdigest()));save()
 assert r.returncode==0,artifact
 m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:save()
print('Final retained-evidence recheck passed; manual acceptance, commit and push remain separate.',flush=True)
