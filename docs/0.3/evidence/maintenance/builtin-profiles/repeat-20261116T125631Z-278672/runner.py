"""Repeat retained binaries on predeclared CPU 8; retain every observation."""
import gzip,hashlib,json,os,pathlib,platform,statistics,subprocess,time
root=pathlib.Path.cwd();base=root/'docs/0.3/evidence/maintenance/builtin-profiles/paired-20261116T125348Z-164088';prior=json.loads((base/'manifest.json').read_text())
assert prior['status']=='passed';cpu=8;assert cpu in os.sched_getaffinity(0)
run=root/'target/performance-matrix'/base.name.removeprefix('paired-')
out=base.parent/('repeat-'+time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())+'-'+str(os.getpid()));out.mkdir()
def sha(b):return hashlib.sha256(b).hexdigest()
def conditions():
 p=pathlib.Path('/sys/devices/system/cpu/cpu8/cpufreq');return dict(utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),load_average=os.getloadavg(),governor=(p/'scaling_governor').read_text().strip(),frequency=(p/'scaling_cur_freq').read_text().strip())
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8');binaries={lane:run/f'lane-{index}-bench' for index,lane in enumerate(prior['lanes'])}
m=dict(candidate_commit=prior['candidate_commit'],source_campaign=str(base.relative_to(root)),source_campaign_sha256=sha((base/'manifest.json').read_bytes()),cpu=cpu,conditions_before=conditions(),status='running',checks=[],binary_sha256={lane:sha(binary.read_bytes()) for lane,binary in binaries.items()},limitations=['Shared host, predeclared CPU 8; no observations discarded, original CPU 0 campaign retained separately.','Same immutable binaries and locks as source campaign; no builds during timing.','Each p95 is a percentile of 31 batch averages, not individual requests.'])
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
def measure(label,lane,args,multiplier):
 command=['taskset','-c',str(cpu),str(binaries[lane]),*args];start=time.monotonic();r=subprocess.run(command,cwd=root,env=dict(env,STATECHRONICLE_BENCH_MULTIPLIER=str(multiplier)),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=120)
 name=label+'.log.gz';data=gzip.compress(r.stdout,mtime=0);(out/name).write_bytes(data);m['checks'].append(dict(label=label,command=command,multiplier=multiplier,exit_code=r.returncode,seconds=time.monotonic()-start,artifact=name,sha256=sha(data)));save()
 if r.returncode:raise RuntimeError(label)
 return json.loads(r.stdout)['workloads']
save()
try:
 for lane,binary in binaries.items():assert sha(binary.read_bytes())==prior['lanes'][lane]['binary_sha256']
 workloads=['checkpoint_resume_256_events_chunk_32_and_root','cross_tenant_trade_plan_including_seed_setup','authenticated_prepare_including_adapter_setup','authenticated_two_intent_batch_to_refusing_sink_including_setup','two_intent_planning_including_adapter_setup']
 observations={'default':{},'extended':{}}
 for regime in observations:
  for cycle in range(7):
   order=['control','candidate'] if cycle%2 else ['candidate','control']
   for lane in order:
    selections=[None] if regime=='default' else workloads
    for name in selections:
     raw=measure(f'{regime}-{cycle}-{lane}-{name or "all"}',lane,[] if name is None else [name],1 if regime=='default' else 20)
     assert len(raw)==(22 if name is None else 1)
     for row in raw:observations[regime].setdefault(row['workload'],{x:[] for x in binaries})[lane].append(row)
   print(regime,cycle,'passed',flush=True)
 summary=[]
 for regime,workloads in observations.items():
  for name,lanes in workloads.items():
   deltas={key:[(c[key]/b[key]-1)*100 if b[key] else 0 for b,c in zip(lanes['control'],lanes['candidate'])] for key in ['median_ns','p95_ns']}
   summary.append(dict(regime=regime,workload=name,observations=lanes,paired_percent_deltas=deltas,median_paired_percent_deltas={key:statistics.median(values) for key,values in deltas.items()}))
 (out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
 for lane,binary in binaries.items():assert sha(binary.read_bytes())==m['binary_sha256'][lane]
 m['status']='passed'
except BaseException as error:m['status']='failed_or_interrupted';m['failure']=repr(error);raise
finally:
 m['conditions_after']=conditions();m['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in out.iterdir() if p.name!='manifest.json'};save()
print(out.relative_to(root),flush=True)
