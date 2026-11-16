import gzip,json,subprocess,time
from pathlib import Path
root=Path.cwd();out=root/'docs/0.3/evidence/f2/batch-completion/parities-1e995ab';out.mkdir(parents=True,exist_ok=False)
(out/'runner.py').write_bytes(Path(__file__).read_bytes())
checks=[]
for fixture,revisions,negative in [('settlement',['c418e98','404dda5','1e995ab'],False),('executor',['c418e98','404dda5','1e995ab'],False),('proof_service',['c418e98','1e995ab'],False),('settlement',['c418e98','1e995ab'],True)]:
 command=['python3','scripts/check_proof_service_parity.py','--fixture',fixture]
 for rev in revisions:command+=['--revision',rev]
 if negative:command+=['--negative-control-commit-on-refusal']
 before=set((root/'docs/0.3/evidence').rglob('manifest.json'));start=time.monotonic()
 r=subprocess.run(command,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600)
 new=set((root/'docs/0.3/evidence').rglob('manifest.json'))-before;assert len(new)==1
 manifest=new.pop();m=json.loads(manifest.read_text())
 label=fixture+('-negative' if negative else '')
 (out/(label+'.log.gz')).write_bytes(gzip.compress(r.stdout,mtime=0))
 checks.append({'command':command,'exit_code':r.returncode,'seconds':time.monotonic()-start,'manifest':str(manifest.relative_to(root)),'status':m['status'],'different_rows':m.get('different_rows')})
 (out/'checks.json').write_text(json.dumps(checks,indent=2)+'\n');print(label,r.returncode,m.get('different_rows'),str(manifest.relative_to(root)),flush=True)
 assert r.returncode==(1 if negative else 0)
 assert m['status']==('failed_or_interrupted' if negative else 'passed')
 if negative:assert m['different_rows']==2556 and m['failure']=="AssertionError('error or port-order trace mismatch')"
