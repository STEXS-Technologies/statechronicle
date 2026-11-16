from pathlib import Path
import subprocess,os,json,time,hashlib
root=Path.cwd(); base=root/'target/release-audit/support';base.mkdir(exist_ok=True)
checks=[]
commands=[('conformance-tests',['cargo','test','--manifest-path','ci/conformance/Cargo.toml','--all-targets','--locked','--offline']),('conformance-clippy',['cargo','clippy','--manifest-path','ci/conformance/Cargo.toml','--all-targets','--locked','--offline','--','-D','warnings']),('conformance-doc',['cargo','doc','--manifest-path','ci/conformance/Cargo.toml','--no-deps','--locked','--offline']),('conformance-format',['cargo','fmt','--manifest-path','ci/conformance/Cargo.toml','--all','--','--check'])]
for crate in ['statechronicle-profile','statechronicle-portable']:
 commands.append((crate+'-baremetal',['cargo','check','-p',crate,'--target','riscv32imafc-unknown-none-elf','--locked','--offline']))
for scope,lock,manifest in [('root','Cargo.lock','Cargo.toml'),('conformance','ci/conformance/Cargo.lock','ci/conformance/Cargo.toml'),('native','tutorials/minimal-native-domain/Cargo.lock','tutorials/minimal-native-domain/Cargo.toml'),('durable','tutorials/durable-custom-ledger/Cargo.lock','tutorials/durable-custom-ledger/Cargo.toml')]:
 commands.extend([(scope+'-audit',['cargo','audit','--file',lock,'--json']),(scope+'-outdated',['cargo','outdated','--manifest-path',manifest,'--workspace','--format','json'])])
commands.append(('deny',['cargo','deny','check','advisories','licenses','bans','sources']))
for label,cmd in commands:
 env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/release-audit/support-build'),RUSTDOCFLAGS='-D warnings'); start=time.monotonic();r=subprocess.run(cmd,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,env=env)
 p=base/(label+'.log');p.write_bytes(r.stdout);okay=r.returncode==0
 if label.endswith('-audit') and okay:
  x=json.loads(r.stdout);okay=x['vulnerabilities']['count']==0 and not any(x.get('warnings',{}).values())
 if label.endswith('-outdated') and okay:
  # cargo-outdated emits one JSON object per workspace member.
  x=[json.loads(line) for line in r.stdout.decode().splitlines() if line.startswith('{')];okay=bool(x) and all(not y['dependencies'] for y in x)
 checks.append({'label':label,'command':cmd,'exit_code':r.returncode,'passed':okay,'seconds':time.monotonic()-start,'log':p.name,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()});(base/'manifest.json').write_text(json.dumps({'candidate_commit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'checks':checks},indent=2)+'\n');print(label,r.returncode,okay,flush=True)
 if not okay:raise SystemExit(1)
