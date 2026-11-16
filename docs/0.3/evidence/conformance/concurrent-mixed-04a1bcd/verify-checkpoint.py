import gzip,hashlib,io,json,pathlib,re,subprocess,tarfile
root=pathlib.Path.cwd();base=root/'docs/0.3/evidence/conformance/concurrent-mixed-04a1bcd'
def sha(b):return hashlib.sha256(b).hexdigest()
def require(ok,message):
 if not ok:raise RuntimeError(message)
m=json.loads((base/'manifest.json').read_text());negative=base/'deadlock-control';n=json.loads((negative/'manifest.json').read_text())
require(m['status']=='failed_or_interrupted','retain initial failed qualification status')
require([c['label'] for c in m['checks']]==['fmt','all-tests','doctests','strict-clippy','strict-rustdoc','negative-harness-deadlock'],'check set')
require(all(c['exit_code']==0 for c in m['checks']),'initial check exits')
require(m['checks'][-1]['expected_exit_code']==101,'initial ineffective mutation must remain rejected')
for c in m['checks']:require(sha((base/c['artifact']).read_bytes())==c['sha256'],'campaign log drift')
text=gzip.decompress((base/'all-tests.log.gz').read_bytes()).decode()
require(sum(map(int,re.findall(r'test result: ok\. (\d+) passed',text)))==26,'test count')
for seed,width in [(0,15),(1,13),(7,16),(42,15),(7331,14),(65537,15),(18446744073709551614,15),(18446744073709551615,16)]:
 require(f'mixed concurrent seed={seed} waves={width} widths=' in text,'missing seed/schedule')
for marker in ['EagerEvents: mixed.concurrent.staged_effects','LoseSinglesOnBatch: lookup.original','LoseBatchesOnSingle: mixed.membership','AppendProjections: mixed.concurrent.visibility']:
 require('mixed concurrent negative '+marker in text,'missing negative oracle')
for test in ['seeded_concurrent_mixed_histories_preserve_each_tenant_and_original_identity','seeded_concurrent_mixed_histories_detect_broken_backends','concurrent_history_allows_a_serialized_writer_without_staging_deadlock','concurrent_history_setup_refuses_before_destructive_reset']:
 require('test '+test+' ...' in text and re.search(r'test '+test+r' \.\.\..*?\bok\b',text,re.S),'missing positive test')
require(n['status']=='passed_expected_rejection' and n['exit_code']==101,'corrected negative exit')
require(sha((negative/n['artifact']).read_bytes())==n['sha256'],'negative log drift')
text=gzip.decompress((negative/n['artifact']).read_bytes()).decode();require('Elapsed(())' in text and 'concurrent_history_allows_a_serialized_writer_without_staging_deadlock ... FAILED' in text,'negative runtime failure')
require(m['candidate']==n['candidate'] and m['archive_sha256']==n['archive_sha256'] and m['source_sha256']==n['source_sha256'],'unmatched negative source')
archive=subprocess.check_output(['git','archive',m['candidate'],'--',*m['archive_paths']]);require(sha(archive)==m['archive_sha256'],'archive drift')
with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
 actual={member.name:sha(tar.extractfile(member).read()) for member in tar.getmembers() if member.isfile()}
require(actual==m['source_sha256'],'source hash drift')
subprocess.run(['git','diff','--exit-code','65e9af7',m['candidate'],'--','crates','Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml'],check=True,stdout=subprocess.PIPE)
(base/'verify-checkpoint.py').write_bytes(pathlib.Path(__file__).read_bytes())
files=sorted(p for p in base.rglob('*') if p.is_file() and p.name!='checkpoint.json')
(base/'checkpoint.json').write_text(json.dumps({'status':'passed_with_scoped_limits','candidate':m['candidate'],'qualification':'first five immutable-archive checks passed; initial ineffective negative rejected; corrected runtime deadlock control rejected as expected','limitations':['logical async reference only','no complete F5/F8 or physical backend qualification'],'artifacts_sha256':{str(p.relative_to(base)):sha(p.read_bytes()) for p in files}},indent=2)+'\n');print('verified',len(files),'artifacts')
