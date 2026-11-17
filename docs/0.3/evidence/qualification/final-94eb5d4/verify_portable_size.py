"""Verify portable source/build provenance and retained archive section arithmetic."""
import gzip,hashlib,io,json,subprocess,tarfile
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;base=out/'portable-size';m=json.loads((base/'manifest.json').read_text());sha=lambda b:hashlib.sha256(b).hexdigest()
assert m['status']=='passed' and m['runner_sha256']==sha((base/'runner.py').read_bytes())==sha((out/'measure_portable_size.py').read_bytes())
assert m['target']=='riscv32imafc-unknown-none-elf';logs={}
for c in m['checks']:
 assert c['exit_code']==0;data=(base/c['artifact']).read_bytes();assert sha(data)==c['sha256'];logs[c['label']]=gzip.decompress(data)
 if c['label'].endswith('-sections'):assert subprocess.check_output(c['command'],stderr=subprocess.STDOUT)==logs[c['label']]
for lane,item in m['lanes'].items():
 data=subprocess.check_output(['git','archive',item['revision'],'--',*m['archive_paths']]);assert sha(data)==item['archive_sha256']
 with tarfile.open(fileobj=io.BytesIO(data)) as handle:assert {f.name:sha(handle.extractfile(f).read()) for f in handle if f.isfile()}==item['source_sha256']
 consumer=Path(item['consumer_path']);assert {str(p.relative_to(consumer)):sha(p.read_bytes()) for p in consumer.rglob('*') if p.is_file()}==item['consumer_sha256']
 assert sha((consumer/'src/lib.rs').read_bytes())==m['fixture_sha256']['ci/compat-0.2/crates/portable/src/lib.rs']
 records=[json.loads(line) for line in logs[lane+'-build'].splitlines() if line.startswith(b'{')]
 assert records[-1]['reason']=='build-finished' and records[-1]['success'];artifacts=[r for r in records if r['reason']=='compiler-artifact'];assert all(not r['fresh'] for r in artifacts)
 for crate,a in item['artifacts'].items():
  assert any(r['target']['name']==crate and a['archive_path'] in r['filenames'] for r in artifacts)
  archive=Path(a['archive_path']);data=archive.read_bytes();assert sha(data)==a['archive_sha256'] and len(data)==a['archive_bytes']
  members=subprocess.check_output(['ar','t',str(archive)],text=True).splitlines();assert [v['member'] for v in a['objects']]==[n for n in members if n.endswith('.o')]
  sections={}
  for index,obj in enumerate(a['objects']):
   data=subprocess.check_output(['ar','p',str(archive),obj['member']]);assert sha(data)==obj['sha256'] and len(data)==obj['bytes']
   parsed={}
   for line in logs[lane+'-'+crate+'-'+str(index)+'-sections'].decode().splitlines():
    fields=line.split()
    if len(fields)==3 and fields[1].isdigit() and fields[0]!='Total':parsed[fields[0]]=int(fields[1]);sections[fields[0]]=sections.get(fields[0],0)+int(fields[1])
   assert parsed==obj['sections']
  assert sections==a['sections'];assert a['code_rodata_bytes']==sum(v for k,v in sections.items() if k.startswith(('.text','.rodata')))
for path,digest in m['fixture_sha256'].items():assert sha(subprocess.check_output(['git','show','e880a2a:'+path]))==digest
result=dict(status='passed',manifest_sha256=sha((base/'manifest.json').read_bytes()),scope='Matched release RISC-V library/object archive sizes and source/consumer/build provenance; not linked firmware.',measurements={lane:{crate:{k:a[k] for k in ['archive_bytes','code_rodata_bytes']} for crate,a in v['artifacts'].items()} for lane,v in m['lanes'].items()},limitations=m['limitations'])
(out/'portable-size-verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
