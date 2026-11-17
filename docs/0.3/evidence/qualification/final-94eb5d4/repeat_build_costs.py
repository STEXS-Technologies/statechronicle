"""Complete three alternating cold-target build samples per revision after timings."""
import gzip,hashlib,json,os
from pathlib import Path
import subprocess,time
root=Path.cwd();out=Path(__file__).resolve().parent
m=dict(status='waiting_for_timings',pid=os.getpid(),checks=[],limitations=['Cold target directories; registry and filesystem caches remain warm.','Three observations per lane on a shared host, not an isolated hardware performance guarantee.','Baseline/candidate package graphs differ by the approved dependency refresh; report total cost without attributing it solely to refactoring.'])
def save():(out/'repeat-build-costs.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 while True:
  prior=json.loads((out/'after-fuzz.json').read_text())
  if prior['status'] not in ['running','waiting_for_extended_fuzz']:
   if prior['status']!='passed':raise RuntimeError('timing/build prerequisite failed')
   break
  if not Path('/proc/'+str(prior['pid'])).exists():raise RuntimeError('timing orchestration is no longer live')
  time.sleep(15)
 m['candidate_commit']=prior['candidate_commit'];m['status']='running';save()
 for index,revision in enumerate([prior['candidate_commit'],'v0.2.0','v0.2.0',prior['candidate_commit']]):
  command=['python3','scripts/refactor_costs.py','--revision',revision];start=time.monotonic()
  p=subprocess.run(command,cwd=root,env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8'),stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
  data=gzip.compress(p.stdout,mtime=0);name=str(index)+'-cold-build.log.gz';(out/name).write_bytes(data)
  m['checks'].append(dict(revision=revision,command=command,exit_code=p.returncode,seconds=time.monotonic()-start,artifact=name,sha256=hashlib.sha256(data).hexdigest()));save()
  if p.returncode:raise RuntimeError('repeat cold build failed')
  print(index,revision,'passed',flush=True)
 m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:save()
