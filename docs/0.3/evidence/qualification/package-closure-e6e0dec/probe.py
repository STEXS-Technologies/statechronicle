from pathlib import Path
import subprocess,tarfile,io,os,json
root=Path.cwd();base=root/'target/package-gate';source=base/'source';source.mkdir(exist_ok=True)
rev=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
archive=subprocess.check_output(['git','archive',rev,'Cargo.toml','Cargo.lock','rust-toolchain.toml','crates','fuzz','LICENSE-APACHE','LICENSE-MIT'])
tarfile.open(fileobj=io.BytesIO(archive)).extractall(source,filter='data')
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(base/'build'))
cmd=['cargo','package','--workspace','--exclude','statechronicle-fuzz','--locked','--offline','--no-verify']
with (base/'initial-package.log').open('w') as out:r=subprocess.run(cmd,cwd=source,env=env,stdout=out,stderr=subprocess.STDOUT)
print('package exit:',r.returncode,flush=True)
(base/'initial-manifest.json').write_text(json.dumps({'candidate_commit':rev,'command':cmd,'exit_code':r.returncode},indent=2)+'\n')
raise SystemExit(r.returncode)
