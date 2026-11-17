"""Resume only unfinished full-scope control allocations after a real timeout."""
import concurrent.futures,gzip,hashlib,json,os,re,subprocess,threading,time
from pathlib import Path
root=Path.cwd();driver=Path(__file__).resolve();out=driver.parent;base=out/'history-performance';dest=out/'history-control-resume';dest.mkdir(exist_ok=False);sha=lambda b:hashlib.sha256(b).hexdigest()
failed=base/'manifest-failed-900-seconds.json';old=json.loads(failed.read_text());after=out/'after-history-failed-900-seconds.json'
assert old['status']=='failed' and 'TimeoutExpired' in old['error'] and ', 900)' in old['error']
assert json.loads(after.read_text())['status']=='failed'
assert (base/'manifest.json').read_bytes()==failed.read_bytes()
assert len([c for c in old['checks'] if c['label'].split('-')[0].isdigit()])==14
expected=[('control',n) for n in [1000,100,10]]+[(f'{prefix}_{n}',count) for n,count in [(1,1000),(32,100),(256,10)] for prefix in ['verify_signed_history','checkpoint_ack_loss_resume_and_root']]
labels=[f'allocation-control-{name}-{count}' for name,count in expected]
positive=[c for c in old['checks'] if c['label'].startswith('allocation-control-')];assert [c['label'] for c in positive]==labels[:7]
binary=base/'bench-control__';assert sha(binary.read_bytes())==old['lanes']['control']['binary_sha256']
cpus={labels[7]:6,labels[8]:8};topology={}
for cpu in cpus.values():
 p=Path(f'/sys/devices/system/cpu/cpu{cpu}/topology');topology[str(cpu)]={n:(p/n).read_text().strip() for n in ['core_id','physical_package_id','thread_siblings_list']}
assert len({(v['physical_package_id'],v['core_id']) for v in topology.values()})==2
m=dict(pid=os.getpid(),status='running',checks=[],allocations={},prior_positive_checks=positive,failed_manifest_sha256=sha(failed.read_bytes()),failed_orchestrator_sha256=sha(after.read_bytes()),timing_summary_sha256=sha((base/'summary.json').read_bytes()),binary_sha256=sha(binary.read_bytes()),runner_sha256=sha(driver.read_bytes()),timeout_seconds=3600,cpu_by_label=cpus,topology=topology,started_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),review='Actual original control verify256 process exceeded900seconds. Retain14timingprocesses+7positiveallocationprobes; rerun ONLY verify256x10 and checkpoint256x10 with unchanged binary/fixtures/128bytearguments/countcontrols. CPUs6/8 are distinctphysicalcores. Allocation-only concurrency is not a latency measurement.')
(dest/'runner.py').write_bytes(driver.read_bytes());lock=threading.RLock()
def save():
 with lock:
  m['checks'].sort(key=lambda c:labels.index(c['label']));temporary=dest/'manifest.json.new';temporary.write_text(json.dumps(m,indent=2)+'\n');temporary.replace(dest/'manifest.json')
def parse(raw,count):
 assert re.search(rb'ERROR SUMMARY: 0 errors',raw)
 match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);assert match
 totals=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]));totals['actions']=count;return totals
for (name,count),c in zip(expected,positive):
 data=(base/c['artifact']).read_bytes();assert c['exit_code']==0 and sha(data)==c['sha256'];m['allocations'][name+'@'+str(count)]=parse(gzip.decompress(data),count)
save()
def measure(name,count):
 label=f'allocation-control-{name}-{count}';assert sha(binary.read_bytes())==m['binary_sha256']
 command=['taskset','-c',str(cpus[label]),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),name.ljust(128,'_')]
 extra=dict(STATECHRONICLE_ALLOCATION_PROBE='1',STATECHRONICLE_ALLOCATION_ACTIONS=str(count));start=time.monotonic();timed_out=False
 try:
  result=subprocess.run(command,cwd=root,env=dict(os.environ,**extra),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=3600);raw=result.stdout;code=result.returncode
 except subprocess.TimeoutExpired as error:raw=error.output or b'';code=None;timed_out=True
 data=gzip.compress(raw,mtime=0);artifact=label+'.log.gz';(dest/artifact).write_bytes(data)
 c=dict(label=label,command=command,exit_code=code,timed_out=timed_out,seconds=time.monotonic()-start,artifact=artifact,sha256=sha(data),extra_environment=extra)
 with lock:m['checks'].append(c);save()
 assert code==0 and not timed_out,label
 totals=parse(raw,count)
 with lock:m['allocations'][name+'@'+str(count)]=totals;save()
 print(label,'passed',flush=True)
try:
 with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
  futures=[pool.submit(measure,name,count) for name,count in expected[7:]]
  for f in concurrent.futures.as_completed(futures):f.result()
 assert len(m['allocations'])==9 and [c['label'] for c in m['checks']]==labels[7:]
 for name,totals in m['allocations'].items():
  count=totals['actions'];control=m['allocations']['control@'+str(count)];totals['control_subtracted_per_action']={key:(totals[key]-control[key])/count for key in ['allocations','bytes']}
 m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:m['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
print('Control allocation scope complete; original timeout remains failed. Integration and independent verification remain required.',flush=True)
