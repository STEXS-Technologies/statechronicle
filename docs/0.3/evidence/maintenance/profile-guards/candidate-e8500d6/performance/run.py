#!/usr/bin/env python3
"""Run from repo root. Build exact archived revisions; no network or CI."""
from pathlib import Path
import subprocess, json, os, hashlib, tarfile, io, shutil, statistics
root=Path.cwd()
base=root/'target/profile-guards/bench'
control='89e2494cce934a1ad7adec47255fb4fccc44cc52'
candidate='e8500d6816c5f9d58abe830c37a983d057fed743'
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8')
manifest={'control':control,'candidate':candidate,'cpu':min(os.sched_getaffinity(0)), 'pairs':7,'samples':31,'iterations_per_sample':50000,'warmup':10000,'compiler':subprocess.check_output(['rustc','-Vv'],text=True),'limitations':['Shared host; sample means, not per-operation tail latency.','Pure public ProfileRules::check only; no reduction, auth, durable write or full workflow.','Allocation counter measures calls/requested bytes, not peak live memory.'],'hashes':{}}
for name,rev in [('control',control),('candidate',candidate)]:
 dest=base/name
 dest.mkdir(parents=True,exist_ok=True)
 source=dest/'source'; source.mkdir(exist_ok=True)
 data=subprocess.check_output(['git','archive',rev])
 with tarfile.open(fileobj=io.BytesIO(data)) as tf:tf.extractall(source,filter='data')
 consumer=dest/'consumer'; (consumer/'src').mkdir(parents=True,exist_ok=True)
 (consumer/'Cargo.toml').write_text('[package]\nname="profile-guard-cost"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\nstatechronicle={path="../source/crates/statechronicle"}\nserde_json="1.0"\n')
 shutil.copyfile(source/'Cargo.lock',consumer/'Cargo.lock')
 shutil.copyfile(base/'check.rs',consumer/'src/main.rs')
 shutil.copyfile(source/'ci/characterization/profile_guards.rs',consumer/'src/fixture.rs')
 runenv=dict(env,CARGO_TARGET_DIR=str(dest/'build'))
 with (dest/'build.log').open('w') as log:
  p=subprocess.run(['cargo','build','--release','--offline','--manifest-path',str(consumer/'Cargo.toml')],env=runenv,stdout=log,stderr=subprocess.STDOUT)
 assert p.returncode==0,(name,p.returncode)
 import tomllib
 def deps(path):
  return {(p['name'],p['version'],p.get('source'),p.get('checksum')) for p in tomllib.loads(path.read_text())['package'] if p.get('source')}
 assert not deps(consumer/'Cargo.lock')-deps(source/'Cargo.lock')
 for p in [source/'Cargo.lock',consumer/'Cargo.lock',consumer/'src/main.rs',consumer/'src/fixture.rs',dest/'build/release/profile-guard-cost']:
  manifest['hashes'][str(p.relative_to(root))]=hashlib.sha256(p.read_bytes()).hexdigest()
results={n:[] for n in ['control','candidate']}
for pair in range(7):
 for name in (['control','candidate'] if pair%2==0 else ['candidate','control']):
  binary=base/name/'build/release/profile-guard-cost'
  raw=subprocess.check_output(['taskset','-c',str(manifest['cpu']),str(binary)])
  (base/f'{name}-{pair}.json').write_bytes(raw)
  results[name].append(json.loads(raw))
summary=[]
for i in range(len(results['control'][0])):
 row={'case':results['control'][0][i]['case']}
 for name in results:
  samples=[x for r in results[name] for x in r[i]['sample_mean_ns']]
  row[name]={'median_ns':statistics.median(samples),'p95_sample_mean_ns':sorted(samples)[int(.95*(len(samples)-1))]}
  signature={(r[i]['result'],r[i]['allocation_calls_per_1000'],r[i]['allocation_bytes_per_1000']) for r in results[name]}
  assert len(signature)==1
  row[name]['result'],row[name]['allocation_calls_per_1000'],row[name]['allocation_bytes_per_1000']=signature.pop()
 assert all(row['control'][k]==row['candidate'][k] for k in ['result','allocation_calls_per_1000','allocation_bytes_per_1000'])
 row['median_delta_percent']=100*(row['candidate']['median_ns']/row['control']['median_ns']-1)
 row['p95_sample_mean_delta_percent']=100*(row['candidate']['p95_sample_mean_ns']/row['control']['p95_sample_mean_ns']-1)
 summary.append(row)
(base/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
(base/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
