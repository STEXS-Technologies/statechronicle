"""Consolidate complete history allocation lanes, then recheck every final gate."""
import gzip,hashlib,json,os,subprocess,time
from pathlib import Path
root=Path.cwd();driver=Path(__file__).resolve();out=driver.parent;base=out/'history-performance';sha=lambda b:hashlib.sha256(b).hexdigest()
m=dict(pid=os.getpid(),status='waiting_for_allocation_lanes',checks=[],runner_sha256=sha(driver.read_bytes()),scope='Full history measurement integration and final evidence recheck only; manual acceptance, requirement closure and Git delivery remain separate.')
def write(path,value):
 temporary=path.with_name(path.name+'.new');temporary.write_text(json.dumps(value,indent=2)+'\n');temporary.replace(path)
def save():write(out/'history-parallel-completion.json',m)
save()
def check(label,command):
 start=time.monotonic();r=subprocess.run(command,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);data=gzip.compress(r.stdout,mtime=0);artifact=label+'.log.gz';(out/artifact).write_bytes(data)
 c=dict(command=command,exit_code=r.returncode,seconds=time.monotonic()-start,artifact=artifact,sha256=sha(data));m['checks'].append(c);save();assert r.returncode==0,artifact;print(label,'passed',flush=True)
try:
 while True:
  ready=True
  for folder,runner in [('history-control-resume','resume_history_control.py'),('history-candidate-allocations','history_candidate_parallel.py')]:
   prior=json.loads((out/folder/'manifest.json').read_text())
   if prior['status']=='running':
    cmd=Path(f"/proc/{prior['pid']}/cmdline");assert cmd.exists() and runner.encode() in cmd.read_bytes(),folder+' process is no longer live';ready=False
   else:assert prior['status']=='passed',(folder,prior.get('error'))
  if ready:break
  time.sleep(15)
 m['status']='verifying_complete_lanes';save()
 for label,script in [('history-control-independent','verify_history_control_resume.py'),('history-candidate-independent','verify_history_candidate_allocations.py')]:check(label,['python3',str(out/script)])
 failed=base/'manifest-failed-900-seconds.json';history=json.loads(failed.read_text());failed_after=out/'after-history-failed-900-seconds.json';old_outer=json.loads(failed_after.read_text())
 assert history['status']=='failed' and old_outer['status']=='failed'
 assert (base/'manifest.json').read_bytes()==failed.read_bytes()
 assert (out/'after-history.json').read_bytes()==failed_after.read_bytes()
 source_paths=dict(retained_control=failed,resumed_control=out/'history-control-resume/manifest.json',candidate=out/'history-candidate-allocations/manifest.json')
 sources={key:json.loads(path.read_text()) for key,path in source_paths.items()}
 assert sources['resumed_control']['status']==sources['candidate']['status']=='passed'
 assert sources['resumed_control']['binary_sha256']==history['lanes']['control']['binary_sha256'] and sources['candidate']['binary_sha256']==history['lanes']['candidate']['binary_sha256']
 source_records={key:dict(path=str(path.relative_to(out)),sha256=sha(path.read_bytes())) for key,path in source_paths.items()}
 def retain(c,key):
  record=json.loads(json.dumps(c));source=source_paths[key].parent/c['artifact'];data=source.read_bytes();assert sha(data)==c['sha256'] and c['exit_code']==0
  if key!='retained_control':
   record['artifact']=key+'-'+c['artifact'];destination=base/record['artifact'];assert not destination.exists();destination.write_bytes(data)
  record['allocation_origin']=key;return record
 retained=[]
 for c in history['checks']:
  retained.append(retain(c,'retained_control') if c['label'].startswith('allocation-') else c)
 retained.extend(retain(c,'resumed_control') for c in sources['resumed_control']['checks'])
 retained.extend(retain(c,'candidate') for c in sources['candidate']['checks'])
 assert len([c for c in retained if c['label'].startswith('allocation-')])==18
 history.update(status='passed',checks=retained,allocations=dict(control=sources['resumed_control']['allocations'],candidate=sources['candidate']['allocations']),allocation_sources=source_records,allocation_integration=dict(utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),runner_sha256=m['runner_sha256'],failed_orchestrator_sha256=sha(failed_after.read_bytes()),independent_reports={name:sha((out/name).read_bytes()) for name in ['history-control-allocation-verification.json','history-candidate-allocation-verification.json']},scope='14 original timing processes,7 original control allocation probes,2 full control retries and9 complete candidate probes. The original900secondtimeout and candidate intentional handoff remain preserved and unqualified.'))
 history.pop('error');history['allocation_limitations'].append('Allocation-only processes use independently recorded CPU cores; no allocation execution times are latency observations. Full control and candidate sources, actual timeout and intentional handoff remain separately retained.')
 write(base/'manifest.json',history)
 outer=dict(pid=os.getpid(),status='passed',checks=[old_outer['checks'][0],*m['checks']],original_failed_attempt=dict(path=failed_after.name,sha256=sha(failed_after.read_bytes())),qualified_manifest_sha256=sha((base/'manifest.json').read_bytes()),scope='Complete positive timing/allocation scope integrated after actual source-lane checks. Original timed-out orchestration remains failed; final retained-evidence gates and manual acceptance remain separate.')
 write(out/'after-history.json',outer)
 m['status']='running_final_gate_recheck';m['qualified_manifest_sha256']=outer['qualified_manifest_sha256'];save()
 check('history-parallel-final-recheck',['python3',str(out/'recheck_final_gates.py')])
 m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:save()
print('Complete history integration and final evidence recheck passed; manual acceptance and Git delivery remain pending.',flush=True)
