from pathlib import Path
import tarfile,subprocess,os
root=Path.cwd();base=root/'target/package-gate';app=base/'unpacked';app.mkdir(exist_ok=True)
crates=sorted((base/'build/package').glob('*.crate'));assert len(crates)==13,len(crates)
for p in crates:tarfile.open(p).extractall(app,filter='data')
names=[p.name.removesuffix('.crate') for p in crates]
patches=['"'+name.rsplit('-',1)[0]+'" = { path = "'+name+'" }' for name in names]
(app/'Cargo.toml').write_text('[workspace]\nresolver = "3"\nmembers = '+str(names).replace("'",'"')+'\n\n[patch.crates-io]\n'+'\n'.join(patches)+'\n')
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(base/'unpacked-build'))
with (base/'initial-unpacked-tests.log').open('w') as out:r=subprocess.run(['cargo','test','-p','statechronicle','--all-targets','--offline'],cwd=app,env=env,stdout=out,stderr=subprocess.STDOUT)
print('unpacked test exit:',r.returncode,flush=True)
raise SystemExit(r.returncode)
