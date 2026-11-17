"""Recheck retained full-scope qualification evidence; never rerun measurements."""
import gzip,hashlib,json,subprocess,time
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent
sha=lambda b:hashlib.sha256(b).hexdigest()
m=dict(status='running',head=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),checks=[],scope='Independent retained-evidence recheck. Manual requirement and regression acceptance remain separate.')
def save():(out/'final-gate-recheck.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
    scopes=list(json.loads((out/'candidate-equivalence.json').read_text())['byte_identical_scopes'])
    for revision in ['94eb5d4','0799afe','e880a2a']:
        subprocess.run(['git','diff','--exit-code',revision,'--',*scopes],check=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
    m['unchanged_qualified_scopes']=scopes
    for name in ['complete-performance.json','queued-verification.json','after-history.json']:
        assert json.loads((out/name).read_text())['status']=='passed',name
    commands=[['python3',str(root/'docs/0.3/evidence/qualification/local-release-f5e9b2c/verify.py'),str(root/'docs/0.3/evidence/qualification/local-release-94eb5d4')]]
    commands += [['python3',str(out/name)] for name in ['verify_baseline.py','verify_api.py','review_api.py','verify_packages.py','verify_compat_coverage.py','verify_controls.py','verify_semantic_parity.py','verify_properties.py','verify_extended_fuzz.py','verify_metrics.py','verify_performance.py','verify_build_costs.py','verify_portable_size.py','verify_history_performance.py']]
    commands += [['python3',str(out/'verify_fuzz.py'),str(root/'docs/0.3/evidence/fuzz'/name)] for name in ['claim-final-0799afe','durable-final-0799afe']]
    for index,command in enumerate(commands):
        start=time.monotonic();r=subprocess.run(command,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
        data=gzip.compress(r.stdout,mtime=0);artifact=f'final-recheck-{index:02d}-{Path(command[1]).stem}.log.gz';(out/artifact).write_bytes(data)
        m['checks'].append(dict(command=command,exit_code=r.returncode,seconds=time.monotonic()-start,artifact=artifact,sha256=sha(data),verifier_sha256=sha(Path(command[1]).read_bytes())));save()
        assert r.returncode==0,artifact
        print(Path(command[1]).name,'passed',flush=True)
    m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:save()
