#!/usr/bin/env python3
"""Compare full-process allocation totals for the three bounded frozen workloads."""
import gzip, hashlib, json, os, re, shutil, subprocess, time
from pathlib import Path
root=Path(subprocess.check_output(['git','rev-parse','--show-toplevel'],text=True).strip())
bench=root/'docs/0.3/evidence/f2/prepared-profile-a52d53f/bench-a52d53f-20261116T112429Z'
baseline=json.loads((bench/'manifest.json').read_text())
out=bench.parent/'allocations';out.mkdir(exist_ok=False)
run=root/'target/prepared-profile-bench/20261116T112429Z'
def sha(data):return hashlib.sha256(data).hexdigest()
(out/'runner.py').write_bytes(Path(__file__).read_bytes())
m={'status':'running','benchmark_manifest_sha256':sha((bench/'manifest.json').read_bytes()),'valgrind':subprocess.check_output(['valgrind','--version'],text=True).strip(),'checks':[],'totals':{},'limitations':['Full-process totals include startup, fixed benchmark setup, ten warmups and 31 samples of five operations.','Not a per-operation allocation claim; no subtraction or normalization.','Both executable paths have equal length; input selector and copied binary hashes are fixed.','These three fixed sentinel-adapter workflows do not establish allocation equivalence for other inputs or public APIs.']}
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 workloads=[row['workload'] for row in json.loads((bench/'summary.json').read_text())]
 for lane,suffix in [('control','a'),('candidate','b')]:
  source=run/lane/'executor-bench';binary=run/('allocation-'+suffix);shutil.copy2(source,binary)
  assert sha(binary.read_bytes())==baseline['lanes'][lane]['binary_sha256']
  m['totals'][lane]={}
  for name in workloads:
   command=['taskset','-c',str(baseline['cpu']),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),name]
   start=time.monotonic();r=subprocess.run(command,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=300)
   data=gzip.compress(r.stdout,mtime=0);artifact=lane+'-'+name+'.log.gz';(out/artifact).write_bytes(data)
   m['checks'].append({'lane':lane,'workload':name,'command':command,'seconds':time.monotonic()-start,'exit_code':r.returncode,'artifact':artifact,'sha256':sha(data),'binary_sha256':sha(binary.read_bytes())})
   save();print(lane,name,r.returncode,flush=True);assert r.returncode==0
   match=re.search(rb'total heap usage: ([\d,]+) allocs, ([\d,]+) frees, ([\d,]+) bytes allocated',r.stdout);assert match
   m['totals'][lane][name]=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]))
   save()
 m['equal_full_process_totals']=m['totals']['control']==m['totals']['candidate'];m['status']='passed'
except BaseException as e:m['status']='failed_or_interrupted';m['failure']=repr(e);raise
finally:
 m['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in sorted(out.iterdir()) if p.name!='manifest.json'};save()
print(json.dumps(m['totals'],indent=2));print('equal',m['equal_full_process_totals'])
