import pathlib,subprocess,json,gzip,hashlib,time
root=pathlib.Path.cwd();out=root/'docs/0.3/evidence/f1/protected-settlement/regression-before';out.mkdir(parents=True,exist_ok=False)
def sha(b):return hashlib.sha256(b).hexdigest()
fixture=root/'crates/statechronicle/tests/protected_settlement.rs';(out/'protected_settlement.rs').write_bytes(fixture.read_bytes());(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
command=['cargo','test','-p','statechronicle','--test','protected_settlement','--locked','--offline'];import os
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8');start=time.monotonic();r=subprocess.run(command,cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=120);data=r.stdout
(out/'regressions.log.gz').write_bytes(gzip.compress(data,mtime=0));old=pathlib.Path('/tmp/statechronicle-protected-settlement-before.log');(out/'setup-key-scope-error.log.gz').write_bytes(gzip.compress(old.read_bytes(),mtime=0))
checks={'cargo_exit_101':r.returncode==101,'valid_sale_refused_by_route':b'ValueLegSettleRouting' in data,'incomplete_bundle_single_accepted':b'incomplete bundle escaped: single=Ok(' in data,'incomplete_bundle_batch_accepted':b'batch=Ok(' in data,'two_tests_fail':b'0 passed; 2 failed;' in data}
m=dict(baseline_commit=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),fixture_sha256=sha(fixture.read_bytes()),command=command,environment={'RUSTC_WRAPPER':'','CARGO_BUILD_JOBS':'8'},exit_code=r.returncode,seconds=time.monotonic()-start,expected_failures=checks,status='expected_regressions_observed' if all(checks.values()) else 'failed_or_incomplete',limitations=['New uncommitted fixture applied to unchanged tracked production; first setup attempt had incorrectly unscoped actor keys and proves no product failure.','Source for the first key-scope setup attempt was not retained; its log is excluded from conclusions.'])
m['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in out.iterdir() if p.name!='manifest.json'};(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n');print(checks)
if not all(checks.values()):raise RuntimeError('did not reproduce intended regressions')
