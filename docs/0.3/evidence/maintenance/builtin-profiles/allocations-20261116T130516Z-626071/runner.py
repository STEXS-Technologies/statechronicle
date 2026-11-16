"""Whole-process allocation totals for the matched two-lane helper extraction binaries."""
import gzip,hashlib,json,os,pathlib,re,subprocess,time
root=pathlib.Path.cwd();base=root/'docs/0.3/evidence/maintenance/builtin-profiles/paired-20261116T130401Z-565324';prior=json.loads((base/'manifest.json').read_text());assert prior['status']=='passed'
run=root/'target/performance-matrix'/base.name.removeprefix('paired-');out=base.parent/('allocations-'+time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())+'-'+str(os.getpid()));out.mkdir()
def sha(b):return hashlib.sha256(b).hexdigest()
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',STATECHRONICLE_BENCH_MULTIPLIER='1')
m=dict(candidate_commit=prior['candidate_commit'],source_campaign_sha256=sha((base/'manifest.json').read_bytes()),source_campaign=str(base.relative_to(root)),valgrind=subprocess.check_output(['valgrind','--version'],text=True).strip(),cpu=prior['cpu'],status='running',checks=[],results={},limitations=['Full process totals include identical fixed startup and 1000 probe actions, not allocator telemetry for arbitrary requests.','Equal-length executable paths and padded probe arguments reduce process-argument differences.','Control-subtracted figures are bounded fixture observations, not universal per-operation allocation guarantees.','No latency inference from instrumentation; no allocation-free claim for complete hosted/canonical facades.'])
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 binaries=[run/f'lane-{index}-bench' for index in range(2)];assert len({len(str(binary)) for binary in binaries})==1
 for binary,(lane,old) in zip(binaries,prior['lanes'].items()):
  assert sha(binary.read_bytes())==old['binary_sha256'];m['results'][lane]={}
  for probe in ['control','native','portable','hosted','proof']:
   command=['taskset','-c',str(prior['cpu']),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),('alloc-'+probe).ljust(32,'_')]
   start=time.monotonic()
   try:r=subprocess.run(command,cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=120);data=r.stdout;code=r.returncode
   except subprocess.TimeoutExpired as error:data=error.stdout or b'';code='timeout'
   artifact=lane+'-'+probe+'.log.gz';compressed=gzip.compress(data,mtime=0);(out/artifact).write_bytes(compressed);m['checks'].append(dict(command=command,exit_code=code,seconds=time.monotonic()-start,artifact=artifact,sha256=sha(compressed)));save()
   assert code==0,(lane,probe,code)
   match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',data);assert match,(lane,probe)
   allocations,frees,bytes_allocated=[int(value.replace(b',',b'')) for value in match.groups()]
   m['results'][lane][probe]=dict(allocations=allocations,frees=frees,bytes=bytes_allocated)
   save();print(lane,probe,allocations,bytes_allocated,flush=True)
  control=m['results'][lane]['control']
  for probe,totals in m['results'][lane].items():
   totals['control_subtracted_per_action']={metric:(totals[metric]-control[metric])/1000 for metric in ['allocations','bytes']}
  assert all(m['results'][lane]['portable'][metric]==control[metric] for metric in ['allocations','bytes'])
  assert sha(binary.read_bytes())==old['binary_sha256']
 m['status']='passed'
except BaseException as error:m['status']='failed_or_interrupted';m['failure']=repr(error);raise
finally:
 m['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in out.iterdir() if p.name!='manifest.json'};save()
print(out.relative_to(root),flush=True)
