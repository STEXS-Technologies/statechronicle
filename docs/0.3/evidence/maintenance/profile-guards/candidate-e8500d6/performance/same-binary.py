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
manifest['hashes']=json.loads((base/'attempt-with-concurrent-builds/manifest.json').read_text())['hashes']
manifest['limitations'].append('Repeated after own qualification processes finished; other host activity remains uncontrolled.')
manifest['same_binary_control']=True
manifest['limitations'].append('Both timing labels execute the identical candidate binary; measures host/order noise, not a source comparison.')
results={n:[] for n in ['control','candidate']}
for pair in range(7):
 for name in (['control','candidate'] if pair%2==0 else ['candidate','control']):
  binary=base/'candidate/build/release/profile-guard-cost'
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
