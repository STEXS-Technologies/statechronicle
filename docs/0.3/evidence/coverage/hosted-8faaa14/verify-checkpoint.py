import gzip,hashlib,io,json,pathlib,re,subprocess,tarfile
root=pathlib.Path.cwd();out=root/'docs/0.3/evidence/coverage/hosted-8faaa14'
def sha(b):return hashlib.sha256(b).hexdigest()
def require(ok,message):
 if not ok:raise RuntimeError(message)
m=json.loads((out/'manifest.json').read_text());require(m['status']=='passed','coverage status');require([(c['label'],c['exit_code']) for c in m['checks']]==[('collection',0),('ratchet',0)],'coverage exits')
for c in m['checks']:require(sha((out/c['artifact']).read_bytes())==c['sha256'],'raw log drift')
text=gzip.decompress((out/'collection.log.gz').read_bytes()).decode();require('Ran 4 tests' in text and sum(map(int,re.findall(r'test result: ok\. (\d+) passed',text)))==897,'test coverage campaign count')
for name,count,covered in [('coverage-full.json',20365,18913),('coverage.json',16410,15493)]:
 data=(out/name).read_bytes();require(sha(data)==m['reports'][name]['sha256'],'report drift');report=json.loads(data)
 require(sum(d['totals']['lines']['count'] for d in report['data'])==count and sum(d['totals']['lines']['covered'] for d in report['data'])==covered,'report totals')
 require(sum(f['summary']['lines']['count'] for d in report['data'] for f in d['files'])==count and sum(f['summary']['lines']['covered'] for d in report['data'] for f in d['files'])==covered,'file totals')
 require(abs(covered/count*100-m['reports'][name]['percent'])<1e-10,'report arithmetic')
full=json.loads((out/'coverage-full.json').read_text());filtered=json.loads((out/'coverage.json').read_text());require(len(full['data'][0]['files'])-len(filtered['data'][0]['files'])==13,'exclusion count')
macro=[f for f in full['data'][0]['files'] if f['filename'].endswith('/validated_string.rs')];require(len(macro)==1 and macro[0]['summary']['lines']['count']==macro[0]['summary']['lines']['covered']==21,'macro attribution')
negative=json.loads((out/'negative-manifest.json').read_text());require(negative['status']=='passed_expected_rejection' and negative['exit_code']==1 and sha((out/negative['artifact']).read_bytes())==negative['sha256'],'negative ratchet')
require(b'coverage regression:' in gzip.decompress((out/negative['artifact']).read_bytes()),'missing refusal diagnostic')
for file in ['scripts/run_coverage_json.sh','scripts/check_coverage_ratchet.sh','scripts/test_coverage_ratchet.py','ci/coverage-baseline.json']:
 require(subprocess.check_output(['git','show','4b133d9:'+file])==subprocess.check_output(['git','show',m['candidate']+':'+file])==(out/pathlib.Path(file).name).read_bytes(),'changed historical ratchet policy')
archive=subprocess.check_output(['git','archive',m['candidate'],'--',*m['archive_paths']]);require(sha(archive)==m['archive_sha256'],'archive drift')
with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
 actual={entry.name:sha(tar.extractfile(entry).read()) for entry in tar.getmembers() if entry.isfile()}
require(actual==m['source_sha256'],'source hash drift')
(out/'verify-checkpoint.py').write_bytes(pathlib.Path(__file__).read_bytes());files=sorted(p for p in out.iterdir() if p.is_file() and p.name!='checkpoint.json');(out/'checkpoint.json').write_text(json.dumps({'status':'passed_with_scoped_limits','candidate':m['candidate'],'artifacts_sha256':{p.name:sha(p.read_bytes()) for p in files}},indent=2)+'\n');print('verified',len(files),'artifacts')
