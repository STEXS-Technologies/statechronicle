"""Verify queued terminal evidence after every timing and allocation stage ends."""
import gzip,hashlib,json,os,subprocess,time
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent
m=dict(pid=os.getpid(),status='waiting_for_complete_performance',checks=[])
def save():(out/'queued-verification.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
    while True:
        prior=json.loads((out/'complete-performance.json').read_text())
        if prior['status'] not in ['waiting_for_build_costs','running']:
            assert prior['status']=='passed',prior
            break
        assert Path('/proc/'+str(prior['pid'])).exists(),'performance orchestrator stopped'
        time.sleep(15)
    m['status']='running';save()
    for driver in ['verify_extended_fuzz.py','verify_performance.py','verify_build_costs.py']:
        command=['python3',str(out/driver)];start=time.monotonic();r=subprocess.run(command,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
        artifact=driver.removesuffix('.py')+'-verification.log.gz';data=gzip.compress(r.stdout,mtime=0);(out/artifact).write_bytes(data)
        m['checks'].append(dict(command=command,exit_code=r.returncode,seconds=time.monotonic()-start,artifact=artifact,sha256=hashlib.sha256(data).hexdigest()));save()
        assert r.returncode==0,driver
        print(driver,'passed',flush=True)
    m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:save()
print('Queued independent verification complete. Manual acceptance remains required.',flush=True)
