import gzip,hashlib,json,pathlib,re,subprocess
root=pathlib.Path.cwd();base=root/'docs/0.3/evidence/maintenance/string-traits'
def sha(data):return hashlib.sha256(data).hexdigest()
def require(ok,message):
 if not ok:raise RuntimeError(message)
names=['StateId','IntentId','EventId','CommitId','SnapshotId','ProfileId','Operation','KeyId','Status']
def corpus(directory,types,negative=False):
 m=json.loads((directory/'manifest.json').read_text());require(m['status']==('failed_or_interrupted' if negative else 'passed'),'corpus status')
 require(m['python_optimization']==1,'optimization guard untested')
 for c in m['checks']:
  require(c['exit_code']==0 and sha((directory/c['artifact']).read_bytes())==c['sha256'],'corpus operation/log drift')
 require(sha((directory/'fixture.rs').read_bytes())==m['fixture_sha256'],'fixture drift')
 expected={f'{name}:{kind}:{i}' for name in types for kind,count in [('string',1128),('json',10),('bcs',7)] for i in range(count)}
 traces=[]
 for i,lane in enumerate(m['lanes']):
  raw=gzip.decompress((directory/f'{i}-trace.log.gz').read_bytes());require(sha(raw)==lane['trace_sha256'],'trace drift');trace=json.loads(raw)
  require(len(trace)==len(expected)==lane['rows'] and {r['case'] for r in trace}==expected,'incomplete or duplicate corpus')
  require(sha((directory/f'{i}.lock').read_bytes())==lane['lock_sha256'],'lock drift')
  traces.append(trace)
 differences=sum(a!=b for trace in traces[1:] for a,b in zip(traces[0],trace));require(differences==m['different_rows']==(5608 if negative else 0),'trace comparison drift')
 return m
original=corpus(base/'20261116T143856Z-4178983',names)
intermediate=corpus(base/'20261116T143605Z-4081186',names[:-1])
corpus(base/'20261116T144017Z-41759',names,True)
a,b=original['lanes'][1:];require(a['lock_sha256']==b['lock_sha256'] and a['registry_features']==b['registry_features'],'unmatched compiled cost')
require((a['binary_bytes'],b['binary_bytes'],a['text_bytes'],b['text_bytes'])==(846160,846880,494054,494070),'compiled metrics drift')
driver=base/'driver-d6aa9f4';m=json.loads((driver/'manifest.json').read_text());require(m['status']=='passed','driver status');require([(c['label'],c['exit_code']) for c in m['checks']]==[('positive',0),('negative',1)],'checker top-level exits')
for c in m['checks']:
 require(c['passed'] and sha((driver/c['artifact']).read_bytes())==c['sha256'],'driver log drift');corpus((root/c['child_manifest']).parent,names,c['label']=='negative')
for name in ['qualification-9f80012','qualification-d6aa9f4']:
 directory=base/name;m=json.loads((directory/'manifest.json').read_text());require(m['status']=='passed','root status')
 for c in m['checks']:require(c['exit_code']==0 and sha((directory/c['artifact']).read_bytes())==c['sha256'],'root check/log')
 for label,count in [('finite-tests',897),('doctests',16)]:
  text=gzip.decompress((directory/(label+'.log.gz')).read_bytes()).decode();require(sum(map(int,re.findall(r'test result: ok\. (\d+) passed',text)))==count,'root test count')
owner=json.loads((base/'owner-audit.json').read_text());require(owner['status']=='passed' and owner['candidate']=='d6aa9f417d765545301261ea9fe8df5aca256b05' and sum(map(len,owner['checks'].values()))==45,'owner evidence')
metrics=json.loads((base/'metrics-final.json').read_text());require(metrics['totals']=={'tracked_bytes':1490324,'syntax_lines':16251,'lexical_tokens':118924,'branch_nodes':1941,'public_syntax_items':1458} and len(metrics['exact_duplicate_functions'])==4,'source metrics drift')
compat=root/'docs/0.3/evidence/compat/20261116T143925Z-4120/manifest.json';require(json.loads(compat.read_text())['status']=='passed','external consumer status')
subprocess.run(['git','diff','--exit-code','dea8a07','d6aa9f4','--','Cargo.toml','Cargo.lock','rust-toolchain.toml'],check=True,stdout=subprocess.PIPE)
(base/'verify-checkpoint.py').write_bytes(pathlib.Path(__file__).read_bytes())
files=sorted(p for p in base.rglob('*') if p.is_file() and p.name!='checkpoint.json')
(base/'checkpoint.json').write_text(json.dumps({'status':'passed_with_scoped_limits','candidate':'d6aa9f417d765545301261ea9fe8df5aca256b05','external_consumer_manifest':str(compat.relative_to(root)),'external_consumer_manifest_sha256':sha(compat.read_bytes()),'limitations':['syntax macro counts do not expand generated implementations','no latency/allocation improvement claim','full roadmap qualification remains open'],'artifacts_sha256':{str(p.relative_to(base)):sha(p.read_bytes()) for p in files}},indent=2)+'\n');print('verified',len(files),'artifacts')
