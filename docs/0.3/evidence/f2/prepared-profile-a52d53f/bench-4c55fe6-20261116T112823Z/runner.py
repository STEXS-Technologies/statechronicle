#!/usr/bin/env python3
"""Paired observations of checked profile preparation stages."""
import gzip,hashlib,io,json,os,shutil,statistics,subprocess,tarfile,time
from pathlib import Path
import sys
root=Path(subprocess.check_output(['git','rev-parse','--show-toplevel'],text=True).strip())
sys.path.insert(0,str(root/'scripts'))
from refactor_compat import packages
def sha(data):return hashlib.sha256(data).hexdigest()
def git(*args):return subprocess.check_output(['git',*args],cwd=root)
candidate=git('rev-parse','HEAD').decode().strip();control='04dd9461e2c87e2a98fb34ce5210f83e7679aeb2'
stamp=time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())
run=root/'target/prepared-profile-bench'/stamp;run.mkdir(parents=True)
out=root/'docs/0.3/evidence/f2/prepared-profile-a52d53f'/('bench-'+candidate[:7]+'-'+stamp);out.mkdir(parents=True,exist_ok=False)
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/proof-delegation-bench/build'))
for key in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS']:env.pop(key,None)
cpu=min(os.sched_getaffinity(0))
m={'control_commit':control,'candidate_commit':candidate,'rustc':subprocess.check_output(['rustc','-vV'],text=True),'cpu':cpu,'environment':{k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},'status':'running','checks':[],'lanes':{},'runs':[],'limitations':['Shared host, seven alternating pairs; no discarded samples or universal performance threshold.','Tail values are percentiles of 31 batch averages, not individual-request latency.','Frozen benchmark byte-identical in both lanes; native custom profile preparation workloads.','Identical compiler, resolved consumer dependencies and workload in both lanes.','Fixed in-memory adapters, including setup; refusal sink never reports durable success. No physical durability or complete release performance claim.']}
(out/'runner.py').write_bytes(Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
def check(label,cmd,cwd):
 start=time.monotonic();result=subprocess.run(cmd,cwd=cwd,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600);data=gzip.compress(result.stdout,mtime=0);(out/(label+'.log.gz')).write_bytes(data)
 m['checks'].append({'label':label,'command':cmd,'cwd':str(cwd),'exit_code':result.returncode,'seconds':time.monotonic()-start,'artifact':label+'.log.gz','sha256':sha(data)});save();print(label,result.returncode,flush=True)
 if result.returncode:raise RuntimeError(label+' failed')
 return result.stdout
save()
try:
 binaries={};locks=[]
 for lane,revision in [('control',control),('candidate',candidate)]:
  source=run/lane/'source';source.mkdir(parents=True)
  data=git('archive',revision,'--','Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz')
  with tarfile.open(fileobj=io.BytesIO(data)) as archive:archive.extractall(source,filter='data')
  consumer_root=source.parent/'consumer';shutil.copytree(root/'ci/compat-0.2',consumer_root)
  consumer=consumer_root/'crates/consumer';cargo=consumer/'Cargo.toml';cargo.write_text(cargo.read_text().replace('../../../../crates/statechronicle',str(source/'crates/statechronicle')))
  shutil.copyfile(source/'Cargo.lock',consumer/'Cargo.lock')
  (consumer_root/'wire').mkdir();shutil.copyfile(root/'ci/wire-0.2/history.json',consumer_root/'wire/history.json')
  bench=consumer/'examples/refactor_bench.rs';code=bench.read_text()
  bench.write_text(code);(out/(lane+'-benchmark.rs')).write_text(code)
  check(lane+'-resolve',['cargo','metadata','--offline','--format-version','1'],consumer)
  assert not packages(consumer/'Cargo.lock')-packages(source/'Cargo.lock')
  lock=(consumer/'Cargo.lock').read_bytes();locks.append(lock);(out/(lane+'.lock')).write_bytes(lock)
  check(lane+'-build',['cargo','build','--example','refactor_bench','--release','--locked','--offline'],consumer)
  binary=source.parent/'executor-bench';shutil.copy2(Path(env['CARGO_TARGET_DIR'])/'release/examples/refactor_bench',binary);binaries[lane]=binary
  m['lanes'][lane]={'revision':revision,'archive_sha256':sha(data),'lock_sha256':sha(lock),'benchmark_sha256':sha(code.encode()),'binary_sha256':sha(binary.read_bytes()),'binary_bytes':binary.stat().st_size}
  save()
 assert locks[0]==locks[1],'resolved locks differ'
 workloads=['authenticated_prepare_including_adapter_setup','authenticated_two_intent_batch_to_refusing_sink_including_setup','two_intent_planning_including_adapter_setup']
 results={name:{lane:[] for lane in binaries} for name in workloads}
 for pair in range(7):
  for lane in (['control','candidate'] if pair%2==0 else ['candidate','control']):
   for name in workloads:
    cmd=['taskset','-c',str(cpu),str(binaries[lane]),name];raw=check(f'pair-{pair}-{lane}-{name}',cmd,root);values=json.loads(raw)['workloads'];assert len(values)==1 and values[0]['workload']==name
    results[name][lane].append(values[0])
 summary=[]
 for name,lanes in results.items():
  deltas={metric:[(c[metric]/b[metric]-1)*100 for b,c in zip(lanes['control'],lanes['candidate'])] for metric in ['median_ns','p95_ns']}
  summary.append({'workload':name,'observations':lanes,'paired_percent_deltas':deltas,'median_paired_percent_deltas':{metric:statistics.median(values) for metric,values in deltas.items()},'median_paired_absolute_ns':statistics.median(c['median_ns']-b['median_ns'] for b,c in zip(lanes['control'],lanes['candidate']))})
 (out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n');m['status']='passed'
except BaseException as error:m['status']='failed_or_interrupted';m['failure']=repr(error);raise
finally:
 m['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in sorted(out.iterdir()) if p.name!='manifest.json'};save()
print(out.relative_to(root),flush=True)
