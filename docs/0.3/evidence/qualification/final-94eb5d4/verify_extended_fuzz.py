"""Independently verify completed all-target extended fuzz evidence."""
import gzip, hashlib, io, json
from pathlib import Path
import re, subprocess, tarfile, tomllib
root=Path.cwd();out=root/'docs/0.3/evidence/fuzz/extended-94eb5d4'
m=json.loads((out/'manifest.json').read_text());sha=lambda b:hashlib.sha256(b).hexdigest()
def require(ok,message):
 if not ok:raise ValueError(message)
require(m['status']=='passed' and m['source_unchanged'],'campaign/source status')
require(m['build_isolation']=='fresh per-archive target directory','fresh compilation policy')
archive=subprocess.check_output(['git','archive',m['candidate_commit'],'--',*m['archive_paths']])
require(sha(archive)==m['archive_sha256'],'archive hash')
with tarfile.open(fileobj=io.BytesIO(archive)) as t:files={p.name:t.extractfile(p).read() for p in t.getmembers() if p.isfile()}
require({p:sha(b) for p,b in files.items()}==m['source_sha256'],'source map')
declared=sorted(i['name'] for i in tomllib.loads(files['fuzz/Cargo.toml'].decode())['bin'])
require(declared==m['targets']==sorted(c['target'] for c in m['campaigns']),'all declared targets exactly once')
require(len({c['seed'] for c in m['campaigns']})==len(declared),'independent recorded seeds')
for c in m['checks']:
 require(c['exit_code']==0 and sha((out/c['artifact']).read_bytes())==c['sha256'],'build result/hash')
require(b'Compiling statechronicle-fuzz' in gzip.decompress((out/'build.log.gz').read_bytes()),'fresh target compilation')
for c in m['campaigns']:
 require(c['status']=='passed' and c['exit_code']==0 and c['reported_seconds']>=3600 and c['completed_runs']>100 and c['crash_artifacts']==0,'outcome '+c['target'])
 raw=(out/c['artifact']).read_bytes();require(sha(raw)==c['sha256'],'log hash');log=gzip.decompress(raw)
 require(re.findall(rb'Done (\d+) runs in (\d+) second',log)==[(str(c['completed_runs']).encode(),str(c['reported_seconds']).encode())],'observed completion')
 require(b'inline 8-bit counters' in log and ('INFO: Seed: '+str(c['seed'])).encode() in log,'instrumentation/seed')
 for kind in ['corpus','artifacts']:
  data=(out/c[kind+'_archive']).read_bytes();require(sha(data)==c[kind+'_sha256'],'retained '+kind)
  with tarfile.open(fileobj=io.BytesIO(data),mode='r:gz') as tar:members=[p for p in tar if p.isfile()]
  require(not members if kind=='artifacts' else all(p.size<=4096 for p in members),'retained '+kind+' scope')
 fixed={'fuzz_claim_verifier':'claim-verifier','fuzz_durable_recovery':'durable-recovery'}.get(c['target'])
 prefix='fuzz/seed-corpus/'+fixed+'/' if fixed else None
 expected={p.removeprefix(prefix):sha(b) for p,b in files.items() if prefix and p.startswith(prefix)}
 require(c['initial_corpus_sha256']==expected,'fixed input provenance')
summary=dict(candidate_commit=m['candidate_commit'],verified=True,targets=len(declared),completed_runs=sum(c['completed_runs'] for c in m['campaigns']),minimum_reported_seconds=min(c['reported_seconds'] for c in m['campaigns']),manifest_sha256=sha((out/'manifest.json').read_bytes()))
(out/'verification.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary))
