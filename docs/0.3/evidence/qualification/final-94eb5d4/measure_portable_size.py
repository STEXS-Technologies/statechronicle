"""Measure matched release RISC-V portable/profile/consumer archive sections."""
import gzip,hashlib,io,json,os,shutil,subprocess,tarfile,time,tomllib
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent/'portable-size';out.mkdir(exist_ok=False)
sha=lambda b:hashlib.sha256(b).hexdigest();paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz'];target='riscv32imafc-unknown-none-elf'
m=dict(status='running',compiler=subprocess.check_output(['rustc','-vV'],text=True),target=target,archive_paths=paths,lanes={},checks=[],limitations=['Release rlib and summed object sections, not a linked firmware flash/RAM bound.','No allocator/runtime/serializer is added to this frozen portable consumer.','Registry dependency refresh affects total code generation; the portable Rust implementation is unchanged.','Archives include metadata and unreferenced code; link-time reachability/LTO can change final application size.'])
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
def check(label,command,cwd,env):
 start=time.monotonic();r=subprocess.run(command,cwd=cwd,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);data=gzip.compress(r.stdout,mtime=0);artifact=label+'.log.gz';(out/artifact).write_bytes(data)
 m['checks'].append(dict(label=label,command=command,exit_code=r.returncode,seconds=time.monotonic()-start,artifact=artifact,sha256=sha(data)));save();assert r.returncode==0,label
 return r.stdout
(out/'runner.py').write_bytes(Path(__file__).read_bytes());m['runner_sha256']=sha(Path(__file__).read_bytes());save()
try:
 work=root/'target/final-portable-size'/str(os.getpid());work.mkdir(parents=True)
 fixture={p:subprocess.check_output(['git','show','HEAD:'+p]) for p in ['ci/compat-0.2/crates/portable/Cargo.toml','ci/compat-0.2/crates/portable/src/lib.rs']};m['fixture_sha256']={p:sha(b) for p,b in fixture.items()}
 for index,(lane,ref) in enumerate([('original','c418e98'),('candidate','e880a2a')]):
  revision=subprocess.check_output(['git','rev-parse',ref],text=True).strip();source=work/f'lane{index}'/'source';source.mkdir(parents=True)
  data=subprocess.check_output(['git','archive',revision,'--',*paths])
  with tarfile.open(fileobj=io.BytesIO(data)) as handle:handle.extractall(source,filter='data')
  source_hashes={str(p.relative_to(source)):sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()};consumer=work/f'lane{index}'/'consumer';(consumer/'src').mkdir(parents=True)
  cargo=fixture['ci/compat-0.2/crates/portable/Cargo.toml'].decode().replace('../../../../crates/',str(source/'crates')+'/')
  (consumer/'Cargo.toml').write_text(cargo);(consumer/'src/lib.rs').write_bytes(fixture['ci/compat-0.2/crates/portable/src/lib.rs']);shutil.copyfile(source/'Cargo.lock',consumer/'Cargo.lock')
  env=dict(os.environ,RUSTC_WRAPPER='',CARGO_TARGET_DIR=str(work/f'lane{index}'/'target'),CARGO_BUILD_JOBS='8',CARGO_PROFILE_RELEASE_CODEGEN_UNITS='1',CARGO_PROFILE_RELEASE_LTO='false',CARGO_PROFILE_RELEASE_DEBUG='0')
  for k in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTDOCFLAGS']:env.pop(k,None)
  check(lane+'-resolve',['cargo','metadata','--offline','--format-version','1','--filter-platform',target],consumer,env)
  def registry(path):return {(p['name'],p['version'],p.get('source'),p.get('checksum')) for p in tomllib.loads(path.read_text())['package'] if p.get('source')}
  assert not registry(consumer/'Cargo.lock')-registry(source/'Cargo.lock')
  build_output=check(lane+'-build',['cargo','build','--release','--target',target,'--locked','--offline','--message-format=json'],consumer,env)
  artifacts={}
  for crate in ['statechronicle_profile','statechronicle_portable','statechronicle_portable_consumer']:
   records=[json.loads(line) for line in build_output.splitlines() if line.startswith(b'{')];matches=[Path(name) for record in records if record.get('reason')=='compiler-artifact' and record['target']['name']==crate for name in record['filenames'] if name.endswith('.rlib')];assert len(matches)==1
   archive=matches[0];members=subprocess.check_output(['ar','t',str(archive)],text=True).splitlines();sections={};objects=[]
   for member in members:
    if not member.endswith('.o'):continue
    raw=subprocess.check_output(['ar','p',str(archive),member]);obj=work/f'lane{index}'/member;obj.write_bytes(raw)
    text=check(lane+'-'+crate+'-'+str(len(objects))+'-sections',['size','-A',str(obj)],root,env).decode()
    parsed={}
    for line in text.splitlines():
     fields=line.split()
     if len(fields)==3 and fields[1].isdigit() and fields[0]!='Total':parsed[fields[0]]=int(fields[1]);sections[fields[0]]=sections.get(fields[0],0)+int(fields[1])
    objects.append(dict(member=member,sha256=sha(raw),bytes=len(raw),sections=parsed))
   artifacts[crate]=dict(archive_sha256=sha(archive.read_bytes()),archive_bytes=archive.stat().st_size,archive_path=str(archive),objects=objects,sections=sections,code_rodata_bytes=sum(v for k,v in sections.items() if k.startswith(('.text','.rodata'))))
  assert {str(p.relative_to(source)):sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()}==source_hashes
  m['lanes'][lane]=dict(revision=revision,archive_sha256=sha(data),source_sha256=source_hashes,consumer_path=str(consumer),consumer_sha256={str(p.relative_to(consumer)):sha(p.read_bytes()) for p in consumer.rglob('*') if p.is_file()},environment={k:env[k] for k in env if k.startswith(('CARGO_PROFILE','CARGO_TARGET','RUSTC_WRAPPER'))},artifacts=artifacts);save();print(lane,'portable size measured',flush=True)
 m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:save()
