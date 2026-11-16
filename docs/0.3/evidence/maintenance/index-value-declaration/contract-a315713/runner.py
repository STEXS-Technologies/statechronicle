import gzip,hashlib,io,json,os,pathlib,subprocess,tarfile,tempfile,time
root=pathlib.Path.cwd();candidate=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
base=root/'docs/0.3/evidence/maintenance/index-value-declaration';out=base/('contract-'+candidate[:7]);out.mkdir()
def sha(b):return hashlib.sha256(b).hexdigest()
path='crates/statechronicle-index/src/build.rs';source=(root/path).read_text();start=source.index('    // Preserve the original presence-only contract');end=source.index('    #[test]\n    fn lock_opens_a_record',start);fixture=source[start:end]
(out/'fixture.rs.gz').write_bytes(gzip.compress(fixture.encode(),mtime=0));(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
m={'candidate':candidate,'fixture_sha256':sha(fixture.encode()),'cases_per_test':512,'checks':[],'status':'running'}
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 for label,rev in [('original','c418e98b91720188158c531cd5c2c4b8ab71cbcf'),('control','c9a4605e460531d1d2b56a5b1adb4fe93ddfe55b'),('candidate',candidate),('negative-all-three-required',candidate)]:
  archive=subprocess.check_output(['git','archive',rev,'--','Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz','.cargo'])
  with tempfile.TemporaryDirectory(prefix='statechronicle-index-contract-') as temporary:
   work=pathlib.Path(temporary)
   with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(work,filter='data')
   p=work/path;s=p.read_text()
   if label in ['original','control']:
    at=s.index('    #[test]\n    fn lock_opens_a_record');s=s[:at]+fixture+s[at:];p.write_text(s)
   if label.startswith('negative'):
    p=work/'crates/statechronicle-profiles/src/atomicity.rs';s=p.read_text();a=s.index('pub fn declares_value_leg');b=s.index('\n}',a);old=s[a:b];new=old.replace('||','&&')
    if old==new:raise RuntimeError('mutation missing')
    p.write_text(s[:a]+new+s[b:]);(out/'negative.patch').write_text(old+'\nREPLACED WITH\n'+new+'\n')
   cmd=['cargo','test','-p','statechronicle-index','--all-targets','--all-features','--locked','--offline']
   if label.startswith('negative'):cmd+=['value_leg_declaration_presence_contract']
   env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/index-contract'))
   before=time.monotonic();r=subprocess.run(cmd,cwd=work,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600)
   name=label+'.log.gz';data=gzip.compress(r.stdout,mtime=0);(out/name).write_bytes(data)
   expected=101 if label.startswith('negative') else 0
   c={'label':label,'revision':rev,'archive_sha256':sha(archive),'lock_sha256':sha((work/'Cargo.lock').read_bytes()),'command':cmd,'exit_code':r.returncode,'expected_exit_code':expected,'artifact':name,'sha256':sha(data),'seconds':time.monotonic()-before};m['checks'].append(c);save();print(label,r.returncode,flush=True)
   if r.returncode!=expected:raise RuntimeError('unexpected exit '+label)
   if label.startswith('negative') and b'assertion `left == right` failed' not in r.stdout:raise RuntimeError('negative not runtime assertion')
 m['status']='passed'
except BaseException as error:m['status']='failed';m['error']=repr(error);raise
finally:save()
