import pathlib,json,hashlib,gzip,re,subprocess
root=pathlib.Path.cwd();base=root/'docs/0.3/evidence/maintenance/outbox-preflight'
def sha(b):return hashlib.sha256(b).hexdigest()
positive=base/'20261116T140328Z-3140629';negative=base/'20261116T140438Z-3173020';q=base/'qualification-52e8e6f';paired=base/'paired-52e8e6f';alloc=base/'allocations-52e8e6f'
for directory in [positive,negative,q,paired,alloc]:
 m=json.loads((directory/'manifest.json').read_text())
 expected='failed_or_interrupted' if directory==negative else 'passed'
 if m['status']!=expected:raise RuntimeError('unexpected status '+str(directory))
 if directory==negative and m['different_rows']!=552:raise RuntimeError('negative differences drift')
 for check in m['checks']:
  if check['exit_code']!=0:raise RuntimeError('build/runtime failure')
  if 'artifact' in check and sha((directory/check['artifact']).read_bytes())!=check['sha256']:raise RuntimeError('log drift')
 if directory==positive and (m['different_rows']!=0 or any(l['rows']!=5688 for l in m['lanes']) or m['python_optimization']!=1):raise RuntimeError('positive incomplete')
 if directory==negative and m['python_optimization']!=1:raise RuntimeError('negative guards not optimized')
for label,count in [('finite-tests',896),('doctests',16)]:
 log=gzip.decompress((q/(label+'.log.gz')).read_bytes()).decode()
 if sum(map(int,re.findall(r'test result: ok\. (\d+) passed',log)))!=count:raise RuntimeError('root count drift')
m=json.loads((paired/'manifest.json').read_text());labels=[c['label'] for c in m['checks'] if c['label'].startswith('cycle-')]
expected=[f'cycle-{cycle}-{lane}' for cycle in range(7) for lane in (['control','candidate'] if cycle%2==0 else ['candidate','control'])]
if labels!=expected:raise RuntimeError('timing did not alternate')
a,b=m['lanes']['control'],m['lanes']['candidate']
if a['lock_sha256']!=b['lock_sha256'] or a['registry_features']!=b['registry_features'] or a['fixture_sha256']!=b['fixture_sha256']:raise RuntimeError('unmatched timing')
subprocess.run(['git','diff','--exit-code','52e8e6f','f99c37d','--','crates'],check=True,stdout=subprocess.PIPE)
compat=root/'docs/0.3/evidence/compat/20261116T140542Z-3200915/manifest.json'
if json.loads(compat.read_text())['status']!='passed':raise RuntimeError('external consumer incomplete')
driver=base/'driver-f99c37d'
dm=json.loads((driver/'manifest.json').read_text())
if dm['status']!='passed' or [(c['label'],c['exit_code']) for c in dm['checks']]!=[('positive',0),('negative',1)]:raise RuntimeError('checker exit guards failed')
for c in dm['checks']:
 if not c['passed'] or sha((driver/c['artifact']).read_bytes())!=c['sha256']:raise RuntimeError('driver artifact drift')
 child=json.loads((root/c['child_manifest']).read_text())
 if child['different_rows']!=(0 if c['label']=='positive' else 552):raise RuntimeError('driver child result drift')
(base/'verify-checkpoint.py').write_bytes(pathlib.Path(__file__).read_bytes())
files=sorted(p for p in base.rglob('*') if p.is_file() and p.name!='checkpoint.json')
(base/'checkpoint.json').write_text(json.dumps({'status':'passed_with_scoped_limits','production_source':'52e8e6fa81167f94998b31ff0980c5549a8d8019','runner_source':'f99c37d3561c63d6fc4c7251d1b13fc365d0acd1','checks':['5688 complete unique cases under optimized Python','552 digest-bypass differences after successful builds/runtime','896 finite tests and 16 doctests','seven actual alternating timing orders and matched dependency/fixture inputs','frozen external consumer portable and bare metal','no library changes between production and runner checkpoints'],'artifacts_sha256':{str(p.relative_to(base)):sha(p.read_bytes()) for p in files}},indent=2)+'\n')
print('verified',len(files),'artifacts')
