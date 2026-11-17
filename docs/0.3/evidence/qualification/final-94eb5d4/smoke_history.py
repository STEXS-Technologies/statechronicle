"""Native assertion smoke for each new signed-history lane, without latency claims."""
import gzip,hashlib,json,os,subprocess,time
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;base=out/'history-performance';prior=json.loads((base/'manifest.json').read_text());assert prior['status']=='built';sha=lambda b:hashlib.sha256(b).hexdigest()
m=dict(status='running',build_manifest_sha256=sha((base/'manifest.json').read_bytes()),checks=[],limitations=['Native one-action assertion smoke only. Output/timing is not performance qualification.','Each process first creates and verifies all 1/32/256-commit fixtures, then checks the selected 256-commit action once.'])
def save():(out/'history-smoke.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 for lane,item in prior['lanes'].items():
  binary=base/('bench-'+lane.ljust(9,'_'));assert sha(binary.read_bytes())==item['binary_sha256']
  for name in ['verify_signed_history_256','checkpoint_ack_loss_resume_and_root_256']:
   command=['taskset','-c',str(sorted(os.sched_getaffinity(0))[6]),str(binary),name.ljust(128,'_')];environment=dict(STATECHRONICLE_ALLOCATION_PROBE='1',STATECHRONICLE_ALLOCATION_ACTIONS='1');start=time.monotonic()
   try:
    r=subprocess.run(command,cwd=root,env=dict(os.environ,**environment),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=180);raw=r.stdout;code=r.returncode
   except subprocess.TimeoutExpired as error:raw=error.stdout or b'';code='timeout'
   data=gzip.compress(raw,mtime=0);artifact=f'history-smoke-{lane}-{name}.log.gz';(out/artifact).write_bytes(data)
   m['checks'].append(dict(command=command,extra_environment=environment,binary_sha256=item['binary_sha256'],exit_code=code,seconds=time.monotonic()-start,artifact=artifact,sha256=sha(data)));save();assert code==0,(lane,name,artifact)
   print(lane,name,'native assertions passed',flush=True)
 m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:save()
