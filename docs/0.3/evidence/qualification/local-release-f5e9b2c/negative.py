import json,pathlib,shutil,subprocess,sys,tempfile
root=pathlib.Path('docs/0.3/evidence/qualification/local-release-f5e9b2c').resolve()
with tempfile.TemporaryDirectory(prefix='statechronicle-release-verifier-negative-') as temp:
 target=pathlib.Path(temp)/'evidence';shutil.copytree(root,target)
 manifest=next((target/'nested/tutorials').rglob('manifest.json'))
 value=json.loads(manifest.read_text());value['checks'][0]['exit_code']=501;manifest.write_text(json.dumps(value))
 result=subprocess.run([sys.executable,str(root/'verify.py'),str(target)],capture_output=True,text=True)
 if result.returncode==0 or 'nested exit metadata' not in result.stderr:raise RuntimeError('bad nested exit was not rejected for intended reason')
 (root/'negative-control.json').write_text(json.dumps({'mutation':'copied tutorial metadata exit changed from 0 to 501','command':[sys.executable,str(root/'verify.py'),'<temporary-evidence-copy>'],'exit_code':result.returncode,'expected_diagnostic':'nested exit metadata','diagnostic':result.stderr},indent=2)+'\n')
 print('nested exit negative control rejected')
