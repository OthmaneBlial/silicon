#!/usr/bin/env python3
"""Run one renderer configuration at a time and retain measured output."""
import argparse, hashlib, json, platform, subprocess, time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--frames',type=int,default=30);p.add_argument('--output',default='output/benchmarks.json');a=p.parse_args()
subprocess.run(['cargo','build','--release','-p','silicon-cli'],check=True)
results=[]
for scene in ('textured_cube','shader_cube','spirv_cube','showcase','spirv_showcase','tile_stress','overdraw'):
    for backend,threads in [('scalar',1),('simd',1),('scalar',2),('scalar',4),('simd',4)]:
        command=['target/release/silicon','benchmark',scene,'--frames',str(a.frames),'--backend',backend,'--threads',str(threads)]
        run=subprocess.run(command,check=True,text=True,capture_output=True)
        print(run.stdout,flush=True);results.append({'scene':scene,'backend':backend,'threads':threads,'command':command,'stdout':run.stdout})
out=Path(a.output);out.parent.mkdir(parents=True,exist_ok=True);out.write_text(json.dumps({'timestamp':time.strftime('%Y-%m-%dT%H:%M:%S%z'),'binary_sha256':hashlib.sha256(Path('target/release/silicon').read_bytes()).hexdigest(),'git_head':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'worktree_dirty':bool(subprocess.check_output(['git','status','--porcelain'],text=True)),'platform':platform.platform(),'cpu':platform.processor(),'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'samples':results},indent=2)+'\n')
