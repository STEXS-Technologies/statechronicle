"""Run matched performance/build-cost work after live extended fuzzing ends."""
import gzip, hashlib, json, os
from pathlib import Path
import subprocess, time
root=Path.cwd();out=Path(__file__).resolve().parent
sha=lambda b:hashlib.sha256(b).hexdigest()
m=dict(candidate_commit=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),pid=os.getpid(),status='waiting_for_extended_fuzz',checks=[])
def save():(out/'after-fuzz.json').write_text(json.dumps(m,indent=2)+'\n')
def execute(label,command):
    started=time.monotonic();m['stage']=label;save()
    log=out/(label+'.log')
    with log.open('wb') as stream:
        p=subprocess.run(command,cwd=root,env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8'),stdout=stream,stderr=subprocess.STDOUT)
    raw=log.read_bytes();compressed=gzip.compress(raw,mtime=0);(out/(label+'.log.gz')).write_bytes(compressed);log.unlink()
    m['checks'].append(dict(label=label,command=command,exit_code=p.returncode,seconds=time.monotonic()-started,artifact=label+'.log.gz',sha256=sha(compressed)));save()
    if p.returncode:raise RuntimeError(label+' failed')
    print(label,'passed',flush=True)
    return raw.decode(errors='replace')
save()
try:
    fuzz=root/'docs/0.3/evidence/fuzz/extended-94eb5d4/manifest.json'
    while True:
        state=json.loads(fuzz.read_text())
        if state['status']!='running':
            if state['status']!='passed':raise RuntimeError('extended fuzz did not pass')
            break
        campaigns=state.get('campaigns',[])
        if campaigns and not any(Path('/proc/'+str(c['pid'])).exists() for c in campaigns):
            raise RuntimeError('no live fuzzers while campaign still marked running')
        time.sleep(15)
    m['status']='running';save()
    s=(root/'docs/0.3/evidence/performance/matrix-20261116T123514Z-3594158/runner.py').read_text()
    s=s.replace("candidate='a889f172a65bf203a0dde989d372dd111ae4b73e'",'candidate='+repr(m['candidate_commit']))
    # A unique build path avoids reusing compilation of earlier archive lanes.
    s=s.replace("str(root/'target/performance-matrix/build')","str(run/'build')")
    matrix=out/'matched_matrix.py';matrix.write_text(s)
    result=execute('matched-matrix',['python3',str(matrix)])
    matrix_path=next(line.strip() for line in reversed(result.splitlines()) if line.startswith('docs/0.3/evidence/performance/matrix-'))
    m['matrix_path']=matrix_path;save()
    for label, old in [('per-request','per-request-20261116T123934Z-3779170'),('allocations','allocations-20261116T124253Z-3908094')]:
        s=(root/'docs/0.3/evidence/performance'/old/'runner.py').read_text()
        s=s.replace("root/'docs/0.3/evidence/performance/matrix-20261116T123514Z-3594158'",'root/'+repr(matrix_path))
        if label=='per-request':
            s=s.replace("str(root/'target/performance-matrix/build')","str(run/'build')")
        p=out/(label+'_final.py');p.write_text(s)
        execute(label,['python3',str(p)])
    execute('canonical-performance',['python3',str(out/'canonical_performance.py'),'--measure'])
    execute('baseline-cold-build',['python3','scripts/refactor_costs.py','--revision','v0.2.0'])
    execute('candidate-cold-build',['python3','scripts/refactor_costs.py','--revision',m['candidate_commit']])
    m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:save()
print('All queued performance and build-cost stages passed.',flush=True)
