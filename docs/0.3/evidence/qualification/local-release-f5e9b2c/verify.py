import gzip,hashlib,io,json,pathlib,re,subprocess,sys,tarfile,tomllib
root=pathlib.Path.cwd();out=pathlib.Path(sys.argv[1]) if len(sys.argv)>1 else root/'docs/0.3/evidence/qualification/local-release-f5e9b2c';m=json.loads((out/'manifest.json').read_text())
def require(ok,message):
 if not ok:raise ValueError(message)
sha=lambda b:hashlib.sha256(b).hexdigest()
require(m['status']=='passed' and m['exit_code']==0 and m['source_unchanged'],'release status/source guard')
archive=subprocess.check_output(['git','archive',m['candidate_commit']])
with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
 inputs={p.name:sha(tar.extractfile(p).read()) for p in tar.getmembers() if p.isfile()}
 fuzz=tomllib.loads(tar.extractfile('fuzz/Cargo.toml').read().decode())
require(inputs==m['source_sha256'],'immutable Git source hash map')
for name,h in m['artifacts_sha256'].items():require(sha((out/name).read_bytes())==h,'artifact '+name)
log=gzip.decompress((out/'release.log.gz').read_bytes()).decode()
require(log.rstrip().endswith('release checks passed'),'driver completion')
require('advisories ok, bans ok, licenses ok, sources ok' in log,'deny stage')
locks=['Cargo.lock','ci/conformance/Cargo.lock','tutorials/minimal-native-domain/Cargo.lock','tutorials/durable-custom-ledger/Cargo.lock']
for lock in locks:require('Scanning '+lock+' for vulnerabilities' in log,'audit '+lock)
# Each actual fuzz launch has an instrumented libFuzzer seed/start and normal completion.
launches=re.findall(r'Running `[^`]*[/\\](fuzz_[a-z_]+) ',log)
require(sorted(launches)==sorted(b['name'] for b in fuzz['bin']),'exact fuzz manifest launches')
counts=[int(n) for n in re.findall(r'^Done (\d+) runs in ',log,re.M)]
require(len(counts)==len(fuzz['bin']) and all(n>=100 for n in counts),'normal fuzz completions')
require(len(re.findall(r'^INFO: Seed: \d+',log,re.M))==len(counts),'fuzz seeds')
require(len(re.findall(r'^INFO: Loaded \d+ modules',log,re.M))==len(counts),'instrumentation')
results=[int(n) for n in re.findall(r'test result: ok\. (\d+) passed;',log)]
# Isolated conformance (two test binaries), root finite targets, then root doctests.
# Keep exact observed individual result counts, rather than infer test coverage from the total.
nested=[]
for p in sorted((out/'nested').rglob('manifest.json')):
 value=json.loads(p.read_text())
 require(value['status']=='passed' and value['candidate_commit']==m['candidate_commit'],'nested status/revision '+str(p))
 for check in value['checks']:
  require(check['exit_code']==check.get('expected_exit_code',0),'nested exit '+check['label'])
  if 'status' in check:require(check['status']=='passed','nested check status')
  filename=check.get('artifact',check.get('log'))
  digest=check.get('sha256',check.get('log_sha256'))
  require(filename is not None and sha((p.parent/filename).read_bytes())==digest,'nested log '+check['label'])
 for filename,digest in value.get('artifacts_sha256',{}).items():require(sha((p.parent/filename).read_bytes())==digest,'nested artifact '+filename)
 nested.append({'path':str(p.relative_to(out)),'sha256':sha(p.read_bytes()),'manifest':value})
require(len(nested)==3,'two tutorials and native-contract manifests')
summary={'candidate_commit':m['candidate_commit'],'verified':True,'fuzz_targets':launches,'fuzz_completed_counts':counts,'rust_test_result_counts':results,'nested_manifests':[{'path':n['path'],'sha256':n['sha256']} for n in nested],'artifact_hashes_checked':len(m['artifacts_sha256']),'limitations':['Combined driver log plus strict shell exit; result counts retained individually.','Fuzz smoke only; no canonical ClaimVerifier/durable receipt fuzz campaign.']}
(out/'verification.json').write_text(json.dumps(summary,indent=2)+'\n')
print('verified',len(counts),'instrumented fuzz completions;',sum(results),'Rust tests across',len(results),'result lines;',len(nested),'nested manifests')
