"""Repeat exact bounded workloads with unchanged binaries and reversed first lane."""
import gzip,hashlib,json,os,platform,statistics,subprocess,time
from pathlib import Path
root=Path.cwd();base=root/'docs/0.3/evidence/f2/batch-completion/bench-1e995ab-20261116T115040Z'
original=json.loads((base/'manifest.json').read_text());run=root/'target/batch-completion-bench/20261116T115040Z'
out=base.parent/('repeat-1e995ab-'+time.strftime('%Y%m%dT%H%M%SZ',time.gmtime()));out.mkdir(exist_ok=False)
def sha(data):return hashlib.sha256(data).hexdigest()
def conditions():
 cpu=original['cpu'];governor=Path(f'/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor')
 model=next((line.split(':',1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),None)
 return {'utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'kernel':platform.release(),'cpu_model':model,'load_average':os.getloadavg(),'parent_affinity':sorted(os.sched_getaffinity(0)),'child_cpu':cpu,'governor':governor.read_text().strip() if governor.exists() else None}
m={'source_campaign_manifest_sha256':sha((base/'manifest.json').read_bytes()),'conditions_before':conditions(),'checks':[],'status':'running','limitations':['Shared host; same source/binaries/workloads/CPU; no discarded samples.','Seven alternating pairs, starting candidate then control, reversing the first campaign.','p95 is the 30th of 31 batch-average samples, not individual-request latency.','No other qualification task from this session runs during the repeat measurements.']}
(out/'runner.py').write_bytes(Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 results={r['workload']:{lane:[] for lane in ['control','candidate']} for r in json.loads((base/'summary.json').read_text())}
 for lane in ['control','candidate']:assert sha((run/lane/'executor-bench').read_bytes())==original['lanes'][lane]['binary_sha256']
 for pair in range(7):
  for lane in (['candidate','control'] if pair%2==0 else ['control','candidate']):
   for name in results:
    command=['taskset','-c',str(original['cpu']),str(run/lane/'executor-bench'),name]
    start=time.monotonic();r=subprocess.run(command,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=120)
    artifact=f'pair-{pair}-{lane}-{name}.log.gz';data=gzip.compress(r.stdout,mtime=0);(out/artifact).write_bytes(data)
    m['checks'].append({'command':command,'exit_code':r.returncode,'seconds':time.monotonic()-start,'artifact':artifact,'sha256':sha(data)});save();assert r.returncode==0
    values=json.loads(r.stdout)['workloads'];assert len(values)==1 and values[0]['workload']==name;results[name][lane].append(values[0])
  print('pair',pair,'complete',flush=True)
 summary=[]
 for name,lanes in results.items():
  deltas={metric:[(c[metric]/b[metric]-1)*100 for b,c in zip(lanes['control'],lanes['candidate'])] for metric in ['median_ns','p95_ns']}
  summary.append({'workload':name,'observations':lanes,'paired_percent_deltas':deltas,'median_paired_percent_deltas':{metric:statistics.median(values) for metric,values in deltas.items()}})
 (out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n');m['status']='passed'
except BaseException as e:m['status']='failed_or_interrupted';m['failure']=repr(e);raise
finally:
 m['conditions_after']=conditions();m['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in out.iterdir() if p.name!='manifest.json'};save()
print(out.relative_to(root),flush=True)
