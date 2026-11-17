"""Measure the unchanged candidate's full history allocation scope independently."""
import gzip,hashlib,json,os,re,subprocess,time
from pathlib import Path
root=Path.cwd();driver=Path(__file__).resolve();out=driver.parent
base=out/'history-performance';dest=out/'history-candidate-allocations';dest.mkdir(exist_ok=False)
sha=lambda b:hashlib.sha256(b).hexdigest()
history=json.loads((base/'manifest.json').read_text());summary=json.loads((base/'summary.json').read_text())
assert len([c for c in history['checks'] if c['label'].split('-')[0].isdigit()])==14
assert len(summary)==7
binary=base/'bench-candidate';assert sha(binary.read_bytes())==history['lanes']['candidate']['binary_sha256']
probes=[('control',n) for n in [1000,100,10]]+[(r['workload'],1000 if r['workload'].endswith('_1') else 100 if r['workload'].endswith('_32') else 10) for r in summary if r['workload']!='control']
assert len(probes)==9
m=dict(pid=os.getpid(),status='running',lane='candidate',cpu=2,timeout_seconds=3600,checks=[],allocations={},binary_sha256=sha(binary.read_bytes()),build_manifest_sha256=sha((base/'build-manifest.json').read_bytes()),summary_sha256=sha((base/'summary.json').read_bytes()),driver_sha256=sha(driver.read_bytes()),started_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),review='Independent allocation-only lane on CPU2 (core2); original control remains on CPU6 (core6). All timing processes already finished. Same binary, three fixtures, 128-byte arguments, matched count controls and full 1000/100/10 action budgets. No live control process restarted or shortened.')
(dest/'runner.py').write_bytes(driver.read_bytes())
def save():(dest/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 for name,count in probes:
  assert sha(binary.read_bytes())==m['binary_sha256']
  command=['taskset','-c',str(m['cpu']),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),name.ljust(128,'_')]
  extra=dict(STATECHRONICLE_ALLOCATION_PROBE='1',STATECHRONICLE_ALLOCATION_ACTIONS=str(count));start=time.monotonic();label=f'allocation-candidate-{name}-{count}';timed_out=False
  try:
   result=subprocess.run(command,cwd=root,env=dict(os.environ,**extra),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=m['timeout_seconds']);raw=result.stdout;code=result.returncode
  except subprocess.TimeoutExpired as error:
   raw=error.output or b'';code=None;timed_out=True
  data=gzip.compress(raw,mtime=0);artifact=label+'.log.gz';(dest/artifact).write_bytes(data)
  c=dict(label=label,command=command,exit_code=code,timed_out=timed_out,seconds=time.monotonic()-start,artifact=artifact,sha256=sha(data),extra_environment=extra);m['checks'].append(c);save()
  assert code==0 and not timed_out,label
  assert re.search(rb'ERROR SUMMARY: 0 errors',raw)
  match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);assert match
  totals=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]));totals['actions']=count;m['allocations'][name+'@'+str(count)]=totals;save();print(label,'passed',flush=True)
 for name,totals in m['allocations'].items():
  count=totals['actions'];control=m['allocations']['control@'+str(count)]
  totals['control_subtracted_per_action']={key:(totals[key]-control[key])/count for key in ['allocations','bytes']}
 m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:m['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
print('Candidate allocation lane complete; independent verification and campaign integration remain required.',flush=True)
