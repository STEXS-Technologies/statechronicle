"""Resume only terminally failed/missing probes; retain every completed full probe."""
import concurrent.futures,gzip,hashlib,json,os,re,subprocess,threading,time
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;complete=json.loads((out/'complete-performance.json').read_text());assert complete['status']=='failed'
base=root/complete['latency_path'];old=json.loads((base/'allocations.json').read_text());assert old['status']=='failed' and not old['live_probes']
sha=lambda b:hashlib.sha256(b).hexdigest();prior=json.loads((base/'manifest.json').read_text());assert prior['status']=='passed'
# Never discard a failed attempt or overwrite a still-live observer.
for name in ['complete-performance.json','queued-verification.json','after-history.json']:
 j=json.loads((out/name).read_text());assert j['status']=='failed',name
 destination=out/(Path(name).stem+'-failed-1200-second-budget.json');assert not destination.exists();destination.write_bytes((out/name).read_bytes())
failed=base/'allocations-failed-1200-second-budget.json';assert not failed.exists();failed.write_bytes((base/'allocations.json').read_bytes())
logs=base/('allocation-resume-'+time.strftime('%Y%m%dT%H%M%SZ',time.gmtime()));logs.mkdir();(logs/'runner.py').write_bytes(Path(__file__).read_bytes())
names=[v['workload'] for v in json.loads((base/'summary.json').read_text())];assert len(names)==22
run=root/'target/per-request'/base.name.removeprefix('all-paths-');cpus=sorted(os.sched_getaffinity(0))[2:6];assert len(cpus)==4
checks=[c for c in old['checks'] if c['exit_code']==0 and not c['timed_out']]
for c in checks:assert sha((base/c['artifact']).read_bytes())==c['sha256']
completed={(c['lane'],c['workload']) for c in checks};assert len(completed)==len(checks)
a={**old,'pid':os.getpid(),'status':'running','checks':checks,'live_probes':{},'retry_of':failed.name,'timeout_seconds_per_probe':3600,'resume_cpu_by_lane':dict(zip(prior['lanes'],cpus)),'retained_successful_probe_count':len(checks),'resume_runner_sha256':sha(Path(__file__).read_bytes())};a.pop('error',None)
a['limitations']=old['limitations']+['After the 1200-second attempt terminally failed on contended CPUs, all successful full 1000-action probes were retained with exact hashes. Only missing/failed probes resume on recorded CPUs with 3600-second wall budgets. All 88 workload/lane combinations remain required; instrumented wall time is not latency.']
m={**complete,'pid':os.getpid(),'status':'running','stage':'resume-missing-full-allocation-probes','failed_attempt':'complete-performance-failed-1200-second-budget.json'};m.pop('error',None);lock=threading.Lock();stop=threading.Event()
def save_json(path,value):
 temporary=path.with_suffix('.json.new');temporary.write_text(json.dumps(value,indent=2)+'\n');temporary.replace(path)
def save():save_json(out/'complete-performance.json',m);save_json(base/'allocations.json',a)
save()
def lane_work(index,lane,item):
 binary=run/f'lane-{index}-bench';assert sha(binary.read_bytes())==item['binary_sha256']
 for name in names:
  if (lane,name) in completed:continue
  if stop.is_set():return
  command=['taskset','-c',str(cpus[index]),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),name.ljust(128,'_')];start=time.monotonic()
  child=subprocess.Popen(command,cwd=root,env=dict(os.environ,STATECHRONICLE_BENCH_MULTIPLIER='1',STATECHRONICLE_ALLOCATION_PROBE='1'),stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
  with lock:a['live_probes'][lane]=dict(pid=child.pid,workload=name);save()
  timed_out=False
  try:raw=child.communicate(timeout=3600)[0]
  except subprocess.TimeoutExpired:timed_out=True;child.kill();raw=child.communicate()[0]
  data=gzip.compress(raw,mtime=0);path=logs/(lane+'-'+name+'.log.gz');path.write_bytes(data)
  check=dict(lane=lane,workload=name,command=command,exit_code=child.returncode,timed_out=timed_out,seconds=time.monotonic()-start,artifact=str(path.relative_to(base)),sha256=sha(data))
  with lock:a['checks'].append(check);del a['live_probes'][lane];save()
  if timed_out or child.returncode:stop.set();raise RuntimeError(lane+' '+name+' failed')
  assert re.search(rb'ERROR SUMMARY: 0 errors',raw);match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);assert match
  totals=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]))
  with lock:a['results'][lane][name]=totals;save()
  print(lane,name,'passed',flush=True)
 assert sha(binary.read_bytes())==item['binary_sha256']
try:
 with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
  futures=[pool.submit(lane_work,i,lane,item) for i,(lane,item) in enumerate(prior['lanes'].items())]
  for future in concurrent.futures.as_completed(futures):future.result()
 assert len(a['checks'])==88 and len({(c['lane'],c['workload']) for c in a['checks']})==88
 for lane,values in a['results'].items():
  assert set(values)==set(names);control=values['control']
  for total in values.values():total['control_subtracted_per_action']={k:(total[k]-control[k])/1000 for k in ['allocations','bytes']}
  for name in ['native_validate_reduce','portable_validate_reduce_hash']:assert all(values[name][k]==control[k] for k in ['allocations','bytes'])
 a['status']='passed';save();m.update(status='passed',allocation_manifest=str((base/'allocations.json').relative_to(root)),allocation_manifest_sha256=sha((base/'allocations.json').read_bytes()))
except BaseException as error:a.update(status='failed',error=repr(error));m.update(status='failed',error=repr(error));raise
finally:save()
print('All 88 full allocation probes passed, including retained independently hashed successes.',flush=True)
