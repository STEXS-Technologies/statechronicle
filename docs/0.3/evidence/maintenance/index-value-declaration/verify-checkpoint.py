import gzip,hashlib,json,pathlib,re,subprocess
root=pathlib.Path.cwd();base=root/'docs/0.3/evidence/maintenance/index-value-declaration'
def sha(b):return hashlib.sha256(b).hexdigest()
def require(ok,message):
 if not ok:raise RuntimeError(message)
contract=base/'contract-a315713';m=json.loads((contract/'manifest.json').read_text());require(m['status']=='passed','contract status')
require([(c['label'],c['exit_code']) for c in m['checks']]==[('original',0),('control',0),('candidate',0),('negative-all-three-required',101)],'contract exits')
require(sha(gzip.decompress((contract/'fixture.rs.gz').read_bytes()))==m['fixture_sha256'],'fixture drift')
for c in m['checks']:
 data=(contract/c['artifact']).read_bytes();require(sha(data)==c['sha256'],'contract log drift')
 text=gzip.decompress(data).decode()
 if c['exit_code']==101:require('assertion `left == right` failed' in text and 'value_leg_declaration_presence_contract' in text,'negative missing runtime assertion')
 else:require('value_leg_declaration_presence_contract ... ok' in text,'contract test missing')
q=base/'qualification-a315713';m=json.loads((q/'manifest.json').read_text());require(m['status']=='passed','root status')
for c in m['checks']:require(c['exit_code']==0 and sha((q/c['artifact']).read_bytes())==c['sha256'],'root check/log')
for label,count in [('finite-tests',897),('doctests',16)]:
 text=gzip.decompress((q/(label+'.log.gz')).read_bytes()).decode();require(sum(map(int,re.findall(r'test result: ok\. (\d+) passed',text)))==count,'root test count')
compat=root/'docs/0.3/evidence/compat/20261116T141657Z-3511401/manifest.json';cm=json.loads(compat.read_text());require(cm['status']=='passed','consumer status')
metrics=json.loads((base/'metrics-a315713.json').read_text());require(metrics['totals']['lexical_tokens']==121053,'metrics drift')
# Retain exact old definitions: their equality is additional static evidence.
path='crates/statechronicle-index/src/build.rs';profile='crates/statechronicle-profiles/src/atomicity.rs';predicates={}
for label,revision,file,signature in [('original','c418e98',path,'fn declares_value_leg'),('control','c9a4605',path,'fn declares_value_leg'),('owner','a315713',profile,'pub fn declares_value_leg')]:
 text=subprocess.check_output(['git','show',revision+':'+file],text=True);a=text.index(signature);b=text.index('\n}',a)+2;definition=text[a:b];predicates[label]=definition
require(predicates['original']==predicates['control'],'old predicate drift');require(predicates['control']=='fn '+predicates['owner'].removeprefix('pub fn '),'owner differs')
(base/'old-predicates.json').write_text(json.dumps(predicates,indent=2)+'\n')
(base/'verify-checkpoint.py').write_bytes(pathlib.Path(__file__).read_bytes())
files=sorted(p for p in base.rglob('*') if p.is_file() and p.name!='checkpoint.json')
(base/'checkpoint.json').write_text(json.dumps({'status':'passed_with_scoped_limits','source':'a3157135199f7c86f74f8eb79cb7bad7b483d11c','external_consumer_manifest':str(compat.relative_to(root)),'external_consumer_manifest_sha256':sha(compat.read_bytes()),'artifacts_sha256':{str(p.relative_to(base)):sha(p.read_bytes()) for p in files}},indent=2)+'\n')
print('verified',len(files),'artifacts')
